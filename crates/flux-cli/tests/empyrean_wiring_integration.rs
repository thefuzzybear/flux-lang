//! Integration tests for empyrean swing wiring — backward compatibility.
//!
//! Validates that `flux live` routing logic correctly dispatches:
//! - `.flux` paths → single-file strategy mode
//! - `.toml` paths → multi-strategy TOML mode
//! - directory without `account.flux` → descriptive error
//!
//! **Validates: Requirements 10.1, 10.2, 10.3, 10.4**

use std::fs;
use tempfile::TempDir;

use flux_cli::commands::live::LiveArgs;

// =============================================================================
// Single-file mode (.flux) backward compatibility
// =============================================================================

/// Test: `flux live strategy.flux` single-file mode routes correctly.
///
/// Creates a valid strategy with no connector block — the harness exits
/// gracefully when the channel sender is dropped (receiver gets None).
///
/// Validates: Requirement 10.1
#[tokio::test]
async fn test_single_file_mode_works_unchanged() {
    let tmp = TempDir::new().unwrap();
    let strategy_path = tmp.path().join("strategy.flux");
    fs::write(
        &strategy_path,
        r#"
strategy BackwardCompatTest {
    params {
        period = 20
    }

    state {
        bar_count = 0
    }

    on bar {
        bar_count = bar_count + 1
        if close > open and not in_position {
            OPEN(symbol, 100.0)
        }
    }
}
"#,
    )
    .unwrap();

    let args = LiveArgs {
        file: strategy_path,
        capital: 10000.0,
        max_position: None,
        max_exposure: None,
        max_positions: None,
        state_file: None,
        heartbeat: 30,
    };

    // Should not return Err — the strategy is valid but has no connector,
    // so the channel sender is dropped → receiver gets None → loop ends → Ok
    let result = flux_cli::commands::live::run_live_cmd(args).await;
    assert!(
        result.is_ok(),
        "Single-file mode should not error for a valid strategy: {:?}",
        result.err()
    );
}

// =============================================================================
// TOML mode (.toml) backward compatibility
// =============================================================================

/// Test: `flux live config.toml` TOML mode routes correctly.
///
/// Creates a minimal TOML config with a valid strategy and a replay connector
/// with one bar of data. The TOML mode routing is validated: the config is
/// parsed, strategy loaded, connector built and executed. The harness returns
/// AllConnectorsFailed when the replay connector finishes (expected behavior).
///
/// Validates: Requirement 10.2
#[tokio::test]
async fn test_toml_mode_works_unchanged() {
    let tmp = TempDir::new().unwrap();

    // Write a valid strategy
    let strategy_path = tmp.path().join("my_strategy.flux");
    fs::write(
        &strategy_path,
        r#"
strategy TomlModeTest {
    params {
        lookback = 10
    }

    state {
        count = 0
    }

    on bar {
        count = count + 1
    }
}
"#,
    )
    .unwrap();

    // Create a CSV with one data row for the replay connector
    let csv_path = tmp.path().join("data.csv");
    fs::write(
        &csv_path,
        "timestamp,symbol,open,high,low,close,volume\n2024-01-02,TEST,100.0,101.0,99.0,100.5,1000\n",
    )
    .unwrap();

    // Write a TOML config referencing the strategy with a replay connector
    let config_path = tmp.path().join("config.toml");
    fs::write(
        &config_path,
        format!(
            r#"
capital = 10000.0

[[strategies]]
path = "{}"
symbols = ["TEST"]

[[connectors]]
kind = "replay"
file = "{}"
symbols = ["TEST"]
"#,
            strategy_path.display(),
            csv_path.display(),
        ),
    )
    .unwrap();

    let args = LiveArgs {
        file: config_path,
        capital: 10000.0,
        max_position: None,
        max_exposure: None,
        max_positions: None,
        state_file: None,
        heartbeat: 30,
    };

    // TOML mode routing is validated: config parsed, strategy loaded, connector executed.
    // The harness returns an error when the replay connector finishes (all connectors
    // "disconnected") — this is expected behavior. The key assertion is that:
    // 1. We didn't get a "parse error" or "routing error" (TOML path was taken)
    // 2. We didn't get "all strategies failed" (strategy was loaded)
    // 3. The error is specifically about connectors finishing (correct code path)
    let result = flux_cli::commands::live::run_live_cmd(args).await;
    match result {
        Ok(()) => {
            // Also acceptable — some harness versions return Ok when replay finishes
        }
        Err(e) => {
            let msg = e.to_string();
            // The error must be about connectors disconnecting (not routing/parsing)
            assert!(
                msg.contains("connector") || msg.contains("AllConnectorsFailed"),
                "TOML mode should route correctly. Expected connector-related exit, got: {}",
                msg
            );
        }
    }
}

// =============================================================================
// Directory mode — missing account.flux error
// =============================================================================

/// Test: `flux live dir/` without account.flux emits correct error.
///
/// Validates: Requirement 10.4
#[tokio::test]
async fn test_directory_without_account_flux_errors() {
    let tmp = TempDir::new().unwrap();
    // Directory exists but has no account.flux

    let args = LiveArgs {
        file: tmp.path().to_path_buf(),
        capital: 10000.0,
        max_position: None,
        max_exposure: None,
        max_positions: None,
        state_file: None,
        heartbeat: 30,
    };

    let result = flux_cli::commands::live::run_live_cmd(args).await;
    assert!(result.is_err(), "Directory without account.flux should error");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("no account.flux found in directory"),
        "Expected 'no account.flux found in directory' error, got: {}",
        err_msg
    );
}

/// Test: `flux live` with a nonexistent directory path emits error.
///
/// Validates: Requirement 10.4 (edge case — path doesn't exist at all)
#[tokio::test]
async fn test_nonexistent_path_errors() {
    let args = LiveArgs {
        file: "/nonexistent/path/to/nowhere".into(),
        capital: 10000.0,
        max_position: None,
        max_exposure: None,
        max_positions: None,
        state_file: None,
        heartbeat: 30,
    };

    let result = flux_cli::commands::live::run_live_cmd(args).await;
    assert!(
        result.is_err(),
        "Nonexistent path should error"
    );
}


// =============================================================================
// Integration tests for full boot with MockBroker
//
// Validates that the account runtime boot sequence correctly wires all components
// when using a mock broker adapter. Since `boot_account_runtime` enters an event
// loop that blocks forever, we test the individual component-building steps that
// lead up to harness creation.
//
// **Validates: Requirements 2.1, 2.8, 2.9, 5.2, 6.1, 6.2**
// =============================================================================

use flux_cli::live::account_config::{
    AccountConfig, AccountSection, DataSection, DatabaseSection, GatewaySection, ProductEntry,
    RiskSection, StrategyEntry,
};
use flux_cli::live::account_runtime::{
    build_execution_policies, connect_broker_with_retry, load_strategies_from_config,
};
use flux_cli::live::broker::ExecutionPolicy;

// BrokerAdapter trait must be in scope for calling methods on Arc<dyn BrokerAdapter>
#[allow(unused_imports)]
use flux_cli::live::broker::BrokerAdapter;

/// Minimal valid Flux strategy source for "aether".
const AETHER_STRATEGY: &str = r#"strategy Aether {
    params {
        period = 20
    }

    state {
        bar_count = 0
    }

    on bar {
        bar_count = bar_count + 1
    }
}
"#;

/// Minimal valid Flux strategy source for "kairos".
const KAIROS_STRATEGY: &str = r#"strategy Kairos {
    params {
        lookback = 10
    }

    state {
        bar_count = 0
    }

    on bar {
        bar_count = bar_count + 1
    }
}
"#;

/// Build a complete AccountConfig mimicking the empyrean/swing manifest
/// but using "mock" broker and temp directory for strategy paths.
fn build_swing_config() -> AccountConfig {
    AccountConfig {
        account: AccountSection {
            name: "swing".into(),
            broker: "mock".into(),
            account_id: "TEST123".into(),
            mode: "paper".into(),
        },
        gateway: GatewaySection {
            host: "127.0.0.1".into(),
            port: 4002,
        },
        data: DataSection {
            source: "mock".into(),
            symbols: vec!["ES".into(), "NQ".into()],
            interval: "1d".into(),
            replay_file: None,
        },
        database: DatabaseSection {
            url: "".into(),
            schema: "".into(),
        },
        risk: RiskSection {
            max_daily_loss: -15000.0,
            max_weekly_loss: -30000.0,
            max_position_per_product: 10,
            max_total_notional: 3000000.0,
            max_drawdown_pct: 0.08,
            correlation_warning_threshold: 4,
            initial_equity: 500000.0,
        },
        products: vec![
            ProductEntry {
                name: "ES".into(),
                multiplier: 50.0,
                tick_size: 0.25,
                margin: 15840.0,
            },
            ProductEntry {
                name: "NQ".into(),
                multiplier: 20.0,
                tick_size: 0.25,
                margin: 21120.0,
            },
        ],
        strategies: vec![
            StrategyEntry {
                name: "aether".into(),
                path: "aether/strategy.flux".into(),
                allocation: 0.6,
                priority: 1,
                execution: Some("market".into()),
                execution_offset_ticks: None,
            },
            StrategyEntry {
                name: "kairos".into(),
                path: "kairos/strategy.flux".into(),
                allocation: 0.4,
                priority: 2,
                execution: Some("aggressive_limit".into()),
                execution_offset_ticks: Some(2),
            },
        ],
        execution_default: None,
    }
}

/// Create strategy files in a temp directory for loading.
fn setup_strategy_files(base: &std::path::Path) {
    fs::create_dir_all(base.join("aether")).unwrap();
    fs::write(base.join("aether/strategy.flux"), AETHER_STRATEGY).unwrap();
    fs::create_dir_all(base.join("kairos")).unwrap();
    fs::write(base.join("kairos/strategy.flux"), KAIROS_STRATEGY).unwrap();
}

// =============================================================================
// Test: load_strategies_from_config returns both strategies
// Validates: Requirements 2.1, 2.8, 2.9
// =============================================================================

#[test]
fn test_load_strategies_returns_both_strategies() {
    let tmp = TempDir::new().unwrap();
    let base = tmp.path();
    setup_strategy_files(base);

    let config = build_swing_config();
    let strategies = load_strategies_from_config(&config, base)
        .expect("should load both strategies successfully");

    assert_eq!(strategies.len(), 2, "expected 2 strategies loaded");

    // Verify both strategy names are present
    let names: Vec<&str> = strategies.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"Aether"), "aether strategy should be loaded");
    assert!(names.contains(&"Kairos"), "kairos strategy should be loaded");
}

// =============================================================================
// Test: build_execution_policies assigns correct policies
// Validates: Requirements 6.1, 6.2
// =============================================================================

#[test]
fn test_execution_policies_assigned_correctly() {
    let config = build_swing_config();
    let policies = build_execution_policies(&config);

    // Aether should have Market policy
    assert_eq!(
        policies.get("aether"),
        Some(&ExecutionPolicy::Market),
        "aether should have Market execution policy"
    );

    // Kairos should have AggressiveLimit with offset_ticks=2
    assert_eq!(
        policies.get("kairos"),
        Some(&ExecutionPolicy::AggressiveLimit { offset_ticks: 2 }),
        "kairos should have AggressiveLimit(offset_ticks=2) execution policy"
    );

    // Only 2 policies should exist
    assert_eq!(policies.len(), 2, "should have exactly 2 execution policies");
}

// =============================================================================
// Test: connect_broker_with_retry succeeds with mock broker
// Validates: Requirements 5.2
// =============================================================================

#[tokio::test]
async fn test_connect_mock_broker_succeeds() {
    let config = build_swing_config();
    let broker = connect_broker_with_retry(&config)
        .await
        .expect("mock broker connection should succeed immediately");

    assert!(
        broker.is_connected(),
        "mock broker should report connected state"
    );
}

// =============================================================================
// Test: Execution policy default fallback when no strategy-level execution
// Validates: Requirements 6.1, 6.2
// =============================================================================

#[test]
fn test_execution_policy_default_fallback() {
    let mut config = build_swing_config();
    // Remove execution field from both strategies
    for strategy in &mut config.strategies {
        strategy.execution = None;
        strategy.execution_offset_ticks = None;
    }

    let policies = build_execution_policies(&config);

    // With no strategy execution and no account default, both should be Market
    assert_eq!(policies.get("aether"), Some(&ExecutionPolicy::Market));
    assert_eq!(policies.get("kairos"), Some(&ExecutionPolicy::Market));
}

// =============================================================================
// Test: Execution policy uses account-level default when strategy has none
// Validates: Requirements 6.1, 6.2
// =============================================================================

#[test]
fn test_execution_policy_account_default() {
    let mut config = build_swing_config();
    // Remove strategy-level execution, set account default
    for strategy in &mut config.strategies {
        strategy.execution = None;
        strategy.execution_offset_ticks = None;
    }
    config.execution_default = Some("aggressive_limit".into());

    let policies = build_execution_policies(&config);

    // Both should use account default (aggressive_limit with default offset 2)
    assert_eq!(
        policies.get("aether"),
        Some(&ExecutionPolicy::AggressiveLimit { offset_ticks: 2 })
    );
    assert_eq!(
        policies.get("kairos"),
        Some(&ExecutionPolicy::AggressiveLimit { offset_ticks: 2 })
    );
}

// =============================================================================
// Test: Full integration — strategies loaded + policies assigned together
// Validates: Requirements 2.1, 2.8, 2.9, 6.1, 6.2
// =============================================================================

#[tokio::test]
async fn test_full_boot_components_with_mock_broker() {
    let tmp = TempDir::new().unwrap();
    let base = tmp.path();
    setup_strategy_files(base);

    let config = build_swing_config();

    // 1. Load strategies
    let strategies = load_strategies_from_config(&config, base)
        .expect("strategies should load");
    assert_eq!(strategies.len(), 2);

    // 2. Build execution policies
    let policies = build_execution_policies(&config);
    assert_eq!(policies.get("aether"), Some(&ExecutionPolicy::Market));
    assert_eq!(
        policies.get("kairos"),
        Some(&ExecutionPolicy::AggressiveLimit { offset_ticks: 2 })
    );

    // 3. Connect mock broker
    let broker = connect_broker_with_retry(&config)
        .await
        .expect("mock broker should connect");
    assert!(broker.is_connected());

    // 4. Verify broker supports expected operations (positions, open orders)
    let positions = broker.get_positions().await.expect("get_positions should work");
    assert!(positions.is_empty(), "fresh broker should have no positions");

    let open_orders = broker.get_open_orders().await.expect("get_open_orders should work");
    assert!(open_orders.is_empty(), "fresh broker should have no open orders");
}
