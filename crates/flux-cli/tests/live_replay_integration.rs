//! Integration test: Replay connector end-to-end.
//!
//! Loads a strategy, connects a `ReplayConnector` with sample CSV data,
//! runs the `LiveHarness` event loop, and verifies that the fills match
//! what the interpreter produces when running the same data in backtest mode.
//!
//! This proves backtest-live equivalence: the same strategy code produces
//! identical signals regardless of whether it runs via `flux backtest` or
//! `flux live` with a replay connector.
//!
//! **Validates: Requirements 1.9, 10.5**

use std::path::PathBuf;
use std::time::Duration;

use flux_cli::csv_loader::load_csv;
use flux_cli::interpreter::Interpreter;
use flux_cli::live::aggregator::{RiskConstraints, SignalAggregator};
use flux_cli::live::connector::{Connector, ReconnectPolicy};
use flux_cli::live::harness::LiveHarness;
use flux_cli::live::loader::StrategyModule;
use flux_cli::live::position::LivePositionTracker;
use flux_cli::live::replay_connector::ReplayConnector;
use flux_runtime::PositionTracker;
use tokio::sync::mpsc;

/// Path to the sample CSV fixture.
fn sample_csv_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("sample_data.csv")
}

/// Strategy source: open when close > open, close when close < open.
const STRATEGY_SOURCE: &str = r#"
strategy ReplayTest {
    on bar {
        if close > open and not in_position {
            OPEN(symbol, 100.0)
        }
        if close < open and in_position {
            CLOSE(symbol)
        }
    }
}
"#;

/// Compile a strategy from source through lex → parse → typecheck → Interpreter.
fn compile_strategy(source: &str) -> Interpreter {
    let tokens = flux_compiler::lexer::lex_with_spans(source).expect("lex failed");
    let ast = flux_compiler::parser::parse(tokens).expect("parse failed");
    let typed_program = flux_compiler::typeck::check(ast).expect("typecheck failed");
    Interpreter::new(&typed_program)
}

/// Run the strategy through the interpreter in backtest mode (same as `flux backtest`).
/// Returns the fills from the PositionTracker.
fn run_backtest(bars: &[flux_runtime::BarContext], initial_capital: f64) -> PositionTracker {
    let mut interpreter = compile_strategy(STRATEGY_SOURCE);
    let mut tracker = PositionTracker::new(initial_capital);

    for (i, bar) in bars.iter().enumerate() {
        interpreter.in_position = tracker.open_position_count() > 0;
        let signals = interpreter.on_bar(bar);
        tracker.process_signals(&signals, bar.close, i);
        tracker.mark_to_market(bar.close, &bar.symbol);
    }

    tracker
}

/// Validates: Requirements 1.9, 10.5
///
/// Run the live harness with a ReplayConnector (playback_rate = 0.0) and
/// verify that signals/fills match what the backtest interpreter produces
/// for the same bar data.
#[tokio::test]
async fn replay_connector_matches_backtest_output() {
    let csv_path = sample_csv_path();
    let initial_capital = 10_000.0;

    // --- Backtest path ---
    let bars = load_csv(&csv_path).expect("failed to load CSV");
    let backtest_tracker = run_backtest(&bars, initial_capital);

    // --- Live harness path ---
    // Compile the same strategy for the live harness
    let interpreter = compile_strategy(STRATEGY_SOURCE);
    let strategy_module = StrategyModule {
        name: "ReplayTest".to_string(),
        source_path: PathBuf::from("test_strategy.flux"),
        interpreter,
        subscribed_symbols: vec!["AAPL".to_string()],
    };

    // No risk constraints (unconstrained — same as backtest)
    let aggregator = SignalAggregator::new(RiskConstraints::default());
    let tracker = LivePositionTracker::new(initial_capital);

    let mut harness = LiveHarness::new(
        vec![strategy_module],
        aggregator,
        tracker,
        None,
        ReconnectPolicy::default(),
        Duration::from_secs(3600),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        std::collections::HashMap::new(),
        flux_cli::live::broker::DeduplicationGuard::new(),
    );

    // Create the replay connector with instant playback
    let mut connector = ReplayConnector::new("replay-test", csv_path, 0.0);
    let (tx, rx) = mpsc::channel(256);

    // Connect the replay connector — it spawns a task that sends all bars
    connector
        .connect(&["AAPL".to_string()], tx)
        .await
        .expect("replay connector failed to connect");

    // Run the harness event loop.
    // The channel closes when all bars are sent, so `run` will exit.
    // We use connector_count=0 so that channel close = clean exit (not error).
    let result = harness.run(rx, 0).await;
    assert!(result.is_ok(), "harness run failed: {:?}", result.err());

    // Disconnect the connector
    connector.disconnect().await.expect("disconnect failed");

    // --- Compare results ---
    let live_fills = harness.tracker.inner.fills();
    let backtest_fills = backtest_tracker.fills();

    // Same number of fills
    assert_eq!(
        live_fills.len(),
        backtest_fills.len(),
        "fill count mismatch: live={}, backtest={}",
        live_fills.len(),
        backtest_fills.len()
    );

    // Each fill matches (symbol, qty, price, side)
    for (i, (live_fill, bt_fill)) in live_fills.iter().zip(backtest_fills.iter()).enumerate() {
        assert_eq!(
            live_fill.symbol, bt_fill.symbol,
            "fill {} symbol mismatch: live={}, backtest={}",
            i, live_fill.symbol, bt_fill.symbol
        );
        assert_eq!(
            live_fill.side, bt_fill.side,
            "fill {} side mismatch: live={:?}, backtest={:?}",
            i, live_fill.side, bt_fill.side
        );
        assert!(
            (live_fill.qty - bt_fill.qty).abs() < 1e-10,
            "fill {} qty mismatch: live={}, backtest={}",
            i, live_fill.qty, bt_fill.qty
        );
        assert!(
            (live_fill.price - bt_fill.price).abs() < 1e-10,
            "fill {} price mismatch: live={}, backtest={}",
            i, live_fill.price, bt_fill.price
        );
    }

    // Also verify the portfolio state matches
    let live_equity = harness.tracker.inner.equity();
    let backtest_equity = backtest_tracker.equity();
    assert!(
        (live_equity - backtest_equity).abs() < 1e-6,
        "equity mismatch: live={}, backtest={}",
        live_equity, backtest_equity
    );

    let live_realized_pnl = harness.tracker.inner.realized_pnl();
    let backtest_realized_pnl = backtest_tracker.realized_pnl();
    assert!(
        (live_realized_pnl - backtest_realized_pnl).abs() < 1e-6,
        "realized P&L mismatch: live={}, backtest={}",
        live_realized_pnl, backtest_realized_pnl
    );

    let live_open_count = harness.tracker.inner.open_position_count();
    let backtest_open_count = backtest_tracker.open_position_count();
    assert_eq!(
        live_open_count, backtest_open_count,
        "open position count mismatch: live={}, backtest={}",
        live_open_count, backtest_open_count
    );
}

/// Validates: Requirements 1.9, 10.5
///
/// Verify that the replay connector actually produces signals and fills
/// (not a vacuous pass). The sample CSV has bars where close > open,
/// so we expect at least one OPEN signal/fill.
#[tokio::test]
async fn replay_connector_produces_fills() {
    let csv_path = sample_csv_path();
    let initial_capital = 10_000.0;

    let interpreter = compile_strategy(STRATEGY_SOURCE);
    let strategy_module = StrategyModule {
        name: "ReplayTest".to_string(),
        source_path: PathBuf::from("test_strategy.flux"),
        interpreter,
        subscribed_symbols: vec!["AAPL".to_string()],
    };

    let aggregator = SignalAggregator::new(RiskConstraints::default());
    let tracker = LivePositionTracker::new(initial_capital);

    let mut harness = LiveHarness::new(
        vec![strategy_module],
        aggregator,
        tracker,
        None,
        ReconnectPolicy::default(),
        Duration::from_secs(3600),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        std::collections::HashMap::new(),
        flux_cli::live::broker::DeduplicationGuard::new(),
    );

    let mut connector = ReplayConnector::new("replay-test", csv_path, 0.0);
    let (tx, rx) = mpsc::channel(256);

    connector
        .connect(&["AAPL".to_string()], tx)
        .await
        .expect("replay connector failed to connect");

    let result = harness.run(rx, 0).await;
    assert!(result.is_ok());

    connector.disconnect().await.expect("disconnect failed");

    // The sample data has bars where close > open, so we should get fills
    let fills = harness.tracker.inner.fills();
    assert!(
        !fills.is_empty(),
        "expected at least one fill from replay, got none"
    );

    // Verify fill attribution is tracked
    assert_eq!(
        harness.tracker.fill_attribution.len(),
        fills.len(),
        "fill attribution count should match fills"
    );
    for attr in &harness.tracker.fill_attribution {
        assert_eq!(attr, "ReplayTest", "all fills should be attributed to ReplayTest");
    }
}
