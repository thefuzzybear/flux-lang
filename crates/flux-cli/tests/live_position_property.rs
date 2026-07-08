// Feature: flux-live-harness, Properties 5 & 6: Position Tracker Properties
//!
//! Property-based tests verifying:
//! - Property 5: `in_position_for` derivation from unified tracker state
//! - Property 6: Fill attribution correctness (attribution vector matches fills)
//!
//! **Validates: Requirements 3.5, 3.7**

use flux_cli::live::position::LivePositionTracker;
use flux_runtime::Signal;
use proptest::prelude::*;

// ============================================================================
// Generators
// ============================================================================

/// Generate a random uppercase symbol (1-5 chars).
fn arb_symbol() -> impl Strategy<Value = String> {
    "[A-Z]{1,5}".prop_map(|s| s)
}

/// Generate a random strategy name (lowercase, 3-10 chars).
fn arb_strategy_name() -> impl Strategy<Value = String> {
    "[a-z]{3,10}".prop_map(|s| s)
}

/// Generate a random positive quantity.
fn arb_qty() -> impl Strategy<Value = f64> {
    1.0..1000.0f64
}

/// Generate a random positive price.
fn arb_price() -> impl Strategy<Value = f64> {
    1.0..5000.0f64
}



// ============================================================================
// Property 5: in_position derivation from unified tracker
// ============================================================================

proptest! {
    /// **Validates: Requirements 3.5**
    ///
    /// Property 5: in_position derivation from unified tracker
    ///
    /// For any set of symbols with open positions and any subset of subscribed
    /// symbols, `in_position_for(subscribed)` returns true IFF at least one
    /// subscribed symbol has qty > 0 in the tracker.
    #[test]
    fn prop_in_position_derivation(
        // Generate 1-5 symbols that will have open positions
        open_symbols in prop::collection::vec(arb_symbol(), 1..=5),
        // Quantities for each open symbol
        qtys in prop::collection::vec(arb_qty(), 1..=5),
        // Prices for each open signal
        prices in prop::collection::vec(arb_price(), 1..=5),
        // Generate 1-5 symbols for the strategy subscription (may or may not overlap)
        subscribed_symbols in prop::collection::vec(arb_symbol(), 1..=5),
    ) {
        let mut tracker = LivePositionTracker::new(1_000_000.0);

        // Open positions for a subset of the generated symbols
        let count = open_symbols.len().min(qtys.len()).min(prices.len());
        for i in 0..count {
            let signal = Signal::Open {
                symbol: open_symbols[i].clone(),
                qty: qtys[i],
            };
            tracker.process_signal(&signal, prices[i], i, "test_strategy");
        }

        // Now check the in_position_for derivation
        let result = tracker.in_position_for(&subscribed_symbols);

        // Manually compute expected value: true IFF any subscribed symbol
        // has qty > 0 in the unified tracker
        let expected = subscribed_symbols.iter().any(|sym| {
            tracker.inner.position(sym).map_or(false, |p| p.qty > 0.0)
        });

        prop_assert_eq!(
            result, expected,
            "in_position_for mismatch: open_symbols={:?}, subscribed={:?}, result={}, expected={}",
            open_symbols, subscribed_symbols, result, expected
        );
    }

    /// **Validates: Requirements 3.5**
    ///
    /// Property 5 (variant): After opening and then fully closing all positions,
    /// in_position_for must return false for any subscription.
    #[test]
    fn prop_in_position_false_after_close(
        symbols in prop::collection::vec(arb_symbol(), 1..=3),
        qtys in prop::collection::vec(arb_qty(), 1..=3),
        open_prices in prop::collection::vec(arb_price(), 1..=3),
        close_prices in prop::collection::vec(arb_price(), 1..=3),
        subscribed in prop::collection::vec(arb_symbol(), 1..=5),
    ) {
        let mut tracker = LivePositionTracker::new(1_000_000.0);
        let count = symbols.len().min(qtys.len()).min(open_prices.len()).min(close_prices.len());

        // Open then close each position
        for i in 0..count {
            let open_signal = Signal::Open {
                symbol: symbols[i].clone(),
                qty: qtys[i],
            };
            tracker.process_signal(&open_signal, open_prices[i], i * 2, "strat");

            let close_signal = Signal::Close {
                symbol: symbols[i].clone(),
            };
            tracker.process_signal(&close_signal, close_prices[i], i * 2 + 1, "strat");
        }

        // After all positions are closed, in_position should be false
        // (regardless of which symbols we subscribe to)
        let result = tracker.in_position_for(&subscribed);
        prop_assert!(!result, "Expected in_position_for to be false after closing all positions");
    }

    /// **Validates: Requirements 3.5**
    ///
    /// Property 5 (variant): in_position_for with empty subscription always returns false.
    #[test]
    fn prop_in_position_empty_subscription(
        open_symbols in prop::collection::vec(arb_symbol(), 1..=3),
        qtys in prop::collection::vec(arb_qty(), 1..=3),
        prices in prop::collection::vec(arb_price(), 1..=3),
    ) {
        let mut tracker = LivePositionTracker::new(1_000_000.0);
        let count = open_symbols.len().min(qtys.len()).min(prices.len());

        for i in 0..count {
            let signal = Signal::Open {
                symbol: open_symbols[i].clone(),
                qty: qtys[i],
            };
            tracker.process_signal(&signal, prices[i], i, "strat");
        }

        // Empty subscription should never report in_position
        let empty: Vec<String> = vec![];
        prop_assert!(!tracker.in_position_for(&empty));
    }
}

// ============================================================================
// Property 6: Fill attribution correctness
// ============================================================================

/// A signal paired with a strategy name for testing attribution.
#[derive(Debug, Clone)]
struct AttributedSignal {
    strategy_name: String,
    signal: Signal,
}

/// Generate a sequence of attributed signals that will produce fills.
/// We use only Open signals here since they always produce fills.
fn arb_open_attributed_signals() -> impl Strategy<Value = Vec<AttributedSignal>> {
    prop::collection::vec(
        (arb_strategy_name(), arb_symbol(), arb_qty()),
        1..=10,
    )
    .prop_map(|entries| {
        entries
            .into_iter()
            .map(|(name, symbol, qty)| AttributedSignal {
                strategy_name: name,
                signal: Signal::Open { symbol, qty },
            })
            .collect()
    })
}

/// Generate a mixed sequence of Open and Close signals for testing.
/// Opens always produce fills. Closes only produce fills if a position exists.
fn arb_mixed_attributed_signals() -> impl Strategy<Value = Vec<AttributedSignal>> {
    prop::collection::vec(
        (arb_strategy_name(), arb_symbol(), arb_qty(), proptest::bool::ANY),
        1..=10,
    )
    .prop_map(|entries| {
        entries
            .into_iter()
            .map(|(name, symbol, qty, is_open)| {
                let signal = if is_open {
                    Signal::Open { symbol, qty }
                } else {
                    Signal::Close { symbol }
                };
                AttributedSignal {
                    strategy_name: name,
                    signal,
                }
            })
            .collect()
    })
}

proptest! {
    /// **Validates: Requirements 3.7**
    ///
    /// Property 6: Fill attribution correctness
    ///
    /// For any sequence of (strategy_name, signal) pairs processed through
    /// the LivePositionTracker, the fill_attribution vector has exactly one
    /// entry per fill produced, and each attribution entry matches the
    /// strategy_name that produced that fill.
    #[test]
    fn prop_fill_attribution_length_matches_fills(
        signals in arb_open_attributed_signals(),
        prices in prop::collection::vec(arb_price(), 1..=10),
    ) {
        let mut tracker = LivePositionTracker::new(1_000_000.0);

        let count = signals.len().min(prices.len());
        let mut expected_fill_count = 0;

        for i in 0..count {
            let fill = tracker.process_signal(
                &signals[i].signal,
                prices[i],
                i,
                &signals[i].strategy_name,
            );
            if fill.is_some() {
                expected_fill_count += 1;
            }
        }

        // Attribution length must equal the number of fills produced
        prop_assert_eq!(
            tracker.fill_attribution.len(),
            expected_fill_count,
            "Attribution vector length {} != fill count {}",
            tracker.fill_attribution.len(),
            expected_fill_count,
        );

        // Also verify it matches the inner tracker's fill count
        prop_assert_eq!(
            tracker.fill_attribution.len(),
            tracker.inner.fills().len(),
            "Attribution length {} != inner fills length {}",
            tracker.fill_attribution.len(),
            tracker.inner.fills().len(),
        );
    }

    /// **Validates: Requirements 3.7**
    ///
    /// Property 6: Fill attribution correctness (strategy name matching)
    ///
    /// Each entry in fill_attribution corresponds to the strategy that
    /// produced that fill. We verify by tracking which strategy names should
    /// produce fills and checking the attribution matches.
    #[test]
    fn prop_fill_attribution_strategy_names_match(
        signals in arb_open_attributed_signals(),
        prices in prop::collection::vec(arb_price(), 1..=10),
    ) {
        let mut tracker = LivePositionTracker::new(1_000_000.0);
        let mut expected_attributions: Vec<String> = Vec::new();

        let count = signals.len().min(prices.len());
        for i in 0..count {
            let fill = tracker.process_signal(
                &signals[i].signal,
                prices[i],
                i,
                &signals[i].strategy_name,
            );
            if fill.is_some() {
                expected_attributions.push(signals[i].strategy_name.clone());
            }
        }

        // Each attribution entry must match the strategy that produced it
        prop_assert_eq!(
            &tracker.fill_attribution,
            &expected_attributions,
            "Attribution mismatch"
        );
    }

    /// **Validates: Requirements 3.7**
    ///
    /// Property 6 (variant): Mixed signal types — Close signals that find no
    /// position produce no fill and no attribution entry.
    #[test]
    fn prop_fill_attribution_mixed_signals(
        signals in arb_mixed_attributed_signals(),
        prices in prop::collection::vec(arb_price(), 1..=10),
    ) {
        let mut tracker = LivePositionTracker::new(1_000_000.0);
        let mut expected_attributions: Vec<String> = Vec::new();

        let count = signals.len().min(prices.len());
        for i in 0..count {
            let fill = tracker.process_signal(
                &signals[i].signal,
                prices[i],
                i,
                &signals[i].strategy_name,
            );
            if fill.is_some() {
                expected_attributions.push(signals[i].strategy_name.clone());
            }
        }

        // Attribution must match actual fills, even with mixed signal types
        prop_assert_eq!(
            tracker.fill_attribution.len(),
            tracker.inner.fills().len(),
            "Attribution length {} != fills length {}",
            tracker.fill_attribution.len(),
            tracker.inner.fills().len(),
        );
        prop_assert_eq!(
            &tracker.fill_attribution,
            &expected_attributions,
        );
    }
}
