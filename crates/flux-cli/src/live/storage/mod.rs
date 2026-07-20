//! Storage backend abstraction for the live trading harness.
//!
//! Defines the `StorageBackend` async trait and all record types used for
//! persisting fills, signals, risk events, positions, equity snapshots,
//! orders, and harness checkpoints.

pub mod file;
pub mod postgres;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::live::state::HarnessState;

/// Result type for all storage operations.
///
/// Uses a boxed error to keep the trait object-safe while allowing
/// backend-specific error types.
pub type StorageResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Async trait defining the interface for all harness persistence operations.
///
/// Implementations must be `Send + Sync` to allow use as `Box<dyn StorageBackend>`
/// in the async harness context.
#[async_trait]
pub trait StorageBackend: Send + Sync {
    /// Record a trade fill.
    async fn record_fill(&self, fill: &FillRecord) -> StorageResult<()>;

    /// Record a signal emission (allowed or rejected).
    async fn record_signal(&self, signal: &SignalRecord) -> StorageResult<()>;

    /// Record a risk event (breach, rejection, halt).
    async fn record_risk_event(&self, event: &RiskEventRecord) -> StorageResult<()>;

    /// Insert or update an open position.
    async fn upsert_position(&self, symbol: &str, qty: f64, avg_entry: f64) -> StorageResult<()>;

    /// Record an equity snapshot.
    async fn snapshot_equity(&self, snapshot: &EquitySnapshot) -> StorageResult<()>;

    /// Persist full harness state as a checkpoint.
    async fn save_checkpoint(&self, state: &HarnessState) -> StorageResult<()>;

    /// Load the most recent checkpoint, or None if no checkpoints exist.
    async fn load_latest_checkpoint(&self) -> StorageResult<Option<HarnessState>>;

    /// Load all current open positions.
    async fn load_positions(&self) -> StorageResult<Vec<PositionRecord>>;

    /// Record a new order.
    async fn record_order(&self, order: &OrderRecord) -> StorageResult<()>;

    /// Update an order's status, optionally with fill information.
    async fn update_order_status(
        &self,
        order_id: &str,
        status: &str,
        fill_info: Option<&FillInfo>,
    ) -> StorageResult<()>;
}

/// A single trade fill record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FillRecord {
    pub timestamp: DateTime<Utc>,
    pub strategy: String,
    pub symbol: String,
    /// "buy" | "sell"
    pub side: String,
    /// Must be > 0
    pub qty: f64,
    /// Must be > 0
    pub price: f64,
    pub order_id: Option<String>,
    pub latency_ms: Option<i32>,
    /// Must be >= 0
    pub bar_index: i64,
}

/// A signal emission record (allowed or rejected).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SignalRecord {
    pub timestamp: DateTime<Utc>,
    pub strategy: String,
    pub symbol: String,
    /// "open" | "short" | "close" | "close_qty"
    pub signal_type: String,
    /// Must be > 0 when present
    pub qty: Option<f64>,
    /// "allow" | "reject" | "flatten_all"
    pub decision: String,
    pub reject_reason: Option<String>,
}

/// A risk event record (breach, rejection, halt).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RiskEventRecord {
    pub timestamp: DateTime<Utc>,
    pub event_type: String,
    pub details: serde_json::Value,
}

/// A point-in-time equity snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EquitySnapshot {
    pub timestamp: DateTime<Utc>,
    pub equity: f64,
    pub equity_peak: f64,
    pub daily_pnl: f64,
    pub weekly_pnl: f64,
    /// Must be >= 0
    pub drawdown_pct: f64,
    /// Must be >= 0
    pub open_positions: i32,
}

/// An order record tracking lifecycle state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrderRecord {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub symbol: String,
    /// "buy" | "sell"
    pub side: String,
    /// Must be > 0
    pub qty: f64,
    /// "market" | "limit"
    pub order_type: String,
    /// "submitted" | "acknowledged" | "filled" | "partial" | "rejected" | "cancelled"
    pub status: String,
}

/// A current open position record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PositionRecord {
    pub symbol: String,
    pub qty: f64,
    /// Must be > 0
    pub avg_entry: f64,
    pub realized_pnl: f64,
    pub updated_at: DateTime<Utc>,
}

/// Supplemental fill data for order status updates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FillInfo {
    /// Must be > 0
    pub fill_price: f64,
    /// Must be > 0
    pub fill_qty: f64,
}
