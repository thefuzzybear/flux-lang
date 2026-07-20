//! Property-based tests for Product Registry & Risk Integration.
//!
//! Feature: product-registry-risk
//!
//! This file contains property tests validating:
//! - Property 1: ProductEntry to registry roundtrip
//! - Property 2: Unknown symbol rejection
//! - Property 3: Multiplier-aware notional enforcement
//! - Property 4: Multiplier in cost basis
//! - Property 5: Margin pre-check gate
//! - Property 6: Check ordering — margin before position limit
//!
//! **Validates: Requirements 1.1, 1.2, 1.3, 1.4, 2.1, 2.2, 2.3, 4.1, 4.2, 4.3, 5.1, 5.2, 5.3, 7.2, 7.3, 7.4, 7.5**

use std::collections::HashMap;

use chrono::TimeZone;
use proptest::prelude::*;

use flux_cli::live::account_config::ProductEntry;
use flux_cli::live::market_calendar::MarketCalendar;
use flux_cli::live::product_registry::ProductRegistry;
use flux_cli::live::risk_limits::{
    AlertEvent, PortfolioState, RejectionReason, RiskDecision, RiskLimits, RiskLimitsConfig,
};
use flux_runtime::Signal;

// =============================================================================
// Helpers
// =============================================================================

/// A valid RiskLimitsConfig for property tests with generous limits.
fn prop_config() -> RiskLimitsConfig {
    RiskLimitsConfig {
        max_daily_loss: -100_000.0,
        max_weekly_loss: -200_000.0,
        max_position_per_product: 100,
        max_total_notional: 10_000_000.0,
        max_drawdown_pct: 0.5,
        correlation_warning_threshold: 10,
        initial_equity: 1_000_000.0,
    }
}

/// A fixed Eastern-time timestamp for property tests (Tuesday July 15, 2025 at 10:00 AM ET).
fn prop_timestamp() -> chrono::DateTime<chrono_tz::Tz> {
    chrono_tz::US::Eastern
        .with_ymd_and_hms(2025, 7, 15, 10, 0, 0)
        .unwrap()
}

/// Build an empty portfolio state with large available margin.
fn generous_portfolio_state() -> PortfolioState {
    PortfolioState {
        positions: HashMap::new(),
        prices: HashMap::new(),
        timestamp: prop_timestamp(),
        available_margin: f64::MAX,
    }
}

/// A minimal valid MarketCalendar for property tests.
fn default_calendar() -> MarketCalendar {
    let toml_str = r#"
[[session]]
exchange = "CME"
open = "09:30"
close = "16:00"
timezone = "US/Eastern"
"#;
    MarketCalendar::from_toml(toml_str).unwrap()
}

// =============================================================================
// Strategies
// =============================================================================

/// Strategy for generating a small set of distinct symbol names.
fn arb_symbol_name() -> impl Strategy<Value = String> {
    prop::sample::select(vec![
        "AAPL".to_string(),
        "MSFT".to_string(),
        "GOOG".to_string(),
        "AMZN".to_string(),
        "TSLA".to_string(),
        "ES".to_string(),
        "NQ".to_string(),
        "CL".to_string(),
        "GC".to_string(),
        "ZB".to_string(),
    ])
}

/// Strategy for generating a valid ProductEntry.
fn arb_product_entry() -> impl Strategy<Value = ProductEntry> {
    (
        arb_symbol_name(),
        1.0..=1000.0_f64,   // multiplier
        0.001..=1.0_f64,    // tick_size
        100.0..=100_000.0_f64, // margin
    )
        .prop_map(|(name, multiplier, tick_size, margin)| ProductEntry {
            name,
            multiplier,
            tick_size,
            margin,
        })
}

/// Strategy for generating a Vec<ProductEntry> with distinct names.
fn arb_product_entries() -> impl Strategy<Value = Vec<ProductEntry>> {
    proptest::collection::vec(arb_product_entry(), 1..=8).prop_map(|entries| {
        // Deduplicate by name, keeping the first occurrence
        let mut seen = std::collections::HashSet::new();
        entries
            .into_iter()
            .filter(|e| seen.insert(e.name.clone()))
            .collect()
    })
}

// =============================================================================
// Property 1: ProductEntry to registry roundtrip
// **Validates: Requirements 1.1, 1.2, 1.3, 1.4, 2.1, 2.2, 2.3**
// =============================================================================

proptest! {
    #[test]
    fn property_1_registry_roundtrip(entries in arb_product_entries()) {
        let registry = ProductRegistry::from_entries(&entries);

        for entry in &entries {
            let spec = registry.get(&entry.name);
            prop_assert!(spec.is_some(), "Symbol '{}' not found in registry", entry.name);

            let spec = spec.unwrap();
            prop_assert_eq!(spec.multiplier, entry.multiplier,
                "Multiplier mismatch for '{}'", entry.name);
            prop_assert_eq!(spec.tick_size, entry.tick_size,
                "Tick size mismatch for '{}'", entry.name);
            prop_assert_eq!(spec.margin_initial, entry.margin,
                "Margin initial mismatch for '{}'", entry.name);
            prop_assert_eq!(spec.margin_maintenance, entry.margin,
                "Margin maintenance mismatch for '{}'", entry.name);
        }
    }
}

// =============================================================================
// Property 2: Unknown symbol rejection
// **Validates: Requirements 4.1, 4.2, 4.3**
// =============================================================================

proptest! {
    #[test]
    fn property_2_unknown_symbol_rejection(
        entries in arb_product_entries(),
        qty in 1.0..100.0_f64,
        use_short in proptest::bool::ANY,
    ) {
        let registry = ProductRegistry::from_entries(&entries);
        let known_names: std::collections::HashSet<String> =
            entries.iter().map(|e| e.name.clone()).collect();

        // Pick a symbol NOT in the registry
        let unknown_symbol = "UNKNOWN_XYZ".to_string();
        prop_assert!(!known_names.contains(&unknown_symbol));

        let config = prop_config();
        let mut rl = RiskLimits::new(config, registry, default_calendar()).unwrap();

        let signal = if use_short {
            Signal::short(unknown_symbol.clone(), qty)
        } else {
            Signal::open(unknown_symbol.clone(), qty)
        };

        let state = generous_portfolio_state();
        let (decision, alerts) = rl.check_signal(&signal, &state);

        // Must reject with UnknownSymbol
        prop_assert_eq!(
            decision,
            RiskDecision::Reject {
                reason: RejectionReason::UnknownSymbol {
                    symbol: unknown_symbol.clone(),
                },
            }
        );

        // Must emit UnknownSymbolRejected alert
        let has_alert = alerts.iter().any(|a| matches!(a, AlertEvent::UnknownSymbolRejected { symbol } if symbol == &unknown_symbol));
        prop_assert!(has_alert, "Expected UnknownSymbolRejected alert");
    }
}

// =============================================================================
// Property 3: Multiplier-aware notional enforcement
// **Validates: Requirements 5.1, 5.2**
// =============================================================================

proptest! {
    #[test]
    fn property_3_notional_exceeds_limit_rejected(
        multiplier in 1.0..100.0_f64,
        qty in 1.0..50.0_f64,
        price in 100.0..10_000.0_f64,
    ) {
        // Set up a config where the notional limit will be exceeded
        let additional_notional = qty * price * multiplier;

        // Set max_total_notional so that the signal exceeds the limit
        // current_notional = 0, additional > limit
        let max_notional = additional_notional * 0.5; // limit is half of what we'd add
        prop_assume!(max_notional > 0.0);

        let config = RiskLimitsConfig {
            max_daily_loss: -1_000_000.0,
            max_weekly_loss: -2_000_000.0,
            max_position_per_product: 1000,
            max_total_notional: max_notional,
            max_drawdown_pct: 0.99,
            correlation_warning_threshold: 100,
            initial_equity: 10_000_000.0,
        };

        let entry = ProductEntry {
            name: "TEST".to_string(),
            multiplier,
            tick_size: 0.01,
            margin: 100.0, // small margin so margin check passes
        };
        let registry = ProductRegistry::from_entries(&[entry]);
        let mut rl = RiskLimits::new(config, registry, default_calendar()).unwrap();

        let signal = Signal::open("TEST".to_string(), qty);
        let mut state = generous_portfolio_state();
        state.prices.insert("TEST".to_string(), price);

        let (decision, _alerts) = rl.check_signal(&signal, &state);

        // Must reject with NotionalLimitExceeded
        match decision {
            RiskDecision::Reject { reason: RejectionReason::NotionalLimitExceeded { .. } } => {}
            other => prop_assert!(false, "Expected NotionalLimitExceeded, got {:?}", other),
        }
    }

    #[test]
    fn property_3_notional_within_limit_allowed(
        multiplier in 1.0..100.0_f64,
        qty in 1.0..10.0_f64,
        price in 100.0..1_000.0_f64,
    ) {
        // Set up a config where the notional limit will NOT be exceeded
        let additional_notional = qty * price * multiplier;
        let max_notional = additional_notional * 10.0; // limit is 10x what we'd add

        let config = RiskLimitsConfig {
            max_daily_loss: -1_000_000.0,
            max_weekly_loss: -2_000_000.0,
            max_position_per_product: 1000,
            max_total_notional: max_notional,
            max_drawdown_pct: 0.99,
            correlation_warning_threshold: 100,
            initial_equity: 10_000_000.0,
        };

        let entry = ProductEntry {
            name: "TEST".to_string(),
            multiplier,
            tick_size: 0.01,
            margin: 100.0, // small margin so margin check passes
        };
        let registry = ProductRegistry::from_entries(&[entry]);
        let mut rl = RiskLimits::new(config, registry, default_calendar()).unwrap();

        let signal = Signal::open("TEST".to_string(), qty);
        let mut state = generous_portfolio_state();
        state.prices.insert("TEST".to_string(), price);

        let (decision, _alerts) = rl.check_signal(&signal, &state);

        prop_assert_eq!(decision, RiskDecision::Allow);
    }
}

// =============================================================================
// Property 4: Multiplier in cost basis
// **Validates: Requirements 5.3**
// =============================================================================

proptest! {
    #[test]
    fn property_4_multiplier_in_cost_basis(
        multiplier in 1.0..100.0_f64,
        qty in 1.0..50.0_f64,
        fill_price in 100.0..10_000.0_f64,
    ) {
        // After an opening fill, total_notional should increase by qty × fill_price × multiplier.
        // We verify this indirectly: after the fill, sending a signal that would just barely
        // exceed the limit proves the cost basis was calculated correctly.

        let expected_notional = qty * fill_price * multiplier;
        // Set the limit to exactly expected_notional + a tiny epsilon above a second signal's cost
        // Strategy: record a fill, then try to add exactly 1 more unit at price 1.0
        // The limit is expected_notional + (1.0 * 1.0 * multiplier) - 0.01 (just under)
        let second_signal_notional = 1.0 * 1.0 * multiplier;
        let max_notional = expected_notional + second_signal_notional - 0.01;
        prop_assume!(max_notional > 0.0);
        prop_assume!(expected_notional > 0.0);

        let config = RiskLimitsConfig {
            max_daily_loss: -1_000_000.0,
            max_weekly_loss: -2_000_000.0,
            max_position_per_product: 1000,
            max_total_notional: max_notional,
            max_drawdown_pct: 0.99,
            correlation_warning_threshold: 100,
            initial_equity: 10_000_000.0,
        };

        let entry = ProductEntry {
            name: "TEST".to_string(),
            multiplier,
            tick_size: 0.01,
            margin: 1.0, // tiny margin so margin check never blocks
        };
        let registry = ProductRegistry::from_entries(&[entry]);
        let mut rl = RiskLimits::new(config, registry, default_calendar()).unwrap();

        // Record an opening fill
        let open_signal = Signal::open("TEST".to_string(), qty);
        rl.record_fill(&open_signal, fill_price, qty, 0.0);

        // Now try another signal: 1 unit at price 1.0
        // additional_notional = 1.0 * 1.0 * multiplier = multiplier
        // total after = expected_notional + multiplier > max_notional (since max = expected + multiplier - 0.01)
        let probe_signal = Signal::open("TEST".to_string(), 1.0);
        let mut state = generous_portfolio_state();
        state.prices.insert("TEST".to_string(), 1.0);

        let (decision, _alerts) = rl.check_signal(&probe_signal, &state);

        // Should be rejected because cost basis was tracked with multiplier
        match decision {
            RiskDecision::Reject { reason: RejectionReason::NotionalLimitExceeded { current_notional, .. } } => {
                // Verify the stored notional matches expected
                let diff = (current_notional - expected_notional).abs();
                prop_assert!(diff < 0.0001,
                    "Expected current_notional ~{}, got {}", expected_notional, current_notional);
            }
            other => prop_assert!(false,
                "Expected NotionalLimitExceeded rejection, got {:?}. \
                 expected_notional={}, max_notional={}, second_signal_notional={}",
                other, expected_notional, max_notional, second_signal_notional),
        }
    }
}

// =============================================================================
// Property 5: Margin pre-check gate
// **Validates: Requirements 7.2, 7.3, 7.4**
// =============================================================================

proptest! {
    #[test]
    fn property_5_margin_exceeded_rejection(
        margin_initial in 1000.0..50_000.0_f64,
        qty in 1.0..20.0_f64,
        use_short in proptest::bool::ANY,
    ) {
        let required_margin = qty * margin_initial;
        // Set available_margin to less than required
        let available_margin = required_margin * 0.5;
        prop_assume!(available_margin > 0.0);

        let entry = ProductEntry {
            name: "TEST".to_string(),
            multiplier: 1.0,
            tick_size: 0.01,
            margin: margin_initial,
        };
        let registry = ProductRegistry::from_entries(&[entry]);
        let mut rl = RiskLimits::new(prop_config(), registry, default_calendar()).unwrap();

        let signal = if use_short {
            Signal::short("TEST".to_string(), qty)
        } else {
            Signal::open("TEST".to_string(), qty)
        };

        let mut state = generous_portfolio_state();
        state.available_margin = available_margin;

        let (decision, alerts) = rl.check_signal(&signal, &state);

        // Must reject with MarginExceeded
        match &decision {
            RiskDecision::Reject { reason: RejectionReason::MarginExceeded { symbol, required, available } } => {
                prop_assert_eq!(symbol, "TEST");
                let expected_required = qty * margin_initial;
                let diff = (required - expected_required).abs();
                prop_assert!(diff < 0.0001, "Required margin mismatch");
                prop_assert_eq!(*available, available_margin);
            }
            other => prop_assert!(false, "Expected MarginExceeded, got {:?}", other),
        }

        // Must emit MarginExceededRejected alert
        let has_alert = alerts.iter().any(|a| matches!(a,
            AlertEvent::MarginExceededRejected { symbol, .. } if symbol == "TEST"
        ));
        prop_assert!(has_alert, "Expected MarginExceededRejected alert");
    }
}

// =============================================================================
// Property 6: Check ordering — margin before position limit
// **Validates: Requirements 7.5**
// =============================================================================

proptest! {
    #[test]
    fn property_6_margin_checked_before_position_limit(
        margin_initial in 1000.0..50_000.0_f64,
        qty in 1.0..20.0_f64,
    ) {
        // Set up a scenario where BOTH margin and position limit would fail.
        // The signal should be rejected with MarginExceeded (margin is checked first).

        let required_margin = qty * margin_initial;
        let available_margin = required_margin * 0.1; // Way too low for margin
        prop_assume!(available_margin > 0.0);

        // Position limit: set max_position_per_product to 1, and the signal has qty > 1
        // This ensures position limit would also fail.
        let config = RiskLimitsConfig {
            max_daily_loss: -1_000_000.0,
            max_weekly_loss: -2_000_000.0,
            max_position_per_product: 1, // Only 1 allowed
            max_total_notional: 100_000_000.0,
            max_drawdown_pct: 0.99,
            correlation_warning_threshold: 100,
            initial_equity: 10_000_000.0,
        };

        let entry = ProductEntry {
            name: "TEST".to_string(),
            multiplier: 1.0,
            tick_size: 0.01,
            margin: margin_initial,
        };
        let registry = ProductRegistry::from_entries(&[entry]);
        let mut rl = RiskLimits::new(config, registry, default_calendar()).unwrap();

        // Use qty >= 2.0 so position limit (max=1) would also fail
        let test_qty = qty.max(2.0);
        let signal = Signal::open("TEST".to_string(), test_qty);

        let mut state = generous_portfolio_state();
        state.available_margin = available_margin;

        let (decision, _alerts) = rl.check_signal(&signal, &state);

        // Should be MarginExceeded, NOT PositionLimitExceeded
        match decision {
            RiskDecision::Reject { reason: RejectionReason::MarginExceeded { .. } } => {
                // Correct: margin check fired before position limit
            }
            RiskDecision::Reject { reason: RejectionReason::PositionLimitExceeded { .. } } => {
                prop_assert!(false, "Got PositionLimitExceeded — margin should be checked first!");
            }
            other => prop_assert!(false, "Expected MarginExceeded, got {:?}", other),
        }
    }
}
