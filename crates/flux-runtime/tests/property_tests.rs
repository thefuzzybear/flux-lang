// crates/flux-runtime/tests/property_tests.rs
//
// Integration property tests for flux-runtime using the PUBLIC API only.
//
// NOTE: Properties 2 (SMA Correctness), 3 (EMA Correctness), and 4 (Indicator Call-Site Isolation)
// are tested inline in their respective source modules (sma.rs, ema.rs) rather than here.
// Reason: `#[track_caller]` means calling `sma()` or `ema()` from within a proptest loop
// (same source line) will share state across iterations, making it impossible to test
// sequential correctness from an integration test. The inline module tests call
// `reset_indicator_state()` between iterations which is not part of the public API.
//
// This file covers Properties 1, 5, and 6 which work cleanly from the public API.

use flux_runtime::{BarContext, Signal, Strategy, run_backtest, sma};
use proptest::prelude::*;
use proptest::strategy::Strategy as PropStrategy;

// --- Generators ---

fn arb_bar_context() -> impl proptest::strategy::Strategy<Value = BarContext> {
    (
        1.0..10000.0f64,
        1.0..10000.0f64,
        1.0..10000.0f64,
        1.0..10000.0f64,
        0.0..1e9f64,
        "[A-Z]{1,5}",
        any::<bool>(),
    )
        .prop_map(|(close, open, high, low, volume, symbol, in_position)| BarContext {
            close,
            open,
            high,
            low,
            volume,
            symbol,
            in_position,
        })
}

// --- Test helper strategies ---

/// Strategy that emits a signal on every other bar (deterministic, no indicator state).
struct CountingStrategy {
    count: usize,
}

impl CountingStrategy {
    fn new() -> Self {
        Self { count: 0 }
    }
}

impl Strategy for CountingStrategy {
    fn on_bar(&mut self, ctx: &BarContext) -> Vec<Signal> {
        self.count += 1;
        if self.count % 2 == 0 {
            vec![Signal::open(ctx.symbol.clone(), self.count as f64)]
        } else {
            Vec::new()
        }
    }
}

/// Strategy that uses SMA indicator (has state that needs resetting between runs).
struct SmaStrategy {
    period: i64,
}

impl Strategy for SmaStrategy {
    fn on_bar(&mut self, ctx: &BarContext) -> Vec<Signal> {
        let avg = sma(ctx.close, self.period);
        if ctx.close > avg {
            vec![Signal::open(ctx.symbol.clone(), 100.0)]
        } else {
            Vec::new()
        }
    }
}

proptest! {
    // Feature: flux-runtime, Property 1: Signal Constructor Round-Trip
    // **Validates: Requirements 3.2, 3.3, 3.4, 3.6**
    #[test]
    fn prop_signal_round_trip(
        symbol in "[A-Z]{1,5}",
        qty in 0.01..10000.0f64,
    ) {
        let open_sig = Signal::open(symbol.clone(), qty);
        prop_assert_eq!(open_sig.symbol(), symbol.as_str());
        prop_assert_eq!(open_sig.qty(), Some(qty));

        let close_sig = Signal::close(symbol.clone());
        prop_assert_eq!(close_sig.symbol(), symbol.as_str());
        prop_assert_eq!(close_sig.qty(), None);

        let close_qty_sig = Signal::close_qty(symbol.clone(), qty);
        prop_assert_eq!(close_qty_sig.symbol(), symbol.as_str());
        prop_assert_eq!(close_qty_sig.qty(), Some(qty));
    }

    // Feature: flux-runtime, Property 5: Backtest Determinism (State Reset)
    // **Validates: Requirements 6.3**
    #[test]
    fn prop_backtest_determinism(
        bars in proptest::collection::vec(arb_bar_context(), 0..20),
        period in 1..10i64,
    ) {
        let mut strategy1 = SmaStrategy { period };
        let mut strategy2 = SmaStrategy { period };

        let result1 = run_backtest(&mut strategy1, &bars);
        let result2 = run_backtest(&mut strategy2, &bars);

        prop_assert_eq!(result1.len(), result2.len(), "Lengths differ between runs");
        for (i, ((idx1, sig1), (idx2, sig2))) in result1.iter().zip(result2.iter()).enumerate() {
            prop_assert_eq!(idx1, idx2, "Bar index differs at position {}", i);
            prop_assert_eq!(sig1.symbol(), sig2.symbol(), "Symbol differs at position {}", i);
            prop_assert_eq!(sig1.qty(), sig2.qty(), "Qty differs at position {}", i);
        }
    }

    // Feature: flux-runtime, Property 6: Backtester Model Equivalence
    // **Validates: Requirements 7.2, 7.3, 7.4, 7.5**
    #[test]
    fn prop_backtest_model_equivalence(
        bars in proptest::collection::vec(arb_bar_context(), 0..20),
    ) {
        let mut strategy = CountingStrategy::new();
        let result = run_backtest(&mut strategy, &bars);

        // Manual model: iterate and collect signals with bar indices
        let mut manual_strategy = CountingStrategy::new();
        let mut expected: Vec<(usize, Signal)> = Vec::new();
        for (i, bar) in bars.iter().enumerate() {
            let signals = manual_strategy.on_bar(bar);
            for sig in signals {
                expected.push((i, sig));
            }
        }

        prop_assert_eq!(result.len(), expected.len(),
            "Result length {} != expected length {}", result.len(), expected.len());
        for (pos, ((idx1, sig1), (idx2, sig2))) in result.iter().zip(expected.iter()).enumerate() {
            prop_assert_eq!(idx1, idx2, "Bar index differs at position {}", pos);
            prop_assert_eq!(sig1.symbol(), sig2.symbol(), "Symbol differs at position {}", pos);
            prop_assert_eq!(sig1.qty(), sig2.qty(), "Qty differs at position {}", pos);
        }
    }
}
