//! Property test for signal aggregator constraint enforcement.
//!
//! Feature: flux-live-harness, Property 4: Signal aggregator constraint enforcement
//!
//! **Validates: Requirements 4.2, 4.3, 4.4, 4.7, 4.8**
//!
//! Generates random signals, portfolio states, and constraint configs to verify:
//! - CLOSE always passes regardless of constraint configuration
//! - OPEN is rejected iff a constraint would be violated
//! - Processing is deterministic (same inputs → same outputs)
//! - Position size enforcement works correctly
//! - Position count enforcement works correctly

use proptest::prelude::*;

use flux_cli::live::aggregator::{RiskConstraints, SignalAggregator};
use flux_runtime::{PositionTracker, Signal};

// =============================================================================
// Strategies (generators) for random test data
// =============================================================================

/// Generate a valid ticker symbol (1–5 uppercase letters).
fn arb_symbol() -> impl Strategy<Value = String> {
    "[A-Z]{1,5}"
}

/// Generate a positive quantity for signals.
fn arb_qty() -> impl Strategy<Value = f64> {
    1.0f64..500.0
}

/// Generate a reasonable price for fills.
fn arb_price() -> impl Strategy<Value = f64> {
    10.0f64..1000.0
}

/// Generate random risk constraints.
fn arb_constraints() -> impl Strategy<Value = RiskConstraints> {
    (
        proptest::option::of(1.0f64..1000.0),   // max_position_size
        proptest::option::of(100.0f64..100000.0), // max_exposure
        proptest::option::of(1usize..20),        // max_positions
    )
        .prop_map(|(max_position_size, max_exposure, max_positions)| RiskConstraints {
            max_position_size,
            max_exposure,
            max_positions,
        })
}

// =============================================================================
// Property 4: Signal aggregator constraint enforcement
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    // -------------------------------------------------------------------------
    // Sub-property 4a: CLOSE always passes
    // -------------------------------------------------------------------------

    /// **Validates: Requirements 4.7**
    ///
    /// Any Close or CloseQty signal passes through the aggregator regardless
    /// of constraint configuration. Even the tightest constraints never block
    /// closing or reducing a position.
    #[test]
    fn prop_close_always_passes(
        constraints in arb_constraints(),
        symbol in arb_symbol(),
        qty in arb_qty(),
        close_variant in 0..2u8,
        strat_name in "[a-z_]{3,10}",
    ) {
        let aggregator = SignalAggregator::new(constraints);

        // Set up a tracker with an existing position so there's something to close
        let mut tracker = PositionTracker::new(100000.0);
        tracker.process_signal(&Signal::Open { symbol: symbol.clone(), qty: 1000.0 }, 100.0, 0);

        let signal = match close_variant {
            0 => Signal::Close { symbol: symbol.clone() },
            _ => Signal::CloseQty { symbol: symbol.clone(), qty },
        };

        let signals = vec![(strat_name.clone(), signal.clone())];
        let approved = aggregator.process(&signals, &tracker);

        prop_assert_eq!(
            approved.len(), 1,
            "CLOSE signal should always pass, but was rejected. \
             Constraints: {:?}, signal: {:?}",
            aggregator.constraints(), signal
        );
        prop_assert_eq!(&approved[0].0, &strat_name);
    }

    // -------------------------------------------------------------------------
    // Sub-property 4b: OPEN rejected iff position size constraint violated
    // -------------------------------------------------------------------------

    /// **Validates: Requirements 4.2**
    ///
    /// For a single OPEN signal in isolation, it is rejected by position size
    /// constraint iff existing_qty + requested_qty > max_position_size.
    #[test]
    fn prop_position_size_enforcement(
        max_size in 1.0f64..1000.0,
        existing_qty in 0.0f64..500.0,
        requested_qty in 1.0f64..500.0,
        symbol in arb_symbol(),
        price in arb_price(),
    ) {
        let constraints = RiskConstraints {
            max_position_size: Some(max_size),
            max_exposure: None,  // disable other constraints
            max_positions: None,
        };
        let aggregator = SignalAggregator::new(constraints);

        let mut tracker = PositionTracker::new(100000.0);
        if existing_qty > 0.0 {
            tracker.process_signal(
                &Signal::Open { symbol: symbol.clone(), qty: existing_qty },
                price,
                0,
            );
        }

        let signals = vec![(
            "test_strat".to_string(),
            Signal::Open { symbol: symbol.clone(), qty: requested_qty },
        )];

        let approved = aggregator.process(&signals, &tracker);
        let would_exceed = existing_qty + requested_qty > max_size;

        if would_exceed {
            prop_assert!(
                approved.is_empty(),
                "OPEN should be rejected: {:.4} + {:.4} = {:.4} > limit {:.4}",
                existing_qty, requested_qty, existing_qty + requested_qty, max_size
            );
        } else {
            prop_assert_eq!(
                approved.len(), 1,
                "OPEN should pass: {:.4} + {:.4} = {:.4} <= limit {:.4}",
                existing_qty, requested_qty, existing_qty + requested_qty, max_size
            );
        }
    }

    // -------------------------------------------------------------------------
    // Sub-property 4c: OPEN rejected iff position count constraint violated
    // -------------------------------------------------------------------------

    /// **Validates: Requirements 4.4**
    ///
    /// OPEN on a new symbol is rejected when open_position_count >= max_positions.
    /// OPEN on an existing symbol is always allowed by position count constraint.
    #[test]
    fn prop_position_count_enforcement(
        max_positions in 1usize..10,
        num_existing in 0usize..10,
        open_on_new_symbol in proptest::bool::ANY,
    ) {
        let constraints = RiskConstraints {
            max_position_size: None,  // disable size constraint
            max_exposure: None,       // disable exposure constraint
            max_positions: Some(max_positions),
        };
        let aggregator = SignalAggregator::new(constraints);

        let mut tracker = PositionTracker::new(100000.0);

        // Open `num_existing` positions on distinct symbols
        let existing_symbols: Vec<String> = (0..num_existing)
            .map(|i| format!("SYM{}", i))
            .collect();
        for (i, sym) in existing_symbols.iter().enumerate() {
            tracker.process_signal(
                &Signal::Open { symbol: sym.clone(), qty: 10.0 },
                100.0,
                i,
            );
        }

        // Try to open on either a new symbol or an existing one
        let target_symbol = if open_on_new_symbol || existing_symbols.is_empty() {
            "NEWSYM".to_string()
        } else {
            existing_symbols[0].clone()
        };

        let is_new = tracker.position(&target_symbol).is_none();

        let signals = vec![(
            "test_strat".to_string(),
            Signal::Open { symbol: target_symbol.clone(), qty: 5.0 },
        )];

        let approved = aggregator.process(&signals, &tracker);

        if is_new && num_existing >= max_positions {
            prop_assert!(
                approved.is_empty(),
                "New position should be rejected: count {} >= limit {}",
                num_existing, max_positions
            );
        } else {
            prop_assert_eq!(
                approved.len(), 1,
                "Signal should pass: is_new={}, count={}, limit={}",
                is_new, num_existing, max_positions
            );
        }
    }

    // -------------------------------------------------------------------------
    // Sub-property 4d: Deterministic processing order
    // -------------------------------------------------------------------------

    /// **Validates: Requirements 4.8**
    ///
    /// Running the same signals + constraints + tracker state twice produces
    /// exactly the same approved signal list. The aggregator is deterministic.
    #[test]
    fn prop_deterministic_processing(
        constraints in arb_constraints(),
        // Generate pre-existing positions as (symbol_idx, qty, price)
        pre_positions in proptest::collection::vec(
            (0..5usize, 1.0f64..200.0, 10.0f64..500.0), 0..5
        ),
        // Generate signals as (strat_name, symbol_idx, qty, variant)
        raw_signals in proptest::collection::vec(
            ("[a-z_]{3,8}", 0..5usize, 1.0f64..200.0, 0..3u8), 0..10
        ),
    ) {
        let symbols: Vec<String> = (0..5).map(|i| format!("SYM{}", i)).collect();

        // Build the tracker with pre-existing positions (deduplicated by symbol)
        let mut tracker = PositionTracker::new(100000.0);
        let mut seen = std::collections::HashSet::new();
        for (sym_idx, qty, price) in &pre_positions {
            let sym = &symbols[*sym_idx];
            if seen.insert(sym.clone()) {
                tracker.process_signal(
                    &Signal::Open { symbol: sym.clone(), qty: *qty },
                    *price,
                    0,
                );
            }
        }

        // Build signal list
        let signals: Vec<(String, Signal)> = raw_signals.iter().map(|(name, sym_idx, qty, variant)| {
            let symbol = symbols[*sym_idx].clone();
            let signal = match variant {
                0 => Signal::Open { symbol, qty: *qty },
                1 => Signal::Close { symbol },
                _ => Signal::CloseQty { symbol, qty: *qty },
            };
            (name.clone(), signal)
        }).collect();

        let aggregator = SignalAggregator::new(constraints);

        // Run twice with identical inputs
        let result1 = aggregator.process(&signals, &tracker);
        let result2 = aggregator.process(&signals, &tracker);

        // Results must be identical
        prop_assert_eq!(
            result1.len(), result2.len(),
            "Determinism violated: different number of approved signals"
        );

        for (i, ((name1, sig1), (name2, sig2))) in
            result1.iter().zip(result2.iter()).enumerate()
        {
            prop_assert_eq!(
                name1, name2,
                "Determinism violated at index {}: different strategy names", i
            );
            prop_assert_eq!(
                sig1.symbol(), sig2.symbol(),
                "Determinism violated at index {}: different symbols", i
            );
            prop_assert_eq!(
                sig1.qty(), sig2.qty(),
                "Determinism violated at index {}: different quantities", i
            );
        }
    }

    // -------------------------------------------------------------------------
    // Sub-property 4e: Mixed CLOSE and OPEN processing
    // -------------------------------------------------------------------------

    /// **Validates: Requirements 4.3, 4.7**
    ///
    /// In a batch of mixed signals, every Close/CloseQty signal appears in the
    /// approved list regardless of constraints, and the order of close signals
    /// is preserved.
    #[test]
    fn prop_close_signals_preserved_in_mixed_batch(
        constraints in arb_constraints(),
        // Generate pre-existing positions as (symbol_idx, qty, price)
        pre_positions in proptest::collection::vec(
            (0..5usize, 1.0f64..200.0, 10.0f64..500.0), 0..5
        ),
        // Generate signals as (strat_name, symbol_idx, qty, variant)
        raw_signals in proptest::collection::vec(
            ("[a-z_]{3,8}", 0..5usize, 1.0f64..200.0, 0..3u8), 1..10
        ),
    ) {
        let symbols: Vec<String> = (0..5).map(|i| format!("SYM{}", i)).collect();

        let mut tracker = PositionTracker::new(100000.0);
        let mut seen = std::collections::HashSet::new();
        for (sym_idx, qty, price) in &pre_positions {
            let sym = &symbols[*sym_idx];
            if seen.insert(sym.clone()) {
                tracker.process_signal(
                    &Signal::Open { symbol: sym.clone(), qty: *qty },
                    *price,
                    0,
                );
            }
        }

        // Build signal list
        let signals: Vec<(String, Signal)> = raw_signals.iter().map(|(name, sym_idx, qty, variant)| {
            let symbol = symbols[*sym_idx].clone();
            let signal = match variant {
                0 => Signal::Open { symbol, qty: *qty },
                1 => Signal::Close { symbol },
                _ => Signal::CloseQty { symbol, qty: *qty },
            };
            (name.clone(), signal)
        }).collect();

        let aggregator = SignalAggregator::new(constraints);
        let approved = aggregator.process(&signals, &tracker);

        // Count close signals in input
        let input_close_count = signals.iter().filter(|(_, sig)| {
            matches!(sig, Signal::Close { .. } | Signal::CloseQty { .. })
        }).count();

        // Count close signals in output
        let output_close_count = approved.iter().filter(|(_, sig)| {
            matches!(sig, Signal::Close { .. } | Signal::CloseQty { .. })
        }).count();

        prop_assert_eq!(
            input_close_count, output_close_count,
            "All CLOSE signals must pass through: input had {}, output had {}",
            input_close_count, output_close_count
        );

        // Verify order preservation of close signals
        let input_closes: Vec<_> = signals.iter()
            .filter(|(_, sig)| matches!(sig, Signal::Close { .. } | Signal::CloseQty { .. }))
            .collect();
        let output_closes: Vec<_> = approved.iter()
            .filter(|(_, sig)| matches!(sig, Signal::Close { .. } | Signal::CloseQty { .. }))
            .collect();

        for (i, ((in_name, in_sig), (out_name, out_sig))) in
            input_closes.iter().zip(output_closes.iter()).enumerate()
        {
            prop_assert_eq!(
                in_name, out_name,
                "Close signal order violated at index {}", i
            );
            prop_assert_eq!(
                in_sig.symbol(), out_sig.symbol(),
                "Close signal symbol mismatch at index {}", i
            );
        }
    }
}
