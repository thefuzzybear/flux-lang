//! Property tests for bar dispatch and strategy purity.
//!
//! Feature: flux-live-harness, Properties 1, 2, 3
//!
//! **Validates: Requirements 1.2, 2.3, 2.4, 10.1, 10.2, 10.3, 10.5**
//!
//! - Property 1: Bar dispatch correctness — only subscribed strategies receive each bar
//! - Property 2: Strategy purity (backtest-live equivalence) — identical signals for same bars
//! - Property 3: Strategy state isolation — two strategies' state evolves independently

use std::path::PathBuf;
use std::time::Duration;

use proptest::prelude::*;

use flux_cli::interpreter::Interpreter;
use flux_cli::live::aggregator::{RiskConstraints, SignalAggregator};
use flux_cli::live::connector::{LiveBar, ReconnectPolicy};
use flux_cli::live::harness::LiveHarness;
use flux_cli::live::loader::StrategyModule;
use flux_cli::live::position::LivePositionTracker;
use flux_runtime::BarContext;

// =============================================================================
// Helpers
// =============================================================================

/// Compile a minimal .flux strategy source and return a StrategyModule.
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

/// Create a LiveBar with given symbol and OHLCV values.
fn make_live_bar(symbol: &str, open: f64, close: f64) -> LiveBar {
    LiveBar {
        bar: BarContext {
            symbol: symbol.to_string(),
            open,
            high: close.max(open) + 1.0,
            low: close.min(open) - 1.0,
            close,
            volume: 1000.0,
            in_position: false,
        },
        connector_id: "test".to_string(),
        received_at: chrono::Utc::now(),
    }
}

/// Create a LiveHarness with the given strategies and no risk constraints.
fn make_harness(strategies: Vec<StrategyModule>) -> LiveHarness {
    LiveHarness::new(
        strategies,
        SignalAggregator::new(RiskConstraints::default()),
        LivePositionTracker::new(100_000.0),
        None,
        ReconnectPolicy::default(),
        Duration::from_secs(30),
        None,
        None,
        None,
        None,
    )
}

/// A simple strategy source that opens when close > open.
const SIMPLE_STRATEGY_SOURCE: &str = r#"strategy Simple {
    on bar {
        if close > open and not in_position {
            OPEN(symbol, 100.0)
        }
        if close < open and in_position {
            CLOSE(symbol)
        }
    }
}"#;

/// A strategy with state that counts bars.
const COUNTING_STRATEGY_SOURCE: &str = r#"strategy Counter {
    state {
        bar_count = 0
    }

    on bar {
        bar_count = bar_count + 1
        if close > open and not in_position {
            OPEN(symbol, 100.0)
        }
    }
}"#;

// =============================================================================
// Generators
// =============================================================================

/// Generate a valid symbol name from a small set (to create meaningful overlaps).
fn arb_symbol() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("AAPL".to_string()),
        Just("MSFT".to_string()),
        Just("GOOG".to_string()),
        Just("TSLA".to_string()),
    ]
}

/// Generate a set of subscribed symbols (1-3 symbols).
fn arb_symbol_subscriptions() -> impl Strategy<Value = Vec<String>> {
    proptest::collection::hash_set(arb_symbol(), 1..=3)
        .prop_map(|s| s.into_iter().collect())
}

/// Generate an OHLCV bar with the given symbol.
fn arb_bar_for_symbol(symbol: String) -> impl Strategy<Value = LiveBar> {
    (50.0f64..200.0, 50.0f64..200.0).prop_map(move |(open, close)| {
        make_live_bar(&symbol, open, close)
    })
}

/// Generate a random bar (random symbol from the set).
fn arb_bar() -> impl Strategy<Value = LiveBar> {
    arb_symbol().prop_flat_map(|sym| arb_bar_for_symbol(sym))
}

/// Generate a sequence of bars (2-10 bars).
fn arb_bar_sequence() -> impl Strategy<Value = Vec<LiveBar>> {
    proptest::collection::vec(arb_bar(), 2..=10)
}

// =============================================================================
// Property 1: Bar dispatch correctness
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 1.2, 2.3**
    ///
    /// For any bar and set of strategy subscriptions, only strategies whose
    /// subscribed_symbols includes the bar's symbol should have their
    /// interpreter's on_bar called (producing signals or state changes).
    #[test]
    fn prop_bar_dispatch_only_to_subscribed_strategies(
        subs_a in arb_symbol_subscriptions(),
        subs_b in arb_symbol_subscriptions(),
        bar in arb_bar(),
    ) {
        // Create two strategies with different subscriptions
        let strat_a = compile_strategy(COUNTING_STRATEGY_SOURCE, "StratA", subs_a.clone());
        let strat_b = compile_strategy(COUNTING_STRATEGY_SOURCE, "StratB", subs_b.clone());

        let mut harness = make_harness(vec![strat_a, strat_b]);

        // Record initial bar_count state for each strategy (as debug string for comparison)
        let initial_a = format!("{:?}", harness.strategies[0].interpreter.state.get("bar_count"));
        let initial_b = format!("{:?}", harness.strategies[1].interpreter.state.get("bar_count"));

        // Dispatch bar
        harness.dispatch_bar(&bar);

        // Check if bar_count changed — indicates on_bar was called
        let final_a = format!("{:?}", harness.strategies[0].interpreter.state.get("bar_count"));
        let final_b = format!("{:?}", harness.strategies[1].interpreter.state.get("bar_count"));

        let a_received = final_a != initial_a;
        let b_received = final_b != initial_b;

        let a_should_receive = subs_a.contains(&bar.bar.symbol);
        let b_should_receive = subs_b.contains(&bar.bar.symbol);

        prop_assert_eq!(
            a_received, a_should_receive,
            "StratA (subscribed to {:?}) {} receive bar for symbol '{}' but {} it",
            subs_a,
            if a_should_receive { "should" } else { "should not" },
            bar.bar.symbol,
            if a_received { "received" } else { "did not receive" }
        );

        prop_assert_eq!(
            b_received, b_should_receive,
            "StratB (subscribed to {:?}) {} receive bar for symbol '{}' but {} it",
            subs_b,
            if b_should_receive { "should" } else { "should not" },
            bar.bar.symbol,
            if b_received { "received" } else { "did not receive" }
        );
    }
}

// =============================================================================
// Property 2: Strategy purity (backtest-live equivalence)
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    /// **Validates: Requirements 10.1, 10.2, 10.3, 10.5**
    ///
    /// For any bar sequence, a strategy produces identical signals whether
    /// executed via direct interpreter.on_bar() calls (backtest mode) or
    /// via the LiveHarness dispatch_bar() path (live mode), given the same
    /// in_position state management.
    #[test]
    fn prop_strategy_purity_backtest_live_equivalence(
        bars in arb_bar_sequence(),
    ) {
        let symbol = "AAPL".to_string();

        // Filter bars to only use AAPL symbol to ensure the strategy receives them
        let bars: Vec<LiveBar> = bars.into_iter().map(|mut b| {
            b.bar.symbol = symbol.clone();
            b
        }).collect();

        prop_assume!(!bars.is_empty());

        // --- Backtest mode: call interpreter.on_bar() directly ---
        let backtest_strat = compile_strategy(
            SIMPLE_STRATEGY_SOURCE,
            "BacktestMode",
            vec![symbol.clone()],
        );
        let mut backtest_interp = backtest_strat.interpreter;
        let mut backtest_signals = Vec::new();
        let mut backtest_in_position = false;

        for bar in &bars {
            backtest_interp.in_position = backtest_in_position;
            let signals = backtest_interp.on_bar(&bar.bar);
            // Track in_position the same way the harness does via the tracker:
            // An OPEN sets it true, a CLOSE sets it false
            for sig in &signals {
                match sig {
                    flux_runtime::Signal::Open { .. } => backtest_in_position = true,
                    flux_runtime::Signal::Close { .. } => backtest_in_position = false,
                    flux_runtime::Signal::CloseQty { .. } => {}
                }
            }
            backtest_signals.push(signals);
        }

        // --- Live mode: use LiveHarness dispatch_bar() ---
        let live_strat = compile_strategy(
            SIMPLE_STRATEGY_SOURCE,
            "LiveMode",
            vec![symbol.clone()],
        );
        let mut harness = make_harness(vec![live_strat]);

        for bar in &bars {
            harness.dispatch_bar(bar);
        }

        // Since we can't directly capture signals from dispatch_bar, compare
        // the final interpreter state which is deterministic for same inputs.
        // The in_position state proves the strategy saw the same bars and
        // made the same decisions.
        let live_in_position = harness.strategies[0].interpreter.in_position;

        prop_assert_eq!(
            backtest_in_position,
            live_in_position,
            "Final in_position state diverged: backtest={}, live={}. \
             This means the strategy produced different signals in live vs backtest mode.",
            backtest_in_position,
            live_in_position,
        );

        // Additionally verify the state variables are identical
        let backtest_state = &backtest_interp.state;
        let live_state = &harness.strategies[0].interpreter.state;

        // Both should have the same keys and values
        prop_assert_eq!(
            backtest_state.len(),
            live_state.len(),
            "State variable count differs"
        );

        for (key, backtest_val) in backtest_state {
            let live_val = live_state.get(key);
            prop_assert!(
                live_val.is_some(),
                "Live mode missing state variable '{}'", key
            );
            let live_val = live_val.unwrap();
            let bt_str = format!("{:?}", backtest_val);
            let lv_str = format!("{:?}", live_val);
            prop_assert!(
                bt_str == lv_str,
                "State variable '{}' diverged: backtest={}, live={}",
                key, bt_str, lv_str
            );
        }
    }
}

// =============================================================================
// Property 3: Strategy state isolation
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(80))]

    /// **Validates: Requirements 2.4, 10.5**
    ///
    /// Two strategies loaded from the same source but with different subscriptions
    /// maintain completely independent state. Feeding bars to both via the harness
    /// results in each strategy's bar_count reflecting only the bars it received
    /// (based on its subscriptions), not the total bars dispatched.
    #[test]
    fn prop_strategy_state_isolation(
        bars in proptest::collection::vec(arb_bar(), 3..=15),
    ) {
        prop_assume!(!bars.is_empty());

        // Strategy A subscribes to AAPL only
        let strat_a = compile_strategy(
            COUNTING_STRATEGY_SOURCE,
            "IsolationA",
            vec!["AAPL".to_string()],
        );

        // Strategy B subscribes to MSFT only
        let strat_b = compile_strategy(
            COUNTING_STRATEGY_SOURCE,
            "IsolationB",
            vec!["MSFT".to_string()],
        );

        let mut harness = make_harness(vec![strat_a, strat_b]);

        // Dispatch all bars
        for bar in &bars {
            harness.dispatch_bar(bar);
        }

        // Count how many bars each strategy should have received
        let expected_a_count = bars.iter().filter(|b| b.bar.symbol == "AAPL").count();
        let expected_b_count = bars.iter().filter(|b| b.bar.symbol == "MSFT").count();

        // Get actual bar_count from each strategy's state
        let actual_a_count = match harness.strategies[0].interpreter.state.get("bar_count") {
            Some(flux_cli::interpreter::Value::Int(n)) => *n as usize,
            Some(flux_cli::interpreter::Value::Float(n)) => *n as usize,
            _ => 0,
        };
        let actual_b_count = match harness.strategies[1].interpreter.state.get("bar_count") {
            Some(flux_cli::interpreter::Value::Int(n)) => *n as usize,
            Some(flux_cli::interpreter::Value::Float(n)) => *n as usize,
            _ => 0,
        };

        prop_assert_eq!(
            actual_a_count,
            expected_a_count,
            "StratA (subscribed to AAPL) should have bar_count={} but has {}. \
             Bars dispatched: {:?}",
            expected_a_count,
            actual_a_count,
            bars.iter().map(|b| b.bar.symbol.as_str()).collect::<Vec<_>>()
        );

        prop_assert_eq!(
            actual_b_count,
            expected_b_count,
            "StratB (subscribed to MSFT) should have bar_count={} but has {}. \
             Bars dispatched: {:?}",
            expected_b_count,
            actual_b_count,
            bars.iter().map(|b| b.bar.symbol.as_str()).collect::<Vec<_>>()
        );

        // Additionally verify that StratA's state is independent of StratB's state:
        // they should only be equal if they received the same number of bars,
        // which should be by chance, not by sharing state.
        if expected_a_count != expected_b_count {
            prop_assert_ne!(
                actual_a_count,
                actual_b_count,
                "Strategies with different bar counts shouldn't have identical bar_count state"
            );
        }
    }
}
