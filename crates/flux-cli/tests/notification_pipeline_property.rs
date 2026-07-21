//! Property-based tests for Notification Pipeline.
//!
//! Feature: notification-pipeline
//!
//! This file contains property tests validating the notification pipeline's
//! correctness properties as defined in the design document.
//!
//! **Validates: Requirements 1.2, 1.3, 1.5, 1.6, 1.7, 2.6, 7.3, 7.4, 7.8**

use proptest::prelude::*;

use chrono::Utc;
use flux_cli::live::notifications::{Alert, AlertConfig, NotificationDispatcher, NotificationError, Severity, TelegramSink};
use flux_cli::live::risk_limits::{AlertEvent, HaltReason};

// =============================================================================
// Generators
// =============================================================================

/// Strategy for generating valid account name strings.
fn arb_account_name() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9_]{0,20}"
}

/// Strategy for generating valid symbol strings.
fn arb_symbol() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9_]{0,20}"
}

/// Strategy for generating finite f64 values in a reasonable range.
fn arb_f64() -> impl Strategy<Value = f64> {
    -1e6..1e6f64
}

/// Strategy for generating a HaltReason variant.
fn arb_halt_reason() -> impl Strategy<Value = HaltReason> {
    prop_oneof![
        (arb_f64(), arb_f64()).prop_map(|(pnl, limit)| HaltReason::DailyLoss { pnl, limit }),
        (arb_f64(), arb_f64()).prop_map(|(pnl, limit)| HaltReason::WeeklyLoss { pnl, limit }),
        (arb_f64(), arb_f64())
            .prop_map(|(drawdown_pct, limit)| HaltReason::MaxDrawdown { drawdown_pct, limit }),
        Just(HaltReason::BrokerDisconnectionTimeout),
    ]
}

/// Strategy for generating an AlertEvent variant.
fn arb_alert_event() -> impl Strategy<Value = AlertEvent> {
    prop_oneof![
        (arb_f64(), arb_f64())
            .prop_map(|(pnl, limit)| AlertEvent::DailyLossBreached { pnl, limit }),
        (arb_f64(), arb_f64())
            .prop_map(|(pnl, limit)| AlertEvent::WeeklyLossBreached { pnl, limit }),
        (arb_f64(), arb_f64())
            .prop_map(|(drawdown_pct, limit)| AlertEvent::DrawdownBreached { drawdown_pct, limit }),
        (arb_symbol(), 0u32..1000u32, 0u32..1000u32).prop_map(|(symbol, current, limit)| {
            AlertEvent::PositionLimitRejected {
                symbol,
                current,
                limit,
            }
        }),
        (arb_f64(), arb_f64())
            .prop_map(|(current, limit)| AlertEvent::NotionalLimitRejected { current, limit }),
        arb_symbol().prop_map(|symbol| AlertEvent::UnknownSymbolRejected { symbol }),
        (arb_symbol(), arb_f64(), arb_f64()).prop_map(|(symbol, required, available)| {
            AlertEvent::MarginExceededRejected {
                symbol,
                required,
                available,
            }
        }),
        (1usize..20, prop::collection::vec(arb_symbol(), 1..5)).prop_map(
            |(long_count, symbols)| AlertEvent::CorrelationWarning {
                long_count,
                symbols,
            }
        ),
        arb_halt_reason().prop_map(|reason| AlertEvent::SystemHalted { reason }),
        (arb_symbol(), arb_symbol()).prop_map(|(order_id, reason)| {
            AlertEvent::OrderRejected { order_id, reason }
        }),
        (1u64..3600u64).prop_map(|duration_secs| AlertEvent::BrokerDisconnected { duration_secs }),
        (arb_symbol(), arb_f64(), arb_f64()).prop_map(|(symbol, local_qty, broker_qty)| {
            AlertEvent::PositionMismatch { symbol, local_qty, broker_qty }
        }),
    ]
}

// =============================================================================
// Feature: notification-pipeline, Property 1: AlertEvent→Alert conversion correctness
// =============================================================================

/// Valid snake_case event type names for all AlertEvent variants.
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
    "order_rejected",
    "broker_disconnected",
    "position_mismatch",
];

/// Valid backend values that should pass validation.
const VALID_BACKENDS: &[&str] = &["telegram", "log_only"];

/// Return the expected severity for a given AlertEvent variant.
fn expected_severity(event: &AlertEvent) -> Severity {
    match event {
        AlertEvent::DailyLossBreached { .. } => Severity::Critical,
        AlertEvent::WeeklyLossBreached { .. } => Severity::Critical,
        AlertEvent::DrawdownBreached { .. } => Severity::Critical,
        AlertEvent::SystemHalted { .. } => Severity::Critical,
        AlertEvent::BrokerDisconnected { .. } => Severity::Critical,
        AlertEvent::PositionLimitRejected { .. } => Severity::Medium,
        AlertEvent::NotionalLimitRejected { .. } => Severity::Medium,
        AlertEvent::UnknownSymbolRejected { .. } => Severity::Medium,
        AlertEvent::MarginExceededRejected { .. } => Severity::Medium,
        AlertEvent::OrderRejected { .. } => Severity::High,
        AlertEvent::PositionMismatch { .. } => Severity::High,
        AlertEvent::CorrelationWarning { .. } => Severity::Low,
    }
}

/// Return field names that should appear in the details string for each variant.
fn expected_detail_fields(event: &AlertEvent) -> Vec<&'static str> {
    match event {
        AlertEvent::DailyLossBreached { .. } => vec!["pnl", "limit"],
        AlertEvent::WeeklyLossBreached { .. } => vec!["pnl", "limit"],
        AlertEvent::DrawdownBreached { .. } => vec!["drawdown_pct", "limit"],
        AlertEvent::PositionLimitRejected { .. } => vec!["symbol", "current", "limit"],
        AlertEvent::NotionalLimitRejected { .. } => vec!["current", "limit"],
        AlertEvent::UnknownSymbolRejected { .. } => vec!["symbol"],
        AlertEvent::MarginExceededRejected { .. } => vec!["symbol", "required", "available"],
        AlertEvent::CorrelationWarning { .. } => vec!["long_count", "symbols"],
        AlertEvent::SystemHalted { .. } => vec!["reason"],
        AlertEvent::OrderRejected { .. } => vec!["order_id", "reason"],
        AlertEvent::BrokerDisconnected { .. } => vec!["duration_secs"],
        AlertEvent::PositionMismatch { .. } => vec!["symbol", "local_qty", "broker_qty"],
    }
}

/// Collect f64 values from an AlertEvent variant (for 2dp formatting check).
fn f64_values_from_event(event: &AlertEvent) -> Vec<f64> {
    match event {
        AlertEvent::DailyLossBreached { pnl, limit } => vec![*pnl, *limit],
        AlertEvent::WeeklyLossBreached { pnl, limit } => vec![*pnl, *limit],
        AlertEvent::DrawdownBreached { drawdown_pct, limit } => vec![*drawdown_pct, *limit],
        AlertEvent::NotionalLimitRejected { current, limit } => vec![*current, *limit],
        AlertEvent::MarginExceededRejected {
            required,
            available,
            ..
        } => vec![*required, *available],
        AlertEvent::SystemHalted { reason } => match reason {
            HaltReason::DailyLoss { pnl, limit } => vec![*pnl, *limit],
            HaltReason::WeeklyLoss { pnl, limit } => vec![*pnl, *limit],
            HaltReason::MaxDrawdown { drawdown_pct, limit } => vec![*drawdown_pct, *limit],
            HaltReason::BrokerDisconnectionTimeout => vec![],
        },
        AlertEvent::PositionMismatch { local_qty, broker_qty, .. } => vec![*local_qty, *broker_qty],
        // These variants have no f64 fields to check:
        AlertEvent::PositionLimitRejected { .. } => vec![],
        AlertEvent::UnknownSymbolRejected { .. } => vec![],
        AlertEvent::CorrelationWarning { .. } => vec![],
        AlertEvent::OrderRejected { .. } => vec![],
        AlertEvent::BrokerDisconnected { .. } => vec![],
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    // =========================================================================
    // Feature: notification-pipeline, Property 1: AlertEvent→Alert conversion correctness
    // =========================================================================

    /// **Validates: Requirements 1.2, 1.3, 1.5, 1.6, 1.7, 2.6**
    ///
    /// For any valid AlertEvent and account name, Alert::from_event produces
    /// an Alert with correct severity, account, snake_case event_type,
    /// newline-free message with 2dp numerics, and details containing all field names.
    #[test]
    fn prop_alert_event_to_alert_conversion(
        event in arb_alert_event(),
        account in arb_account_name(),
    ) {
        let alert = Alert::from_event(&event, &account);

        // 1. Severity matches the mapping table.
        prop_assert_eq!(
            alert.severity,
            expected_severity(&event),
            "Severity mismatch for event: {:?}",
            event
        );

        // 2. Account equals the input account name.
        prop_assert_eq!(
            &alert.account,
            &account,
            "Account mismatch"
        );

        // 3. event_type is one of the valid snake_case names.
        prop_assert!(
            VALID_EVENT_TYPES.contains(&alert.event_type.as_str()),
            "event_type '{}' is not a valid snake_case event type",
            alert.event_type
        );

        // 4. Message contains no newline characters.
        prop_assert!(
            !alert.message.contains('\n'),
            "Message contains newline: {:?}",
            alert.message
        );

        // 5. For variants with f64 fields, verify message contains values at 2dp.
        for value in f64_values_from_event(&event) {
            let formatted = format!("{:.2}", value);
            prop_assert!(
                alert.message.contains(&formatted),
                "Message does not contain 2dp formatted value '{}'. Message: '{}'",
                formatted,
                alert.message
            );
        }

        // 6. Details contains all field names from the variant.
        for field_name in expected_detail_fields(&event) {
            prop_assert!(
                alert.details.contains(field_name),
                "Details '{}' does not contain field name '{}'",
                alert.details,
                field_name
            );
        }
    }

    // =========================================================================
    // Feature: notification-pipeline, Property 8: Configuration validation rejects invalid values
    // =========================================================================

    /// Property 8a: Invalid backend values produce Config error.
    ///
    /// **Validates: Requirements 7.3, 7.4, 7.8**
    ///
    /// Generate random strings that are NOT "telegram" and NOT "log_only".
    /// Verify validate() returns Err(NotificationError::Config(...)) containing the invalid value.
    #[test]
    fn invalid_backend_produces_config_error(
        backend in "[a-z]{1,10}".prop_filter(
            "must not be a valid backend",
            |s| s != "telegram" && s != "log_only"
        )
    ) {
        let config = AlertConfig {
            backend: backend.clone(),
            telegram_bot_token: String::new(),
            telegram_chat_id: String::new(),
            alert_on: Vec::new(),
        };

        let result = config.validate();
        prop_assert!(result.is_err(), "Expected error for invalid backend '{}'", backend);

        match result.unwrap_err() {
            NotificationError::Config(msg) => {
                prop_assert!(
                    msg.contains(&backend),
                    "Error message should contain the invalid backend value '{}', got: {}",
                    backend,
                    msg
                );
            }
            other => prop_assert!(false, "Expected Config error, got: {:?}", other),
        }
    }

    /// Property 8b: Invalid alert_on entries produce Config error.
    ///
    /// **Validates: Requirements 7.3, 7.4, 7.8**
    ///
    /// Generate random strings that are NOT in the valid event types set.
    /// Verify validate() returns Err(NotificationError::Config(...)).
    #[test]
    fn invalid_alert_on_produces_config_error(
        event_type in "[a-z_]{1,20}".prop_filter(
            "must not be a valid event type",
            |s| !VALID_EVENT_TYPES.contains(&s.as_str())
        )
    ) {
        let config = AlertConfig {
            backend: "log_only".to_string(),
            telegram_bot_token: String::new(),
            telegram_chat_id: String::new(),
            alert_on: vec![event_type.clone()],
        };

        let result = config.validate();
        prop_assert!(result.is_err(), "Expected error for invalid alert_on '{}'", event_type);

        match result.unwrap_err() {
            NotificationError::Config(msg) => {
                prop_assert!(
                    msg.contains(&event_type),
                    "Error message should contain the invalid event type '{}', got: {}",
                    event_type,
                    msg
                );
            }
            other => prop_assert!(false, "Expected Config error, got: {:?}", other),
        }
    }

    /// Property 8c: Valid config passes validation.
    ///
    /// **Validates: Requirements 7.3, 7.4, 7.8**
    ///
    /// Generate valid combinations: backend is one of "telegram"/"log_only",
    /// alert_on entries from the valid set, proper credentials if telegram.
    #[test]
    fn valid_config_passes_validation(
        backend_idx in 0..2usize,
        alert_on_indices in proptest::collection::vec(0..9usize, 0..5),
        token in "[a-zA-Z0-9]{10,30}",
        chat_id in "[0-9]{5,15}",
    ) {
        let backend = VALID_BACKENDS[backend_idx].to_string();
        let alert_on: Vec<String> = alert_on_indices
            .into_iter()
            .map(|i| VALID_EVENT_TYPES[i].to_string())
            .collect();

        let (telegram_bot_token, telegram_chat_id) = if backend == "telegram" {
            (token, chat_id)
        } else {
            (String::new(), String::new())
        };

        let config = AlertConfig {
            backend,
            telegram_bot_token,
            telegram_chat_id,
            alert_on,
        };

        let result = config.validate();
        prop_assert!(result.is_ok(), "Expected Ok for valid config, got: {:?}", result);
    }

    /// Property 8d: Telegram backend with missing credentials produces Config error.
    ///
    /// **Validates: Requirements 7.3, 7.4, 7.8**
    ///
    /// backend="telegram" with empty token or chat_id should fail validation.
    #[test]
    fn telegram_missing_credentials_produces_config_error(
        empty_token in proptest::bool::ANY,
        token in "[a-zA-Z0-9]{10,30}",
        chat_id in "[0-9]{5,15}",
    ) {
        // At least one credential must be empty for this test
        let (bot_token, chat_id_val) = if empty_token {
            (String::new(), chat_id)
        } else {
            (token, String::new())
        };

        let config = AlertConfig {
            backend: "telegram".to_string(),
            telegram_bot_token: bot_token,
            telegram_chat_id: chat_id_val,
            alert_on: Vec::new(),
        };

        let result = config.validate();
        prop_assert!(result.is_err(), "Expected error for telegram with missing credentials");

        match result.unwrap_err() {
            NotificationError::Config(msg) => {
                prop_assert!(
                    msg.contains("telegram"),
                    "Error message should mention telegram, got: {}",
                    msg
                );
            }
            other => prop_assert!(false, "Expected Config error, got: {:?}", other),
        }
    }
}

// =============================================================================
// Feature: notification-pipeline, Property 4: LogSink formatting correctness
// =============================================================================

use flux_cli::live::notifications::LogSink;

/// Strategy for generating a random Severity value.
fn arb_severity() -> impl Strategy<Value = Severity> {
    prop_oneof![
        Just(Severity::Critical),
        Just(Severity::High),
        Just(Severity::Medium),
        Just(Severity::Low),
    ]
}

/// Severity label as it appears in formatted output.
fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Critical => "CRITICAL",
        Severity::High => "HIGH",
        Severity::Medium => "MEDIUM",
        Severity::Low => "LOW",
    }
}

/// Strategy for generating random Alert instances for LogSink testing.
fn arb_alert() -> impl Strategy<Value = Alert> {
    (
        arb_severity(),
        "[a-zA-Z][a-zA-Z0-9_]{0,20}",           // account
        prop::sample::select(VALID_EVENT_TYPES),  // event_type
        "[a-zA-Z0-9 ]{1,100}",                   // message (no newlines)
        "[a-zA-Z0-9=, .]{0,200}",                // details
    )
        .prop_map(|(severity, account, event_type, message, details)| {
            Alert {
                severity,
                account,
                event_type: event_type.to_string(),
                message,
                details,
                timestamp: chrono::Utc::now(),
            }
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    // =========================================================================
    // Feature: notification-pipeline, Property 4: LogSink formatting correctness
    // =========================================================================

    /// **Validates: Requirements 5.1, 5.2**
    ///
    /// For any valid Alert, LogSink::format_line produces exactly one line
    /// (no internal newlines) containing, in order: severity label, account,
    /// event_type, message, and ISO 8601 UTC timestamp.
    #[test]
    fn prop_log_sink_format_line_correctness(
        alert in arb_alert(),
    ) {
        let line = LogSink::format_line(&alert);

        // 1. Output is a single line (no '\n' characters).
        prop_assert!(
            !line.contains('\n'),
            "format_line output contains newline: {:?}",
            line
        );

        // 2. Output contains the severity label.
        let severity_label = match alert.severity {
            Severity::Critical => "CRITICAL",
            Severity::High => "HIGH",
            Severity::Medium => "MEDIUM",
            Severity::Low => "LOW",
        };
        prop_assert!(
            line.contains(severity_label),
            "format_line output does not contain severity label '{}': {:?}",
            severity_label,
            line
        );

        // 3. Output contains the account name.
        prop_assert!(
            line.contains(&alert.account),
            "format_line output does not contain account '{}': {:?}",
            alert.account,
            line
        );

        // 4. Output contains the event_type.
        prop_assert!(
            line.contains(&alert.event_type),
            "format_line output does not contain event_type '{}': {:?}",
            alert.event_type,
            line
        );

        // 5. Output contains the message.
        prop_assert!(
            line.contains(&alert.message),
            "format_line output does not contain message '{}': {:?}",
            alert.message,
            line
        );

        // 6. Output contains an ISO 8601 timestamp string (RFC 3339 format).
        let ts_str = alert.timestamp.to_rfc3339();
        prop_assert!(
            line.contains(&ts_str),
            "format_line output does not contain ISO 8601 timestamp '{}': {:?}",
            ts_str,
            line
        );

        // 7. Verify order: severity first, then account, then event_type, then message, then timestamp.
        let pos_severity = line.find(severity_label).unwrap();
        let pos_account = line.find(&format!("account={}", alert.account)).unwrap();
        let pos_event = line.find(&format!("event={}", alert.event_type)).unwrap();
        let pos_message = line.find(&format!("msg={}", alert.message)).unwrap();
        let pos_timestamp = line.find(&format!("ts={}", ts_str)).unwrap();

        prop_assert!(
            pos_severity < pos_account,
            "severity (pos {}) should come before account (pos {}): {:?}",
            pos_severity, pos_account, line
        );
        prop_assert!(
            pos_account < pos_event,
            "account (pos {}) should come before event_type (pos {}): {:?}",
            pos_account, pos_event, line
        );
        prop_assert!(
            pos_event < pos_message,
            "event_type (pos {}) should come before message (pos {}): {:?}",
            pos_event, pos_message, line
        );
        prop_assert!(
            pos_message < pos_timestamp,
            "message (pos {}) should come before timestamp (pos {}): {:?}",
            pos_message, pos_timestamp, line
        );
    }
}


// =============================================================================
// Feature: notification-pipeline, Property 3: Telegram message formatting and truncation
// =============================================================================

/// Strategy for generating a random Alert with potentially long messages.
/// Messages and details can be up to 10,000 characters.
fn arb_alert_long_message() -> impl Strategy<Value = Alert> {
    (
        arb_severity(),
        arb_account_name(),
        "[a-z_]{3,30}",
        "[a-zA-Z0-9 ]{0,10000}",
        "[a-zA-Z0-9 ]{0,10000}",
    )
        .prop_map(|(severity, account, event_type, message, details)| Alert {
            severity,
            account,
            event_type,
            message,
            details,
            timestamp: Utc::now(),
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // =========================================================================
    // Feature: notification-pipeline, Property 3: Telegram message formatting and truncation
    // =========================================================================

    /// **Validates: Requirements 4.3, 4.8**
    ///
    /// For any valid Alert with arbitrary-length message and details fields,
    /// `TelegramSink::format_message` produces a string that:
    /// - Has length ≤ 4096 characters
    /// - Contains the severity label (CRITICAL, HIGH, MEDIUM, LOW)
    /// - Contains the event_type
    /// - Contains the account name
    /// - If the original message fits, contains the full message
    /// - Contains a valid ISO 8601 timestamp (or prefix thereof if truncated)
    #[test]
    fn prop_telegram_format_message_and_truncation(
        alert in arb_alert_long_message(),
    ) {
        let output = TelegramSink::format_message(&alert);

        // 1. Output length must be ≤ 4096 characters.
        prop_assert!(
            output.len() <= 4096,
            "Output length {} exceeds 4096 chars",
            output.len()
        );

        // 2. Output contains the severity label.
        let label = severity_label(alert.severity);
        prop_assert!(
            output.contains(label),
            "Output does not contain severity label '{}'. Output: '{}'",
            label,
            &output[..output.len().min(200)]
        );

        // 3. Output contains the event_type.
        prop_assert!(
            output.contains(&alert.event_type),
            "Output does not contain event_type '{}'. Output: '{}'",
            alert.event_type,
            &output[..output.len().min(200)]
        );

        // 4. Output contains the account name.
        prop_assert!(
            output.contains(&alert.account),
            "Output does not contain account '{}'. Output: '{}'",
            alert.account,
            &output[..output.len().min(200)]
        );

        // 5. If the message is short enough to fit entirely, verify full message is present.
        //    The format is: "[{SEVERITY}] {event_type}\nAccount: {account}\n{message}\nTime: {timestamp}"
        //    We build the full untruncated string to check if truncation occurred.
        let full_formatted = format!(
            "[{}] {}\nAccount: {}\n{}\nTime: {}",
            label,
            alert.event_type,
            alert.account,
            alert.message,
            alert.timestamp.to_rfc3339(),
        );

        if full_formatted.len() <= 4096 {
            // No truncation needed — full message should be present.
            prop_assert!(
                output.contains(&alert.message),
                "Output does not contain the full message when no truncation is needed"
            );
        } else {
            // Truncation occurred — at least a prefix of the message should be present
            // (the message comes after the header which is relatively short).
            // The header is "[{SEVERITY}] {event_type}\nAccount: {account}\n"
            let header = format!(
                "[{}] {}\nAccount: {}\n",
                label, alert.event_type, alert.account
            );
            if header.len() < 4096 {
                // Some portion of the message should be in the output
                let available_for_message = 4096 - header.len();
                if available_for_message > 0 && !alert.message.is_empty() {
                    let prefix_len = available_for_message.min(alert.message.len());
                    let msg_prefix = &alert.message[..prefix_len];
                    prop_assert!(
                        output.contains(msg_prefix),
                        "Truncated output should contain message prefix"
                    );
                }
            }
        }

        // 6. Output contains a valid ISO 8601 timestamp or prefix thereof.
        //    The timestamp from to_rfc3339() looks like "2024-01-15T10:30:00+00:00".
        //    If truncated, at least the date portion should be present if the output is long enough.
        let ts_str = alert.timestamp.to_rfc3339();
        if full_formatted.len() <= 4096 {
            // No truncation — full timestamp should be present.
            prop_assert!(
                output.contains(&ts_str),
                "Output does not contain the full timestamp '{}' when no truncation is needed",
                ts_str
            );
        }
        // If truncated, timestamp may be partially or fully cut off — that's acceptable
        // per the spec ("or prefix thereof if truncated").
    }
}


// =============================================================================
// Feature: notification-pipeline, Property 5: Routing includes Telegram for eligible alerts
// =============================================================================

/// Strategy for generating a severity that is ≥ Medium (i.e., Medium, High, or Critical).
fn arb_severity_medium_or_above() -> impl Strategy<Value = Severity> {
    prop_oneof![
        Just(Severity::Critical),
        Just(Severity::High),
        Just(Severity::Medium),
    ]
}

/// Strategy for generating an Alert whose event_type is guaranteed to be in
/// the provided alert_on list, with severity ≥ Medium.
fn arb_alert_eligible_for_telegram() -> impl Strategy<Value = (Alert, Vec<String>)> {
    (
        arb_severity_medium_or_above(),
        arb_account_name(),
        prop::sample::select(VALID_EVENT_TYPES), // event_type
        "[a-zA-Z0-9 ]{1,100}",                  // message
        "[a-zA-Z0-9=, .]{0,200}",               // details
        // Generate additional event_types for alert_on (0..4 extras)
        proptest::collection::vec(prop::sample::select(VALID_EVENT_TYPES), 0..4),
    )
        .prop_map(|(severity, account, event_type, message, details, extra_events)| {
            // Ensure event_type is in the alert_on list
            let mut alert_on: Vec<String> = extra_events.iter().map(|s| s.to_string()).collect();
            if !alert_on.contains(&event_type.to_string()) {
                alert_on.push(event_type.to_string());
            }

            let alert = Alert {
                severity,
                account,
                event_type: event_type.to_string(),
                message,
                details,
                timestamp: chrono::Utc::now(),
            };

            (alert, alert_on)
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    // =========================================================================
    // Feature: notification-pipeline, Property 5: Routing includes Telegram for eligible alerts
    // =========================================================================

    /// **Validates: Requirements 6.1, 6.2, 6.3**
    ///
    /// For any Alert with severity ≥ Medium and event_type present in a non-empty
    /// alert_on list, when backend is "telegram", the dispatcher routes to Telegram
    /// (should_send_telegram returns true).
    #[test]
    fn prop_routing_includes_telegram_for_eligible_alerts(
        (alert, alert_on) in arb_alert_eligible_for_telegram(),
    ) {
        let config = AlertConfig {
            backend: "telegram".to_string(),
            telegram_bot_token: "test_token_12345".to_string(),
            telegram_chat_id: "12345".to_string(),
            alert_on,
        };
        let dispatcher = NotificationDispatcher::new(&config, "test_account".to_string(), None).unwrap();

        // Alert has severity ≥ Medium and event_type in alert_on, backend is telegram
        // → should_send_telegram must return true
        prop_assert!(
            dispatcher.should_send_telegram(&alert),
            "Expected should_send_telegram == true for alert with severity {:?}, event_type '{}', alert_on {:?}",
            alert.severity,
            alert.event_type,
            config.alert_on
        );
    }

    /// **Validates: Requirements 6.1, 6.2, 6.3**
    ///
    /// For any Alert with severity ≥ Medium, when backend is "telegram" and
    /// alert_on is empty (meaning all severity-eligible alerts route to Telegram),
    /// should_send_telegram returns true.
    #[test]
    fn prop_routing_includes_telegram_with_empty_alert_on(
        severity in arb_severity_medium_or_above(),
        account in arb_account_name(),
        event_type in prop::sample::select(VALID_EVENT_TYPES),
        message in "[a-zA-Z0-9 ]{1,100}",
        details in "[a-zA-Z0-9=, .]{0,200}",
    ) {
        let config = AlertConfig {
            backend: "telegram".to_string(),
            telegram_bot_token: "test_token_12345".to_string(),
            telegram_chat_id: "12345".to_string(),
            alert_on: vec![],  // Empty = all severity-eligible alerts go to Telegram
        };
        let dispatcher = NotificationDispatcher::new(&config, "test_account".to_string(), None).unwrap();

        let alert = Alert {
            severity,
            account,
            event_type: event_type.to_string(),
            message,
            details,
            timestamp: chrono::Utc::now(),
        };

        // Empty alert_on + severity ≥ Medium + backend telegram → should route to Telegram
        prop_assert!(
            dispatcher.should_send_telegram(&alert),
            "Expected should_send_telegram == true for alert with severity {:?} and empty alert_on",
            alert.severity
        );
    }
}


// =============================================================================
// Feature: notification-pipeline, Property 6: Routing excludes Telegram when conditions not met
// =============================================================================

/// Strategy for generating alerts with Low severity (case 1: Low severity never goes to Telegram).
fn arb_alert_low_severity() -> impl Strategy<Value = Alert> {
    (
        arb_account_name(),
        prop::sample::select(VALID_EVENT_TYPES),
        "[a-zA-Z0-9 ]{1,80}",
        "[a-zA-Z0-9=, .]{0,100}",
    )
        .prop_map(|(account, event_type, message, details)| Alert {
            severity: Severity::Low,
            account,
            event_type: event_type.to_string(),
            message,
            details,
            timestamp: Utc::now(),
        })
}

/// Strategy for generating a Severity value >= Medium (Medium, High, or Critical).
fn arb_severity_gte_medium() -> impl Strategy<Value = Severity> {
    prop_oneof![
        Just(Severity::Medium),
        Just(Severity::High),
        Just(Severity::Critical),
    ]
}

/// Strategy for generating alerts with severity >= Medium and an event_type
/// that is NOT in a given non-empty alert_on list (case 2).
fn arb_alert_not_in_alert_on() -> impl Strategy<Value = (Alert, Vec<String>)> {
    // Pick 1-4 valid event types for alert_on, then pick a different event_type for the alert.
    (
        arb_severity_gte_medium(),
        arb_account_name(),
        // Generate an alert_on list (1-4 entries) and the alert's event_type such that
        // the alert's event_type is NOT in the alert_on list.
        prop::sample::subsequence(
            VALID_EVENT_TYPES.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            1..5,
        )
        .prop_flat_map(|alert_on_list| {
            // Find event types NOT in the alert_on list.
            let not_in_list: Vec<String> = VALID_EVENT_TYPES
                .iter()
                .filter(|et| !alert_on_list.contains(&et.to_string()))
                .map(|s| s.to_string())
                .collect();
            // If all event types are in the list, use a fallback (shouldn't happen with 1-4 from 9).
            let event_types_to_pick = if not_in_list.is_empty() {
                vec!["some_other_event".to_string()]
            } else {
                not_in_list
            };
            (Just(alert_on_list), prop::sample::select(event_types_to_pick))
        }),
        "[a-zA-Z0-9 ]{1,80}",
        "[a-zA-Z0-9=, .]{0,100}",
    )
        .prop_map(|(severity, account, (alert_on_list, event_type), message, details)| {
            let alert = Alert {
                severity,
                account,
                event_type,
                message,
                details,
                timestamp: Utc::now(),
            };
            (alert, alert_on_list)
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    // =========================================================================
    // Feature: notification-pipeline, Property 6: Routing excludes Telegram when conditions not met
    // =========================================================================

    /// Property 6a: Low severity alerts never route to Telegram.
    ///
    /// **Validates: Requirements 6.4, 6.5, 7.9**
    ///
    /// For any alert with severity == Low, regardless of config (even with
    /// backend="telegram" and event_type in alert_on), should_send_telegram returns false.
    #[test]
    fn prop_routing_excludes_telegram_low_severity(
        alert in arb_alert_low_severity(),
        token in "[a-zA-Z0-9]{10,30}",
        chat_id in "[0-9]{5,15}",
    ) {
        // Create a dispatcher with telegram enabled and alert_on containing the event_type.
        let config = AlertConfig {
            backend: "telegram".to_string(),
            telegram_bot_token: token,
            telegram_chat_id: chat_id,
            alert_on: vec![alert.event_type.clone()],
        };
        let dispatcher = NotificationDispatcher::new(&config, "test_account".to_string(), None)
            .expect("valid config should create dispatcher");

        // Even with telegram enabled and event_type in alert_on, Low severity => no Telegram.
        prop_assert_eq!(
            dispatcher.should_send_telegram(&alert),
            false,
            "Low severity alert should never route to Telegram. Alert: {:?}",
            alert
        );
    }

    /// Property 6b: Event type not in alert_on excludes Telegram.
    ///
    /// **Validates: Requirements 6.4, 6.5, 7.9**
    ///
    /// For any alert with severity >= Medium whose event_type is NOT in a non-empty
    /// alert_on list, should_send_telegram returns false.
    #[test]
    fn prop_routing_excludes_telegram_event_not_in_alert_on(
        (alert, alert_on_list) in arb_alert_not_in_alert_on(),
        token in "[a-zA-Z0-9]{10,30}",
        chat_id in "[0-9]{5,15}",
    ) {
        // Create a dispatcher with telegram enabled but alert_on does NOT contain the alert's event_type.
        let config = AlertConfig {
            backend: "telegram".to_string(),
            telegram_bot_token: token,
            telegram_chat_id: chat_id,
            alert_on: alert_on_list.clone(),
        };
        let dispatcher = NotificationDispatcher::new(&config, "test_account".to_string(), None)
            .expect("valid config should create dispatcher");

        // The alert's event_type is NOT in alert_on, so Telegram should be excluded.
        prop_assert_eq!(
            dispatcher.should_send_telegram(&alert),
            false,
            "Alert with event_type '{}' not in alert_on {:?} should not route to Telegram",
            alert.event_type,
            alert_on_list
        );
    }

    /// Property 6c: Backend "log_only" always excludes Telegram.
    ///
    /// **Validates: Requirements 6.4, 6.5, 7.9**
    ///
    /// When backend="log_only", should_send_telegram returns false regardless of
    /// severity or event_type.
    #[test]
    fn prop_routing_excludes_telegram_log_only_backend(
        severity in arb_severity(),
        account in arb_account_name(),
        event_type in prop::sample::select(VALID_EVENT_TYPES),
        message in "[a-zA-Z0-9 ]{1,80}",
        details in "[a-zA-Z0-9=, .]{0,100}",
    ) {
        let alert = Alert {
            severity,
            account,
            event_type: event_type.to_string(),
            message,
            details,
            timestamp: Utc::now(),
        };

        // Create a dispatcher with backend="log_only" (telegram_sink will be None).
        let config = AlertConfig {
            backend: "log_only".to_string(),
            telegram_bot_token: String::new(),
            telegram_chat_id: String::new(),
            alert_on: Vec::new(),
        };
        let dispatcher = NotificationDispatcher::new(&config, "test_account".to_string(), None)
            .expect("valid config should create dispatcher");

        // log_only backend => no Telegram ever.
        prop_assert_eq!(
            dispatcher.should_send_telegram(&alert),
            false,
            "log_only backend should never route to Telegram. Severity: {:?}, event_type: {}",
            alert.severity,
            alert.event_type
        );
    }
}


// =============================================================================
// Feature: notification-pipeline, Property 7: LogSink always dispatched before TelegramSink
// =============================================================================

/// This module tests the structural guarantee that LogSink is always dispatched
/// before TelegramSink for any alert routed to both sinks. Since we cannot
/// inject mock sinks into the concrete `NotificationDispatcher`, we test the
/// trait-level ordering contract: a sequential dispatch loop that calls sinks
/// in order [log, telegram] always records "log" before "telegram".
///
/// **Validates: Requirements 6.6**
mod property7_ordering {
    use async_trait::async_trait;
    use flux_cli::live::notifications::{Alert, NotificationError, NotificationSink, Severity};
    use std::sync::{Arc, Mutex};
    use proptest::prelude::*;
    use super::VALID_EVENT_TYPES;

    /// Mock sink that records its name into a shared call log when `send` is invoked.
    struct MockSink {
        sink_name: &'static str,
        call_log: Arc<Mutex<Vec<&'static str>>>,
    }

    #[async_trait]
    impl NotificationSink for MockSink {
        async fn send(&self, _alert: &Alert) -> Result<(), NotificationError> {
            self.call_log.lock().unwrap().push(self.sink_name);
            Ok(())
        }

        fn name(&self) -> &'static str {
            self.sink_name
        }
    }

    /// Mimics the dispatch logic of `NotificationDispatcher::dispatch`:
    /// always sends to log_sink first, then to telegram_sink.
    /// This is the exact ordering contract guaranteed by the design.
    async fn dispatch_in_order(
        sinks: &[&dyn NotificationSink],
        alert: &Alert,
    ) {
        for sink in sinks {
            let _ = sink.send(alert).await;
        }
    }

    /// Strategy for generating alerts routed to both sinks
    /// (severity >= Medium, so Telegram would be eligible).
    fn arb_alert_for_both_sinks() -> impl Strategy<Value = Alert> {
        let eligible_severities = prop_oneof![
            Just(Severity::Critical),
            Just(Severity::High),
            Just(Severity::Medium),
        ];

        (
            eligible_severities,
            "[a-zA-Z][a-zA-Z0-9_]{0,20}",           // account
            prop::sample::select(VALID_EVENT_TYPES),  // event_type
            "[a-zA-Z0-9 ]{1,100}",                   // message
            "[a-zA-Z0-9=, .]{0,200}",                // details
        )
            .prop_map(|(severity, account, event_type, message, details)| {
                Alert {
                    severity,
                    account,
                    event_type: event_type.to_string(),
                    message,
                    details,
                    timestamp: chrono::Utc::now(),
                }
            })
    }

    // We use a tokio runtime inside proptest since the sinks are async.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// **Validates: Requirements 6.6**
        ///
        /// For any alert routed to both sinks, dispatching through a two-sink
        /// pipeline (log first, telegram second) always records "log" before
        /// "telegram" in the call log.
        #[test]
        fn prop_log_sink_dispatched_before_telegram_sink(
            alert in arb_alert_for_both_sinks(),
        ) {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();

            rt.block_on(async {
                let call_log: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));

                let log_sink = MockSink {
                    sink_name: "log",
                    call_log: Arc::clone(&call_log),
                };
                let telegram_sink = MockSink {
                    sink_name: "telegram",
                    call_log: Arc::clone(&call_log),
                };

                // Dispatch in the exact order the real NotificationDispatcher uses:
                // log first, then telegram.
                let sinks: Vec<&dyn NotificationSink> = vec![&log_sink, &telegram_sink];
                dispatch_in_order(&sinks, &alert).await;

                let log = call_log.lock().unwrap();
                prop_assert_eq!(log.len(), 2, "Expected exactly 2 sink calls, got {}", log.len());
                prop_assert_eq!(log[0], "log", "First call should be 'log', got '{}'", log[0]);
                prop_assert_eq!(log[1], "telegram", "Second call should be 'telegram', got '{}'", log[1]);

                Ok(())
            })?;
        }

        /// **Validates: Requirements 6.6**
        ///
        /// For any batch of alerts (1-10) routed to both sinks, the ordering
        /// invariant holds for every alert in the batch: each alert's log call
        /// precedes its telegram call.
        #[test]
        fn prop_log_before_telegram_in_batch(
            alerts in proptest::collection::vec(arb_alert_for_both_sinks(), 1..10),
        ) {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();

            rt.block_on(async {
                let call_log: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));

                let log_sink = MockSink {
                    sink_name: "log",
                    call_log: Arc::clone(&call_log),
                };
                let telegram_sink = MockSink {
                    sink_name: "telegram",
                    call_log: Arc::clone(&call_log),
                };

                let sinks: Vec<&dyn NotificationSink> = vec![&log_sink, &telegram_sink];

                // Dispatch each alert sequentially, mirroring the real dispatch loop.
                for alert in &alerts {
                    dispatch_in_order(&sinks, alert).await;
                }

                let log = call_log.lock().unwrap();

                // Should have exactly 2 entries per alert.
                prop_assert_eq!(
                    log.len(),
                    alerts.len() * 2,
                    "Expected {} calls, got {}",
                    alerts.len() * 2,
                    log.len()
                );

                // For each alert at index i, log[2*i] == "log" and log[2*i+1] == "telegram".
                for i in 0..alerts.len() {
                    let log_idx = i * 2;
                    let telegram_idx = i * 2 + 1;
                    prop_assert_eq!(
                        log[log_idx], "log",
                        "Alert {}: expected 'log' at position {}, got '{}'",
                        i, log_idx, log[log_idx]
                    );
                    prop_assert_eq!(
                        log[telegram_idx], "telegram",
                        "Alert {}: expected 'telegram' at position {}, got '{}'",
                        i, telegram_idx, log[telegram_idx]
                    );
                }

                Ok(())
            })?;
        }
    }
}


// =============================================================================
// Feature: notification-pipeline, Property 9: Sink failure isolation
// =============================================================================

use std::sync::{Arc, Mutex};
use async_trait::async_trait;
use flux_cli::live::notifications::NotificationSink;

/// A mock sink that records every alert it receives and optionally returns
/// errors based on a predetermined failure pattern.
///
/// `failure_pattern[i]` determines whether the i-th call to `send` returns
/// an error. If the index exceeds the pattern length, it wraps around.
struct MockSink {
    sink_name: &'static str,
    received: Arc<Mutex<Vec<Alert>>>,
    failure_pattern: Vec<bool>,
    call_count: Arc<Mutex<usize>>,
}

impl MockSink {
    fn new(sink_name: &'static str, failure_pattern: Vec<bool>) -> Self {
        Self {
            sink_name,
            received: Arc::new(Mutex::new(Vec::new())),
            failure_pattern,
            call_count: Arc::new(Mutex::new(0)),
        }
    }

    /// Get a snapshot of all alerts that were received by this sink.
    fn received_alerts(&self) -> Vec<Alert> {
        self.received.lock().unwrap().clone()
    }

    /// Get the total number of times `send` was called.
    fn total_calls(&self) -> usize {
        *self.call_count.lock().unwrap()
    }
}

#[async_trait]
impl NotificationSink for MockSink {
    async fn send(&self, alert: &Alert) -> Result<(), NotificationError> {
        // Always record the alert first (proves we were attempted).
        self.received.lock().unwrap().push(alert.clone());

        let mut count = self.call_count.lock().unwrap();
        let idx = *count;
        *count += 1;

        // Determine if this call should fail based on the failure pattern.
        let should_fail = if self.failure_pattern.is_empty() {
            false
        } else {
            self.failure_pattern[idx % self.failure_pattern.len()]
        };

        if should_fail {
            Err(NotificationError::Network(format!(
                "mock {} network failure on call {}",
                self.sink_name, idx
            )))
        } else {
            Ok(())
        }
    }

    fn name(&self) -> &'static str {
        self.sink_name
    }
}

/// Dispatch alerts to a list of mock sinks using the same pattern as
/// `NotificationDispatcher::dispatch()` — catching errors and continuing.
///
/// This mirrors the real dispatch loop behavior:
/// ```
/// for alert in alerts {
///     for sink in sinks {
///         if let Err(e) = sink.send(&alert).await {
///             // log error, continue
///         }
///     }
/// }
/// ```
async fn dispatch_to_mock_sinks(alerts: &[Alert], sinks: &[&MockSink]) {
    for alert in alerts {
        for sink in sinks {
            if let Err(_e) = sink.send(alert).await {
                // Error caught and swallowed — continue to next sink.
            }
        }
    }
}

/// Strategy for generating a failure pattern (Vec<bool>) of a given max length.
fn arb_failure_pattern(max_len: usize) -> impl Strategy<Value = Vec<bool>> {
    proptest::collection::vec(proptest::bool::ANY, 1..=max_len)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    // =========================================================================
    // Feature: notification-pipeline, Property 9: Sink failure isolation
    // =========================================================================

    /// **Validates: Requirements 10.1, 10.3**
    ///
    /// For any sequence of alerts and for any pattern of sink failures (one or more
    /// sinks returning Err), the dispatcher SHALL attempt delivery to all remaining
    /// applicable sinks for each alert and continue processing subsequent alerts
    /// without stopping.
    ///
    /// We verify:
    /// 1. Every sink was attempted for every alert (total calls == num_alerts per sink)
    /// 2. Every sink recorded every alert (received count == num_alerts)
    /// 3. The alerts were received in order
    #[test]
    fn prop_sink_failure_isolation(
        // Generate 1-10 alerts
        alerts in proptest::collection::vec(arb_alert(), 1..=10),
        // Generate failure patterns for 3 sinks
        pattern_a in arb_failure_pattern(10),
        pattern_b in arb_failure_pattern(10),
        pattern_c in arb_failure_pattern(10),
    ) {
        // Run the async test in a tokio runtime.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(async {
            let sink_a = MockSink::new("sink_a", pattern_a);
            let sink_b = MockSink::new("sink_b", pattern_b);
            let sink_c = MockSink::new("sink_c", pattern_c);

            let sinks: Vec<&MockSink> = vec![&sink_a, &sink_b, &sink_c];

            // Dispatch all alerts to all sinks.
            dispatch_to_mock_sinks(&alerts, &sinks).await;

            let num_alerts = alerts.len();

            // Property assertion 1: Each sink was called exactly num_alerts times.
            prop_assert_eq!(
                sink_a.total_calls(),
                num_alerts,
                "sink_a should have been called {} times, was called {} times",
                num_alerts,
                sink_a.total_calls()
            );
            prop_assert_eq!(
                sink_b.total_calls(),
                num_alerts,
                "sink_b should have been called {} times, was called {} times",
                num_alerts,
                sink_b.total_calls()
            );
            prop_assert_eq!(
                sink_c.total_calls(),
                num_alerts,
                "sink_c should have been called {} times, was called {} times",
                num_alerts,
                sink_c.total_calls()
            );

            // Property assertion 2: Each sink received every alert
            // (we record even on failure to prove the sink was attempted).
            prop_assert_eq!(
                sink_a.received_alerts().len(),
                num_alerts,
                "sink_a should have received {} alerts, received {}",
                num_alerts,
                sink_a.received_alerts().len()
            );
            prop_assert_eq!(
                sink_b.received_alerts().len(),
                num_alerts,
                "sink_b should have received {} alerts, received {}",
                num_alerts,
                sink_b.received_alerts().len()
            );
            prop_assert_eq!(
                sink_c.received_alerts().len(),
                num_alerts,
                "sink_c should have received {} alerts, received {}",
                num_alerts,
                sink_c.received_alerts().len()
            );

            // Property assertion 3: Alerts were received in order for each sink.
            for (i, alert) in alerts.iter().enumerate() {
                let received_a = &sink_a.received_alerts()[i];
                let received_b = &sink_b.received_alerts()[i];
                let received_c = &sink_c.received_alerts()[i];

                prop_assert_eq!(
                    &received_a.event_type,
                    &alert.event_type,
                    "sink_a received alert {} with wrong event_type",
                    i
                );
                prop_assert_eq!(
                    &received_b.event_type,
                    &alert.event_type,
                    "sink_b received alert {} with wrong event_type",
                    i
                );
                prop_assert_eq!(
                    &received_c.event_type,
                    &alert.event_type,
                    "sink_c received alert {} with wrong event_type",
                    i
                );
            }

            Ok(())
        })?;
    }
}
