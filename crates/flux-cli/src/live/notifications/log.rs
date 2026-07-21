//! Log notification sink implementation.
//!
//! Writes alerts to stderr and optionally persists via StorageBackend.
//! Always active — processes every alert regardless of severity.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use super::{Alert, NotificationError, NotificationSink, Severity};
use crate::live::storage::{RiskEventRecord, StorageBackend};

/// Writes alerts to stderr and optionally persists via StorageBackend.
/// Always active — processes every alert regardless of severity.
pub struct LogSink {
    storage: Option<Arc<dyn StorageBackend>>,
}

impl LogSink {
    /// Create a new `LogSink` with an optional storage backend for persistence.
    pub fn new(storage: Option<Arc<dyn StorageBackend>>) -> Self {
        Self { storage }
    }

    /// Format an Alert into a single stderr line.
    ///
    /// Format: `[SEVERITY] account={} event={} msg={} ts={}`
    pub fn format_line(alert: &Alert) -> String {
        let severity_label = match alert.severity {
            Severity::Critical => "CRITICAL",
            Severity::High => "HIGH",
            Severity::Medium => "MEDIUM",
            Severity::Low => "LOW",
        };

        format!(
            "[{}] account={} event={} msg={} ts={}",
            severity_label,
            alert.account,
            alert.event_type,
            alert.message,
            alert.timestamp.to_rfc3339()
        )
    }
}

#[async_trait]
impl NotificationSink for LogSink {
    async fn send(&self, alert: &Alert) -> Result<(), NotificationError> {
        // Always write to stderr.
        eprintln!("{}", Self::format_line(alert));

        // If a storage backend is available, persist as a risk event record.
        if let Some(ref storage) = self.storage {
            let record = RiskEventRecord {
                timestamp: alert.timestamp,
                event_type: alert.event_type.clone(),
                details: json!({
                    "severity": match alert.severity {
                        Severity::Critical => "critical",
                        Severity::High => "high",
                        Severity::Medium => "medium",
                        Severity::Low => "low",
                    },
                    "account": alert.account,
                    "message": alert.message,
                    "details": alert.details,
                }),
            };

            if let Err(e) = storage.record_risk_event(&record).await {
                eprintln!(
                    "[notifications] log storage persistence failed: {}",
                    e
                );
            }
        }

        // Always return Ok — storage errors are swallowed.
        Ok(())
    }

    fn name(&self) -> &'static str {
        "log"
    }
}
