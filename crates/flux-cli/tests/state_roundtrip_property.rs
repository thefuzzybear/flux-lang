// Feature: flux-live-harness, Property 8: State persistence round-trip
//
// **Validates: Requirements 7.1, 7.2, 7.6**
//
// Property 8: State persistence round-trip
// For any valid HarnessState value, serializing to JSON via save_state()
// then deserializing via load_state() returns Some(state) equal to the original.

use proptest::prelude::*;
use tempfile::TempDir;

use flux_cli::live::state::{
    save_state, load_state, HarnessState, PositionState, SerializedPosition,
    StrategyState, SerializedValue,
};

/// Strategy for generating random uppercase symbols (1-5 chars).
fn arb_symbol() -> impl Strategy<Value = String> {
    proptest::string::string_regex("[A-Z]{1,5}").unwrap()
}

/// Strategy for generating f64 values that survive JSON round-trip.
/// We generate values with limited decimal digits to ensure exact
/// serialization/deserialization without the `float_roundtrip` feature.
fn json_roundtrip_f64(min: f64, max: f64) -> impl Strategy<Value = f64> {
    let min_cents = (min * 100.0) as i64;
    let max_cents = (max * 100.0) as i64;
    (min_cents..max_cents).prop_map(|cents| cents as f64 / 100.0)
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

/// Strategy for generating a single SerializedValue (recursive for List variant).
fn arb_serialized_value() -> impl Strategy<Value = SerializedValue> {
    let leaf = prop_oneof![
        any::<i64>().prop_map(SerializedValue::Int),
        json_roundtrip_f64(-100_000.0, 100_000.0).prop_map(SerializedValue::Float),
        "[a-zA-Z0-9_]{0,20}".prop_map(|s| SerializedValue::Str(s)),
        any::<bool>().prop_map(SerializedValue::Bool),
    ];

    // Use prop_recursive for the List variant to avoid infinite recursion
    leaf.prop_recursive(
        2,   // max depth
        8,   // max nodes
        4,   // items per collection
        |inner| {
            proptest::collection::vec(inner, 0..=3)
                .prop_map(SerializedValue::List)
        },
    )
}

/// Strategy for generating a StrategyState with random name, state variables, and indicator buffers.
fn arb_strategy_state() -> impl Strategy<Value = StrategyState> {
    (
        "[a-zA-Z][a-zA-Z0-9_]{0,15}",
        proptest::collection::vec(
            ("[a-z_]{1,10}", arb_serialized_value()),
            0..=5,
        ),
        proptest::collection::vec(
            (
                "[a-z_]{1,10}",
                proptest::collection::vec(json_roundtrip_f64(-10_000.0, 10_000.0), 0..=10),
            ),
            0..=3,
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

/// Strategy for generating a complete HarnessState (always version 2).
fn arb_harness_state() -> impl Strategy<Value = HarnessState> {
    (
        arb_position_state(),
        proptest::collection::vec(arb_strategy_state(), 0..=3),
        0..1000u64,
        "[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z",
        0..10000u64,
    )
        .prop_map(|(positions, strategy_states, fill_count, checkpoint_timestamp, bars_processed)| {
            HarnessState {
                version: 2,
                positions,
                strategy_states,
                fill_count,
                checkpoint_timestamp,
                bars_processed,
            }
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Property 8: save_state followed by load_state returns the original state.
    #[test]
    fn prop_state_persistence_roundtrip(state in arb_harness_state()) {
        let dir = TempDir::new().unwrap();
        let state_path = dir.path().join("harness_state.json");

        // Serialize to disk
        save_state(&state, &state_path).unwrap();

        // Deserialize from disk
        let loaded = load_state(&state_path).unwrap();

        // Must return Some and be equal to the original
        prop_assert!(
            loaded.is_some(),
            "load_state returned None after successful save_state"
        );
        prop_assert_eq!(
            loaded.unwrap(),
            state,
            "Deserialized state does not match original"
        );
    }
}
