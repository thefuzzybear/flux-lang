//! Integration test: TOML config parsing for multi-strategy live mode.
//!
//! Tests that:
//! 1. Valid TOML with multiple strategies and connectors parses correctly
//! 2. Invalid (malformed) TOML reports actionable error
//! 3. TOML with missing strategy files → partial loading continues and errors report the missing file
//! 4. TOML with all missing strategy files → error listing all failures
//! 5. `build_connectors()` with valid connector configs → correct connector types created
//! 6. `build_connectors()` with invalid config (missing required fields) → error
//!
//! **Validates: Requirements 2.1, 5.1**

use std::fs;
use tempfile::TempDir;

use flux_cli::live::loader::{
    build_connectors, load_strategies, ConnectorConfig,
};

/// A minimal valid strategy source for testing.
const STRATEGY_ALPHA: &str = r#"strategy Alpha {
    params {
        threshold = 1.5
    }
    on bar {
        if close > open {
            OPEN(symbol, 50.0)
        }
    }
}"#;

const STRATEGY_BETA: &str = r#"strategy Beta {
    on bar {
        if close < open and in_position {
            CLOSE(symbol)
        }
    }
}"#;

/// Helper: create a temp directory with strategy files and a TOML config.
fn setup_valid_multi_strategy_dir() -> TempDir {
    let dir = tempfile::tempdir().unwrap();

    // Write .flux strategy files
    fs::write(dir.path().join("alpha.flux"), STRATEGY_ALPHA).unwrap();
    fs::write(dir.path().join("beta.flux"), STRATEGY_BETA).unwrap();

    // Write a valid TOML config referencing both strategies
    let config = r#"
capital = 50000.0
state_file = "harness_state.json"

[risk]
max_position_size = 1000.0
max_exposure = 100000.0
max_positions = 5

[[strategies]]
path = "alpha.flux"
symbols = ["AAPL", "MSFT"]

[[strategies]]
path = "beta.flux"
symbols = ["GOOG"]

[[connectors]]
kind = "replay"
file = "test_data.csv"
symbols = ["AAPL", "MSFT", "GOOG"]
playback_rate = 0.0

[[connectors]]
kind = "websocket"
url = "wss://stream.example.com/v1"
symbols = ["AAPL", "MSFT"]
"#;
    fs::write(dir.path().join("config.toml"), config).unwrap();

    dir
}

/// Test 1: Valid TOML with multiple strategies and connectors parses correctly.
///
/// Validates: Requirement 2.1 — THE Live_Harness SHALL accept a configuration
/// specifying one or more Strategy_Module file paths to load.
#[test]
fn valid_toml_loads_multiple_strategies_with_correct_metadata() {
    let dir = setup_valid_multi_strategy_dir();
    let config_path = dir.path().join("config.toml");

    let result = load_strategies(&config_path);
    assert!(result.is_ok(), "expected successful load, got: {:?}", result.err());

    let modules = result.unwrap();
    assert_eq!(modules.len(), 2, "expected 2 strategy modules");

    // Verify first strategy
    assert_eq!(modules[0].name, "Alpha");
    assert_eq!(modules[0].subscribed_symbols, vec!["AAPL", "MSFT"]);

    // Verify second strategy
    assert_eq!(modules[1].name, "Beta");
    assert_eq!(modules[1].subscribed_symbols, vec!["GOOG"]);
}

/// Test 2: Invalid (malformed) TOML reports actionable error.
///
/// Validates: Requirement 5.1 — THE Live_Command SHALL accept a positional
/// argument specifying a configuration file path (must parse correctly).
#[test]
fn invalid_toml_reports_actionable_error() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("broken.toml");

    // Write malformed TOML (unclosed bracket, missing values)
    let malformed = r#"
[[strategies]
path = "alpha.flux"
symbols = ["AAPL"
"#;
    fs::write(&config_path, malformed).unwrap();

    let result = load_strategies(&config_path);
    assert!(result.is_err(), "expected error for malformed TOML");

    let errors = result.unwrap_err();
    assert_eq!(errors.len(), 1);

    // The error message should mention TOML parsing failure and be actionable
    // (i.e., it should indicate what went wrong so the user can fix it)
    let msg = &errors[0].message;
    assert!(
        msg.contains("failed to parse TOML config"),
        "error should mention TOML parsing: got '{}'",
        msg
    );
    // Should contain line/column or description of the syntax issue
    assert!(
        msg.len() > 30,
        "error should be descriptive/actionable, got: '{}'",
        msg
    );
}

/// Test 3: TOML with some missing strategy files → partial loading continues
/// and errors report the missing file path.
///
/// Validates: Requirement 2.1 (partial loading behavior from 2.5)
#[test]
fn toml_with_missing_strategy_files_continues_loading_others() {
    let dir = tempfile::tempdir().unwrap();

    // Only write one of the two strategy files
    fs::write(dir.path().join("alpha.flux"), STRATEGY_ALPHA).unwrap();
    // "missing.flux" is intentionally not created

    let config = r#"
[[strategies]]
path = "alpha.flux"
symbols = ["AAPL"]

[[strategies]]
path = "missing.flux"
symbols = ["MSFT"]

[[connectors]]
kind = "replay"
file = "data.csv"
symbols = ["AAPL", "MSFT"]
"#;
    fs::write(dir.path().join("config.toml"), config).unwrap();

    let config_path = dir.path().join("config.toml");
    let result = load_strategies(&config_path);

    // Should succeed with the valid strategy loaded
    assert!(result.is_ok(), "expected partial success, got: {:?}", result.err());

    let modules = result.unwrap();
    assert_eq!(modules.len(), 1, "expected 1 module (the valid one)");
    assert_eq!(modules[0].name, "Alpha");
    assert_eq!(modules[0].subscribed_symbols, vec!["AAPL"]);
}

/// Test 4: TOML with ALL missing strategy files → error listing all failures.
///
/// Validates: Requirement 2.1 (all-fail behavior from 2.6)
#[test]
fn toml_with_all_missing_strategies_returns_error_listing_all_failures() {
    let dir = tempfile::tempdir().unwrap();

    // No strategy files exist
    let config = r#"
[[strategies]]
path = "nonexistent_a.flux"
symbols = ["AAPL"]

[[strategies]]
path = "nonexistent_b.flux"
symbols = ["MSFT"]

[[strategies]]
path = "nonexistent_c.flux"
symbols = ["GOOG"]

[[connectors]]
kind = "replay"
file = "data.csv"
symbols = ["AAPL", "MSFT", "GOOG"]
"#;
    fs::write(dir.path().join("config.toml"), config).unwrap();

    let config_path = dir.path().join("config.toml");
    let result = load_strategies(&config_path);

    assert!(result.is_err(), "expected error when all strategies fail");

    let errors = result.unwrap_err();
    assert_eq!(errors.len(), 3, "expected 3 errors (one per missing strategy)");

    // Each error should reference the specific missing file
    for error in &errors {
        assert!(
            error.message.contains("failed to read file"),
            "error should mention file read failure: got '{}'",
            error.message
        );
    }

    // Verify the paths are reported correctly
    let paths: Vec<String> = errors.iter().map(|e| e.path.display().to_string()).collect();
    assert!(paths.iter().any(|p| p.contains("nonexistent_a.flux")));
    assert!(paths.iter().any(|p| p.contains("nonexistent_b.flux")));
    assert!(paths.iter().any(|p| p.contains("nonexistent_c.flux")));
}

/// Test 5: `build_connectors()` with valid connector configs creates correct types.
///
/// Validates: Requirement 5.1 (connector instantiation from config)
#[test]
fn build_connectors_with_valid_configs_creates_correct_types() {
    let configs = vec![
        ConnectorConfig {
            kind: "replay".to_string(),
            url: None,
            file: Some("market_data.csv".to_string()),
            symbols: vec!["AAPL".to_string(), "MSFT".to_string()],
            interval: None,
            playback_rate: Some(0.0),
        },
        ConnectorConfig {
            kind: "poll".to_string(),
            url: Some("https://api.example.com/ohlcv".to_string()),
            file: None,
            symbols: vec!["GOOG".to_string()],
            interval: Some("5m".to_string()),
            playback_rate: None,
        },
        ConnectorConfig {
            kind: "websocket".to_string(),
            url: Some("wss://stream.example.com/v1".to_string()),
            file: None,
            symbols: vec!["AAPL".to_string(), "MSFT".to_string(), "GOOG".to_string()],
            interval: None,
            playback_rate: None,
        },
    ];

    let result = build_connectors(&configs);
    assert!(result.is_ok(), "expected successful build, got: {:?}", result.err());

    let connectors = result.unwrap();
    assert_eq!(connectors.len(), 3);

    // Verify connector IDs follow the pattern "{kind}-{index}"
    assert_eq!(connectors[0].id(), "replay-0");
    assert_eq!(connectors[1].id(), "poll-1");
    assert_eq!(connectors[2].id(), "websocket-2");
}

/// Test 6: `build_connectors()` with invalid config (missing required fields)
/// reports actionable error.
///
/// Validates: Requirement 5.1 (error reporting for bad config)
#[test]
fn build_connectors_with_missing_required_fields_reports_error() {
    // Poll connector requires 'url', replay requires 'file'
    let configs = vec![
        ConnectorConfig {
            kind: "poll".to_string(),
            url: None, // missing!
            file: None,
            symbols: vec!["AAPL".to_string()],
            interval: Some("1m".to_string()),
            playback_rate: None,
        },
    ];

    let result = build_connectors(&configs);
    assert!(result.is_err(), "expected error for missing 'url'");

    let errors = match result {
        Err(e) => e,
        Ok(_) => unreachable!(),
    };
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].kind, "poll");
    assert!(
        errors[0].message.contains("missing required field 'url'"),
        "error should identify the missing field: got '{}'",
        errors[0].message
    );
}

/// Test 6b: `build_connectors()` with unknown kind reports actionable error.
///
/// Validates: Requirement 5.1
#[test]
fn build_connectors_with_unknown_kind_reports_error() {
    let configs = vec![ConnectorConfig {
        kind: "kafka".to_string(),
        url: Some("kafka://broker:9092".to_string()),
        file: None,
        symbols: vec!["AAPL".to_string()],
        interval: None,
        playback_rate: None,
    }];

    let result = build_connectors(&configs);
    assert!(result.is_err());

    let errors = match result {
        Err(e) => e,
        Ok(_) => unreachable!(),
    };
    assert_eq!(errors.len(), 1);
    assert!(
        errors[0].message.contains("unknown connector kind 'kafka'"),
        "error should name the unknown kind: got '{}'",
        errors[0].message
    );
    // Should suggest valid options
    assert!(
        errors[0].message.contains("replay") && errors[0].message.contains("poll") && errors[0].message.contains("websocket"),
        "error should suggest valid connector kinds: got '{}'",
        errors[0].message
    );
}
