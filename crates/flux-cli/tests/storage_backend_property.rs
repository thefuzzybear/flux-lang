//! Property-based tests for StorageBackend implementations.
//!
//! Feature: postgres-storage-backend
//!
//! This file contains property tests validating correctness properties
//! of the storage backend record types and FileBackend implementation.

use proptest::prelude::*;
use std::fs;

use chrono::{DateTime, Utc};
use flux_cli::live::storage::{
    EquitySnapshot, FillInfo, FillRecord, OrderRecord, RiskEventRecord, SignalRecord,
    StorageBackend,
};
use flux_cli::live::storage::file::FileBackend;
use tempfile::TempDir;

// =============================================================================
// Strategy Generators
// =============================================================================

fn arb_datetime() -> impl Strategy<Value = DateTime<Utc>> {
    (0i64..2_000_000_000i64).prop_map(|secs| chrono::DateTime::from_timestamp(secs, 0).unwrap())
}

fn arb_symbol() -> impl Strategy<Value = String> {
    "[a-zA-Z]{1,10}"
}

fn arb_strategy_name() -> impl Strategy<Value = String> {
    "[a-zA-Z]{1,10}"
}

fn arb_side() -> impl Strategy<Value = String> {
    prop_oneof![Just("buy".to_string()), Just("sell".to_string())]
}

fn arb_signal_type() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("open".to_string()),
        Just("short".to_string()),
        Just("close".to_string()),
        Just("close_qty".to_string()),
    ]
}

fn arb_decision() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("allow".to_string()),
        Just("reject".to_string()),
        Just("flatten_all".to_string()),
    ]
}

fn arb_order_type() -> impl Strategy<Value = String> {
    prop_oneof![Just("market".to_string()), Just("limit".to_string())]
}

fn arb_order_status() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("submitted".to_string()),
        Just("acknowledged".to_string()),
        Just("filled".to_string()),
        Just("partial".to_string()),
        Just("rejected".to_string()),
        Just("cancelled".to_string()),
    ]
}

fn arb_positive_f64() -> impl Strategy<Value = f64> {
    (1u64..1_000_000u64).prop_map(|v| v as f64 / 100.0)
}

fn arb_non_negative_f64() -> impl Strategy<Value = f64> {
    (0u64..1_000_000u64).prop_map(|v| v as f64 / 100.0)
}

// =============================================================================
// Record Generators
// =============================================================================

fn arb_fill_record() -> impl Strategy<Value = FillRecord> {
    (
        arb_datetime(),
        arb_strategy_name(),
        arb_symbol(),
        arb_side(),
        arb_positive_f64(),
        arb_positive_f64(),
        proptest::option::of("[a-zA-Z0-9]{1,10}"),
        proptest::option::of(1i32..1000i32),
        0i64..100_000i64,
    )
        .prop_map(
            |(timestamp, strategy, symbol, side, qty, price, order_id, latency_ms, bar_index)| {
                FillRecord {
                    timestamp,
                    strategy,
                    symbol,
                    side,
                    qty,
                    price,
                    order_id,
                    latency_ms,
                    bar_index,
                }
            },
        )
}

fn arb_signal_record() -> impl Strategy<Value = SignalRecord> {
    (
        arb_datetime(),
        arb_strategy_name(),
        arb_symbol(),
        arb_signal_type(),
        proptest::option::of(arb_positive_f64()),
        arb_decision(),
        proptest::option::of("[a-zA-Z ]{1,20}"),
    )
        .prop_map(
            |(timestamp, strategy, symbol, signal_type, qty, decision, reject_reason)| {
                SignalRecord {
                    timestamp,
                    strategy,
                    symbol,
                    signal_type,
                    qty,
                    decision,
                    reject_reason,
                }
            },
        )
}

fn arb_risk_event_record() -> impl Strategy<Value = RiskEventRecord> {
    (arb_datetime(), "[a-zA-Z_]{1,10}").prop_map(|(timestamp, event_type)| RiskEventRecord {
        timestamp,
        event_type,
        details: serde_json::json!({"reason": "test"}),
    })
}

fn arb_pnl_f64() -> impl Strategy<Value = f64> {
    (-1_000_000i64..1_000_000i64).prop_map(|v| v as f64 / 100.0)
}

fn arb_equity_snapshot() -> impl Strategy<Value = EquitySnapshot> {
    (
        arb_datetime(),
        arb_positive_f64(),
        arb_positive_f64(),
        arb_pnl_f64(),
        arb_pnl_f64(),
        arb_non_negative_f64(),
        0i32..100i32,
    )
        .prop_map(
            |(timestamp, equity, equity_peak, daily_pnl, weekly_pnl, drawdown_pct, open_positions)| {
                EquitySnapshot {
                    timestamp,
                    equity,
                    equity_peak,
                    daily_pnl,
                    weekly_pnl,
                    drawdown_pct,
                    open_positions,
                }
            },
        )
}

fn arb_order_record() -> impl Strategy<Value = OrderRecord> {
    (
        "[a-zA-Z0-9]{1,10}",
        arb_datetime(),
        arb_symbol(),
        arb_side(),
        arb_positive_f64(),
        arb_order_type(),
        arb_order_status(),
    )
        .prop_map(
            |(id, timestamp, symbol, side, qty, order_type, status)| OrderRecord {
                id,
                timestamp,
                symbol,
                side,
                qty,
                order_type,
                status,
            },
        )
}

fn arb_fill_info() -> impl Strategy<Value = FillInfo> {
    (arb_positive_f64(), arb_positive_f64())
        .prop_map(|(fill_price, fill_qty)| FillInfo {
            fill_price,
            fill_qty,
        })
}

// =============================================================================
// Property 2: FileBackend JSONL append round-trip
// Feature: postgres-storage-backend, Property 2: FileBackend JSONL append round-trip
// Validates: Requirements 6.2, 6.7, 6.8, 6.9, 6.10, 6.12
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// For any valid FillRecord, calling record_fill on a FileBackend and reading the
    /// last line of fills.jsonl should deserialize to the original record.
    #[test]
    fn file_backend_fill_roundtrip(fill in arb_fill_record()) {
        let tmp = TempDir::new().unwrap();
        let backend = FileBackend::new(tmp.path().to_path_buf()).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async { backend.record_fill(&fill).await.unwrap(); });

        let content = fs::read_to_string(tmp.path().join("fills.jsonl")).unwrap();
        let last_line = content.trim().lines().last().unwrap();
        let deserialized: FillRecord = serde_json::from_str(last_line).unwrap();
        prop_assert_eq!(deserialized, fill);
    }

    /// For any valid SignalRecord, calling record_signal on a FileBackend and reading the
    /// last line of signals.jsonl should deserialize to the original record.
    #[test]
    fn file_backend_signal_roundtrip(signal in arb_signal_record()) {
        let tmp = TempDir::new().unwrap();
        let backend = FileBackend::new(tmp.path().to_path_buf()).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async { backend.record_signal(&signal).await.unwrap(); });

        let content = fs::read_to_string(tmp.path().join("signals.jsonl")).unwrap();
        let last_line = content.trim().lines().last().unwrap();
        let deserialized: SignalRecord = serde_json::from_str(last_line).unwrap();
        prop_assert_eq!(deserialized, signal);
    }

    /// For any valid RiskEventRecord, calling record_risk_event on a FileBackend and reading
    /// the last line of risk_events.jsonl should deserialize to the original record.
    #[test]
    fn file_backend_risk_event_roundtrip(event in arb_risk_event_record()) {
        let tmp = TempDir::new().unwrap();
        let backend = FileBackend::new(tmp.path().to_path_buf()).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async { backend.record_risk_event(&event).await.unwrap(); });

        let content = fs::read_to_string(tmp.path().join("risk_events.jsonl")).unwrap();
        let last_line = content.trim().lines().last().unwrap();
        let deserialized: RiskEventRecord = serde_json::from_str(last_line).unwrap();
        prop_assert_eq!(deserialized, event);
    }

    /// For any valid EquitySnapshot, calling snapshot_equity on a FileBackend and reading
    /// the last line of equity.jsonl should deserialize to the original snapshot.
    #[test]
    fn file_backend_equity_snapshot_roundtrip(snapshot in arb_equity_snapshot()) {
        let tmp = TempDir::new().unwrap();
        let backend = FileBackend::new(tmp.path().to_path_buf()).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async { backend.snapshot_equity(&snapshot).await.unwrap(); });

        let content = fs::read_to_string(tmp.path().join("equity.jsonl")).unwrap();
        let last_line = content.trim().lines().last().unwrap();
        let deserialized: EquitySnapshot = serde_json::from_str(last_line).unwrap();
        prop_assert_eq!(deserialized, snapshot);
    }

    /// For any valid OrderRecord, calling record_order on a FileBackend and reading
    /// the last line of orders.jsonl should deserialize to the original record.
    #[test]
    fn file_backend_order_roundtrip(order in arb_order_record()) {
        let tmp = TempDir::new().unwrap();
        let backend = FileBackend::new(tmp.path().to_path_buf()).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async { backend.record_order(&order).await.unwrap(); });

        let content = fs::read_to_string(tmp.path().join("orders.jsonl")).unwrap();
        let last_line = content.trim().lines().last().unwrap();
        let deserialized: OrderRecord = serde_json::from_str(last_line).unwrap();
        prop_assert_eq!(deserialized, order);
    }

    /// For any valid order_id and status with FillInfo, calling update_order_status on a
    /// FileBackend and reading the last line of orders.jsonl should contain the correct
    /// order_id, status, fill_price, and fill_qty.
    #[test]
    fn file_backend_update_order_status_with_fill_info(
        order_id in "[a-zA-Z0-9]{1,10}",
        status in arb_order_status(),
        fill_info in arb_fill_info(),
    ) {
        let tmp = TempDir::new().unwrap();
        let backend = FileBackend::new(tmp.path().to_path_buf()).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            backend.update_order_status(&order_id, &status, Some(&fill_info)).await.unwrap();
        });

        let content = fs::read_to_string(tmp.path().join("orders.jsonl")).unwrap();
        let last_line = content.trim().lines().last().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(last_line).unwrap();

        prop_assert_eq!(parsed["order_id"].as_str().unwrap(), order_id.as_str());
        prop_assert_eq!(parsed["status"].as_str().unwrap(), status.as_str());
        prop_assert_eq!(parsed["fill_price"].as_f64().unwrap(), fill_info.fill_price);
        prop_assert_eq!(parsed["fill_qty"].as_f64().unwrap(), fill_info.fill_qty);
    }

    /// For any valid order_id and status without FillInfo, calling update_order_status
    /// on a FileBackend should write a line with null fill_price and fill_qty.
    #[test]
    fn file_backend_update_order_status_without_fill_info(
        order_id in "[a-zA-Z0-9]{1,10}",
        status in arb_order_status(),
    ) {
        let tmp = TempDir::new().unwrap();
        let backend = FileBackend::new(tmp.path().to_path_buf()).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            backend.update_order_status(&order_id, &status, None).await.unwrap();
        });

        let content = fs::read_to_string(tmp.path().join("orders.jsonl")).unwrap();
        let last_line = content.trim().lines().last().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(last_line).unwrap();

        prop_assert_eq!(parsed["order_id"].as_str().unwrap(), order_id.as_str());
        prop_assert_eq!(parsed["status"].as_str().unwrap(), status.as_str());
        // fill_price and fill_qty should be absent (skip_serializing_if = "Option::is_none")
        prop_assert!(parsed.get("fill_price").is_none() || parsed["fill_price"].is_null());
        prop_assert!(parsed.get("fill_qty").is_none() || parsed["fill_qty"].is_null());
    }
}

// =============================================================================
// Property 3: FileBackend checkpoint and positions round-trip
// Feature: postgres-storage-backend, Property 3: FileBackend checkpoint and positions round-trip
// Validates: Requirements 6.3, 6.6, 6.11
// =============================================================================

use flux_cli::live::state::{HarnessState, PositionState, STATE_VERSION};
use std::collections::HashMap;

/// Generate a random HarnessState with minimal content for checkpoint testing.
fn arb_harness_state() -> impl Strategy<Value = HarnessState> {
    (
        0u64..10000u64,     // fill_count
        0u64..100000u64,    // bars_processed
    )
        .prop_map(|(fill_count, bars_processed)| HarnessState {
            version: STATE_VERSION,
            positions: PositionState {
                initial_capital: 10_000.0,
                positions: vec![],
                total_realized_pnl: 0.0,
                last_prices: vec![],
            },
            strategy_states: vec![],
            fill_count,
            checkpoint_timestamp: "2024-06-15T14:30:00.000Z".to_string(),
            bars_processed,
        })
}

/// Generate a random set of (symbol, qty, avg_entry) tuples for position upserts.
fn arb_positions() -> impl Strategy<Value = Vec<(String, f64, f64)>> {
    proptest::collection::vec(
        (
            "[a-z]{2,5}".prop_map(|s| s.to_uppercase()),
            1.0f64..10000.0,
            0.01f64..10000.0,
        ),
        1..10,
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// For any valid HarnessState and sequence of upsert_position calls,
    /// save_checkpoint followed by load_latest_checkpoint returns equivalent state,
    /// and load_positions reflects the last-written qty/avg_entry per symbol.
    #[test]
    fn checkpoint_and_positions_roundtrip(
        state in arb_harness_state(),
        positions in arb_positions(),
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let tmp = TempDir::new().unwrap();
            let backend = FileBackend::new(tmp.path().to_path_buf()).unwrap();

            // Upsert all generated positions
            for (symbol, qty, avg_entry) in &positions {
                backend.upsert_position(symbol, *qty, *avg_entry).await.unwrap();
            }

            // Save checkpoint
            backend.save_checkpoint(&state).await.unwrap();

            // Load latest checkpoint
            let loaded = backend.load_latest_checkpoint().await.unwrap()
                .expect("checkpoint should exist after save");

            // Verify non-position fields match
            prop_assert_eq!(loaded.version, state.version);
            prop_assert_eq!(loaded.fill_count, state.fill_count);
            prop_assert_eq!(loaded.bars_processed, state.bars_processed);
            prop_assert_eq!(&loaded.checkpoint_timestamp, &state.checkpoint_timestamp);
            prop_assert_eq!(&loaded.strategy_states, &state.strategy_states);

            // Deduplicate positions by symbol (keep last written)
            let mut expected_positions: HashMap<String, (f64, f64)> = HashMap::new();
            for (symbol, qty, avg_entry) in &positions {
                expected_positions.insert(symbol.clone(), (*qty, *avg_entry));
            }

            // Verify positions in loaded state match last-written values
            prop_assert_eq!(loaded.positions.positions.len(), expected_positions.len());
            for pos in &loaded.positions.positions {
                let (expected_qty, expected_avg) = expected_positions.get(&pos.symbol)
                    .expect("unexpected symbol in loaded positions");
                prop_assert!(
                    (pos.qty - expected_qty).abs() < 1e-10,
                    "qty mismatch for {}: {} vs {}", pos.symbol, pos.qty, expected_qty
                );
                prop_assert!(
                    (pos.avg_entry_price - expected_avg).abs() < 1e-10,
                    "avg_entry mismatch for {}: {} vs {}", pos.symbol, pos.avg_entry_price, expected_avg
                );
            }

            // Verify load_positions also reflects last-written values
            let loaded_positions = backend.load_positions().await.unwrap();
            prop_assert_eq!(loaded_positions.len(), expected_positions.len());
            for pos in &loaded_positions {
                let (expected_qty, expected_avg) = expected_positions.get(&pos.symbol)
                    .expect("unexpected symbol in load_positions");
                prop_assert!(
                    (pos.qty - expected_qty).abs() < 1e-10,
                    "qty mismatch for {}: {} vs {}", pos.symbol, pos.qty, expected_qty
                );
                prop_assert!(
                    (pos.avg_entry - expected_avg).abs() < 1e-10,
                    "avg_entry mismatch for {}: {} vs {}", pos.symbol, pos.avg_entry, expected_avg
                );
            }

            Ok(())
        })?;
    }
}

// ============================================================================
// Property 4: Schema name validation
// Feature: postgres-storage-backend, Property 4: Schema name validation
// **Validates: Requirements 4.4**
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn prop_valid_schema_names_accepted(name in "[a-z0-9_]{1,30}") {
        prop_assert!(
            flux_cli::live::storage::postgres::validate_schema_name(&name),
            "Valid schema name '{}' should be accepted", name
        );
    }

    #[test]
    fn prop_invalid_schema_names_rejected(name in prop_oneof![
        // Empty string
        Just(String::new()),
        // Contains uppercase
        "[a-z0-9_]*[A-Z][a-zA-Z0-9_]*",
        // Contains hyphen
        "[a-z0-9_]*-[a-z0-9_]*",
        // Contains space
        "[a-z0-9_]* [a-z0-9_]*",
        // Contains special chars
        "[a-z0-9_]*[!@#$%^&*()][a-z0-9_]*",
    ]) {
        prop_assert!(
            !flux_cli::live::storage::postgres::validate_schema_name(&name),
            "Invalid schema name '{}' should be rejected", name
        );
    }
}
