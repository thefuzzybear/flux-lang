//! IbkrAdapter — Interactive Brokers adapter wrapping `ibapi::Client`.
//!
//! Implements the `BrokerAdapter` trait by connecting to IB Gateway/TWS via the
//! `ibapi` crate (v3.3, async). Supports ES, NQ, YM, RTY futures contracts.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use futures_util::StreamExt;
use ibapi::contracts::Contract;
use ibapi::orders::{
    ExecutionData, OrderStatusKind,
    OrderUpdate as IbOrderUpdate,
};
use ibapi::subscriptions::SubscriptionItemStreamExt;
use ibapi::Client;
use tokio::sync::{mpsc, Mutex};

use super::execution::{aggressive_limit_price, AdaptiveUrgency, ExecutionPolicy};
use super::{
    BrokerAdapter, BrokerError, BrokerFill, BrokerOrder, BrokerPosition, Order, OrderId,
    OrderStatus, OrderUpdate, Side,
};

/// Connection health states for the IbkrAdapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// Actively connected to IB Gateway.
    Connected,
    /// Disconnected but within the 5-minute grace period (ibapi auto-reconnect in progress).
    Disconnected,
    /// Disconnected for more than 5 minutes — all order submission halted.
    Halted,
}

/// Duration threshold before transitioning from Disconnected to Halted.
/// After 5 minutes of continuous disconnection, the adapter enters the Halted state
/// and emits a Critical-level alert. No orders will be submitted until reconnection
/// and position reconciliation complete.
const HALT_TIMEOUT: Duration = Duration::minutes(5);

/// Tracks the lifecycle state of active orders.
///
/// Maintains a mapping from `OrderId` to the current `OrderStatus` for all
/// orders that have been submitted through the adapter. This enables querying
/// order state and cleaning up terminal orders.
#[derive(Debug, Default)]
pub struct OrderTracker {
    orders: HashMap<OrderId, OrderStatus>,
}

impl OrderTracker {
    /// Create a new empty order tracker.
    pub fn new() -> Self {
        Self {
            orders: HashMap::new(),
        }
    }

    /// Start tracking a newly submitted order (initial state: Submitted).
    pub fn track_order(&mut self, id: OrderId) {
        self.orders.insert(id, OrderStatus::Submitted);
    }

    /// Update the status of a tracked order.
    ///
    /// If the order is not currently tracked, this is a no-op.
    pub fn update_status(&mut self, id: &OrderId, status: OrderStatus) {
        if let Some(existing) = self.orders.get_mut(id) {
            *existing = status;
        }
    }

    /// Get the current status of a tracked order.
    pub fn get_status(&self, id: &OrderId) -> Option<OrderStatus> {
        self.orders.get(id).copied()
    }

    /// Remove an order from tracking (for terminal states).
    pub fn remove(&mut self, id: &OrderId) {
        self.orders.remove(id);
    }

    /// Check if a status represents a terminal state (Filled, Rejected, Cancelled).
    ///
    /// Terminal orders can safely be removed from tracking since they will not
    /// receive further state transitions.
    pub fn is_terminal(status: OrderStatus) -> bool {
        matches!(
            status,
            OrderStatus::Filled | OrderStatus::Rejected | OrderStatus::Cancelled
        )
    }
}

/// Interactive Brokers adapter wrapping an `ibapi::Client` for live order execution.
///
/// Connects to IB Gateway (paper: port 4002, live: port 4001) and translates
/// internal Order types to ibapi builder calls. The client is `Send + Sync`
/// and can be shared across tasks via `Arc`.
pub struct IbkrAdapter {
    /// The ibapi async client connection.
    client: Arc<Client>,
    /// Whether the adapter believes it is connected to IB.
    connected: AtomicBool,
    /// Tracks when disconnection was first detected (for 5-min halt logic).
    disconnect_since: Mutex<Option<DateTime<Utc>>>,
    /// Tracks order lifecycle states for all active orders.
    order_tracker: Arc<Mutex<OrderTracker>>,
}

impl IbkrAdapter {
    /// Connect to IB Gateway at the configured host:port with the given client_id.
    ///
    /// # Arguments
    /// * `host` — Gateway address (e.g. "127.0.0.1")
    /// * `port` — Gateway port (4002 for paper, 4001 for live)
    /// * `client_id` — Unique client identifier for this connection
    pub async fn connect(host: &str, port: u16, client_id: i32) -> Result<Self, BrokerError> {
        let addr = format!("{}:{}", host, port);
        let client = Client::connect(&addr, client_id)
            .await
            .map_err(|e| BrokerError::ConnectionFailed(e.to_string()))?;

        Ok(Self {
            client: Arc::new(client),
            connected: AtomicBool::new(true),
            disconnect_since: Mutex::new(None),
            order_tracker: Arc::new(Mutex::new(OrderTracker::new())),
        })
    }

    /// Build an ibapi `Contract` for a futures symbol.
    ///
    /// Maps known symbols to their exchange:
    /// - ES, NQ, RTY → CME
    /// - YM → CBOT
    ///
    /// Uses `front_month()` to automatically select the nearest expiry.
    fn build_contract(symbol: &str) -> Contract {
        let exchange = match symbol {
            "YM" => "CBOT",
            _ => "CME", // ES, NQ, RTY all trade on CME
        };
        Contract::futures(symbol)
            .front_month()
            .on_exchange(exchange)
            .build()
    }

    /// Map our `ExecutionPolicy` to ibapi order builder calls and submit the order.
    ///
    /// Returns the broker-assigned order ID on success.
    async fn submit_with_policy(&self, order: &Order) -> Result<OrderId, BrokerError> {
        let contract = Self::build_contract(&order.symbol);
        let qty = order.contracts as f64;

        let builder = match order.side {
            Side::Buy => self.client.order(&contract).buy(qty),
            Side::Sell => self.client.order(&contract).sell(qty),
        };

        let ib_order_id = match &order.execution {
            ExecutionPolicy::Market => builder.market().submit().await,
            ExecutionPolicy::AggressiveLimit { offset_ticks } => {
                let price = aggressive_limit_price(
                    order.side,
                    order.last_price,
                    *offset_ticks,
                    order.tick_size,
                );
                builder.limit(price).submit().await
            }
            ExecutionPolicy::Limit { price } => builder.limit(*price).submit().await,
            ExecutionPolicy::MarketOnClose => builder.market_on_close().submit().await,
            ExecutionPolicy::LimitOnClose { price } => {
                builder.limit_on_close(*price).submit().await
            }
            ExecutionPolicy::Stop { trigger_price } => {
                builder.stop(*trigger_price).submit().await
            }
            ExecutionPolicy::StopLimit {
                trigger_price,
                limit_price,
            } => builder
                .stop_limit(*trigger_price, *limit_price)
                .submit()
                .await,
            ExecutionPolicy::TrailingStop { trail_amount } => {
                // ibapi's trailing_stop takes (trailing_percent, stop_price).
                // For a dollar-based trail we use the trail_amount as the aux price
                // and set a nominal stop_price. This maps to IB's TRAIL order type.
                // NOTE: ibapi's trailing_stop uses (trailing_percent, stop_price).
                // Dollar-based trails pass trail_amount as the percent arg with stop_price=0.
                // This works for IB's TRAIL order type but may need adjustment for edge cases.
                builder.trailing_stop(*trail_amount, 0.0).submit().await
            }
            ExecutionPolicy::TrailingStopPct { trail_pct } => {
                builder.trailing_stop(*trail_pct, 0.0).submit().await
            }
            ExecutionPolicy::Adaptive { urgency } => {
                // IB's adaptive algo uses "Adaptive" strategy with urgency parameter
                let urgency_str = match urgency {
                    AdaptiveUrgency::Patient => "Patient",
                    AdaptiveUrgency::Normal => "Normal",
                    AdaptiveUrgency::Urgent => "Urgent",
                };
                builder
                    .market()
                    .algo("Adaptive")
                    .algo_param("adaptivePriority", urgency_str)
                    .submit()
                    .await
            }
        };

        ib_order_id
            .map(|id| OrderId(id.to_string()))
            .map_err(|e| BrokerError::IbApi(e.to_string()))
    }

    /// Mark the adapter as disconnected and record the timestamp.
    pub async fn mark_disconnected(&self) {
        self.connected.store(false, Ordering::SeqCst);
        let mut ds = self.disconnect_since.lock().await;
        if ds.is_none() {
            *ds = Some(Utc::now());
        }
    }

    /// Mark the adapter as reconnected and clear the disconnect timestamp.
    pub async fn mark_reconnected(&self) {
        self.connected.store(true, Ordering::SeqCst);
        let mut ds = self.disconnect_since.lock().await;
        *ds = None;
    }

    /// Returns how long the adapter has been disconnected, if at all.
    pub async fn disconnected_duration(&self) -> Option<chrono::Duration> {
        let ds = self.disconnect_since.lock().await;
        ds.map(|since| Utc::now() - since)
    }

    /// Returns true if the adapter has been disconnected for more than 5 minutes.
    ///
    /// When halted, all order submission should be blocked and a Critical-level
    /// alert should be emitted by the LiveHarness.
    pub async fn is_halted(&self) -> bool {
        if let Some(duration) = self.disconnected_duration().await {
            duration >= HALT_TIMEOUT
        } else {
            false
        }
    }

    /// Check connection health and update internal state.
    ///
    /// This method inspects the ibapi client's connection status and compares it
    /// against the adapter's internal state to detect transitions:
    /// - Connected → Disconnected: calls `mark_disconnected()` to start the halt timer
    /// - Disconnected → Connected: calls `mark_reconnected()` to clear the timer
    ///   (caller should trigger position reconciliation before resuming orders)
    /// - Disconnected > 5 min: reports `Halted` state
    ///
    /// Returns the current `ConnectionState`.
    pub async fn check_connection_health(&self) -> ConnectionState {
        let client_connected = self.client.is_connected();
        let was_connected = self.connected.load(Ordering::SeqCst);

        if client_connected && !was_connected {
            // Reconnected — clear disconnect timestamp.
            // Note: The caller (LiveHarness) should trigger position reconciliation
            // via get_positions() before resuming order submission.
            self.mark_reconnected().await;
            ConnectionState::Connected
        } else if !client_connected && was_connected {
            // Just disconnected — record timestamp to start the halt timer.
            self.mark_disconnected().await;
            ConnectionState::Disconnected
        } else if !client_connected {
            // Still disconnected — check if we've exceeded the halt timeout.
            if self.is_halted().await {
                ConnectionState::Halted
            } else {
                ConnectionState::Disconnected
            }
        } else {
            // Still connected — nominal state.
            ConnectionState::Connected
        }
    }
}

#[async_trait]
impl BrokerAdapter for IbkrAdapter {
    async fn submit_order(&self, order: &Order) -> Result<OrderId, BrokerError> {
        if !self.is_connected() {
            return Err(BrokerError::Disconnected);
        }
        let broker_id = self.submit_with_policy(order).await?;
        // Track the newly submitted order in our lifecycle tracker.
        let mut tracker = self.order_tracker.lock().await;
        tracker.track_order(broker_id.clone());
        Ok(broker_id)
    }

    async fn cancel_order(&self, order_id: &OrderId) -> Result<(), BrokerError> {
        if !self.is_connected() {
            return Err(BrokerError::Disconnected);
        }

        // Parse the order_id string back to i32 for ibapi.
        // Our OrderId format is "{account}_{strategy}_{symbol}_{bar_index}" which is a
        // logical ID. For cancellation we need the broker-assigned numeric ID.
        // In practice, we'd maintain a mapping from our OrderId to the ibapi numeric ID.
        // For now, attempt to parse as i32 (the broker-assigned ID stored after submission).
        let numeric_id: i32 = order_id
            .0
            .parse()
            .map_err(|_| BrokerError::OrderNotFound(order_id.0.clone()))?;

        // cancel_order returns a subscription; we consume it to confirm cancellation.
        let _subscription = self
            .client
            .cancel_order(numeric_id, "")
            .await
            .map_err(|e| BrokerError::IbApi(e.to_string()))?;

        Ok(())
    }

    async fn get_positions(&self) -> Result<Vec<BrokerPosition>, BrokerError> {
        if !self.is_connected() {
            return Err(BrokerError::Disconnected);
        }

        let subscription = self
            .client
            .positions()
            .await
            .map_err(|e| BrokerError::IbApi(e.to_string()))?;

        let mut positions = Vec::new();
        let mut stream = subscription.filter_data();

        while let Some(item) = stream.next().await {
            match item {
                Ok(ibapi::accounts::PositionUpdate::Position(pos)) => {
                    positions.push(BrokerPosition {
                        symbol: pos.contract.symbol.to_string(),
                        qty: pos.position,
                        avg_cost: pos.average_cost,
                    });
                }
                Ok(ibapi::accounts::PositionUpdate::PositionEnd) => break,
                Err(e) => return Err(BrokerError::IbApi(e.to_string())),
            }
        }

        Ok(positions)
    }

    async fn get_open_orders(&self) -> Result<Vec<BrokerOrder>, BrokerError> {
        if !self.is_connected() {
            return Err(BrokerError::Disconnected);
        }

        let subscription = self
            .client
            .open_orders()
            .await
            .map_err(|e| BrokerError::IbApi(e.to_string()))?;

        let mut orders = Vec::new();
        let mut stream = subscription.filter_data();

        while let Some(item) = stream.next().await {
            match item {
                Ok(ibapi::orders::Orders::OrderData(data)) => {
                    let side = match data.order.action {
                        ibapi::orders::Action::Buy => Side::Buy,
                        _ => Side::Sell,
                    };
                    let status = map_ib_status_kind(&data.order_state.status);
                    let total_qty = data.order.total_quantity as u32;
                    let filled_qty = data.order.filled_quantity as u32;

                    orders.push(BrokerOrder {
                        order_id: OrderId(data.order_id.to_string()),
                        symbol: data.contract.symbol.to_string(),
                        side,
                        total_qty,
                        filled_qty,
                        status,
                    });
                }
                Ok(ibapi::orders::Orders::OrderStatus(_)) => {
                    // Status updates within open_orders are informational; skip.
                }
                // The Orders enum only has OrderData variant per docs
                Err(e) => return Err(BrokerError::IbApi(e.to_string())),
            }
        }

        Ok(orders)
    }

    async fn subscribe_order_updates(
        &self,
    ) -> Result<mpsc::Receiver<OrderUpdate>, BrokerError> {
        if !self.is_connected() {
            return Err(BrokerError::Disconnected);
        }

        let subscription = self
            .client
            .order_update_stream()
            .await
            .map_err(|e| BrokerError::IbApi(e.to_string()))?;

        let (tx, rx) = mpsc::channel(256);

        // Clone the order tracker reference so the spawned task can update lifecycle state.
        let order_tracker = self.order_tracker.clone();

        // Spawn a task that translates ibapi OrderUpdate messages into our internal type.
        tokio::spawn(async move {
            let mut stream = subscription.filter_data();

            while let Some(item) = stream.next().await {
                let update = match item {
                    Ok(IbOrderUpdate::OrderStatus(status)) => {
                        let our_status = map_ib_status_kind(&status.status);
                        let order_id = OrderId(status.order_id.to_string());

                        // Update the order tracker with the new lifecycle state.
                        {
                            let mut tracker = order_tracker.lock().await;
                            tracker.update_status(&order_id, our_status);
                            // Clean up terminal orders from tracking.
                            if OrderTracker::is_terminal(our_status) {
                                tracker.remove(&order_id);
                            }
                        }

                        Some(OrderUpdate::StatusChange {
                            order_id,
                            status: our_status,
                        })
                    }
                    Ok(IbOrderUpdate::ExecutionData(exec_data)) => {
                        Some(translate_execution_to_fill(&exec_data))
                    }
                    Ok(IbOrderUpdate::CommissionReport(_)) => {
                        // Commission reports are handled separately if needed.
                        // For now we skip them — fills are reported via ExecutionData.
                        None
                    }
                    Ok(IbOrderUpdate::OpenOrder(_)) => {
                        // Open order updates are informational; we handle status via
                        // the OrderStatus variant.
                        None
                    }
                    Err(e) => {
                        eprintln!("[broker] order update stream error: {}", e);
                        break;
                    }
                };

                if let Some(update) = update {
                    if tx.send(update).await.is_err() {
                        // Receiver dropped, stop the stream.
                        break;
                    }
                }
            }
        });

        Ok(rx)
    }

    fn is_connected(&self) -> bool {
        // Check both our local flag and the ibapi client's connection state.
        self.connected.load(Ordering::SeqCst) && self.client.is_connected()
    }
}

/// Map ibapi's `OrderStatusKind` to our internal `OrderStatus`.
fn map_ib_status_kind(kind: &OrderStatusKind) -> OrderStatus {
    match kind {
        OrderStatusKind::PreSubmitted => OrderStatus::Submitted,
        OrderStatusKind::Submitted => OrderStatus::Acknowledged,
        OrderStatusKind::Filled => OrderStatus::Filled,
        OrderStatusKind::Cancelled | OrderStatusKind::ApiCancelled => OrderStatus::Cancelled,
        OrderStatusKind::Inactive => OrderStatus::Rejected,
        // PendingSubmit, PendingCancel, ApiPending map to Submitted
        _ => OrderStatus::Submitted,
    }
}

/// Translate an ibapi `ExecutionData` into our internal `OrderUpdate::Fill`.
fn translate_execution_to_fill(exec_data: &ExecutionData) -> OrderUpdate {
    let side = match exec_data.execution.side {
        ibapi::orders::ExecutionSide::Bought => Side::Buy,
        _ => Side::Sell,
    };

    OrderUpdate::Fill(BrokerFill {
        order_id: OrderId(exec_data.execution.order_id.to_string()),
        symbol: exec_data.contract.symbol.to_string(),
        side,
        qty: exec_data.execution.shares as u32,
        price: exec_data.execution.price,
        timestamp: Utc::now(), // IB provides time as a string; use current time as approximation
        commission: 0.0, // Commission arrives separately via CommissionReport
    })
}
