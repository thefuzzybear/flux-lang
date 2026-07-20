//! Constraint enforcement end-to-end integration test.
//!
//! Tests that the signal aggregator correctly rejects signals that violate
//! portfolio-level risk constraints, and approves signals that respect them.
//!
//! **Validates: Requirements 4.2, 4.5**

use std::path::PathBuf;
use std::time::Duration;

use flux_cli::interpreter::Interpreter;
use flux_cli::live::aggregator::{RiskConstraints, SignalAggregator};
use flux_cli::live::connector::{LiveBar, ReconnectPolicy};
use flux_cli::live::harness::LiveHarness;
use flux_cli::live::loader::StrategyModule;
use flux_cli::live::position::LivePositionTracker;
use flux_runtime::BarContext;
use tokio::sync::mpsc;

// =============================================================================
// Helpers
// =============================================================================

/// Compile a .flux strategy source and return a StrategyModule.
fn compile_strategy(source: &str, name: &str, symbols: Vec<String>) -> StrategyModule {
    let tokens = flux_compiler::lexer::lex_with_spans(source).expect("lex failed");
    let ast = flux_compiler::parser::parse(tokens).expect("parse failed");
    let typed = flux_compiler::typeck::check(ast).expect("typecheck failed");
    let interpreter = Interpreter::new(&typed);

    StrategyModule {
        name: name.to_string(),
        source_path: PathBuf::from(format!("{}.flux", name)),
        interpreter,
        subscribed_symbols: symbols,
    }
}

/// Create a LiveBar with the given symbol and close price.
fn make_live_bar(symbol: &str, close: f64) -> LiveBar {
    LiveBar {
        bar: BarContext {
            symbol: symbol.to_string(),
            open: close - 1.0,
            high: close + 1.0,
            low: close - 2.0,
            close,
            volume: 10000.0,
            in_position: false,
        },
        connector_id: "test-connector".to_string(),
        received_at: chrono::Utc::now(),
    }
}

/// A strategy that always emits OPEN(symbol, 200.0) on every bar.
/// This is used to test constraint enforcement — the strategy always
/// wants to open a large position regardless of constraints.
const ALWAYS_OPEN_200_STRATEGY: &str = r#"strategy AlwaysOpen200 {
    on bar {
        if not in_position {
            OPEN(symbol, 200.0)
        }
    }
}"#;

// =============================================================================
// Tests
// =============================================================================

/// Test that a signal exceeding max_position_size is rejected.
///
/// Configure max_position_size = 100, strategy emits OPEN(AAPL, 200).
/// The signal should be rejected: no fills generated, position stays at 0.
///
/// **Validates: Requirements 4.2, 4.5**
#[tokio::test]
async fn constraint_rejects_open_exceeding_max_position_size() {
    // 1. Compile a strategy that always opens with OPEN(symbol, 200.0)
    let strategy = compile_strategy(
        ALWAYS_OPEN_200_STRATEGY,
        "AlwaysOpen200",
        vec!["AAPL".to_string()],
    );

    // 2. Create a LiveHarness with max_position_size = 100.0 (less than 200.0)
    let constraints = RiskConstraints {
        max_position_size: Some(100.0),
        max_exposure: None,
        max_positions: None,
    };

    let mut harness = LiveHarness::new(
        vec![strategy],
        SignalAggregator::new(constraints),
        LivePositionTracker::new(100_000.0),
        None,
        ReconnectPolicy::default(),
        Duration::from_secs(30),
        None,
        None,
        None,
        None,
    );

    // 3. Send a bar via mpsc channel where the strategy generates OPEN(AAPL, 200.0)
    let (tx, rx) = mpsc::channel::<LiveBar>(16);
    let bar = make_live_bar("AAPL", 150.0);
    tx.send(bar).await.unwrap();
    drop(tx); // Close channel so harness exits after processing

    // 4. Run the harness (processes bar, then exits when channel closes)
    let result = harness.run(rx, 0).await;
    assert!(result.is_ok());

    // 5. Verify the signal was rejected: position should remain at 0
    let position = harness.tracker.inner.position("AAPL");
    assert!(
        position.is_none() || position.unwrap().qty == 0.0,
        "Expected no position for AAPL (signal should be rejected), but found: {:?}",
        position
    );

    // No fills should have been generated
    assert!(
        harness.tracker.fill_attribution.is_empty(),
        "Expected no fills (signal rejected), but found {} fills",
        harness.tracker.fill_attribution.len()
    );

    // No fills in the inner tracker either
    assert_eq!(
        harness.tracker.inner.fills().len(),
        0,
        "Expected 0 fills in tracker, but found {}",
        harness.tracker.inner.fills().len()
    );
}

/// Test that the same signal is approved when max_position_size is large enough.
///
/// Configure max_position_size = 300 (> 200), strategy emits OPEN(AAPL, 200).
/// The signal should be approved: fills generated, position updated to 200.
///
/// **Validates: Requirements 4.2, 4.5**
#[tokio::test]
async fn constraint_approves_open_within_max_position_size() {
    // 1. Compile the same strategy that always opens with OPEN(symbol, 200.0)
    let strategy = compile_strategy(
        ALWAYS_OPEN_200_STRATEGY,
        "AlwaysOpen200",
        vec!["AAPL".to_string()],
    );

    // 2. Create a LiveHarness with max_position_size = 300.0 (greater than 200.0)
    let constraints = RiskConstraints {
        max_position_size: Some(300.0),
        max_exposure: None,
        max_positions: None,
    };

    let mut harness = LiveHarness::new(
        vec![strategy],
        SignalAggregator::new(constraints),
        LivePositionTracker::new(100_000.0),
        None,
        ReconnectPolicy::default(),
        Duration::from_secs(30),
        None,
        None,
        None,
        None,
    );

    // 3. Send a bar via mpsc channel where the strategy generates OPEN(AAPL, 200.0)
    let (tx, rx) = mpsc::channel::<LiveBar>(16);
    let bar = make_live_bar("AAPL", 150.0);
    tx.send(bar).await.unwrap();
    drop(tx); // Close channel so harness exits after processing

    // 4. Run the harness (processes bar, then exits when channel closes)
    let result = harness.run(rx, 0).await;
    assert!(result.is_ok());

    // 5. Verify the signal was approved: position should be 200
    let position = harness.tracker.inner.position("AAPL");
    assert!(
        position.is_some(),
        "Expected a position for AAPL (signal should be approved), but found none"
    );
    let position = position.unwrap();
    assert!(
        (position.qty - 200.0).abs() < 0.001,
        "Expected position qty of 200.0, but found {}",
        position.qty
    );

    // Fills should have been generated
    assert_eq!(
        harness.tracker.fill_attribution.len(),
        1,
        "Expected 1 fill attribution, but found {}",
        harness.tracker.fill_attribution.len()
    );
    assert_eq!(harness.tracker.fill_attribution[0], "AlwaysOpen200");

    // Inner tracker should have 1 fill
    assert_eq!(
        harness.tracker.inner.fills().len(),
        1,
        "Expected 1 fill in tracker, but found {}",
        harness.tracker.inner.fills().len()
    );

    // Fill should be a BUY for AAPL at close price (150.0) with qty 200
    let fill = &harness.tracker.inner.fills()[0];
    assert_eq!(fill.symbol, "AAPL");
    assert!((fill.qty - 200.0).abs() < 0.001);
    assert!((fill.price - 150.0).abs() < 0.001);
}
