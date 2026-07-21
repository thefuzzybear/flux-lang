//! Telegram notification sink implementation.
//!
//! Delivers alerts to a Telegram chat via the Bot API.

use async_trait::async_trait;

use super::{Alert, NotificationError, NotificationSink, Severity};

/// Maximum Telegram message length (API limit).
const MAX_MESSAGE_LENGTH: usize = 4096;

/// Delivers alerts to a Telegram chat via the Bot API.
pub struct TelegramSink {
    client: reqwest::Client,
    bot_token: String,
    chat_id: String,
}

impl TelegramSink {
    /// Create a new `TelegramSink` with the given bot token and chat ID.
    pub fn new(bot_token: String, chat_id: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            bot_token,
            chat_id,
        }
    }

    /// Format an `Alert` into the Telegram message text.
    ///
    /// Includes: severity (uppercase), event_type, account, message, and ISO 8601 timestamp.
    /// Truncates to 4096 characters if needed.
    pub fn format_message(alert: &Alert) -> String {
        let severity_str = match alert.severity {
            Severity::Critical => "CRITICAL",
            Severity::High => "HIGH",
            Severity::Medium => "MEDIUM",
            Severity::Low => "LOW",
        };

        let formatted = format!(
            "[{}] {}\nAccount: {}\n{}\nTime: {}",
            severity_str,
            alert.event_type,
            alert.account,
            alert.message,
            alert.timestamp.to_rfc3339(),
        );

        if formatted.len() > MAX_MESSAGE_LENGTH {
            formatted[..MAX_MESSAGE_LENGTH].to_string()
        } else {
            formatted
        }
    }
}

#[async_trait]
impl NotificationSink for TelegramSink {
    async fn send(&self, alert: &Alert) -> Result<(), NotificationError> {
        let text = Self::format_message(alert);
        let url = format!(
            "https://api.telegram.org/bot{}/sendMessage",
            self.bot_token
        );

        let body = serde_json::json!({
            "chat_id": self.chat_id,
            "text": text,
        });

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| NotificationError::Network(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<failed to read response body>"));
            return Err(NotificationError::Http {
                status: status.as_u16(),
                body,
            });
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        "telegram"
    }
}
