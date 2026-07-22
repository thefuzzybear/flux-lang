//! Execution policy definitions, signal-to-order translation, and deduplication.
//!
//! Defines how strategy signals map to broker order types (Market, Limit, Stop, etc.),
//! provides the `translate_signal` function for deterministic conversion, and implements
//! the `DeduplicationGuard` to prevent double-submission of orders.

/// How to execute an order against the broker.
/// Configured per-strategy in account.flux, resolved at runtime.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum ExecutionPolicy {
    /// Market order — guaranteed fill, worst price. Default.
    #[default]
    Market,
    /// Aggressive limit: last price ± offset_ticks (chases the market).
    AggressiveLimit { offset_ticks: i32 },
    /// Passive limit at a specific price.
    Limit { price: f64 },
    /// Market-on-close — for EOD flatten.
    MarketOnClose,
    /// Limit-on-close at a specific price.
    LimitOnClose { price: f64 },
    /// Stop order (entry or stop-loss).
    Stop { trigger_price: f64 },
    /// Stop-limit: stop triggers a limit order.
    StopLimit {
        trigger_price: f64,
        limit_price: f64,
    },
    /// Trailing stop with fixed dollar offset from market.
    TrailingStop { trail_amount: f64 },
    /// Trailing stop with percentage offset.
    TrailingStopPct { trail_pct: f64 },
    /// Adaptive algo (IB's adaptive order type — better fills).
    Adaptive { urgency: AdaptiveUrgency },
}

/// Urgency level for IB's adaptive order algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdaptiveUrgency {
    Patient,
    Normal,
    Urgent,
}



/// Parse a string representation into an ExecutionPolicy variant.
/// Returns the default (Market) for unrecognized strings.
pub fn parse_execution_policy(s: &str, offset_ticks: Option<i32>) -> ExecutionPolicy {
    match s {
        "market" => ExecutionPolicy::Market,
        "aggressive_limit" => ExecutionPolicy::AggressiveLimit {
            offset_ticks: offset_ticks.unwrap_or(2),
        },
        "limit" => ExecutionPolicy::Limit { price: 0.0 }, // price set at order time
        "market_on_close" => ExecutionPolicy::MarketOnClose,
        "limit_on_close" => ExecutionPolicy::LimitOnClose { price: 0.0 },
        "stop" => ExecutionPolicy::Stop { trigger_price: 0.0 },
        "stop_limit" => ExecutionPolicy::StopLimit {
            trigger_price: 0.0,
            limit_price: 0.0,
        },
        "trailing_stop" => ExecutionPolicy::TrailingStop { trail_amount: 0.0 },
        "trailing_stop_pct" => ExecutionPolicy::TrailingStopPct { trail_pct: 0.0 },
        "adaptive" => ExecutionPolicy::Adaptive { urgency: AdaptiveUrgency::Normal },
        _ => ExecutionPolicy::Market,
    }
}

/// Resolve the execution policy for a strategy.
/// Priority: strategy-specific > account-level default > Market.
pub fn resolve_execution_policy(
    strategy_execution: Option<&str>,
    strategy_offset: Option<i32>,
    account_default: Option<&str>,
) -> ExecutionPolicy {
    if let Some(exec) = strategy_execution {
        parse_execution_policy(exec, strategy_offset)
    } else if let Some(default) = account_default {
        parse_execution_policy(default, None)
    } else {
        ExecutionPolicy::Market
    }
}

use std::collections::HashSet;
use chrono::{DateTime, Utc};
use flux_runtime::Signal;
use super::{BrokerAdapter, BrokerError, Order, OrderId, Side};
use crate::live::market_calendar::MarketCalendar;

/// Translate a Flux Signal + ExecutionPolicy into a broker Order.
///
/// Floor policy: fractional qty is rounded DOWN to nearest integer.
/// Returns None if qty rounds to 0 (signal "sized out").
///
/// For Close signals, `current_position_qty` provides the full position size.
#[allow(clippy::too_many_arguments)]
pub fn translate_signal(
    signal: &Signal,
    policy: &ExecutionPolicy,
    account: &str,
    strategy: &str,
    bar_index: u64,
    last_price: f64,
    tick_size: f64,
    current_position_qty: f64,
) -> Option<Order> {
    let (symbol, raw_qty, side) = match signal {
        Signal::Open { symbol, qty } => (symbol.clone(), *qty, Side::Buy),
        Signal::Short { symbol, qty } => (symbol.clone(), *qty, Side::Sell),
        Signal::Close { symbol } => (symbol.clone(), current_position_qty.abs(), Side::Sell),
        Signal::CloseQty { symbol, qty } => (symbol.clone(), *qty, Side::Sell),
    };

    let contracts = raw_qty.floor() as u32;
    if contracts == 0 {
        return None;
    }

    let id = OrderId(format!("{}_{}_{}_{}",account, strategy, symbol, bar_index));

    Some(Order {
        id,
        symbol,
        side,
        contracts,
        execution: policy.clone(),
        last_price,
        tick_size,
    })
}

/// Calculate the effective limit price for AggressiveLimit orders.
/// Buy: last_price + (offset_ticks * tick_size)
/// Sell: last_price - (offset_ticks * tick_size)
pub fn aggressive_limit_price(side: Side, last_price: f64, offset_ticks: i32, tick_size: f64) -> f64 {
    match side {
        Side::Buy => last_price + (offset_ticks as f64 * tick_size),
        Side::Sell => last_price - (offset_ticks as f64 * tick_size),
    }
}

/// Tracks submitted order IDs to prevent double-submission within a session.
///
/// The guard maintains an in-memory set of OrderIds that have been submitted
/// during the current session. On restart, `reconcile()` repopulates the set
/// from broker open orders to avoid re-submitting orders that are already
/// in-flight.
pub struct DeduplicationGuard {
    /// In-memory set of OrderIds submitted this session.
    submitted: HashSet<OrderId>,
}

impl DeduplicationGuard {
    /// Create a new empty deduplication guard.
    pub fn new() -> Self {
        Self {
            submitted: HashSet::new(),
        }
    }

    /// Check if an order ID has already been submitted. Returns true if duplicate.
    pub fn is_duplicate(&self, id: &OrderId) -> bool {
        self.submitted.contains(id)
    }

    /// Mark an order ID as submitted.
    pub fn mark_submitted(&mut self, id: OrderId) {
        self.submitted.insert(id);
    }

    /// Reconciliation on restart: populate from broker open orders.
    ///
    /// Queries the broker for all currently open orders and marks them as
    /// submitted in the local set. Returns the list of order IDs that are
    /// still open (waiting for fill) so the caller knows not to re-submit them.
    pub async fn reconcile(
        &mut self,
        broker: &dyn BrokerAdapter,
    ) -> Result<Vec<OrderId>, BrokerError> {
        let open_orders = broker.get_open_orders().await?;
        let mut waiting_for_fill = Vec::new();

        for broker_order in &open_orders {
            self.submitted.insert(broker_order.order_id.clone());
            waiting_for_fill.push(broker_order.order_id.clone());
        }

        Ok(waiting_for_fill)
    }
}

impl Default for DeduplicationGuard {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if the current time is within Regular Trading Hours for the given exchange.
///
/// This serves as a defensive layer ensuring orders are only submitted during RTH,
/// even though the LiveHarness already gates bar processing to trading sessions.
///
/// Returns `Ok(())` if within session, or `Err(BrokerError::SessionClosed)` if outside RTH.
pub fn check_session_gate(
    calendar: &MarketCalendar,
    exchange: &str,
    now: DateTime<Utc>,
) -> Result<(), BrokerError> {
    // Get exchange timezone
    let tz = calendar.timezone(exchange).map_err(|_| BrokerError::SessionClosed {
        exchange: exchange.to_string(),
    })?;

    // Convert UTC time to exchange local time
    let local_now = now.with_timezone(&tz);
    let local_date = local_now.date_naive();
    let local_time = local_now.time();

    // Check if it's a trading day and get session times
    let (open, close) = calendar
        .session_times_for_date(exchange, local_date)
        .map_err(|_| BrokerError::SessionClosed {
            exchange: exchange.to_string(),
        })?;

    // Check if current time is within session
    if local_time >= open && local_time <= close {
        Ok(())
    } else {
        Err(BrokerError::SessionClosed {
            exchange: exchange.to_string(),
        })
    }
}
