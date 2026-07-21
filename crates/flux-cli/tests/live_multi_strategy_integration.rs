//! Integration test: Multi-strategy isolation in the LiveHarness.
//!
//! Validates Requirements 2.4 (independent state per strategy) and
//! 3.1 (single unified position tracker across all strategies).
//!
//! Two strategies with different parameters both subscribe to "AAPL".
//! We verify:
//! - Each strategy's internal state (bar_count) evolves independently
//! - Both strategies generate signals based on their own conditions
//! - The unified position tracker correctly aggregates fills with attribution

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

/// Compile a strategy from source text and return a StrategyModule.
fn compile_strategy(source: &str, symbols: Vec<String>) -> StrategyModule {
    let tokens = flux_compiler::lexer::lex_with_spans(source)
        .expect("strategy should lex successfully");
    let ast = flux_compiler::parser::parse(tokens)
        .expect("strategy should parse successfully");
    let typed_program = flux_compiler::typeck::check(ast)
        .expect("strategy should typecheck successfully");

    let name = typed_program.strategy.name.clone();
    let interpreter = Interpreter::new(&typed_program);

    StrategyModule {
        name,
        source_path: PathBuf::from("test.flux"),
        interpreter,
        subscribed_symbols: symbols,
    }
}

/// Create a LiveBar for testing.
fn make_bar(symbol: &str, open: f64, high: f64, low: f64, close: f64) -> LiveBar {
    LiveBar {
        bar: BarContext {
            close,
            open,
            high,
            low,
            volume: 1_000_000.0,
            symbol: symbol.to_string(),
            in_position: false,
        },
        connector_id: "test-connector".to_string(),
        received_at: chrono::Utc::now(),
    }
}

/// Strategy A: Opens when close > open (bullish bar), tracks bar_count.
const STRATEGY_A_SOURCE: &str = r#"
strategy BullishOpener {
    params {
        qty = 100.0
    }

    state {
        bar_count = 0
    }

    on bar {
        bar_count = bar_count + 1
        if close > open and not in_position {
            OPEN(symbol, qty)
        }
    }
}
"#;

/// Strategy B: Opens when close > high * 0.95 (near high), uses different qty.
/// Has its own independent bar_count state variable.
const STRATEGY_B_SOURCE: &str = r#"
strategy NearHighOpener {
    params {
        threshold = 0.95
        qty = 50.0
    }

    state {
        bar_count = 0
    }

    on bar {
        bar_count = bar_count + 1
        if close > high * threshold and not in_position {
            OPEN(symbol, qty)
        }
    }
}
"#;

#[tokio::test]
async fn two_strategies_maintain_independent_state() {
    // Compile both strategies subscribing to AAPL
    let strategy_a = compile_strategy(STRATEGY_A_SOURCE, vec!["AAPL".to_string()]);
    let strategy_b = compile_strategy(STRATEGY_B_SOURCE, vec!["AAPL".to_string()]);

    // Build harness with no constraints (unlimited)
    let aggregator = SignalAggregator::new(RiskConstraints::default());
    let tracker = LivePositionTracker::new(100_000.0);

    let mut harness = LiveHarness::new(
        vec![strategy_a, strategy_b],
        aggregator,
        tracker,
        None,
        ReconnectPolicy::default(),
        Duration::from_secs(60),
        None,
        None,
        None,
        None,
        None,
        None,
    );

    // Feed 3 bars and verify state evolves independently
    let bars = vec![
        // Bar 1: close > open (bullish) AND close > high*0.95 → both trigger
        make_bar("AAPL", 100.0, 105.0, 99.0, 104.0),
        // Bar 2: close < open (bearish) but close (97) > high*0.95 (100*0.95=95) → only B would trigger
        // But since both are in_position after bar 1, neither re-opens
        make_bar("AAPL", 100.0, 100.0, 95.0, 97.0),
        // Bar 3: another bar to verify bar_count increments
        make_bar("AAPL", 98.0, 102.0, 97.0, 101.0),
    ];

    for bar in &bars {
        harness.dispatch_bar(bar);
    }

    // Verify each strategy processed all 3 bars independently
    assert!(
        matches!(
            harness.strategies[0].interpreter.state.get("bar_count"),
            Some(flux_cli::interpreter::Value::Int(3))
        ),
        "Strategy A bar_count should be 3"
    );
    assert!(
        matches!(
            harness.strategies[1].interpreter.state.get("bar_count"),
            Some(flux_cli::interpreter::Value::Int(3))
        ),
        "Strategy B bar_count should be 3"
    );
}

#[tokio::test]
async fn two_strategies_generate_signals_independently() {
    // Compile both strategies subscribing to AAPL
    let strategy_a = compile_strategy(STRATEGY_A_SOURCE, vec!["AAPL".to_string()]);
    let strategy_b = compile_strategy(STRATEGY_B_SOURCE, vec!["AAPL".to_string()]);

    let aggregator = SignalAggregator::new(RiskConstraints::default());
    let tracker = LivePositionTracker::new(100_000.0);

    let mut harness = LiveHarness::new(
        vec![strategy_a, strategy_b],
        aggregator,
        tracker,
        None,
        ReconnectPolicy::default(),
        Duration::from_secs(60),
        None,
        None,
        None,
        None,
        None,
        None,
    );

    // Bar where close > open BUT close < high * 0.95
    // close=101, open=100 → bullish → Strategy A triggers OPEN
    // close=101, high=110, threshold=0.95 → 110*0.95=104.5 → 101 < 104.5 → Strategy B does NOT trigger
    let bar = make_bar("AAPL", 100.0, 110.0, 99.0, 101.0);
    harness.dispatch_bar(&bar);

    // After this bar: Strategy A opened (qty=100), Strategy B did not open
    // The unified tracker should have a position of 100 shares (only from A)
    let position = harness.tracker.inner.position("AAPL");
    assert!(position.is_some(), "AAPL should have a position from Strategy A");
    assert_eq!(position.unwrap().qty, 100.0, "Position should be 100 shares from Strategy A");

    // Fill attribution should show only strategy A
    assert_eq!(harness.tracker.fill_attribution.len(), 1);
    assert_eq!(harness.tracker.fill_attribution[0], "BullishOpener");
}

#[tokio::test]
async fn both_strategies_signal_and_fills_are_attributed_correctly() {
    // Compile both strategies subscribing to AAPL
    let strategy_a = compile_strategy(STRATEGY_A_SOURCE, vec!["AAPL".to_string()]);
    let strategy_b = compile_strategy(STRATEGY_B_SOURCE, vec!["AAPL".to_string()]);

    let aggregator = SignalAggregator::new(RiskConstraints::default());
    let tracker = LivePositionTracker::new(100_000.0);

    let mut harness = LiveHarness::new(
        vec![strategy_a, strategy_b],
        aggregator,
        tracker,
        None,
        ReconnectPolicy::default(),
        Duration::from_secs(60),
        None,
        None,
        None,
        None,
        None,
        None,
    );

    // Bar where BOTH strategies trigger:
    // close=104 > open=100 → Strategy A triggers (bullish)
    // close=104 > high*0.95 = 105*0.95 = 99.75 → Strategy B also triggers (near high)
    let bar = make_bar("AAPL", 100.0, 105.0, 99.0, 104.0);
    harness.dispatch_bar(&bar);

    // Both opened — but since this is a UNIFIED tracker, the position is the sum
    // Strategy A opens 100, Strategy B opens 50 → total = 150
    let position = harness.tracker.inner.position("AAPL");
    assert!(position.is_some(), "AAPL should have a position");
    assert_eq!(
        position.unwrap().qty, 150.0,
        "Unified position should be 150 (100 from A + 50 from B)"
    );

    // Fill attribution should record both strategies
    assert_eq!(harness.tracker.fill_attribution.len(), 2);
    assert_eq!(harness.tracker.fill_attribution[0], "BullishOpener");
    assert_eq!(harness.tracker.fill_attribution[1], "NearHighOpener");
}

#[tokio::test]
async fn strategies_share_unified_position_for_in_position_derivation() {
    // Strategy A opens, then on next bar both see in_position=true
    // (because the unified tracker has a position for AAPL)
    let strategy_a = compile_strategy(STRATEGY_A_SOURCE, vec!["AAPL".to_string()]);
    let strategy_b = compile_strategy(STRATEGY_B_SOURCE, vec!["AAPL".to_string()]);

    let aggregator = SignalAggregator::new(RiskConstraints::default());
    let tracker = LivePositionTracker::new(100_000.0);

    let mut harness = LiveHarness::new(
        vec![strategy_a, strategy_b],
        aggregator,
        tracker,
        None,
        ReconnectPolicy::default(),
        Duration::from_secs(60),
        None,
        None,
        None,
        None,
        None,
        None,
    );

    // Bar 1: Only A triggers (close > open, but close < high*0.95)
    // close=101, open=100 → bullish → A opens
    // close=101, high=110 → 110*0.95=104.5 → 101 < 104.5 → B does not open
    let bar1 = make_bar("AAPL", 100.0, 110.0, 99.0, 101.0);
    harness.dispatch_bar(&bar1);

    // After bar 1: position is 100 from A
    assert_eq!(harness.tracker.inner.position("AAPL").unwrap().qty, 100.0);

    // Bar 2: Both strategies would trigger their open conditions, but
    // in_position is now derived from the unified tracker (which has qty=100),
    // so BOTH see in_position=true and neither re-opens.
    // close=104 > open=100 → A's condition met, but in_position=true → skip
    // close=104 > 105*0.95=99.75 → B's condition met, but in_position=true → skip
    let bar2 = make_bar("AAPL", 100.0, 105.0, 99.0, 104.0);
    harness.dispatch_bar(&bar2);

    // Position should still be 100 — no new opens
    assert_eq!(
        harness.tracker.inner.position("AAPL").unwrap().qty, 100.0,
        "Position should remain 100 — both strategies see in_position=true from unified tracker"
    );

    // Only 1 fill total (from bar 1, strategy A)
    assert_eq!(harness.tracker.fill_attribution.len(), 1);
}

#[tokio::test]
async fn harness_runs_via_channel_with_multiple_strategies() {
    // End-to-end test: feed bars via mpsc channel through harness.run()
    let strategy_a = compile_strategy(STRATEGY_A_SOURCE, vec!["AAPL".to_string()]);
    let strategy_b = compile_strategy(STRATEGY_B_SOURCE, vec!["AAPL".to_string()]);

    let aggregator = SignalAggregator::new(RiskConstraints::default());
    let tracker = LivePositionTracker::new(100_000.0);

    let mut harness = LiveHarness::new(
        vec![strategy_a, strategy_b],
        aggregator,
        tracker,
        None,
        ReconnectPolicy::default(),
        Duration::from_secs(60),
        None,
        None,
        None,
        None,
        None,
        None,
    );

    let (tx, rx) = mpsc::channel::<LiveBar>(16);

    // Send bars in a background task then drop sender to close channel
    tokio::spawn(async move {
        // Bar 1: both trigger (close > open AND close > high*0.95)
        tx.send(make_bar("AAPL", 100.0, 105.0, 99.0, 104.0))
            .await
            .unwrap();
        // Bar 2: neither re-opens (in_position=true)
        tx.send(make_bar("AAPL", 103.0, 106.0, 102.0, 105.0))
            .await
            .unwrap();
        // Bar 3: another bar for state tracking
        tx.send(make_bar("AAPL", 104.0, 107.0, 103.0, 106.0))
            .await
            .unwrap();
        // Drop sender to close channel and end the harness loop
    });

    // Run the harness — it exits when channel closes (connector_count=0 means clean exit)
    let result = harness.run(rx, 0).await;
    assert!(result.is_ok(), "Harness should exit cleanly when channel closes");

    // Verify state: both strategies processed 3 bars
    assert!(
        matches!(
            harness.strategies[0].interpreter.state.get("bar_count"),
            Some(flux_cli::interpreter::Value::Int(3))
        ),
    );
    assert!(
        matches!(
            harness.strategies[1].interpreter.state.get("bar_count"),
            Some(flux_cli::interpreter::Value::Int(3))
        ),
    );

    // Verify unified tracker: position should be 150 (100 from A + 50 from B on bar 1)
    let position = harness.tracker.inner.position("AAPL");
    assert!(position.is_some());
    assert_eq!(position.unwrap().qty, 150.0);

    // Verify attribution: 2 fills (one from each strategy on bar 1)
    assert_eq!(harness.tracker.fill_attribution.len(), 2);
    assert_eq!(harness.tracker.fill_attribution[0], "BullishOpener");
    assert_eq!(harness.tracker.fill_attribution[1], "NearHighOpener");
}

#[tokio::test]
async fn strategies_on_different_symbols_dont_interfere() {
    // Strategy A subscribes to AAPL, Strategy B subscribes to MSFT
    // Verify bars are correctly routed and state is independent
    let strategy_a = compile_strategy(STRATEGY_A_SOURCE, vec!["AAPL".to_string()]);
    let strategy_b = compile_strategy(STRATEGY_B_SOURCE, vec!["MSFT".to_string()]);

    let aggregator = SignalAggregator::new(RiskConstraints::default());
    let tracker = LivePositionTracker::new(100_000.0);

    let mut harness = LiveHarness::new(
        vec![strategy_a, strategy_b],
        aggregator,
        tracker,
        None,
        ReconnectPolicy::default(),
        Duration::from_secs(60),
        None,
        None,
        None,
        None,
        None,
        None,
    );

    // Send an AAPL bar — only strategy A should process it
    let aapl_bar = make_bar("AAPL", 100.0, 105.0, 99.0, 104.0);
    harness.dispatch_bar(&aapl_bar);

    // Strategy A processed 1 bar, Strategy B processed 0 bars
    assert!(
        matches!(
            harness.strategies[0].interpreter.state.get("bar_count"),
            Some(flux_cli::interpreter::Value::Int(1))
        ),
    );
    assert!(
        matches!(
            harness.strategies[1].interpreter.state.get("bar_count"),
            Some(flux_cli::interpreter::Value::Int(0))
        ),
        "Strategy B should not have processed the AAPL bar"
    );

    // Only strategy A opened a position
    assert_eq!(harness.tracker.fill_attribution.len(), 1);
    assert_eq!(harness.tracker.fill_attribution[0], "BullishOpener");

    // Now send an MSFT bar — only strategy B should process it
    let msft_bar = make_bar("MSFT", 200.0, 210.0, 195.0, 208.0);
    harness.dispatch_bar(&msft_bar);

    // Strategy A still at 1 bar, Strategy B now at 1 bar
    assert!(
        matches!(
            harness.strategies[0].interpreter.state.get("bar_count"),
            Some(flux_cli::interpreter::Value::Int(1))
        ),
    );
    assert!(
        matches!(
            harness.strategies[1].interpreter.state.get("bar_count"),
            Some(flux_cli::interpreter::Value::Int(1))
        ),
    );

    // Strategy B should have opened MSFT (close=208 > high*0.95=210*0.95=199.5)
    assert_eq!(harness.tracker.fill_attribution.len(), 2);
    assert_eq!(harness.tracker.fill_attribution[1], "NearHighOpener");

    // Positions: AAPL=100 (from A), MSFT=50 (from B)
    assert_eq!(harness.tracker.inner.position("AAPL").unwrap().qty, 100.0);
    assert_eq!(harness.tracker.inner.position("MSFT").unwrap().qty, 50.0);
}
