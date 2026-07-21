//! Mock broker adapter for testing.
//!
//! Provides `MockBrokerAdapter` — a configurable test double that implements `BrokerAdapter`.
//! Supports immediate fills, partial fills, rejections, and pending orders, allowing
//! integration tests to verify the full signal-to-fill flow without a live IB Gateway.

use super::{
    BrokerAdapter, BrokerError, BrokerFill, BrokerOrder, BrokerPosition, Order, OrderId,
    OrderStatus, OrderUpdate, Side,
};
use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tokio::sync::mpsc;

/// Configurable fill behavior for the mock.
#[derive(Debug, Clone)]
pub enum MockFillBehavior {
    /// Immediately fill at the order's last_price.
    ImmediateFill,
    /// Partial fill with the given quantity, remainder stays open.
    PartialFill { fill_qty: u32 },
    /// Reject the order with the given reason.
    Reject { reason: String },
    /// Order stays pending (no fill, no reject).
    Pending,
}

/// A test double for `BrokerAdapter` with configurable fill responses.
///
/// Records all submitted orders and maintains simulated positions and open orders
/// for state consistency testing.
pub struct MockBrokerAdapter {
    /// All orders submitted (inspectable in tests).
    pub submitted_orders: Mutex<Vec<Order>>,
    /// Configurable fill behavior (default: ImmediateFill).
    pub fill_behavior: Mutex<MockFillBehavior>,
    /// Simulated connection state.
    pub connected: AtomicBool,
    /// Channel sender for delivering synthetic OrderUpdates.
    update_tx: Mutex<Option<mpsc::Sender<OrderUpdate>>>,
    /// Simulated positions (updated on fills).
    positions: Mutex<Vec<BrokerPosition>>,
    /// Simulated open orders.
    open_orders: Mutex<Vec<BrokerOrder>>,
}

impl MockBrokerAdapter {
    /// Create a new MockBrokerAdapter with ImmediateFill behavior and connected state.
    pub fn new() -> Self {
        Self {
            submitted_orders: Mutex::new(Vec::new()),
            fill_behavior: Mutex::new(MockFillBehavior::ImmediateFill),
            connected: AtomicBool::new(true),
            update_tx: Mutex::new(None),
            positions: Mutex::new(Vec::new()),
            open_orders: Mutex::new(Vec::new()),
        }
    }

    /// Set the fill behavior for subsequent orders.
    pub fn set_fill_behavior(&self, behavior: MockFillBehavior) {
        *self.fill_behavior.lock().unwrap() = behavior;
    }

    /// Set the connection state (for testing disconnect logic).
    pub fn set_connected(&self, connected: bool) {
        self.connected.store(connected, Ordering::SeqCst);
    }

    /// Get a snapshot of all submitted orders (for test assertions).
    pub fn orders(&self) -> Vec<Order> {
        self.submitted_orders.lock().unwrap().clone()
    }

    /// Update the simulated position for a symbol after a fill.
    fn update_position(&self, symbol: &str, side: Side, qty: u32, price: f64) {
        let mut positions = self.positions.lock().unwrap();
        if let Some(pos) = positions.iter_mut().find(|p| p.symbol == symbol) {
            match side {
                Side::Buy => {
                    let new_qty = pos.qty + qty as f64;
                    if new_qty != 0.0 {
                        pos.avg_cost =
                            (pos.avg_cost * pos.qty + price * qty as f64) / new_qty;
                    }
                    pos.qty = new_qty;
                }
                Side::Sell => {
                    pos.qty -= qty as f64;
                    // avg_cost unchanged on sells
                }
            }
        } else {
            let signed_qty = match side {
                Side::Buy => qty as f64,
                Side::Sell => -(qty as f64),
            };
            positions.push(BrokerPosition {
                symbol: symbol.to_string(),
                qty: signed_qty,
                avg_cost: price,
            });
        }
    }
}

impl Default for MockBrokerAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BrokerAdapter for MockBrokerAdapter {
    async fn submit_order(&self, order: &Order) -> Result<OrderId, BrokerError> {
        if !self.is_connected() {
            return Err(BrokerError::Disconnected);
        }

        // Record the order
        self.submitted_orders.lock().unwrap().push(order.clone());

        let behavior = self.fill_behavior.lock().unwrap().clone();

        match behavior {
            MockFillBehavior::ImmediateFill => {
                // Generate a full fill
                let fill = BrokerFill {
                    order_id: order.id.clone(),
                    symbol: order.symbol.clone(),
                    side: order.side,
                    qty: order.contracts,
                    price: order.last_price,
                    timestamp: chrono::Utc::now(),
                    commission: 0.0,
                };

                // Update positions
                self.update_position(
                    &order.symbol,
                    order.side,
                    order.contracts,
                    order.last_price,
                );

                // Send fill update through channel if subscriber exists
                if let Some(tx) = self.update_tx.lock().unwrap().as_ref() {
                    let _ = tx.try_send(OrderUpdate::Fill(fill));
                }

                // Do NOT add to open_orders — immediately filled
            }
            MockFillBehavior::PartialFill { fill_qty } => {
                let actual_fill = fill_qty.min(order.contracts);

                // Generate a partial fill
                let fill = BrokerFill {
                    order_id: order.id.clone(),
                    symbol: order.symbol.clone(),
                    side: order.side,
                    qty: actual_fill,
                    price: order.last_price,
                    timestamp: chrono::Utc::now(),
                    commission: 0.0,
                };

                // Update positions with the partial amount
                self.update_position(
                    &order.symbol,
                    order.side,
                    actual_fill,
                    order.last_price,
                );

                // Send partial fill update
                if let Some(tx) = self.update_tx.lock().unwrap().as_ref() {
                    let _ = tx.try_send(OrderUpdate::Fill(fill));
                }

                // Add to open_orders with partial fill status
                self.open_orders.lock().unwrap().push(BrokerOrder {
                    order_id: order.id.clone(),
                    symbol: order.symbol.clone(),
                    side: order.side,
                    total_qty: order.contracts,
                    filled_qty: actual_fill,
                    status: OrderStatus::PartialFill,
                });
            }
            MockFillBehavior::Reject { reason } => {
                // Send rejection update
                if let Some(tx) = self.update_tx.lock().unwrap().as_ref() {
                    let _ = tx.try_send(OrderUpdate::Rejection {
                        order_id: order.id.clone(),
                        reason: reason.clone(),
                    });
                }

                return Err(BrokerError::OrderRejected(reason));
            }
            MockFillBehavior::Pending => {
                // Add to open_orders with Submitted status — no fill, no reject
                self.open_orders.lock().unwrap().push(BrokerOrder {
                    order_id: order.id.clone(),
                    symbol: order.symbol.clone(),
                    side: order.side,
                    total_qty: order.contracts,
                    filled_qty: 0,
                    status: OrderStatus::Submitted,
                });
            }
        }

        Ok(order.id.clone())
    }

    async fn cancel_order(&self, order_id: &OrderId) -> Result<(), BrokerError> {
        let mut open_orders = self.open_orders.lock().unwrap();
        if let Some(pos) = open_orders.iter().position(|o| o.order_id == *order_id) {
            open_orders.remove(pos);
            Ok(())
        } else {
            Err(BrokerError::OrderNotFound(order_id.0.clone()))
        }
    }

    async fn get_positions(&self) -> Result<Vec<BrokerPosition>, BrokerError> {
        if !self.is_connected() {
            return Err(BrokerError::Disconnected);
        }
        Ok(self.positions.lock().unwrap().clone())
    }

    async fn get_open_orders(&self) -> Result<Vec<BrokerOrder>, BrokerError> {
        if !self.is_connected() {
            return Err(BrokerError::Disconnected);
        }
        Ok(self.open_orders.lock().unwrap().clone())
    }

    async fn subscribe_order_updates(&self) -> Result<mpsc::Receiver<OrderUpdate>, BrokerError> {
        let (tx, rx) = mpsc::channel(256);
        *self.update_tx.lock().unwrap() = Some(tx);
        Ok(rx)
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::broker::ExecutionPolicy;

    fn make_test_order(symbol: &str, side: Side, contracts: u32) -> Order {
        Order {
            id: OrderId(format!("test_strat_{}_1", symbol)),
            symbol: symbol.to_string(),
            side,
            contracts,
            execution: ExecutionPolicy::Market,
            last_price: 100.0,
            tick_size: 0.25,
        }
    }

    #[tokio::test]
    async fn test_immediate_fill_produces_fill_update() {
        let mock = MockBrokerAdapter::new();
        let mut rx = mock.subscribe_order_updates().await.unwrap();

        let order = make_test_order("ES", Side::Buy, 2);
        let result = mock.submit_order(&order).await;
        assert!(result.is_ok());

        let update = rx.try_recv().unwrap();
        match update {
            OrderUpdate::Fill(fill) => {
                assert_eq!(fill.order_id, order.id);
                assert_eq!(fill.qty, 2);
                assert_eq!(fill.price, 100.0);
            }
            _ => panic!("expected Fill update"),
        }
    }

    #[tokio::test]
    async fn test_partial_fill_produces_correct_qty() {
        let mock = MockBrokerAdapter::new();
        mock.set_fill_behavior(MockFillBehavior::PartialFill { fill_qty: 1 });
        let mut rx = mock.subscribe_order_updates().await.unwrap();

        let order = make_test_order("NQ", Side::Buy, 3);
        mock.submit_order(&order).await.unwrap();

        let update = rx.try_recv().unwrap();
        match update {
            OrderUpdate::Fill(fill) => {
                assert_eq!(fill.qty, 1);
            }
            _ => panic!("expected Fill update"),
        }

        // Should be in open orders with partial fill
        let open = mock.get_open_orders().await.unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].filled_qty, 1);
        assert_eq!(open[0].total_qty, 3);
        assert_eq!(open[0].status, OrderStatus::PartialFill);
    }

    #[tokio::test]
    async fn test_reject_returns_error() {
        let mock = MockBrokerAdapter::new();
        mock.set_fill_behavior(MockFillBehavior::Reject {
            reason: "insufficient margin".to_string(),
        });

        let order = make_test_order("YM", Side::Buy, 1);
        let result = mock.submit_order(&order).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            BrokerError::OrderRejected(reason) => {
                assert_eq!(reason, "insufficient margin");
            }
            _ => panic!("expected OrderRejected error"),
        }
    }

    #[tokio::test]
    async fn test_pending_produces_no_updates() {
        let mock = MockBrokerAdapter::new();
        mock.set_fill_behavior(MockFillBehavior::Pending);
        let mut rx = mock.subscribe_order_updates().await.unwrap();

        let order = make_test_order("RTY", Side::Buy, 2);
        mock.submit_order(&order).await.unwrap();

        // No update should be available
        assert!(rx.try_recv().is_err());

        // Should be in open orders
        let open = mock.get_open_orders().await.unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].status, OrderStatus::Submitted);
    }

    #[tokio::test]
    async fn test_disconnected_returns_error() {
        let mock = MockBrokerAdapter::new();
        mock.set_connected(false);

        let order = make_test_order("ES", Side::Buy, 1);
        let result = mock.submit_order(&order).await;
        assert!(matches!(result, Err(BrokerError::Disconnected)));
    }

    #[tokio::test]
    async fn test_connected_state_toggleable() {
        let mock = MockBrokerAdapter::new();
        assert!(mock.is_connected());

        mock.set_connected(false);
        assert!(!mock.is_connected());

        mock.set_connected(true);
        assert!(mock.is_connected());
    }

    #[tokio::test]
    async fn test_orders_records_all_submissions() {
        let mock = MockBrokerAdapter::new();

        let order1 = make_test_order("ES", Side::Buy, 1);
        let order2 = make_test_order("NQ", Side::Sell, 2);
        mock.submit_order(&order1).await.unwrap();
        mock.submit_order(&order2).await.unwrap();

        let orders = mock.orders();
        assert_eq!(orders.len(), 2);
        assert_eq!(orders[0].symbol, "ES");
        assert_eq!(orders[1].symbol, "NQ");
    }

    #[tokio::test]
    async fn test_immediate_fill_updates_positions() {
        let mock = MockBrokerAdapter::new();

        let order = make_test_order("ES", Side::Buy, 3);
        mock.submit_order(&order).await.unwrap();

        let positions = mock.get_positions().await.unwrap();
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].symbol, "ES");
        assert_eq!(positions[0].qty, 3.0);
        assert_eq!(positions[0].avg_cost, 100.0);
    }

    #[tokio::test]
    async fn test_immediate_fill_no_open_orders() {
        let mock = MockBrokerAdapter::new();

        let order = make_test_order("ES", Side::Buy, 2);
        mock.submit_order(&order).await.unwrap();

        let open = mock.get_open_orders().await.unwrap();
        assert!(open.is_empty());
    }

    #[tokio::test]
    async fn test_cancel_removes_from_open_orders() {
        let mock = MockBrokerAdapter::new();
        mock.set_fill_behavior(MockFillBehavior::Pending);

        let order = make_test_order("ES", Side::Buy, 1);
        mock.submit_order(&order).await.unwrap();

        assert_eq!(mock.get_open_orders().await.unwrap().len(), 1);

        mock.cancel_order(&order.id).await.unwrap();
        assert!(mock.get_open_orders().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_cancel_nonexistent_returns_error() {
        let mock = MockBrokerAdapter::new();
        let id = OrderId("nonexistent_order".to_string());
        let result = mock.cancel_order(&id).await;
        assert!(matches!(result, Err(BrokerError::OrderNotFound(_))));
    }
}
