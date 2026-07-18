//! State persistence for the live harness.
//!
//! Serializes and deserializes harness state (positions, strategy state,
//! indicator buffers) to disk. Uses atomic write (tmp + rename) to prevent
//! corruption and includes a version field for forward compatibility.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Current state file format version.
/// Increment when making breaking changes to the serialized format.
pub const STATE_VERSION: u32 = 2;

/// Complete serializable harness state.
#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct HarnessState {
    /// Format version for compatibility detection.
    pub version: u32,
    /// Position tracker state (positions, P&L, prices).
    pub positions: PositionState,
    /// Per-strategy interpreter state (state variables, indicator buffers).
    pub strategy_states: Vec<StrategyState>,
    /// Total fills processed at this checkpoint.
    pub fill_count: u64,
    /// ISO 8601 timestamp of when this checkpoint was taken.
    pub checkpoint_timestamp: String,
    /// Total bars processed since the session started.
    pub bars_processed: u64,
}

/// Serialized state of the position tracker.
#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct PositionState {
    /// Starting capital for the portfolio.
    pub initial_capital: f64,
    /// All open and historical positions.
    pub positions: Vec<SerializedPosition>,
    /// Cumulative realized P&L across all closed positions.
    pub total_realized_pnl: f64,
    /// Last known prices per symbol (for mark-to-market on restore).
    pub last_prices: Vec<(String, f64)>,
}

/// A single position serialized for persistence.
#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct SerializedPosition {
    /// Trading symbol (e.g. "AAPL").
    pub symbol: String,
    /// Current quantity held.
    pub qty: f64,
    /// Volume-weighted average entry price.
    pub avg_entry_price: f64,
    /// Realized P&L for this position.
    pub realized_pnl: f64,
}

/// Per-strategy state for persistence across restarts.
#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct StrategyState {
    /// Strategy name (from the strategy declaration).
    pub name: String,
    /// User-defined state variables and their current values.
    pub state_variables: Vec<(String, SerializedValue)>,
    /// Indicator buffer contents (e.g. SMA window values).
    pub indicator_buffers: Vec<(String, Vec<f64>)>,
}

/// A dynamically-typed value that can be serialized.
#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub enum SerializedValue {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    List(Vec<SerializedValue>),
    /// HashMap with string keys and recursively serialized values.
    HashMap(Vec<(String, SerializedValue)>),
    /// Struct with type name and named fields.
    Struct {
        type_name: String,
        fields: Vec<(String, SerializedValue)>,
    },
}

/// Errors that can occur during state persistence operations.
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("serialization failed: {0}")]
    Serialize(String),
    #[error("deserialization failed: {0}")]
    Deserialize(String),
    #[error("I/O error: {0}")]
    Io(String),
    #[error("incompatible state version: found {found}, expected {expected}")]
    IncompatibleVersion { found: u32, expected: u32 },
}

/// Persist harness state atomically to disk.
///
/// Writes to a temporary file first, then renames into place. This ensures
/// that a crash mid-write won't corrupt the state file — the previous version
/// remains intact until the rename completes.
pub fn save_state(state: &HarnessState, path: &Path) -> Result<(), StateError> {
    let json =
        serde_json::to_string_pretty(state).map_err(|e| StateError::Serialize(e.to_string()))?;

    let tmp_path = path.with_extension("tmp");
    std::fs::write(&tmp_path, &json).map_err(|e| StateError::Io(e.to_string()))?;
    std::fs::rename(&tmp_path, path).map_err(|e| StateError::Io(e.to_string()))?;

    Ok(())
}

/// Restore harness state from disk.
///
/// Returns `Ok(None)` if the file doesn't exist (fresh start).
/// Returns `Err` on corruption (invalid JSON) or version mismatch.
/// The caller should handle errors by logging a warning and starting fresh
/// rather than crashing.
pub fn load_state(path: &Path) -> Result<Option<HarnessState>, StateError> {
    if !path.exists() {
        return Ok(None);
    }

    let json = std::fs::read_to_string(path).map_err(|e| StateError::Io(e.to_string()))?;

    let state: HarnessState =
        serde_json::from_str(&json).map_err(|e| StateError::Deserialize(e.to_string()))?;

    if state.version != STATE_VERSION {
        return Err(StateError::IncompatibleVersion {
            found: state.version,
            expected: STATE_VERSION,
        });
    }

    Ok(Some(state))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Helper to create a sample HarnessState for testing.
    fn sample_state() -> HarnessState {
        HarnessState {
            version: STATE_VERSION,
            positions: PositionState {
                initial_capital: 10_000.0,
                positions: vec![SerializedPosition {
                    symbol: "AAPL".to_string(),
                    qty: 100.0,
                    avg_entry_price: 150.25,
                    realized_pnl: 0.0,
                }],
                total_realized_pnl: 500.0,
                last_prices: vec![
                    ("AAPL".to_string(), 155.0),
                    ("MSFT".to_string(), 380.0),
                ],
            },
            strategy_states: vec![StrategyState {
                name: "MeanReversion".to_string(),
                state_variables: vec![
                    ("bar_count".to_string(), SerializedValue::Int(42)),
                    ("threshold".to_string(), SerializedValue::Float(2.5)),
                    ("active".to_string(), SerializedValue::Bool(true)),
                    ("mode".to_string(), SerializedValue::Str("aggressive".to_string())),
                ],
                indicator_buffers: vec![(
                    "sma_20".to_string(),
                    vec![150.0, 151.0, 152.0, 153.0, 154.0],
                )],
            }],
            fill_count: 5,
            checkpoint_timestamp: "2024-06-15T14:30:00.000Z".to_string(),
            bars_processed: 100,
        }
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = TempDir::new().unwrap();
        let state_path = dir.path().join("harness_state.json");

        let original = sample_state();
        save_state(&original, &state_path).unwrap();

        let loaded = load_state(&state_path).unwrap();
        assert_eq!(loaded, Some(original));
    }

    #[test]
    fn load_missing_file_returns_none() {
        let dir = TempDir::new().unwrap();
        let state_path = dir.path().join("nonexistent.json");

        let result = load_state(&state_path).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn load_incompatible_version_returns_error() {
        let dir = TempDir::new().unwrap();
        let state_path = dir.path().join("old_state.json");

        let mut state = sample_state();
        state.version = 99; // Future version
        let json = serde_json::to_string_pretty(&state).unwrap();
        fs::write(&state_path, &json).unwrap();

        let result = load_state(&state_path);
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            StateError::IncompatibleVersion { found, expected } => {
                assert_eq!(found, 99);
                assert_eq!(expected, STATE_VERSION);
            }
            other => panic!("expected IncompatibleVersion, got: {:?}", other),
        }
    }

    #[test]
    fn load_corrupted_file_returns_deserialize_error() {
        let dir = TempDir::new().unwrap();
        let state_path = dir.path().join("corrupt.json");

        fs::write(&state_path, "not valid json {{{{").unwrap();

        let result = load_state(&state_path);
        assert!(result.is_err());
        match result.unwrap_err() {
            StateError::Deserialize(_) => {} // expected
            other => panic!("expected Deserialize error, got: {:?}", other),
        }
    }

    #[test]
    fn save_creates_parent_directory_content() {
        let dir = TempDir::new().unwrap();
        let state_path = dir.path().join("harness_state.json");

        let state = sample_state();
        save_state(&state, &state_path).unwrap();

        // Verify the file contains valid JSON
        let content = fs::read_to_string(&state_path).unwrap();
        let parsed: HarnessState = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed.version, STATE_VERSION);
    }

    #[test]
    fn save_atomic_no_tmp_file_remains() {
        let dir = TempDir::new().unwrap();
        let state_path = dir.path().join("harness_state.json");
        let tmp_path = state_path.with_extension("tmp");

        let state = sample_state();
        save_state(&state, &state_path).unwrap();

        // The .tmp file should not remain after successful save
        assert!(!tmp_path.exists());
        assert!(state_path.exists());
    }

    #[test]
    fn serialized_value_variants_roundtrip() {
        let dir = TempDir::new().unwrap();
        let state_path = dir.path().join("values.json");

        let state = HarnessState {
            version: STATE_VERSION,
            positions: PositionState {
                initial_capital: 5000.0,
                positions: vec![],
                total_realized_pnl: 0.0,
                last_prices: vec![],
            },
            strategy_states: vec![StrategyState {
                name: "TestStrategy".to_string(),
                state_variables: vec![
                    ("int_val".to_string(), SerializedValue::Int(-10)),
                    ("float_val".to_string(), SerializedValue::Float(3.14)),
                    ("str_val".to_string(), SerializedValue::Str("hello".to_string())),
                    ("bool_val".to_string(), SerializedValue::Bool(false)),
                    (
                        "list_val".to_string(),
                        SerializedValue::List(vec![
                            SerializedValue::Int(1),
                            SerializedValue::Float(2.0),
                            SerializedValue::Str("three".to_string()),
                        ]),
                    ),
                ],
                indicator_buffers: vec![],
            }],
            fill_count: 0,
            checkpoint_timestamp: "2024-01-01T00:00:00.000Z".to_string(),
            bars_processed: 0,
        };

        save_state(&state, &state_path).unwrap();
        let loaded = load_state(&state_path).unwrap().unwrap();
        assert_eq!(loaded, state);
    }

    #[test]
    fn hashmap_serialization_roundtrip() {
        let dir = TempDir::new().unwrap();
        let state_path = dir.path().join("hashmap_state.json");

        let state = HarnessState {
            version: STATE_VERSION,
            positions: PositionState {
                initial_capital: 10_000.0,
                positions: vec![],
                total_realized_pnl: 0.0,
                last_prices: vec![],
            },
            strategy_states: vec![StrategyState {
                name: "Kairos".to_string(),
                state_variables: vec![
                    (
                        "pivot_levels".to_string(),
                        SerializedValue::HashMap(vec![
                            ("AAPL".to_string(), SerializedValue::Float(185.0)),
                            ("MSFT".to_string(), SerializedValue::Float(380.0)),
                            ("GOOG".to_string(), SerializedValue::Float(140.5)),
                        ]),
                    ),
                    (
                        "scores".to_string(),
                        SerializedValue::HashMap(vec![
                            ("AAPL".to_string(), SerializedValue::Int(3)),
                            ("MSFT".to_string(), SerializedValue::Int(-1)),
                        ]),
                    ),
                    (
                        "nested_map".to_string(),
                        SerializedValue::HashMap(vec![
                            (
                                "inner".to_string(),
                                SerializedValue::List(vec![
                                    SerializedValue::Float(1.0),
                                    SerializedValue::Float(2.0),
                                ]),
                            ),
                            ("flag".to_string(), SerializedValue::Bool(true)),
                            ("label".to_string(), SerializedValue::Str("test".to_string())),
                        ]),
                    ),
                ],
                indicator_buffers: vec![],
            }],
            fill_count: 0,
            checkpoint_timestamp: "2024-01-01T00:00:00Z".to_string(),
            bars_processed: 0,
        };

        save_state(&state, &state_path).unwrap();
        let loaded = load_state(&state_path).unwrap().unwrap();
        assert_eq!(loaded, state);
    }

    #[test]
    fn struct_serialization_roundtrip() {
        let dir = TempDir::new().unwrap();
        let state_path = dir.path().join("struct_state.json");

        let state = HarnessState {
            version: STATE_VERSION,
            positions: PositionState {
                initial_capital: 10_000.0,
                positions: vec![],
                total_realized_pnl: 0.0,
                last_prices: vec![],
            },
            strategy_states: vec![StrategyState {
                name: "PairsTrading".to_string(),
                state_variables: vec![
                    (
                        "pair_state".to_string(),
                        SerializedValue::Struct {
                            type_name: "PairState".to_string(),
                            fields: vec![
                                ("mean_spread".to_string(), SerializedValue::Float(2.35)),
                                ("z_score".to_string(), SerializedValue::Float(-1.8)),
                                ("lookback".to_string(), SerializedValue::Int(20)),
                                ("active".to_string(), SerializedValue::Bool(true)),
                                ("name".to_string(), SerializedValue::Str("AAPL_MSFT".to_string())),
                            ],
                        },
                    ),
                    (
                        "config".to_string(),
                        SerializedValue::Struct {
                            type_name: "Config".to_string(),
                            fields: vec![
                                ("threshold".to_string(), SerializedValue::Float(2.0)),
                                ("max_positions".to_string(), SerializedValue::Int(5)),
                                (
                                    "symbols".to_string(),
                                    SerializedValue::List(vec![
                                        SerializedValue::Str("AAPL".to_string()),
                                        SerializedValue::Str("MSFT".to_string()),
                                    ]),
                                ),
                            ],
                        },
                    ),
                ],
                indicator_buffers: vec![("spread_sma".to_string(), vec![1.0, 1.5, 2.0, 2.35])],
            }],
            fill_count: 10,
            checkpoint_timestamp: "2024-06-15T14:30:00.000Z".to_string(),
            bars_processed: 200,
        };

        save_state(&state, &state_path).unwrap();
        let loaded = load_state(&state_path).unwrap().unwrap();
        assert_eq!(loaded, state);
    }

    #[test]
    fn deeply_nested_combinations_roundtrip() {
        let dir = TempDir::new().unwrap();
        let state_path = dir.path().join("nested_state.json");

        // HashMap containing Struct containing List
        let hashmap_with_struct = SerializedValue::HashMap(vec![
            (
                "AAPL".to_string(),
                SerializedValue::Struct {
                    type_name: "SymbolState".to_string(),
                    fields: vec![
                        ("pivot".to_string(), SerializedValue::Float(185.0)),
                        (
                            "history".to_string(),
                            SerializedValue::List(vec![
                                SerializedValue::Float(180.0),
                                SerializedValue::Float(182.5),
                                SerializedValue::Float(185.0),
                            ]),
                        ),
                        ("count".to_string(), SerializedValue::Int(3)),
                    ],
                },
            ),
            (
                "MSFT".to_string(),
                SerializedValue::Struct {
                    type_name: "SymbolState".to_string(),
                    fields: vec![
                        ("pivot".to_string(), SerializedValue::Float(380.0)),
                        (
                            "history".to_string(),
                            SerializedValue::List(vec![
                                SerializedValue::Float(375.0),
                                SerializedValue::Float(378.0),
                            ]),
                        ),
                        ("count".to_string(), SerializedValue::Int(2)),
                    ],
                },
            ),
        ]);

        // Struct containing HashMap
        let struct_with_hashmap = SerializedValue::Struct {
            type_name: "PortfolioState".to_string(),
            fields: vec![
                (
                    "weights".to_string(),
                    SerializedValue::HashMap(vec![
                        ("AAPL".to_string(), SerializedValue::Float(0.4)),
                        ("MSFT".to_string(), SerializedValue::Float(0.6)),
                    ]),
                ),
                ("total_value".to_string(), SerializedValue::Float(50_000.0)),
                ("rebalanced".to_string(), SerializedValue::Bool(false)),
            ],
        };

        // List containing HashMap and Struct
        let list_with_mixed = SerializedValue::List(vec![
            SerializedValue::HashMap(vec![
                ("key1".to_string(), SerializedValue::Int(100)),
                ("key2".to_string(), SerializedValue::Str("value".to_string())),
            ]),
            SerializedValue::Struct {
                type_name: "Entry".to_string(),
                fields: vec![
                    ("id".to_string(), SerializedValue::Int(1)),
                    ("active".to_string(), SerializedValue::Bool(true)),
                ],
            },
            SerializedValue::Float(42.0),
        ]);

        // Deeply nested: HashMap → Struct → HashMap → List
        let deep_nested = SerializedValue::HashMap(vec![(
            "strategy_registry".to_string(),
            SerializedValue::Struct {
                type_name: "Registry".to_string(),
                fields: vec![(
                    "entries".to_string(),
                    SerializedValue::HashMap(vec![(
                        "momentum".to_string(),
                        SerializedValue::List(vec![
                            SerializedValue::Float(0.5),
                            SerializedValue::Float(0.75),
                            SerializedValue::Float(1.0),
                        ]),
                    )]),
                )],
            },
        )]);

        let state = HarnessState {
            version: STATE_VERSION,
            positions: PositionState {
                initial_capital: 100_000.0,
                positions: vec![SerializedPosition {
                    symbol: "AAPL".to_string(),
                    qty: 200.0,
                    avg_entry_price: 182.0,
                    realized_pnl: 0.0,
                }],
                total_realized_pnl: 1500.0,
                last_prices: vec![
                    ("AAPL".to_string(), 185.0),
                    ("MSFT".to_string(), 380.0),
                ],
            },
            strategy_states: vec![StrategyState {
                name: "MultiStrategy".to_string(),
                state_variables: vec![
                    ("symbol_states".to_string(), hashmap_with_struct),
                    ("portfolio".to_string(), struct_with_hashmap),
                    ("history".to_string(), list_with_mixed),
                    ("registry".to_string(), deep_nested),
                ],
                indicator_buffers: vec![
                    ("sma_20".to_string(), vec![150.0, 151.0, 152.0]),
                    ("ema_12".to_string(), vec![149.5, 150.5]),
                ],
            }],
            fill_count: 25,
            checkpoint_timestamp: "2024-06-15T15:00:00.000Z".to_string(),
            bars_processed: 500,
        };

        save_state(&state, &state_path).unwrap();
        let loaded = load_state(&state_path).unwrap().unwrap();
        assert_eq!(loaded, state);
    }
}
