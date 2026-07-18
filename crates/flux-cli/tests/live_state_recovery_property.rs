//! Property-based tests for Live State Recovery.
//!
//! Feature: live-state-recovery
//!
//! This file contains property tests validating the correctness properties
//! specified in the design document for crash recovery.
//!
//! **Validates: Requirements 1.2, 2.1, 2.2, 2.3, 2.4, 2.5, 2.6, 3.2, 3.3, 3.4, 4.2, 4.3, 6.1, 6.2, 6.3, 7.5**

use proptest::prelude::*;
use std::io::BufRead;
use tempfile::TempDir;

use flux_cli::live::fill_logger::{FillLogger, FillRecord};
use flux_cli::live::state::{
    save_state, load_state, HarnessState, PositionState, SerializedPosition,
    StrategyState, SerializedValue, STATE_VERSION,
};

// =============================================================================
// Generators
// =============================================================================

/// Strategy for generating f64 values that survive JSON round-trip.
/// We generate values with limited decimal digits to ensure exact
/// serialization/deserialization without the `float_roundtrip` feature.
fn json_roundtrip_f64(min: f64, max: f64) -> impl Strategy<Value = f64> {
    let min_cents = (min * 100.0) as i64;
    let max_cents = (max * 100.0) as i64;
    (min_cents..max_cents).prop_map(|cents| cents as f64 / 100.0)
}

/// Strategy for generating random uppercase symbols (1-5 chars).
fn arb_symbol() -> impl Strategy<Value = String> {
    proptest::string::string_regex("[A-Z]{1,5}").unwrap()
}

/// Strategy for generating a full SerializedValue including HashMap and Struct variants.
/// Uses prop_recursive for arbitrary nesting with depth capped at 4.
fn arb_full_serialized_value() -> impl Strategy<Value = SerializedValue> {
    let leaf = prop_oneof![
        any::<i64>().prop_map(SerializedValue::Int),
        json_roundtrip_f64(-100_000.0, 100_000.0).prop_map(SerializedValue::Float),
        "[a-zA-Z0-9_]{0,20}".prop_map(|s| SerializedValue::Str(s)),
        any::<bool>().prop_map(SerializedValue::Bool),
    ];

    leaf.prop_recursive(
        4,   // max depth
        16,  // max nodes
        4,   // items per collection
        |inner| {
            prop_oneof![
                // List variant
                proptest::collection::vec(inner.clone(), 0..=3)
                    .prop_map(SerializedValue::List),
                // HashMap variant
                proptest::collection::vec(
                    ("[a-z_]{1,10}", inner.clone()),
                    0..=3,
                ).prop_map(SerializedValue::HashMap),
                // Struct variant
                ("[A-Z][a-zA-Z]{0,10}", proptest::collection::vec(
                    ("[a-z_]{1,10}", inner),
                    0..=3,
                )).prop_map(|(type_name, fields)| SerializedValue::Struct { type_name, fields }),
            ]
        },
    )
}

/// Generate an arbitrary FillRecord with valid fields.
fn arb_fill_record() -> impl Strategy<Value = FillRecord> {
    (
        1..10000u64,                        // seq
        "[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\\.[0-9]{3}Z",  // timestamp
        "[A-Z]{1,5}",                       // symbol
        prop_oneof![Just("buy".to_string()), Just("sell".to_string())],  // side
        (1i64..10000i64).prop_map(|x| x as f64 / 100.0),  // qty (positive)
        (100i64..100000i64).prop_map(|x| x as f64 / 100.0),  // price (positive)
        "[A-Z][a-zA-Z]{0,15}",              // strategy
        0..10000u64,                        // bar_index
    )
        .prop_map(|(seq, timestamp, symbol, side, qty, price, strategy, bar_index)| {
            FillRecord { seq, timestamp, symbol, side, qty, price, strategy, bar_index }
        })
}

/// Strategy for generating a single SerializedPosition.
fn arb_position() -> impl Strategy<Value = SerializedPosition> {
    (
        arb_symbol(),
        json_roundtrip_f64(0.0, 10_000.0),
        json_roundtrip_f64(1.0, 5_000.0),
        json_roundtrip_f64(-10_000.0, 10_000.0),
    )
        .prop_map(|(symbol, qty, avg_entry_price, realized_pnl)| {
            SerializedPosition {
                symbol,
                qty,
                avg_entry_price,
                realized_pnl,
            }
        })
}

/// Strategy for generating a complete PositionState.
fn arb_position_state() -> impl Strategy<Value = PositionState> {
    (
        json_roundtrip_f64(1_000.0, 1_000_000.0),
        proptest::collection::vec(arb_position(), 0..=5),
        json_roundtrip_f64(-10_000.0, 10_000.0),
        proptest::collection::vec(
            (arb_symbol(), json_roundtrip_f64(1.0, 5_000.0)),
            0..=5,
        ),
    )
        .prop_map(|(initial_capital, positions, total_realized_pnl, last_prices)| {
            PositionState {
                initial_capital,
                positions,
                total_realized_pnl,
                last_prices,
            }
        })
}

/// Strategy for generating a StrategyState with state variables including HashMap/Struct.
fn arb_full_strategy_state() -> impl Strategy<Value = StrategyState> {
    (
        "[a-zA-Z][a-zA-Z0-9_]{0,15}",
        proptest::collection::vec(
            ("[a-z_]{1,10}", arb_full_serialized_value()),
            0..=10,
        ),
        proptest::collection::vec(
            (
                "[a-z_]{1,10}",
                proptest::collection::vec(json_roundtrip_f64(-10_000.0, 10_000.0), 0..=50),
            ),
            0..=5,
        ),
    )
        .prop_map(|(name, state_variables, indicator_buffers)| {
            StrategyState {
                name,
                state_variables,
                indicator_buffers,
            }
        })
}

/// Strategy for generating a complete HarnessState with full SerializedValue support.
fn arb_full_harness_state() -> impl Strategy<Value = HarnessState> {
    (
        arb_position_state(),
        proptest::collection::vec(arb_full_strategy_state(), 0..=3),
        0..1000u64,
        "[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z",
        0..10000u64,
    )
        .prop_map(|(positions, strategy_states, fill_count, checkpoint_timestamp, bars_processed)| {
            HarnessState {
                version: STATE_VERSION,
                positions,
                strategy_states,
                fill_count,
                checkpoint_timestamp,
                bars_processed,
            }
        })
}

// =============================================================================
// Property 1: Value Serialization Round-Trip
// Validates: Requirements 2.1, 2.2, 2.3, 2.4, 2.5, 2.6
//
// For any valid SerializedValue (including HashMap and Struct variants with
// arbitrary nesting), serializing to JSON and deserializing back produces an
// equal value.
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Feature: live-state-recovery, Property 1: Value Serialization Round-Trip
    ///
    /// **Validates: Requirements 2.1, 2.2, 2.3, 2.4, 2.5, 2.6**
    #[test]
    fn prop_serialized_value_roundtrip(value in arb_full_serialized_value()) {
        let json = serde_json::to_string(&value).unwrap();
        let deserialized: SerializedValue = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(deserialized, value);
    }
}

// =============================================================================
// Property 2: Full HarnessState Round-Trip
// Validates: Requirements 2.6, 3.2, 3.3, 3.4
//
// For any valid HarnessState (containing arbitrary positions, strategy states
// with HashMap/Struct variables, and indicator buffers), serializing to JSON
// via save_state and deserializing via load_state produces a state equal to
// the original.
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Feature: live-state-recovery, Property 2: Full HarnessState Round-Trip
    ///
    /// **Validates: Requirements 2.6, 3.2, 3.3, 3.4**
    #[test]
    fn prop_harness_state_roundtrip(state in arb_full_harness_state()) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.json");
        save_state(&state, &path).unwrap();
        let loaded = load_state(&path).unwrap().unwrap();
        prop_assert_eq!(loaded, state);
    }
}

// =============================================================================
// Property 3: Fill Record Serialization Completeness
// Validates: Requirements 1.2, 6.1, 6.2
//
// For any FillRecord with arbitrary valid field values, serializing it to a JSON
// line SHALL produce a single line (no embedded newlines) containing all required
// fields with their correct values recoverable by deserialization.
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Feature: live-state-recovery, Property 3: Fill Record Serialization Completeness
    ///
    /// **Validates: Requirements 1.2, 6.1, 6.2**
    #[test]
    fn prop_fill_record_serialization_completeness(record in arb_fill_record()) {
        // Serialize to JSON line
        let json_line = serde_json::to_string(&record).unwrap();

        // Must be a single line (no embedded newlines)
        prop_assert!(!json_line.contains('\n'), "serialized record contains newline");

        // Must be fully recoverable by deserialization
        let deserialized: FillRecord = serde_json::from_str(&json_line).unwrap();
        prop_assert_eq!(deserialized.seq, record.seq);
        prop_assert_eq!(&deserialized.timestamp, &record.timestamp);
        prop_assert_eq!(&deserialized.symbol, &record.symbol);
        prop_assert_eq!(&deserialized.side, &record.side);
        prop_assert_eq!(deserialized.qty, record.qty);
        prop_assert_eq!(deserialized.price, record.price);
        prop_assert_eq!(&deserialized.strategy, &record.strategy);
        prop_assert_eq!(deserialized.bar_index, record.bar_index);
    }
}

// =============================================================================
// Property 4: Fill Sequence Monotonicity
// Validates: Requirements 6.3
//
// For any sequence of fills appended to the FillLogger, the seq values in the
// resulting JSONL file SHALL be strictly monotonically increasing (each seq
// equals the previous seq + 1).
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Feature: live-state-recovery, Property 4: Fill Sequence Monotonicity
    ///
    /// **Validates: Requirements 6.3**
    #[test]
    fn prop_fill_sequence_monotonicity(records in proptest::collection::vec(arb_fill_record(), 1..=20)) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("fills.jsonl");

        // Append all records via FillLogger
        let mut logger = FillLogger::open(&path).unwrap();
        for record in &records {
            logger.append(record).unwrap();
        }
        drop(logger);

        // Read back and verify seq monotonicity
        let file = std::fs::File::open(&path).unwrap();
        let reader = std::io::BufReader::new(file);
        let mut prev_seq: Option<u64> = None;

        for line in reader.lines() {
            let line = line.unwrap();
            let parsed: FillRecord = serde_json::from_str(&line).unwrap();

            if let Some(prev) = prev_seq {
                prop_assert!(
                    parsed.seq == prev + 1,
                    "seq not monotonically increasing: prev={}, current={}",
                    prev,
                    parsed.seq
                );
            } else {
                prop_assert_eq!(parsed.seq, 1, "first seq should be 1");
            }
            prev_seq = Some(parsed.seq);
        }
    }
}

// =============================================================================
// Property 5: Fill Replay Correctness
// Validates: Requirements 4.2, 4.3, 7.5
//
// For any fill sequence of length M with a split point N (0 <= N <= M),
// replaying fills[N..M] on a tracker that already processed fills[0..N]
// SHALL produce the same position tracker state as processing all fills[0..M]
// from a fresh tracker.
// =============================================================================

use flux_cli::live::replay::FillReplayer;
use flux_cli::live::position::LivePositionTracker;

/// Generator for "buy-only" fill records.
///
/// All fills use side="buy" on random symbols to avoid the issue of selling
/// shares that haven't been purchased. This keeps the property test focused on
/// replay correctness rather than order-dependency of buy/sell sequences.
fn arb_buy_fill_record() -> impl Strategy<Value = FillRecord> {
    (
        1..10000u64,
        "[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\\.[0-9]{3}Z",
        "[A-Z]{1,5}",
        (1i64..1000i64).prop_map(|x| x as f64 / 100.0),   // qty: 0.01..10.0
        (100i64..10000i64).prop_map(|x| x as f64 / 100.0), // price: 1.0..100.0
        "[A-Z][a-zA-Z]{0,10}",
        0..10000u64,
    )
        .prop_map(|(seq, timestamp, symbol, qty, price, strategy, bar_index)| {
            FillRecord {
                seq,
                timestamp,
                symbol,
                side: "buy".to_string(),
                qty,
                price,
                strategy,
                bar_index,
            }
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Feature: live-state-recovery, Property 5: Fill Replay Correctness
    ///
    /// **Validates: Requirements 4.2, 4.3, 7.5**
    #[test]
    fn prop_fill_replay_correctness(
        fills in proptest::collection::vec(arb_buy_fill_record(), 1..=20),
        split_pct in 0..=100u32,
    ) {
        let m = fills.len();
        let n = ((split_pct as usize) * m) / 100;
        let n = n.min(m); // clamp to valid range

        // Assign correct sequential seq numbers
        let fills: Vec<FillRecord> = fills.into_iter().enumerate().map(|(i, mut f)| {
            f.seq = (i + 1) as u64;
            f
        }).collect();

        // Ground truth: process ALL fills from a fresh tracker
        let mut full_tracker = LivePositionTracker::new(1_000_000.0);
        FillReplayer::replay_fills(&fills, &mut full_tracker);

        // Checkpoint state: process fills[0..N] from fresh
        let mut checkpoint_tracker = LivePositionTracker::new(1_000_000.0);
        if n > 0 {
            FillReplayer::replay_fills(&fills[..n], &mut checkpoint_tracker);
        }

        // Replay fills[N..M] on the checkpoint tracker
        if n < m {
            FillReplayer::replay_fills(&fills[n..], &mut checkpoint_tracker);
        }

        // Verify: position states must match

        // 1. Equity must be equal (captures capital + realized + unrealized)
        let full_equity = full_tracker.inner.equity();
        let replay_equity = checkpoint_tracker.inner.equity();
        prop_assert!(
            (full_equity - replay_equity).abs() < 0.01,
            "equity mismatch: full={}, replay={}, split at N={} of M={}",
            full_equity, replay_equity, n, m
        );

        // 2. Open position count must match
        let full_open = full_tracker.inner.open_position_count();
        let replay_open = checkpoint_tracker.inner.open_position_count();
        prop_assert_eq!(
            full_open, replay_open,
            "open position count mismatch: full={}, replay={}, split at N={} of M={}",
            full_open, replay_open, n, m
        );

        // 3. Number of fills processed must match
        let full_fills = full_tracker.inner.fills().len();
        let replay_fills_count = checkpoint_tracker.inner.fills().len();
        prop_assert_eq!(
            full_fills, replay_fills_count,
            "fill count mismatch: full={}, replay={}, split at N={} of M={}",
            full_fills, replay_fills_count, n, m
        );

        // 4. Each position's qty must match
        let full_positions = full_tracker.inner.positions();
        let replay_positions = checkpoint_tracker.inner.positions();
        for (symbol, full_pos) in full_positions.iter() {
            let replay_pos = replay_positions.get(symbol);
            prop_assert!(
                replay_pos.is_some(),
                "symbol '{}' missing in replay tracker, split at N={} of M={}",
                symbol, n, m
            );
            let replay_pos = replay_pos.unwrap();
            prop_assert!(
                (full_pos.qty - replay_pos.qty).abs() < 0.001,
                "qty mismatch for '{}': full={}, replay={}, split at N={} of M={}",
                symbol, full_pos.qty, replay_pos.qty, n, m
            );
        }
    }
}
