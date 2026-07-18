//! Integration test: State persistence and restoration.
//!
//! Validates Requirements 7.2 (restore positions, strategy state, indicator buffers)
//! and 7.6 (persist unified position tracker state and per-strategy interpreter state).
//!
//! Tests:
//! 1. Save state with known positions and strategy state, load it back, verify match
//! 2. Verify indicator buffers are restored correctly
//! 3. Corrupt state file → load_state returns Err
//! 4. Version mismatch (version=99) → load_state returns Err
//! 5. Missing file → load_state returns Ok(None)

use flux_cli::live::state::{
    save_state, load_state, HarnessState, PositionState, SerializedPosition, StrategyState,
    SerializedValue, StateError,
};
use tempfile::TempDir;

/// Build a HarnessState with known positions and strategy state for testing.
fn build_test_state() -> HarnessState {
    HarnessState {
        version: 2, // STATE_VERSION
        positions: PositionState {
            initial_capital: 50_000.0,
            positions: vec![
                SerializedPosition {
                    symbol: "AAPL".to_string(),
                    qty: 200.0,
                    avg_entry_price: 175.50,
                    realized_pnl: 320.0,
                },
                SerializedPosition {
                    symbol: "MSFT".to_string(),
                    qty: 150.0,
                    avg_entry_price: 390.25,
                    realized_pnl: -45.0,
                },
            ],
            total_realized_pnl: 275.0,
            last_prices: vec![
                ("AAPL".to_string(), 180.0),
                ("MSFT".to_string(), 385.0),
            ],
        },
        strategy_states: vec![
            StrategyState {
                name: "MomentumAlpha".to_string(),
                state_variables: vec![
                    ("bar_count".to_string(), SerializedValue::Int(1042)),
                    ("threshold".to_string(), SerializedValue::Float(1.5)),
                    ("is_active".to_string(), SerializedValue::Bool(true)),
                    ("mode".to_string(), SerializedValue::Str("aggressive".to_string())),
                    (
                        "targets".to_string(),
                        SerializedValue::List(vec![
                            SerializedValue::Float(180.0),
                            SerializedValue::Float(200.0),
                        ]),
                    ),
                ],
                indicator_buffers: vec![
                    ("sma_20".to_string(), vec![170.0, 171.5, 172.3, 173.8, 174.2]),
                    ("ema_10".to_string(), vec![175.0, 176.1, 177.3]),
                ],
            },
            StrategyState {
                name: "MeanRevert".to_string(),
                state_variables: vec![
                    ("bar_count".to_string(), SerializedValue::Int(500)),
                    ("z_score".to_string(), SerializedValue::Float(-1.2)),
                ],
                indicator_buffers: vec![
                    ("sma_50".to_string(), vec![160.0, 161.0, 162.0, 163.0]),
                ],
            },
        ],
        fill_count: 10,
        checkpoint_timestamp: "2024-06-15T14:30:00.000Z".to_string(),
        bars_processed: 200,
    }
}

#[tokio::test]
async fn save_and_restore_positions_match() {
    let dir = TempDir::new().unwrap();
    let state_path = dir.path().join("state.json");

    let original = build_test_state();
    save_state(&original, &state_path).unwrap();

    let restored = load_state(&state_path).unwrap().expect("state should exist");

    // Verify positions match
    assert_eq!(restored.positions.positions.len(), 2);

    let aapl = &restored.positions.positions[0];
    assert_eq!(aapl.symbol, "AAPL");
    assert_eq!(aapl.qty, 200.0);
    assert_eq!(aapl.avg_entry_price, 175.50);
    assert_eq!(aapl.realized_pnl, 320.0);

    let msft = &restored.positions.positions[1];
    assert_eq!(msft.symbol, "MSFT");
    assert_eq!(msft.qty, 150.0);
    assert_eq!(msft.avg_entry_price, 390.25);
    assert_eq!(msft.realized_pnl, -45.0);

    // Verify portfolio-level state
    assert_eq!(restored.positions.initial_capital, 50_000.0);
    assert_eq!(restored.positions.total_realized_pnl, 275.0);
    assert_eq!(restored.positions.last_prices.len(), 2);
}

#[tokio::test]
async fn save_and_restore_strategy_state_variables_match() {
    let dir = TempDir::new().unwrap();
    let state_path = dir.path().join("state.json");

    let original = build_test_state();
    save_state(&original, &state_path).unwrap();

    let restored = load_state(&state_path).unwrap().expect("state should exist");

    // Verify strategy state variables for MomentumAlpha
    assert_eq!(restored.strategy_states.len(), 2);
    let momentum = &restored.strategy_states[0];
    assert_eq!(momentum.name, "MomentumAlpha");
    assert_eq!(momentum.state_variables.len(), 5);

    // Check each state variable type and value
    assert_eq!(
        momentum.state_variables[0],
        ("bar_count".to_string(), SerializedValue::Int(1042))
    );
    assert_eq!(
        momentum.state_variables[1],
        ("threshold".to_string(), SerializedValue::Float(1.5))
    );
    assert_eq!(
        momentum.state_variables[2],
        ("is_active".to_string(), SerializedValue::Bool(true))
    );
    assert_eq!(
        momentum.state_variables[3],
        ("mode".to_string(), SerializedValue::Str("aggressive".to_string()))
    );
    assert_eq!(
        momentum.state_variables[4],
        (
            "targets".to_string(),
            SerializedValue::List(vec![
                SerializedValue::Float(180.0),
                SerializedValue::Float(200.0),
            ])
        )
    );

    // Verify MeanRevert strategy state
    let mean_revert = &restored.strategy_states[1];
    assert_eq!(mean_revert.name, "MeanRevert");
    assert_eq!(mean_revert.state_variables.len(), 2);
    assert_eq!(
        mean_revert.state_variables[0],
        ("bar_count".to_string(), SerializedValue::Int(500))
    );
    assert_eq!(
        mean_revert.state_variables[1],
        ("z_score".to_string(), SerializedValue::Float(-1.2))
    );
}

#[tokio::test]
async fn save_and_restore_indicator_buffers_match() {
    let dir = TempDir::new().unwrap();
    let state_path = dir.path().join("state.json");

    let original = build_test_state();
    save_state(&original, &state_path).unwrap();

    let restored = load_state(&state_path).unwrap().expect("state should exist");

    // Verify MomentumAlpha indicator buffers
    let momentum = &restored.strategy_states[0];
    assert_eq!(momentum.indicator_buffers.len(), 2);

    let (sma_name, sma_buf) = &momentum.indicator_buffers[0];
    assert_eq!(sma_name, "sma_20");
    assert_eq!(sma_buf, &vec![170.0, 171.5, 172.3, 173.8, 174.2]);

    let (ema_name, ema_buf) = &momentum.indicator_buffers[1];
    assert_eq!(ema_name, "ema_10");
    assert_eq!(ema_buf, &vec![175.0, 176.1, 177.3]);

    // Verify MeanRevert indicator buffers
    let mean_revert = &restored.strategy_states[1];
    assert_eq!(mean_revert.indicator_buffers.len(), 1);

    let (sma50_name, sma50_buf) = &mean_revert.indicator_buffers[0];
    assert_eq!(sma50_name, "sma_50");
    assert_eq!(sma50_buf, &vec![160.0, 161.0, 162.0, 163.0]);
}

#[tokio::test]
async fn load_corrupt_state_file_returns_error() {
    let dir = TempDir::new().unwrap();
    let state_path = dir.path().join("corrupt.json");

    // Write invalid JSON to the state file
    std::fs::write(&state_path, "{{not valid json at all!!! %%% ~~~").unwrap();

    let result = load_state(&state_path);
    assert!(result.is_err());
    match result.unwrap_err() {
        StateError::Deserialize(msg) => {
            // Should contain some indication of parse failure
            assert!(!msg.is_empty(), "error message should be non-empty");
        }
        other => panic!("expected Deserialize error, got: {:?}", other),
    }
}

#[tokio::test]
async fn load_version_mismatch_returns_error() {
    let dir = TempDir::new().unwrap();
    let state_path = dir.path().join("future_version.json");

    // Create a state with version=99 (incompatible future version)
    let mut state = build_test_state();
    state.version = 99;

    // Manually serialize (bypass save_state which doesn't validate version on write)
    let json = serde_json::to_string_pretty(&state).unwrap();
    std::fs::write(&state_path, &json).unwrap();

    let result = load_state(&state_path);
    assert!(result.is_err());
    match result.unwrap_err() {
        StateError::IncompatibleVersion { found, expected } => {
            assert_eq!(found, 99);
            assert_eq!(expected, 2); // STATE_VERSION = 2
        }
        other => panic!("expected IncompatibleVersion error, got: {:?}", other),
    }
}

#[tokio::test]
async fn load_missing_file_returns_ok_none() {
    let dir = TempDir::new().unwrap();
    let state_path = dir.path().join("does_not_exist.json");

    let result = load_state(&state_path).unwrap();
    assert_eq!(result, None);
}

#[tokio::test]
async fn full_roundtrip_preserves_complete_state() {
    let dir = TempDir::new().unwrap();
    let state_path = dir.path().join("full_roundtrip.json");

    let original = build_test_state();
    save_state(&original, &state_path).unwrap();

    let restored = load_state(&state_path).unwrap().expect("state should exist");

    // Full equality check — positions, strategy states, and indicator buffers
    assert_eq!(original, restored);
}
