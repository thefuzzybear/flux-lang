//! Signal aggregator with portfolio-level risk constraints.
//!
//! Collects signals from all strategies for a given bar and applies
//! constraints in deterministic order: position size → exposure →
//! position count. CLOSE signals always pass through unconstrained.

use std::collections::HashMap;

use flux_runtime::{PositionTracker, Signal};

/// Portfolio-level risk constraints applied to aggregated signals.
#[derive(Debug, Clone)]
pub struct RiskConstraints {
    /// Maximum position size per symbol (in quantity units).
    /// If `None`, no per-symbol size limit is enforced.
    pub max_position_size: Option<f64>,
    /// Maximum gross exposure (in capital units).
    /// If `None`, no exposure limit is enforced.
    pub max_exposure: Option<f64>,
    /// Maximum number of concurrent open positions.
    /// If `None`, no position count limit is enforced.
    pub max_positions: Option<usize>,
}

impl Default for RiskConstraints {
    fn default() -> Self {
        Self {
            max_position_size: None,
            max_exposure: None,
            max_positions: None,
        }
    }
}

/// Reason a signal was rejected by the aggregator.
#[derive(Debug, Clone)]
pub enum RejectionReason {
    /// The signal would push the per-symbol position size above the limit.
    PositionSizeExceeded {
        symbol: String,
        current: f64,
        requested: f64,
        limit: f64,
    },
    /// The signal would push gross portfolio exposure above the limit.
    ExposureExceeded {
        current: f64,
        additional: f64,
        limit: f64,
    },
    /// The signal would open a new position beyond the maximum count.
    PositionCountExceeded {
        current: usize,
        limit: usize,
    },
}

/// Aggregates signals from multiple strategies and applies risk constraints.
///
/// The aggregator processes all signals for a given bar in sequence,
/// tracking pending state changes so that multiple signals targeting
/// the same symbol are evaluated correctly against cumulative limits.
pub struct SignalAggregator {
    constraints: RiskConstraints,
}

impl SignalAggregator {
    /// Create a new aggregator with the given risk constraints.
    pub fn new(constraints: RiskConstraints) -> Self {
        Self { constraints }
    }

    /// Process signals from all strategies, applying constraints.
    ///
    /// Returns approved `(strategy_name, signal)` pairs.
    ///
    /// Constraint application order (deterministic):
    /// 1. Position size limit (per-symbol)
    /// 2. Exposure limit (portfolio-wide)
    /// 3. Position count limit (portfolio-wide)
    ///
    /// CLOSE and CLOSE_QTY signals always pass through unconstrained.
    pub fn process(
        &self,
        signals: &[(String, Signal)],
        tracker: &PositionTracker,
    ) -> Vec<(String, Signal)> {
        let mut approved: Vec<(String, Signal)> = Vec::new();

        // Track pending state changes during processing so that
        // multi-signal evaluation is correct (e.g., two OPENs on same symbol).
        let mut pending_qty: HashMap<String, f64> = HashMap::new();
        let mut pending_new_positions: usize = 0;
        let mut pending_exposure: f64 = 0.0;

        for (strategy_name, signal) in signals {
            match signal {
                // CLOSE and CLOSE_QTY signals always pass through unconstrained
                Signal::Close { .. } | Signal::CloseQty { .. } => {
                    approved.push((strategy_name.clone(), signal.clone()));
                }
                Signal::Open { symbol, qty } | Signal::Short { symbol, qty } => {
                    // Check 1: Position size limit (per-symbol)
                    if let Some(max_size) = self.constraints.max_position_size {
                        let current_qty = tracker
                            .position(symbol)
                            .map_or(0.0, |p| p.qty)
                            + pending_qty.get(symbol).copied().unwrap_or(0.0);

                        if current_qty + qty > max_size {
                            eprintln!(
                                "  [RISK] rejected OPEN({}, {}) from {}: \
                                 position size {:.4} + {:.4} > limit {:.4}",
                                symbol, qty, strategy_name, current_qty, qty, max_size
                            );
                            continue;
                        }
                    }

                    // Check 2: Gross exposure limit (portfolio-wide)
                    if let Some(max_exp) = self.constraints.max_exposure {
                        let current_exposure = tracker.gross_exposure() + pending_exposure;
                        // Estimate exposure contribution using best available price
                        let price_est = tracker
                            .position(symbol)
                            .map_or(100.0, |p| p.avg_entry_price);
                        let additional_exposure = qty * price_est;

                        if current_exposure + additional_exposure > max_exp {
                            eprintln!(
                                "  [RISK] rejected OPEN({}, {}) from {}: \
                                 exposure {:.2} + {:.2} > limit {:.2}",
                                symbol, qty, strategy_name,
                                current_exposure, additional_exposure, max_exp
                            );
                            continue;
                        }

                        // Track pending exposure for subsequent signals
                        pending_exposure += additional_exposure;
                    }

                    // Check 3: Position count limit (portfolio-wide)
                    if let Some(max_pos) = self.constraints.max_positions {
                        let current_count =
                            tracker.open_position_count() + pending_new_positions;
                        let is_new_position = tracker.position(symbol).is_none()
                            && !pending_qty.contains_key(symbol);

                        if is_new_position && current_count >= max_pos {
                            eprintln!(
                                "  [RISK] rejected OPEN({}, {}) from {}: \
                                 position count {} >= limit {}",
                                symbol, qty, strategy_name, current_count, max_pos
                            );
                            continue;
                        }

                        // Track pending new position for subsequent signals
                        if is_new_position {
                            pending_new_positions += 1;
                        }
                    }

                    // Signal approved — update pending state
                    *pending_qty.entry(symbol.clone()).or_insert(0.0) += qty;
                    approved.push((strategy_name.clone(), signal.clone()));
                }
            }
        }

        approved
    }

    /// Get a reference to the current risk constraints.
    pub fn constraints(&self) -> &RiskConstraints {
        &self.constraints
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flux_runtime::PositionTracker;

    /// Helper to create an aggregator with given constraints.
    fn aggregator_with(
        max_position_size: Option<f64>,
        max_exposure: Option<f64>,
        max_positions: Option<usize>,
    ) -> SignalAggregator {
        SignalAggregator::new(RiskConstraints {
            max_position_size,
            max_exposure,
            max_positions,
        })
    }

    #[test]
    fn close_signals_always_pass_through() {
        let agg = aggregator_with(Some(10.0), Some(100.0), Some(1));
        let tracker = PositionTracker::new(10000.0);

        let signals = vec![
            ("strat_a".to_string(), Signal::close("AAPL".to_string())),
            (
                "strat_b".to_string(),
                Signal::close_qty("MSFT".to_string(), 50.0),
            ),
        ];

        let approved = agg.process(&signals, &tracker);
        assert_eq!(approved.len(), 2);
        assert_eq!(approved[0].0, "strat_a");
        assert_eq!(approved[0].1.symbol(), "AAPL");
        assert_eq!(approved[1].0, "strat_b");
        assert_eq!(approved[1].1.symbol(), "MSFT");
    }

    #[test]
    fn position_size_constraint_rejects_exceeding_open() {
        let agg = aggregator_with(Some(100.0), None, None);
        let tracker = PositionTracker::new(10000.0);

        let signals = vec![(
            "strat_a".to_string(),
            Signal::open("AAPL".to_string(), 150.0),
        )];

        let approved = agg.process(&signals, &tracker);
        assert!(approved.is_empty(), "Signal should be rejected: qty 150 > limit 100");
    }

    #[test]
    fn position_size_constraint_includes_existing_position() {
        let agg = aggregator_with(Some(100.0), None, None);
        let mut tracker = PositionTracker::new(10000.0);

        // Pre-existing position of 80 shares
        tracker.process_signal(&Signal::open("AAPL".to_string(), 80.0), 150.0, 0);

        let signals = vec![(
            "strat_a".to_string(),
            Signal::open("AAPL".to_string(), 30.0),
        )];

        let approved = agg.process(&signals, &tracker);
        assert!(
            approved.is_empty(),
            "Signal should be rejected: 80 + 30 = 110 > limit 100"
        );
    }

    #[test]
    fn position_size_constraint_includes_pending_qty() {
        let agg = aggregator_with(Some(100.0), None, None);
        let tracker = PositionTracker::new(10000.0);

        // Two signals from different strategies for same symbol
        let signals = vec![
            ("strat_a".to_string(), Signal::open("AAPL".to_string(), 60.0)),
            ("strat_b".to_string(), Signal::open("AAPL".to_string(), 60.0)),
        ];

        let approved = agg.process(&signals, &tracker);
        // First signal (60) passes, second (60+60=120 > 100) is rejected
        assert_eq!(approved.len(), 1);
        assert_eq!(approved[0].0, "strat_a");
    }

    #[test]
    fn exposure_constraint_rejects_exceeding_open() {
        // max_exposure = 1000, opening 20 shares at price ~100 = 2000 exposure
        let agg = aggregator_with(None, Some(1000.0), None);
        let tracker = PositionTracker::new(10000.0);

        // With no existing position, price estimate defaults to 100.0
        let signals = vec![(
            "strat_a".to_string(),
            Signal::open("AAPL".to_string(), 20.0),
        )];

        let approved = agg.process(&signals, &tracker);
        assert!(
            approved.is_empty(),
            "Signal should be rejected: 20 * 100 = 2000 > limit 1000"
        );
    }

    #[test]
    fn exposure_constraint_uses_avg_entry_price() {
        let agg = aggregator_with(None, Some(50000.0), None);
        let mut tracker = PositionTracker::new(100000.0);

        // Open position at price 200, so avg_entry_price = 200
        tracker.process_signal(&Signal::open("AAPL".to_string(), 100.0), 200.0, 0);

        // Existing exposure: 100 * 200 = 20000
        // New signal: 200 qty * 200 price = 40000 additional
        // Total: 20000 + 40000 = 60000 > 50000
        let signals = vec![(
            "strat_a".to_string(),
            Signal::open("AAPL".to_string(), 200.0),
        )];

        let approved = agg.process(&signals, &tracker);
        assert!(
            approved.is_empty(),
            "Signal should be rejected: exposure 60000 > limit 50000"
        );
    }

    #[test]
    fn position_count_constraint_rejects_new_position() {
        let agg = aggregator_with(None, None, Some(2));
        let mut tracker = PositionTracker::new(10000.0);

        // Open 2 existing positions
        tracker.process_signal(&Signal::open("AAPL".to_string(), 10.0), 150.0, 0);
        tracker.process_signal(&Signal::open("MSFT".to_string(), 10.0), 300.0, 1);

        // Try to open a third position
        let signals = vec![(
            "strat_a".to_string(),
            Signal::open("GOOG".to_string(), 5.0),
        )];

        let approved = agg.process(&signals, &tracker);
        assert!(
            approved.is_empty(),
            "Signal should be rejected: 2 open positions >= limit 2"
        );
    }

    #[test]
    fn position_count_allows_adding_to_existing_position() {
        let agg = aggregator_with(None, None, Some(2));
        let mut tracker = PositionTracker::new(10000.0);

        // Open 2 existing positions
        tracker.process_signal(&Signal::open("AAPL".to_string(), 10.0), 150.0, 0);
        tracker.process_signal(&Signal::open("MSFT".to_string(), 10.0), 300.0, 1);

        // Adding to an existing position should be allowed
        let signals = vec![(
            "strat_a".to_string(),
            Signal::open("AAPL".to_string(), 5.0),
        )];

        let approved = agg.process(&signals, &tracker);
        assert_eq!(
            approved.len(),
            1,
            "Adding to existing position should be allowed even at max count"
        );
    }

    #[test]
    fn position_count_tracks_pending_new_positions() {
        let agg = aggregator_with(None, None, Some(2));
        let tracker = PositionTracker::new(10000.0);

        // Two signals opening new positions — second should be rejected
        let signals = vec![
            ("strat_a".to_string(), Signal::open("AAPL".to_string(), 10.0)),
            ("strat_b".to_string(), Signal::open("MSFT".to_string(), 10.0)),
            ("strat_c".to_string(), Signal::open("GOOG".to_string(), 10.0)),
        ];

        let approved = agg.process(&signals, &tracker);
        // First two should pass (0 → 1, 1 → 2), third rejected (2 >= 2)
        assert_eq!(approved.len(), 2);
        assert_eq!(approved[0].0, "strat_a");
        assert_eq!(approved[1].0, "strat_b");
    }

    #[test]
    fn no_constraints_passes_all_signals() {
        let agg = aggregator_with(None, None, None);
        let tracker = PositionTracker::new(10000.0);

        let signals = vec![
            ("strat_a".to_string(), Signal::open("AAPL".to_string(), 1000.0)),
            ("strat_b".to_string(), Signal::open("MSFT".to_string(), 2000.0)),
            ("strat_c".to_string(), Signal::close("GOOG".to_string())),
        ];

        let approved = agg.process(&signals, &tracker);
        assert_eq!(approved.len(), 3);
    }

    #[test]
    fn deterministic_constraint_order_position_size_first() {
        // Scenario: a signal violates both position size and exposure limits.
        // It should be rejected by position size check (first in order) without
        // affecting exposure tracking.
        let agg = aggregator_with(Some(50.0), Some(100.0), None);
        let tracker = PositionTracker::new(10000.0);

        // Signal of 100 qty violates position size limit of 50
        // It also violates exposure (100 * 100 = 10000 > 100)
        // After rejection, a second smaller signal should still check exposure correctly
        let signals = vec![
            ("strat_a".to_string(), Signal::open("AAPL".to_string(), 100.0)),
            ("strat_b".to_string(), Signal::open("MSFT".to_string(), 0.5)),
        ];

        let approved = agg.process(&signals, &tracker);
        // First rejected by position size, second passes position size (0.5 < 50)
        // and exposure (0.5 * 100 = 50 < 100)
        assert_eq!(approved.len(), 1);
        assert_eq!(approved[0].0, "strat_b");
    }

    #[test]
    fn mixed_signals_processed_correctly() {
        let agg = aggregator_with(Some(100.0), None, Some(3));
        let mut tracker = PositionTracker::new(10000.0);
        tracker.process_signal(&Signal::open("AAPL".to_string(), 50.0), 150.0, 0);

        let signals = vec![
            // Close always passes
            ("strat_a".to_string(), Signal::close("AAPL".to_string())),
            // Open new position - count was 1, now 0 after close conceptually,
            // but tracker still shows 1 (we process approved signals later)
            ("strat_b".to_string(), Signal::open("MSFT".to_string(), 30.0)),
            // Open on existing symbol (no count increase)
            ("strat_c".to_string(), Signal::open("AAPL".to_string(), 40.0)),
        ];

        let approved = agg.process(&signals, &tracker);
        // Close passes, MSFT open passes (new position, count 1+1=2 < 3),
        // AAPL open passes (existing position in tracker, just checks size: 50+40=90 < 100)
        assert_eq!(approved.len(), 3);
    }

    #[test]
    fn empty_signals_returns_empty() {
        let agg = aggregator_with(Some(100.0), Some(1000.0), Some(5));
        let tracker = PositionTracker::new(10000.0);

        let signals: Vec<(String, Signal)> = vec![];
        let approved = agg.process(&signals, &tracker);
        assert!(approved.is_empty());
    }

    #[test]
    fn default_risk_constraints_are_unconstrained() {
        let constraints = RiskConstraints::default();
        assert!(constraints.max_position_size.is_none());
        assert!(constraints.max_exposure.is_none());
        assert!(constraints.max_positions.is_none());
    }
}
