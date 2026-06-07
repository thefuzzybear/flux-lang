use crate::context::BarContext;
use crate::indicators::state::reset_indicator_state;
use crate::signal::Signal;
use crate::strategy::Strategy;

/// Result of a backtest run — bar index paired with signals produced.
pub type BacktestResult = Vec<(usize, Signal)>;

/// Run a backtest: feed each bar to the strategy and collect all emitted signals.
///
/// Resets indicator state before beginning so each run starts clean.
///
/// # Arguments
/// * `strategy` - Mutable reference to any type implementing `Strategy`
/// * `bars` - Slice of bar data to feed in order
///
/// # Returns
/// A vector of (bar_index, signal) pairs in chronological order
pub fn run_backtest(strategy: &mut dyn Strategy, bars: &[BarContext]) -> BacktestResult {
    reset_indicator_state();

    let mut results: BacktestResult = Vec::new();

    for (i, bar) in bars.iter().enumerate() {
        let signals = strategy.on_bar(bar);
        for signal in signals {
            results.push((i, signal));
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indicators::sma;
    use crate::strategy::Strategy;
    use proptest::prelude::prop_assert_eq;
    use proptest::proptest;
    #[allow(unused_imports)]
    use proptest::strategy::Strategy as _;

    // --- Test helper strategies ---

    /// A strategy that never emits signals.
    struct EmptyStrategy;

    impl Strategy for EmptyStrategy {
        fn on_bar(&mut self, _ctx: &BarContext) -> Vec<Signal> {
            Vec::new()
        }
    }

    /// A strategy that always opens a position on every bar.
    struct AlwaysOpenStrategy;

    impl Strategy for AlwaysOpenStrategy {
        fn on_bar(&mut self, ctx: &BarContext) -> Vec<Signal> {
            vec![Signal::open(ctx.symbol.clone(), 100.0)]
        }
    }

    /// A strategy that emits multiple signals per bar.
    struct MultiSignalStrategy;

    impl Strategy for MultiSignalStrategy {
        fn on_bar(&mut self, ctx: &BarContext) -> Vec<Signal> {
            vec![
                Signal::open(ctx.symbol.clone(), 50.0),
                Signal::close(ctx.symbol.clone()),
            ]
        }
    }

    /// A strategy that emits signals only on even-indexed bars (tracked internally).
    struct EvenBarStrategy {
        call_count: usize,
    }

    impl EvenBarStrategy {
        fn new() -> Self {
            Self { call_count: 0 }
        }
    }

    impl Strategy for EvenBarStrategy {
        fn on_bar(&mut self, ctx: &BarContext) -> Vec<Signal> {
            let signals = if self.call_count % 2 == 0 {
                vec![Signal::open(ctx.symbol.clone(), 10.0)]
            } else {
                Vec::new()
            };
            self.call_count += 1;
            signals
        }
    }

    // --- Helper to create a BarContext ---

    fn make_bar(symbol: &str, close: f64) -> BarContext {
        BarContext {
            close,
            open: close - 1.0,
            high: close + 1.0,
            low: close - 2.0,
            volume: 1000.0,
            symbol: symbol.to_string(),
            in_position: false,
        }
    }

    // --- Tests ---

    #[test]
    fn empty_bars_returns_empty() {
        let mut strategy = EmptyStrategy;
        let result = run_backtest(&mut strategy, &[]);
        assert!(result.is_empty());
    }

    #[test]
    fn single_bar_no_signals() {
        let mut strategy = EmptyStrategy;
        let bars = vec![make_bar("AAPL", 150.0)];
        let result = run_backtest(&mut strategy, &bars);
        assert!(result.is_empty());
    }

    #[test]
    fn single_bar_with_signals() {
        let mut strategy = AlwaysOpenStrategy;
        let bars = vec![make_bar("AAPL", 150.0)];
        let result = run_backtest(&mut strategy, &bars);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, 0);
        assert_eq!(result[0].1.symbol(), "AAPL");
        assert_eq!(result[0].1.qty(), Some(100.0));
    }

    #[test]
    fn multiple_bars_all_signal() {
        let mut strategy = AlwaysOpenStrategy;
        let bars = vec![
            make_bar("AAPL", 150.0),
            make_bar("AAPL", 155.0),
            make_bar("AAPL", 160.0),
        ];
        let result = run_backtest(&mut strategy, &bars);
        assert_eq!(result.len(), 3);
        for (i, (idx, sig)) in result.iter().enumerate() {
            assert_eq!(*idx, i);
            assert_eq!(sig.symbol(), "AAPL");
            assert_eq!(sig.qty(), Some(100.0));
        }
    }

    #[test]
    fn multiple_signals_per_bar() {
        let mut strategy = MultiSignalStrategy;
        let bars = vec![make_bar("MSFT", 300.0)];
        let result = run_backtest(&mut strategy, &bars);
        assert_eq!(result.len(), 2);
        // Both signals should be at index 0
        assert_eq!(result[0].0, 0);
        assert_eq!(result[1].0, 0);
        // First signal is Open
        assert_eq!(result[0].1.symbol(), "MSFT");
        assert_eq!(result[0].1.qty(), Some(50.0));
        // Second signal is Close
        assert_eq!(result[1].1.symbol(), "MSFT");
        assert_eq!(result[1].1.qty(), None);
    }

    #[test]
    fn bar_index_correctness() {
        let mut strategy = EvenBarStrategy::new();
        let bars = vec![
            make_bar("SPY", 400.0),
            make_bar("SPY", 401.0),
            make_bar("SPY", 402.0),
            make_bar("SPY", 403.0),
        ];
        let result = run_backtest(&mut strategy, &bars);
        // Signals only on even call counts: bars 0 and 2
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, 0);
        assert_eq!(result[1].0, 2);
    }

    #[test]
    fn empty_bars_with_always_open_returns_empty() {
        let mut strategy = AlwaysOpenStrategy;
        let result = run_backtest(&mut strategy, &[]);
        assert!(result.is_empty());
    }

    // --- Property test helper strategy using SMA indicator ---

    /// A strategy that uses SMA indicator state — useful for testing state reset.
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

    fn arb_bar_context() -> impl proptest::strategy::Strategy<Value = BarContext> {
        (
            1.0..10000.0f64,
            1.0..10000.0f64,
            1.0..10000.0f64,
            1.0..10000.0f64,
            0.0..1e9f64,
            "[A-Z]{1,5}",
            proptest::bool::ANY,
        )
            .prop_map(|(close, open, high, low, volume, symbol, in_position)| {
                BarContext { close, open, high, low, volume, symbol, in_position }
            })
    }

    // Feature: flux-runtime, Property 5: Backtest Determinism (State Reset)
    // **Validates: Requirements 6.3**
    proptest! {
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
    }

    // --- Property test helper for model equivalence ---

    /// A deterministic strategy for model equivalence testing.
    /// Emits an Open signal on every even call (no indicator state involved).
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

    // Feature: flux-runtime, Property 6: Backtester Model Equivalence
    // **Validates: Requirements 7.2, 7.3, 7.4, 7.5**
    proptest! {
        #[test]
        fn prop_backtest_model_equivalence(
            bars in proptest::collection::vec(arb_bar_context(), 0..20),
        ) {
            // Run through the backtester
            let mut strategy = CountingStrategy::new();
            let result = run_backtest(&mut strategy, &bars);

            // Manual model: reset state, iterate bars, collect signals
            reset_indicator_state();
            let mut manual_strategy = CountingStrategy::new();
            let mut expected: Vec<(usize, Signal)> = Vec::new();
            for (i, bar) in bars.iter().enumerate() {
                let signals = manual_strategy.on_bar(bar);
                for sig in signals {
                    expected.push((i, sig));
                }
            }

            prop_assert_eq!(result.len(), expected.len(), "Length mismatch");
            for (pos, ((idx1, sig1), (idx2, sig2))) in result.iter().zip(expected.iter()).enumerate() {
                prop_assert_eq!(idx1, idx2, "Bar index differs at position {}", pos);
                prop_assert_eq!(sig1.symbol(), sig2.symbol(), "Symbol differs at position {}", pos);
                prop_assert_eq!(sig1.qty(), sig2.qty(), "Qty differs at position {}", pos);
            }
        }
    }
}
