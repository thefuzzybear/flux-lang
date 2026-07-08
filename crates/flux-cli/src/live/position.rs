//! Unified position tracker wrapper with per-strategy fill attribution.
//!
//! Wraps the existing `PositionTracker` from `flux-runtime` and adds
//! per-strategy attribution so fills can be traced back to the strategy
//! that generated the original signal.

use flux_runtime::{Fill, PositionTracker, Signal};

/// Wraps PositionTracker with per-strategy fill attribution.
///
/// The live harness maintains a single `LivePositionTracker` shared across
/// all loaded strategies. The inner `PositionTracker` handles fill simulation
/// identically to backtest mode, while `fill_attribution` maps each fill
/// (by index) to the strategy name that generated the signal.
pub struct LivePositionTracker {
    /// The underlying position tracker (same as backtest).
    pub inner: PositionTracker,
    /// Maps fill index → strategy name that generated the signal.
    pub fill_attribution: Vec<String>,
}

impl LivePositionTracker {
    /// Create a new tracker with the given initial capital.
    pub fn new(initial_capital: f64) -> Self {
        Self {
            inner: PositionTracker::new(initial_capital),
            fill_attribution: Vec::new(),
        }
    }

    /// Process a signal with strategy attribution.
    ///
    /// Delegates to the inner `PositionTracker::process_signal` and records
    /// the strategy name if a fill is produced. Returns the fill if one was
    /// generated (i.e., the signal was actionable given current positions).
    pub fn process_signal(
        &mut self,
        signal: &Signal,
        price: f64,
        bar_index: usize,
        strategy_name: &str,
    ) -> Option<Fill> {
        let fill = self.inner.process_signal(signal, price, bar_index);
        if fill.is_some() {
            self.fill_attribution.push(strategy_name.to_string());
        }
        fill
    }

    /// Derive `in_position` for a strategy based on its subscribed symbols.
    ///
    /// Returns true if ANY of the strategy's subscribed symbols have qty > 0
    /// in the unified position book. This is used to set the `in_position`
    /// field on `BarContext` before calling a strategy's `on bar` handler.
    pub fn in_position_for(&self, symbols: &[String]) -> bool {
        symbols.iter().any(|sym| {
            self.inner.position(sym).map_or(false, |p| p.qty > 0.0)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flux_runtime::Signal;

    #[test]
    fn new_tracker_starts_empty() {
        let tracker = LivePositionTracker::new(10_000.0);
        assert!(tracker.fill_attribution.is_empty());
        assert_eq!(tracker.inner.fills().len(), 0);
    }

    #[test]
    fn process_signal_records_attribution_on_fill() {
        let mut tracker = LivePositionTracker::new(10_000.0);
        let signal = Signal::open("AAPL".to_string(), 100.0);

        let fill = tracker.process_signal(&signal, 150.0, 0, "momentum");
        assert!(fill.is_some());
        assert_eq!(tracker.fill_attribution.len(), 1);
        assert_eq!(tracker.fill_attribution[0], "momentum");
    }

    #[test]
    fn process_signal_no_attribution_when_no_fill() {
        let mut tracker = LivePositionTracker::new(10_000.0);
        // Close on a symbol with no position → no fill
        let signal = Signal::close("AAPL".to_string());

        let fill = tracker.process_signal(&signal, 150.0, 0, "momentum");
        assert!(fill.is_none());
        assert!(tracker.fill_attribution.is_empty());
    }

    #[test]
    fn in_position_for_returns_false_when_no_positions() {
        let tracker = LivePositionTracker::new(10_000.0);
        let symbols = vec!["AAPL".to_string(), "MSFT".to_string()];
        assert!(!tracker.in_position_for(&symbols));
    }

    #[test]
    fn in_position_for_returns_true_when_any_symbol_has_position() {
        let mut tracker = LivePositionTracker::new(10_000.0);
        let signal = Signal::open("AAPL".to_string(), 50.0);
        tracker.process_signal(&signal, 150.0, 0, "mean_reversion");

        let symbols = vec!["AAPL".to_string(), "MSFT".to_string()];
        assert!(tracker.in_position_for(&symbols));
    }

    #[test]
    fn in_position_for_returns_false_when_subscribed_symbols_have_no_position() {
        let mut tracker = LivePositionTracker::new(10_000.0);
        // Open position in GOOG, but check for AAPL and MSFT
        let signal = Signal::open("GOOG".to_string(), 50.0);
        tracker.process_signal(&signal, 2800.0, 0, "other_strategy");

        let symbols = vec!["AAPL".to_string(), "MSFT".to_string()];
        assert!(!tracker.in_position_for(&symbols));
    }

    #[test]
    fn multiple_fills_track_attribution_correctly() {
        let mut tracker = LivePositionTracker::new(100_000.0);

        // Strategy A opens AAPL
        let signal_a = Signal::open("AAPL".to_string(), 100.0);
        tracker.process_signal(&signal_a, 150.0, 0, "strategy_a");

        // Strategy B opens MSFT
        let signal_b = Signal::open("MSFT".to_string(), 50.0);
        tracker.process_signal(&signal_b, 300.0, 1, "strategy_b");

        // Strategy A closes AAPL
        let close_a = Signal::close("AAPL".to_string());
        tracker.process_signal(&close_a, 155.0, 2, "strategy_a");

        assert_eq!(tracker.fill_attribution.len(), 3);
        assert_eq!(tracker.fill_attribution[0], "strategy_a");
        assert_eq!(tracker.fill_attribution[1], "strategy_b");
        assert_eq!(tracker.fill_attribution[2], "strategy_a");
    }
}
