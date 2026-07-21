//! Notification pipeline for the live trading harness.
//!
//! Converts `AlertEvent` instances from the risk limits module into uniform
//! `Alert` payloads and dispatches them to configured notification sinks
//! (Telegram, stderr log) based on severity and account configuration.

pub mod telegram;
pub mod log;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::live::storage::StorageBackend;

use crate::live::risk_limits::{AlertEvent, HaltReason};

// Re-export public types for convenient access.
pub use self::log::*;
pub use self::telegram::*;

/// Alert severity levels, ordered Critical > High > Medium > Low.
///
/// The enum uses ascending numeric values so that the derived `Ord`
/// implementation gives `Critical > High > Medium > Low`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Lowest priority — informational warnings.
    Low = 0,
    /// Position/notional rejections.
    Medium = 1,
    /// System connectivity / reconciliation issues.
    High = 2,
    /// Loss breaches, system halts — requires immediate attention.
    Critical = 3,
}

/// Uniform notification payload produced from an AlertEvent.
#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    pub severity: Severity,
    pub account: String,
    pub event_type: String,
    pub message: String,
    pub details: String,
    pub timestamp: DateTime<Utc>,
}

impl Alert {
    /// Convert an `AlertEvent` into a uniform `Alert`.
    ///
    /// - `severity`: looked up from the severity mapping table
    /// - `account`: injected from pipeline config
    /// - `event_type`: variant name converted to snake_case
    /// - `message`: single-line summary with numeric values at 2 decimal places
    /// - `details`: key=value pairs for all variant fields
    /// - `timestamp`: `Utc::now()` at conversion time
    pub fn from_event(event: &AlertEvent, account: &str) -> Self {
        let (severity, event_type, message, details) = match event {
            AlertEvent::DailyLossBreached { pnl, limit } => (
                Severity::Critical,
                "daily_loss_breached",
                format!("daily_loss_breached: pnl={:.2}, limit={:.2}", pnl, limit),
                format!("pnl={:.2}, limit={:.2}", pnl, limit),
            ),
            AlertEvent::WeeklyLossBreached { pnl, limit } => (
                Severity::Critical,
                "weekly_loss_breached",
                format!("weekly_loss_breached: pnl={:.2}, limit={:.2}", pnl, limit),
                format!("pnl={:.2}, limit={:.2}", pnl, limit),
            ),
            AlertEvent::DrawdownBreached { drawdown_pct, limit } => (
                Severity::Critical,
                "drawdown_breached",
                format!(
                    "drawdown_breached: drawdown_pct={:.2}, limit={:.2}",
                    drawdown_pct, limit
                ),
                format!("drawdown_pct={:.2}, limit={:.2}", drawdown_pct, limit),
            ),
            AlertEvent::SystemHalted { reason } => {
                let reason_str = match reason {
                    HaltReason::DailyLoss { pnl, limit } => {
                        format!("DailyLoss(pnl={:.2}, limit={:.2})", pnl, limit)
                    }
                    HaltReason::WeeklyLoss { pnl, limit } => {
                        format!("WeeklyLoss(pnl={:.2}, limit={:.2})", pnl, limit)
                    }
                    HaltReason::MaxDrawdown { drawdown_pct, limit } => {
                        format!(
                            "MaxDrawdown(drawdown_pct={:.2}, limit={:.2})",
                            drawdown_pct, limit
                        )
                    }
                };
                (
                    Severity::Critical,
                    "system_halted",
                    format!("system_halted: reason={}", reason_str),
                    format!("reason={}", reason_str),
                )
            }
            AlertEvent::PositionLimitRejected { symbol, current, limit } => (
                Severity::Medium,
                "position_limit_rejected",
                format!(
                    "position_limit_rejected: symbol={}, current={}, limit={}",
                    symbol, current, limit
                ),
                format!("symbol={}, current={}, limit={}", symbol, current, limit),
            ),
            AlertEvent::NotionalLimitRejected { current, limit } => (
                Severity::Medium,
                "notional_limit_rejected",
                format!(
                    "notional_limit_rejected: current={:.2}, limit={:.2}",
                    current, limit
                ),
                format!("current={:.2}, limit={:.2}", current, limit),
            ),
            AlertEvent::UnknownSymbolRejected { symbol } => (
                Severity::Medium,
                "unknown_symbol_rejected",
                format!("unknown_symbol_rejected: symbol={}", symbol),
                format!("symbol={}", symbol),
            ),
            AlertEvent::MarginExceededRejected { symbol, required, available } => (
                Severity::Medium,
                "margin_exceeded_rejected",
                format!(
                    "margin_exceeded_rejected: symbol={}, required={:.2}, available={:.2}",
                    symbol, required, available
                ),
                format!(
                    "symbol={}, required={:.2}, available={:.2}",
                    symbol, required, available
                ),
            ),
            AlertEvent::CorrelationWarning { long_count, symbols } => (
                Severity::Low,
                "correlation_warning",
                format!(
                    "correlation_warning: long_count={}, symbols=[{}]",
                    long_count,
                    symbols.join(", ")
                ),
                format!(
                    "long_count={}, symbols=[{}]",
                    long_count,
                    symbols.join(", ")
                ),
            ),
        };

        Alert {
            severity,
            account: account.to_string(),
            event_type: event_type.to_string(),
            message,
            details,
            timestamp: Utc::now(),
        }
    }
}

/// Errors that can occur during notification operations.
/// Never propagated to the trading loop — always caught and logged.
#[derive(Debug, thiserror::Error)]
pub enum NotificationError {
    /// HTTP request returned a non-2xx status.
    #[error("HTTP error {status}: {body}")]
    Http { status: u16, body: String },

    /// Network-level failure (connection refused, DNS, TLS).
    #[error("network error: {0}")]
    Network(String),

    /// Send operation exceeded the configured timeout.
    #[error("timeout after {0:?}")]
    Timeout(Duration),

    /// JSON serialization failure.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// Invalid configuration detected at startup.
    #[error("config error: {0}")]
    Config(String),
}

/// Known valid event type strings (snake_case variants of `AlertEvent`).
const VALID_EVENT_TYPES: &[&str] = &[
    "daily_loss_breached",
    "weekly_loss_breached",
    "drawdown_breached",
    "position_limit_rejected",
    "notional_limit_rejected",
    "unknown_symbol_rejected",
    "margin_exceeded_rejected",
    "correlation_warning",
    "system_halted",
];

/// Configuration for the notification pipeline, parsed from `[alerts]` in account manifest.
#[derive(Debug, Clone, PartialEq)]
pub struct AlertConfig {
    /// "telegram" or "log_only"
    pub backend: String,
    /// Bot token (resolved from env var). Empty when backend="log_only".
    pub telegram_bot_token: String,
    /// Chat ID (resolved from env var). Empty when backend="log_only".
    pub telegram_chat_id: String,
    /// Event types that enable Telegram delivery. Empty = all severity-eligible.
    pub alert_on: Vec<String>,
}

impl AlertConfig {
    /// Validate the config at startup. Returns `NotificationError::Config` on failure.
    ///
    /// Checks:
    /// - `backend` must be "telegram" or "log_only"
    /// - If backend is "telegram", `telegram_bot_token` and `telegram_chat_id` must not be empty
    /// - All entries in `alert_on` must be valid snake_case event type names
    pub fn validate(&self) -> Result<(), NotificationError> {
        // Validate backend value.
        if self.backend != "telegram" && self.backend != "log_only" {
            return Err(NotificationError::Config(format!(
                "invalid backend '{}': accepted values are \"telegram\" or \"log_only\"",
                self.backend
            )));
        }

        // If backend is telegram, credentials must be present.
        if self.backend == "telegram" {
            if self.telegram_bot_token.is_empty() {
                return Err(NotificationError::Config(
                    "telegram_bot_token must not be empty when backend is \"telegram\"".to_string(),
                ));
            }
            if self.telegram_chat_id.is_empty() {
                return Err(NotificationError::Config(
                    "telegram_chat_id must not be empty when backend is \"telegram\"".to_string(),
                ));
            }
        }

        // Validate alert_on entries.
        for event_type in &self.alert_on {
            if !VALID_EVENT_TYPES.contains(&event_type.as_str()) {
                return Err(NotificationError::Config(format!(
                    "unrecognized event type '{}' in alert_on: valid values are {:?}",
                    event_type, VALID_EVENT_TYPES
                )));
            }
        }

        Ok(())
    }

    /// Default config when `[alerts]` section is absent.
    /// Routes all alerts exclusively to LogSink.
    pub fn default_log_only() -> Self {
        AlertConfig {
            backend: "log_only".to_string(),
            telegram_bot_token: String::new(),
            telegram_chat_id: String::new(),
            alert_on: Vec::new(),
        }
    }
}

/// Async trait for notification delivery backends.
///
/// Object-safe, `Send + Sync` for use via `Box<dyn NotificationSink>`.
#[async_trait]
pub trait NotificationSink: Send + Sync {
    /// Deliver an alert to this sink.
    /// Returns `Ok(())` on success, `NotificationError` on failure.
    async fn send(&self, alert: &Alert) -> Result<(), NotificationError>;

    /// Human-readable name for logging (e.g., "telegram", "log").
    fn name(&self) -> &'static str;
}

/// Central dispatcher that converts AlertEvents, applies routing, and
/// delivers to configured sinks with timeout isolation.
///
/// - `log_sink` always fires first (audit trail)
/// - `telegram_sink` fires only when `should_send_telegram` returns true
/// - All sink failures are logged and swallowed — never propagated
pub struct NotificationDispatcher {
    /// Account name injected into every Alert.
    account_name: String,
    /// LogSink — always present, always fires first.
    log_sink: LogSink,
    /// Optional TelegramSink — present when backend="telegram".
    telegram_sink: Option<TelegramSink>,
    /// Set of event_type strings (snake_case) that enable Telegram delivery.
    /// Empty means all severity-eligible alerts go to Telegram.
    alert_on: Vec<String>,
    /// Per-sink send timeout.
    send_timeout: Duration,
}

impl NotificationDispatcher {
    /// Build a new dispatcher from validated `AlertConfig`.
    ///
    /// - Validates the config (returns early on invalid)
    /// - Creates `LogSink` with optional storage backend
    /// - Creates `TelegramSink` only if backend == "telegram"
    /// - Sets send_timeout to 5 seconds
    pub fn new(
        config: &AlertConfig,
        account_name: String,
        storage: Option<Arc<dyn StorageBackend>>,
    ) -> Result<Self, NotificationError> {
        config.validate()?;

        let log_sink = LogSink::new(storage);

        let telegram_sink = if config.backend == "telegram" {
            Some(TelegramSink::new(
                config.telegram_bot_token.clone(),
                config.telegram_chat_id.clone(),
            ))
        } else {
            None
        };

        Ok(Self {
            account_name,
            log_sink,
            telegram_sink,
            alert_on: config.alert_on.clone(),
            send_timeout: Duration::from_secs(5),
        })
    }

    /// Determine whether this alert should be sent to TelegramSink.
    ///
    /// Returns `false` if:
    /// - No telegram_sink is configured (backend = "log_only")
    /// - Alert severity is Low
    /// - `alert_on` is non-empty and the alert's `event_type` is not in the list
    ///
    /// Returns `true` otherwise.
    pub fn should_send_telegram(&self, alert: &Alert) -> bool {
        // No telegram sink configured — backend is log_only.
        if self.telegram_sink.is_none() {
            return false;
        }

        // Low severity alerts never go to Telegram.
        if alert.severity == Severity::Low {
            return false;
        }

        // If alert_on is non-empty, only listed event types go to Telegram.
        if !self.alert_on.is_empty() && !self.alert_on.contains(&alert.event_type) {
            return false;
        }

        true
    }

    /// Convert each `AlertEvent` to an `Alert` and dispatch to applicable sinks.
    ///
    /// Called from a spawned tokio task — never blocks the trading loop.
    /// LogSink always fires first. TelegramSink fires conditionally with timeout.
    /// All errors are logged to stderr and swallowed.
    pub async fn dispatch(&self, events: Vec<AlertEvent>) {
        for event in &events {
            let alert = Alert::from_event(event, &self.account_name);

            // Always send to LogSink first (audit trail).
            if let Err(e) = self.log_sink.send(&alert).await {
                eprintln!(
                    "[notifications] {} failed for {}: {}",
                    self.log_sink.name(),
                    alert.event_type,
                    e
                );
            }

            // Conditionally send to TelegramSink with timeout.
            if self.should_send_telegram(&alert) {
                if let Some(ref tg) = self.telegram_sink {
                    match tokio::time::timeout(self.send_timeout, tg.send(&alert)).await {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => {
                            eprintln!(
                                "[notifications] {} failed for {}: {}",
                                tg.name(),
                                alert.event_type,
                                e
                            );
                        }
                        Err(_elapsed) => {
                            eprintln!(
                                "[notifications] {} failed for {}: timeout after {:?}",
                                tg.name(),
                                alert.event_type,
                                self.send_timeout
                            );
                        }
                    }
                }
            }
        }
    }
}
