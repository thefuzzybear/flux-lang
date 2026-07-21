//! Broker adapter module.
//!
//! Defines the `BrokerAdapter` async trait for broker communication, along with
//! the core order, fill, position, and error types used throughout the live
//! execution layer.

pub mod execution;
pub mod ibkr;
pub mod mock;

pub use execution::ExecutionPolicy;
pub use execution::DeduplicationGuard;
pub use execution::{parse_execution_policy, resolve_execution_policy};

use async_trait::async_trait;
use tokio::sync::mpsc;

/// Unique identifier for a broker order.
/// Format: `"{account}_{strategy}_{symbol}_{bar_index}"`
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OrderId(pub String);

/// Side of an order (direction).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Buy,
    Sell,
}

/// An order ready for broker submission.
#[derive(Debug, Clone)]
pub struct Order {
    pub id: OrderId,
    pub symbol: String,
    pub side: Side,
    pub contracts: u32,
    pub execution: ExecutionPolicy,
    /// Last known price (used for AggressiveLimit calculation).
    pub last_price: f64,
    /// Tick size from ProductRegistry.
    pub tick_size: f64,
}

/// A fill reported by the broker.
#[derive(Debug, Clone)]
pub struct BrokerFill {
    pub order_id: OrderId,
    pub symbol: String,
    pub side: Side,
    pub qty: u32,
    pub price: f64,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub commission: f64,
}

/// Current broker-reported position for a symbol.
#[derive(Debug, Clone)]
pub struct BrokerPosition {
    pub symbol: String,
    pub qty: f64,
    pub avg_cost: f64,
}

/// An open order as reported by the broker.
#[derive(Debug, Clone)]
pub struct BrokerOrder {
    pub order_id: OrderId,
    pub symbol: String,
    pub side: Side,
    pub total_qty: u32,
    pub filled_qty: u32,
    pub status: OrderStatus,
}

/// Order lifecycle states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderStatus {
    Submitted,
    Acknowledged,
    PartialFill,
    Filled,
    Rejected,
    Cancelled,
}

/// Messages received from the broker's order update stream.
#[derive(Debug, Clone)]
pub enum OrderUpdate {
    StatusChange {
        order_id: OrderId,
        status: OrderStatus,
    },
    Fill(BrokerFill),
    Rejection {
        order_id: OrderId,
        reason: String,
    },
}

/// Errors from broker operations.
#[derive(Debug, thiserror::Error)]
pub enum BrokerError {
    #[error("connection failed: {0}")]
    ConnectionFailed(String),
    #[error("order rejected by broker: {0}")]
    OrderRejected(String),
    #[error("order not found: {0}")]
    OrderNotFound(String),
    #[error("session closed for exchange: {exchange}")]
    SessionClosed { exchange: String },
    #[error("disconnected from broker")]
    Disconnected,
    #[error("timeout waiting for response")]
    Timeout,
    #[error("ibapi error: {0}")]
    IbApi(String),
}

/// The unified async trait for broker communication.
///
/// Implementations must be `Send + Sync` for sharing across async tasks via `Arc`.
#[async_trait]
pub trait BrokerAdapter: Send + Sync {
    /// Submit an order to the broker. Returns the broker-assigned order ID.
    async fn submit_order(&self, order: &Order) -> Result<OrderId, BrokerError>;

    /// Cancel a pending order.
    async fn cancel_order(&self, order_id: &OrderId) -> Result<(), BrokerError>;

    /// Query current positions from the broker (for reconciliation).
    async fn get_positions(&self) -> Result<Vec<BrokerPosition>, BrokerError>;

    /// Query open/recent orders (for deduplication on restart).
    async fn get_open_orders(&self) -> Result<Vec<BrokerOrder>, BrokerError>;

    /// Subscribe to fill/execution updates (streaming).
    /// Returns a receiver that yields `OrderUpdate` messages as they arrive.
    async fn subscribe_order_updates(&self) -> Result<mpsc::Receiver<OrderUpdate>, BrokerError>;

    /// Check if the broker connection is active.
    fn is_connected(&self) -> bool;
}
