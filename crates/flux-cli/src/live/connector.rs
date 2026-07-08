//! Connector trait and core types for pluggable live market data sources.
//!
//! Defines the async `Connector` trait that all data sources (WebSocket,
//! REST polling, CSV replay) implement, along with shared types like
//! `LiveBar`, `ConnectorState`, and `ReconnectPolicy`.

use async_trait::async_trait;
use flux_runtime::BarContext;
use tokio::sync::mpsc;

/// Metadata attached to each bar by the harness.
///
/// Wraps a `BarContext` with additional live-specific metadata:
/// the connector that produced the bar and the wall-clock receive time.
#[derive(Debug, Clone)]
pub struct LiveBar {
    /// The bar data (same struct as backtest)
    pub bar: BarContext,
    /// Connector identifier that produced this bar
    pub connector_id: String,
    /// Wall-clock timestamp when bar was received
    pub received_at: chrono::DateTime<chrono::Utc>,
}

/// Connection state for observability.
///
/// Tracks the lifecycle of a connector from initial connection through
/// potential reconnection attempts and permanent failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectorState {
    Connecting,
    Connected,
    Disconnected,
    Reconnecting { attempt: u32 },
    PermanentlyFailed,
}

/// Configuration for reconnection behavior.
///
/// Controls exponential backoff parameters when a connector loses
/// its connection and needs to retry.
#[derive(Debug, Clone)]
pub struct ReconnectPolicy {
    /// Initial backoff duration in milliseconds (default: 1000)
    pub initial_backoff_ms: u64,
    /// Maximum backoff duration in milliseconds (default: 60000)
    pub max_backoff_ms: u64,
    /// Maximum number of reconnection attempts (default: 10)
    pub max_attempts: u32,
    /// Backoff multiplier (default: 2.0)
    pub multiplier: f64,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            initial_backoff_ms: 1000,
            max_backoff_ms: 60_000,
            max_attempts: 10,
            multiplier: 2.0,
        }
    }
}

/// Errors that can occur during connector operations.
#[derive(Debug, thiserror::Error)]
pub enum ConnectorError {
    #[error("connection failed: {0}")]
    ConnectionFailed(String),
    #[error("subscription failed: {0}")]
    SubscriptionFailed(String),
    #[error("stream ended unexpectedly")]
    StreamEnded,
    #[error("parse error: {0}")]
    ParseError(String),
}

/// Trait for pluggable live market data connectors.
///
/// Each connector runs as an independent async task, pushing bars
/// into the provided channel. The harness manages lifecycle and
/// reconnection externally.
#[async_trait]
pub trait Connector: Send + Sync {
    /// Human-readable identifier for this connector instance.
    fn id(&self) -> &str;

    /// Current connection state.
    fn state(&self) -> ConnectorState;

    /// Connect to the data source and begin streaming bars.
    /// Bars are sent over the provided channel.
    async fn connect(
        &mut self,
        symbols: &[String],
        tx: mpsc::Sender<LiveBar>,
    ) -> Result<(), ConnectorError>;

    /// Disconnect from the data source.
    async fn disconnect(&mut self) -> Result<(), ConnectorError>;

    /// Subscribe to additional symbols on an active connection.
    async fn subscribe(&mut self, symbols: &[String]) -> Result<(), ConnectorError>;
}
