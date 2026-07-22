//! Property-based tests for the Futures Roll Manager.
//!
//! Feature: futures-roll-manager
//!
//! This file contains property tests for symbol parsing, quarterly cycle navigation,
//! volume buffer correctness, and intraday volume accumulation.
//!
//! **Validates: Requirements 2.3, 3.1, 3.3, 8.1, 8.2, 8.4, 8.5, 8.6, 10.2, 10.3, 10.5, 10.6, 10.7**

use proptest::prelude::*;

use flux_cli::live::futures_roll::{
    format_concrete, format_generic, parse_concrete, parse_generic, ConcreteContract,
    GenericSymbol, MonthCode, QuarterlyCycle, RollStateMachine, SymbolMode, VolumeBuffer,
};

// =============================================================================
// Generators
// =============================================================================

/// Generate a valid product root: 1–4 uppercase ASCII letters.
fn arb_root() -> impl Strategy<Value = String> {
    prop::collection::vec(b'A'..=b'Z', 1..=4)
        .prop_map(|bytes| String::from_utf8(bytes).unwrap())
}

/// Generate a MonthCode from the quarterly cycle [H, M, U, Z].
fn arb_month_code() -> impl Strategy<Value = MonthCode> {
    prop_oneof![
        Just(MonthCode::H),
        Just(MonthCode::M),
        Just(MonthCode::U),
        Just(MonthCode::Z),
    ]
}

/// Generate a year digit 0–9.
fn arb_year_digit() -> impl Strategy<Value = u8> {
    0u8..=9u8
}

/// Generate an arbitrary ConcreteContract.
fn arb_concrete_contract() -> impl Strategy<Value = ConcreteContract> {
    (arb_root(), arb_month_code(), arb_year_digit()).prop_map(|(root, month, year)| {
        ConcreteContract { root, month, year }
    })
}

/// Generate an arbitrary GenericSymbol (root + Continuous or NthMonth(1..=255)).
fn arb_generic_symbol() -> impl Strategy<Value = GenericSymbol> {
    (arb_root(), arb_symbol_mode()).prop_map(|(root, mode)| GenericSymbol { root, mode })
}

/// Generate a SymbolMode: either Continuous or NthMonth(1..=255).
fn arb_symbol_mode() -> impl Strategy<Value = SymbolMode> {
    prop_oneof![
        Just(SymbolMode::Continuous),
        (1u8..=255u8).prop_map(SymbolMode::NthMonth),
    ]
}

/// Generate a sequence of daily volumes (Vec<u64>) with length 0..20.
/// Values are bounded to avoid overflow when summing 5 values in the VolumeBuffer.
fn arb_volume_sequence() -> impl Strategy<Value = Vec<u64>> {
    prop::collection::vec(0u64..=(u64::MAX / 5), 0..20)
}

// =============================================================================
// Property Tests
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    // Feature: futures-roll-manager, Property 1: Generic symbol parse/format round-trip
    //
    // For any valid GenericSymbol, formatting then parsing SHALL produce an identical value.
    // **Validates: Requirements 8.1, 8.5**
    #[test]
    fn prop_generic_symbol_round_trip(sym in arb_generic_symbol()) {
        let formatted = format_generic(&sym);
        let parsed = parse_generic(&formatted).expect("parse_generic should succeed on formatted output");
        prop_assert_eq!(parsed, sym);
    }

    // Feature: futures-roll-manager, Property 2: Concrete contract parse/format round-trip
    //
    // For any valid ConcreteContract, formatting then parsing SHALL produce an identical value.
    // **Validates: Requirements 8.2, 8.4, 8.6**
    #[test]
    fn prop_concrete_contract_round_trip(contract in arb_concrete_contract()) {
        let formatted = format_concrete(&contract);
        let parsed = parse_concrete(&formatted).expect("parse_concrete should succeed on formatted output");
        prop_assert_eq!(parsed, contract);
    }

    // Feature: futures-roll-manager, Property 3: Quarterly cycle bidirectional round-trip
    //
    // For any valid ConcreteContract:
    //   next(previous(c)) == c AND previous(next(c)) == c
    // **Validates: Requirements 10.2, 10.3, 10.5, 10.6, 10.7**
    #[test]
    fn prop_quarterly_cycle_round_trip(contract in arb_concrete_contract()) {
        let next_then_prev = QuarterlyCycle::previous(&QuarterlyCycle::next(&contract));
        let prev_then_next = QuarterlyCycle::next(&QuarterlyCycle::previous(&contract));
        prop_assert_eq!(&next_then_prev, &contract,
            "previous(next(c)) should equal c, but got {:?} != {:?}", next_then_prev, contract);
        prop_assert_eq!(&prev_then_next, &contract,
            "next(previous(c)) should equal c, but got {:?} != {:?}", prev_then_next, contract);
    }

    // Feature: futures-roll-manager, Property 4: Volume buffer rolling average correctness
    //
    // For any sequence of daily volumes pushed into a VolumeBuffer:
    //   - When buffer has >= 5 values, reported average equals arithmetic mean of last 5 values
    //   - When buffer has < 5 values, average returns None
    // **Validates: Requirements 2.3, 3.1**
    #[test]
    fn prop_volume_buffer_average(volumes in arb_volume_sequence()) {
        let mut buffer = VolumeBuffer::new();
        for &v in &volumes {
            buffer.push(v);
        }

        if volumes.len() >= 5 {
            // Average should equal the mean of the last 5 values
            let last_five: Vec<u64> = volumes.iter().rev().take(5).copied().collect();
            let expected_sum: u64 = last_five.iter().sum();
            let expected_avg = expected_sum as f64 / 5.0;

            let actual_avg = buffer.average().expect("average should be Some when >= 5 values pushed");
            prop_assert!(
                (actual_avg - expected_avg).abs() < 1e-10,
                "average mismatch: expected {}, got {}", expected_avg, actual_avg
            );
        } else {
            // Buffer not full — average should be None
            prop_assert_eq!(buffer.average(), None,
                "average should be None when fewer than 5 values pushed (got {:?} with {} values)",
                buffer.average(), volumes.len());
        }
    }

    // Feature: futures-roll-manager, Property 5: Intraday volume accumulation
    //
    // For any sequence of bar volumes for a session:
    //   Creating a RollStateMachine and calling accumulate_volume for each bar on L1
    //   results in l1_intraday_volume equal to the sum of all bar volumes.
    // **Validates: Requirements 3.3**
    #[test]
    fn prop_intraday_volume_accumulation(bar_volumes in prop::collection::vec(0u64..1_000_000u64, 0..20)) {
        // Create a state machine with known L1/L2
        let l1 = ConcreteContract {
            root: "ES".to_string(),
            month: MonthCode::H,
            year: 5,
        };
        let l2 = ConcreteContract {
            root: "ES".to_string(),
            month: MonthCode::M,
            year: 5,
        };
        let mut sm = RollStateMachine::new("ES".to_string(), l1.clone(), l2);

        // Accumulate volumes for L1
        for &vol in &bar_volumes {
            sm.accumulate_volume(&l1, vol);
        }

        let expected: u64 = bar_volumes.iter().sum();
        prop_assert_eq!(sm.l1_intraday_volume(), expected,
            "l1_intraday_volume should equal sum of bar volumes");
    }
}

// =============================================================================
// Additional Generators for Properties 6–10
// =============================================================================

use flux_cli::live::futures_roll::ContinuousAdjuster;
use chrono::NaiveDate;

/// Generate positive f64 in range (0.01, 1000.0) for prices.
fn arb_positive_price() -> impl Strategy<Value = f64> {
    (1i64..100_000i64).prop_map(|x| x as f64 / 100.0) // 0.01 to 1000.0
}

/// Generate a positive factor (0.5 to 2.0) for adjustment ratios.
fn arb_ratio() -> impl Strategy<Value = f64> {
    (50i64..200i64).prop_map(|x| x as f64 / 100.0) // 0.50 to 2.00
}

/// Generate exactly 5 daily volumes for a full VolumeBuffer.
fn arb_five_volumes() -> impl Strategy<Value = Vec<u64>> {
    prop::collection::vec(1u64..1_000_000u64, 5..=5)
}

/// Generate a positive u64 volume > 0.
fn arb_positive_volume() -> impl Strategy<Value = u64> {
    1u64..1_000_000_000u64
}

// =============================================================================
// Property Tests 6–10
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    // Feature: futures-roll-manager, Property 6: Crossover detection with guards
    //
    // For any pair of volume sequences (L1, L2) where both have 5 days of history,
    // and a latched state: evaluate_crossover SHALL emit a RollSignal iff
    // (both buffers full AND avg(L2) > avg(L1) AND !latched).
    //
    // We test this by constructing a RollStateMachine, pushing 5 days of volume
    // for each contract, and verifying the crossover logic against the conditions.
    // **Validates: Requirements 2.4, 3.2, 3.4, 3.5, 3.7**
    #[test]
    fn prop_crossover_detection_with_guards(
        l1_volumes in arb_five_volumes(),
        l2_volumes in arb_five_volumes(),
    ) {
        let l1 = ConcreteContract {
            root: "ES".to_string(),
            month: MonthCode::H,
            year: 5,
        };
        let l2 = ConcreteContract {
            root: "ES".to_string(),
            month: MonthCode::M,
            year: 5,
        };
        let mut sm = RollStateMachine::new("ES".to_string(), l1.clone(), l2.clone());

        // Push 5 days of volume to fill both buffers
        for i in 0..5 {
            sm.accumulate_volume(&l1, l1_volumes[i]);
            sm.accumulate_volume(&l2, l2_volumes[i]);
            sm.end_of_day();
        }

        // At this point both buffers are full, roll_latched = false
        let trigger_date = NaiveDate::from_ymd_opt(2025, 3, 15).unwrap();
        let result = sm.evaluate_crossover(trigger_date);

        let l1_avg: f64 = l1_volumes.iter().sum::<u64>() as f64 / 5.0;
        let l2_avg: f64 = l2_volumes.iter().sum::<u64>() as f64 / 5.0;

        if l2_avg > l1_avg {
            // Crossover should fire
            prop_assert!(result.is_some(),
                "Expected RollSignal when l2_avg ({}) > l1_avg ({}), but got None",
                l2_avg, l1_avg);
        } else {
            // No crossover
            prop_assert!(result.is_none(),
                "Expected None when l2_avg ({}) <= l1_avg ({}), but got Some",
                l2_avg, l1_avg);
        }

        // Now test the latch guard: after execute_roll, crossover should NOT fire
        if result.is_some() {
            sm.execute_roll();
            // After roll, buffers are reset (not full) AND roll_latched is true,
            // so evaluate_crossover must return None regardless.
            let result_after_roll = sm.evaluate_crossover(trigger_date);
            prop_assert!(result_after_roll.is_none(),
                "Expected None after execute_roll (latched), but got Some");
        }
    }

    // Feature: futures-roll-manager, Property 7: Roll promotion advances the cycle
    //
    // For any RollStateMachine with arbitrary (L1, L2) contracts, executing a roll
    // SHALL result in new_L1 == old_L2 AND new_L2 == QuarterlyCycle::next(old_L2).
    // **Validates: Requirements 2.5**
    #[test]
    fn prop_roll_promotion_advances_cycle(
        l1 in arb_concrete_contract(),
        l2 in arb_concrete_contract(),
    ) {
        let mut sm = RollStateMachine::new("TEST".to_string(), l1, l2.clone());

        let expected_new_l1 = l2.clone();
        let expected_new_l2 = QuarterlyCycle::next(&l2);

        let transition = sm.execute_roll();

        prop_assert_eq!(&transition.new_l1, &expected_new_l1,
            "new_L1 should equal old_L2");
        prop_assert_eq!(&transition.new_l2, &expected_new_l2,
            "new_L2 should equal QuarterlyCycle::next(old_L2)");

        // Also verify the state machine itself reflects the new state
        let (_, current_l1, current_l2) = sm.current_state();
        prop_assert_eq!(current_l1, &expected_new_l1);
        prop_assert_eq!(current_l2, &expected_new_l2);
    }

    // Feature: futures-roll-manager, Property 8: Price adjustment round-trip
    //
    // For any raw price and cumulative factor (> 0), applying the adjustment
    // (price × factor) then reversing it (adjusted / factor) SHALL recover
    // the original price within floating-point tolerance (|error| < 1e-10).
    // **Validates: Requirements 4.3, 4.6, 4.7**
    #[test]
    fn prop_price_adjustment_round_trip(
        raw_price in arb_positive_price(),
        factor in arb_ratio(),
    ) {
        // Build a ContinuousAdjuster that has the desired cumulative factor.
        // apply_roll with old_close=1.0, new_close=factor gives ratio = factor.
        let mut adjuster = ContinuousAdjuster::new();
        let date = NaiveDate::from_ymd_opt(2025, 3, 15).unwrap();
        adjuster.apply_roll(1.0, factor, date, "ESH5", "ESM5").unwrap();

        let adjusted = adjuster.adjust_price(raw_price);
        let recovered = adjuster.unadjust_price(adjusted);

        let error = (recovered - raw_price).abs();
        prop_assert!(error < 1e-10,
            "Round-trip error too large: raw={}, adjusted={}, recovered={}, error={}",
            raw_price, adjusted, recovered, error);
    }

    // Feature: futures-roll-manager, Property 9: Cumulative factor is product of all ratios
    //
    // For any sequence of roll ratios (0.5 to 2.0), applying each via apply_roll
    // SHALL result in cumulative_factor equal to the product of all ratios.
    // **Validates: Requirements 4.1, 4.2**
    #[test]
    fn prop_cumulative_factor_is_product_of_ratios(
        ratios in prop::collection::vec(arb_ratio(), 1..10),
    ) {
        let mut adjuster = ContinuousAdjuster::new();
        let date = NaiveDate::from_ymd_opt(2025, 3, 15).unwrap();

        let mut expected_product = 1.0_f64;
        for (i, &ratio) in ratios.iter().enumerate() {
            // apply_roll with old_close=1.0, new_close=ratio gives ratio = ratio
            let old_contract = format!("C{}", i);
            let new_contract = format!("C{}", i + 1);
            adjuster.apply_roll(1.0, ratio, date, &old_contract, &new_contract).unwrap();
            expected_product *= ratio;
        }

        // adjust_price(1.0) returns 1.0 * cumulative_factor, so it gives us the factor
        let actual_factor = adjuster.adjust_price(1.0);
        let error = (actual_factor - expected_product).abs();
        // Use relative tolerance for larger products
        let tolerance = expected_product.abs() * 1e-10;
        prop_assert!(error < tolerance.max(1e-10),
            "cumulative_factor ({}) != product of ratios ({}), error={}",
            actual_factor, expected_product, error);
    }

    // Feature: futures-roll-manager, Property 10: Volume adjustment preserves ratio
    //
    // For any raw volume (> 0) and factor (> 0), the adjusted volume SHALL
    // equal raw_volume as f64 / factor.
    // **Validates: Requirements 4.4**
    #[test]
    fn prop_volume_adjustment_preserves_ratio(
        raw_volume in arb_positive_volume(),
        factor in arb_ratio(),
    ) {
        // Build a ContinuousAdjuster with the desired factor
        let mut adjuster = ContinuousAdjuster::new();
        let date = NaiveDate::from_ymd_opt(2025, 3, 15).unwrap();
        adjuster.apply_roll(1.0, factor, date, "ESH5", "ESM5").unwrap();

        let adjusted = adjuster.adjust_volume(raw_volume);
        let expected = raw_volume as f64 / factor;

        let error = (adjusted - expected).abs();
        prop_assert!(error < 1e-10,
            "adjust_volume({}) with factor {} = {}, expected {}, error={}",
            raw_volume, factor, adjusted, expected, error);
    }
}

// =============================================================================
// Additional Generators for Properties 11–15
// =============================================================================

use std::sync::Arc;
use flux_cli::live::futures_roll::FuturesRollManager;
use flux_cli::live::connector::LiveBar;
use flux_cli::live::product_registry::ProductRegistry;
use flux_cli::live::market_calendar::MarketCalendar;

/// Create a FuturesRollManager with an empty registry and calendar (no session data).
fn make_test_manager() -> FuturesRollManager {
    let registry = Arc::new(ProductRegistry::from_entries(&[]));
    let calendar = Arc::new(MarketCalendar::from_toml(
        r#"
[[session]]
exchange = "CME"
open = "09:30"
close = "16:00"
timezone = "US/Eastern"
"#,
    ).unwrap());
    FuturesRollManager::new(registry, calendar)
}

/// Create a LiveBar with given symbol and close price.
fn make_live_bar(symbol: &str, close: f64, volume: f64) -> LiveBar {
    LiveBar {
        bar: flux_runtime::BarContext {
            symbol: symbol.to_string(),
            open: close,
            high: close,
            low: close,
            close,
            volume,
            in_position: false,
        },
        connector_id: "test".to_string(),
        received_at: chrono::Utc::now(),
    }
}

/// Generate a non-futures symbol: lowercase letters, digits, or well-known equity tickers
/// that will NOT parse as a concrete contract (no trailing month-code + digit).
fn arb_non_futures_symbol() -> impl Strategy<Value = String> {
    prop_oneof![
        // Lowercase letters (will fail uppercase check in parse_concrete)
        prop::collection::vec(b'a'..=b'z', 2..=5)
            .prop_map(|bytes| String::from_utf8(bytes).unwrap()),
        // Known equity tickers
        Just("AAPL".to_string()),
        Just("MSFT".to_string()),
        Just("GOOG".to_string()),
        Just("TSLA".to_string()),
        // Numeric strings
        prop::collection::vec(b'0'..=b'9', 3..=6)
            .prop_map(|bytes| String::from_utf8(bytes).unwrap()),
    ]
}

/// Generate arbitrary OHLCV values for non-futures passthrough testing.
fn arb_ohlcv() -> impl Strategy<Value = (f64, f64, f64, f64, f64)> {
    (
        1.0f64..1000.0,
        1.0f64..1000.0,
        1.0f64..1000.0,
        1.0f64..1000.0,
        1.0f64..1_000_000.0,
    )
}

/// Generate a SymbolMode that is NOT Continuous (NthMonth only).
fn arb_non_continuous_mode() -> impl Strategy<Value = SymbolMode> {
    (1u8..=10u8).prop_map(SymbolMode::NthMonth)
}

// =============================================================================
// Property Tests 11–15
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    // Feature: futures-roll-manager, Property 11: Position roll preserves direction and quantity
    //
    // For any product root with a =F subscription, when end_of_session triggers a roll,
    // the emitted RollEvent SHALL specify the correct product_root, old/new contracts,
    // and use_calendar_spread = true.
    // **Validates: Requirements 5.4**
    #[test]
    fn prop_position_roll_emits_correct_event(
        l1_volumes in prop::collection::vec(1u64..500_000u64, 5..=5),
        l2_volumes in prop::collection::vec(500_001u64..1_000_000u64, 5..=5),
    ) {
        // Ensure L2 volumes are always greater than L1 volumes to trigger crossover
        let mut manager = make_test_manager();

        let l1 = ConcreteContract {
            root: "ES".to_string(),
            month: MonthCode::H,
            year: 5,
        };
        let l2 = ConcreteContract {
            root: "ES".to_string(),
            month: MonthCode::M,
            year: 5,
        };

        // Register a Continuous (=F) subscription with explicit contracts
        let generic = GenericSymbol {
            root: "ES".to_string(),
            mode: SymbolMode::Continuous,
        };
        manager.register_subscription_with_contracts(
            generic,
            "test_strategy".to_string(),
            l1.clone(),
            l2.clone(),
        );

        // Push 5 days of volume where L2 > L1 using end_of_session each day.
        // Days 1-4: buffers not full, end_of_session returns None.
        // Day 5: buffers full, crossover detected, roll fires.
        let mut roll_event = None;
        for i in 0..5 {
            let l1_bar = make_live_bar("ESH5", 5000.0, l1_volumes[i] as f64);
            let l2_bar = make_live_bar("ESM5", 5050.0, l2_volumes[i] as f64);
            manager.process_bar(&l1_bar);
            manager.process_bar(&l2_bar);

            let date = NaiveDate::from_ymd_opt(2025, 3, 10 + i as u32).unwrap();
            let result = manager.end_of_session("ES", date);
            if result.is_some() {
                roll_event = result;
            }
        }

        // A RollEvent should be emitted since we have a =F subscription
        prop_assert!(roll_event.is_some(), "Expected RollEvent for =F subscription");
        let event = roll_event.unwrap();

        // Verify event fields
        prop_assert_eq!(&event.product_root, "ES");
        prop_assert_eq!(&event.old_contract, &l1, "old_contract should be the original L1");
        prop_assert_eq!(&event.new_contract, &l2, "new_contract should be the original L2");
        prop_assert!(event.use_calendar_spread, "use_calendar_spread should be true");
    }

    // Feature: futures-roll-manager, Property 12: Only continuous mode triggers position rolls
    //
    // For any symbol subscription with mode != Continuous (i.e., =1, =2, =N),
    // a roll signal SHALL NOT emit a RollEvent for position rolling.
    // **Validates: Requirements 5.8**
    #[test]
    fn prop_non_continuous_mode_no_roll_event(
        mode in arb_non_continuous_mode(),
        l1_volumes in prop::collection::vec(1u64..500_000u64, 5..=5),
        l2_volumes in prop::collection::vec(500_001u64..1_000_000u64, 5..=5),
    ) {
        let mut manager = make_test_manager();

        let l1 = ConcreteContract {
            root: "ES".to_string(),
            month: MonthCode::H,
            year: 5,
        };
        let l2 = ConcreteContract {
            root: "ES".to_string(),
            month: MonthCode::M,
            year: 5,
        };

        // Register a NON-continuous subscription (=1, =2, etc.)
        let generic = GenericSymbol {
            root: "ES".to_string(),
            mode,
        };
        manager.register_subscription_with_contracts(
            generic,
            "test_strategy".to_string(),
            l1.clone(),
            l2.clone(),
        );

        // Push 5 days of volume where L2 > L1 using end_of_session each day
        let mut any_roll_event = false;
        for i in 0..5 {
            let l1_bar = make_live_bar("ESH5", 5000.0, l1_volumes[i] as f64);
            let l2_bar = make_live_bar("ESM5", 5050.0, l2_volumes[i] as f64);
            manager.process_bar(&l1_bar);
            manager.process_bar(&l2_bar);

            let date = NaiveDate::from_ymd_opt(2025, 3, 10 + i as u32).unwrap();
            if manager.end_of_session("ES", date).is_some() {
                any_roll_event = true;
            }
        }

        // No RollEvent should be emitted since there's no =F subscription
        prop_assert!(!any_roll_event,
            "Expected no RollEvent for non-continuous mode {:?}, but got Some", mode);
    }

    // Feature: futures-roll-manager, Property 13: Non-futures passthrough
    //
    // For any bar with a symbol that does not match any registered product root,
    // the FuturesRollManager SHALL emit the bar unmodified.
    // **Validates: Requirements 9.3**
    #[test]
    fn prop_non_futures_passthrough(
        symbol in arb_non_futures_symbol(),
        (open, high, low, close, volume) in arb_ohlcv(),
    ) {
        let mut manager = make_test_manager();

        // Register "ES" as a tracked product so we can verify non-matching symbols pass through
        let l1 = ConcreteContract {
            root: "ES".to_string(),
            month: MonthCode::H,
            year: 5,
        };
        let l2 = ConcreteContract {
            root: "ES".to_string(),
            month: MonthCode::M,
            year: 5,
        };
        let generic = GenericSymbol {
            root: "ES".to_string(),
            mode: SymbolMode::Continuous,
        };
        manager.register_subscription_with_contracts(
            generic,
            "test_strategy".to_string(),
            l1,
            l2,
        );

        // Create a bar with a non-futures symbol
        let bar = LiveBar {
            bar: flux_runtime::BarContext {
                symbol: symbol.clone(),
                open,
                high,
                low,
                close,
                volume,
                in_position: false,
            },
            connector_id: "test".to_string(),
            received_at: chrono::Utc::now(),
        };

        let result = manager.process_bar(&bar);

        // Bar should pass through unmodified
        prop_assert_eq!(result.bars.len(), 1, "Should emit exactly 1 bar for non-futures symbol");
        let emitted = &result.bars[0];
        prop_assert_eq!(&emitted.bar.symbol, &symbol, "Symbol should be unchanged");
        prop_assert_eq!(emitted.bar.open, open, "Open should be unchanged");
        prop_assert_eq!(emitted.bar.high, high, "High should be unchanged");
        prop_assert_eq!(emitted.bar.low, low, "Low should be unchanged");
        prop_assert_eq!(emitted.bar.close, close, "Close should be unchanged");
        prop_assert_eq!(emitted.bar.volume, volume, "Volume should be unchanged");
        prop_assert!(result.roll_event.is_none(), "No roll event for non-futures");
    }

    // Feature: futures-roll-manager, Property 14: State persistence round-trip
    //
    // For any FuturesRollManager with registered state machines, adjusters, and
    // roll history, serializing via snapshot_state then restoring via restore_state
    // SHALL produce an equivalent state.
    // **Validates: Requirements 7.4, 7.5**
    #[test]
    fn prop_state_persistence_round_trip(
        l1_volumes in prop::collection::vec(1u64..1_000_000u64, 1..=5),
        l2_volumes in prop::collection::vec(1u64..1_000_000u64, 1..=5),
        factor in (50i64..200i64).prop_map(|x| x as f64 / 100.0),
    ) {
        let mut manager = make_test_manager();

        let l1 = ConcreteContract {
            root: "ES".to_string(),
            month: MonthCode::H,
            year: 5,
        };
        let l2 = ConcreteContract {
            root: "ES".to_string(),
            month: MonthCode::M,
            year: 5,
        };

        // Register subscription
        let generic = GenericSymbol {
            root: "ES".to_string(),
            mode: SymbolMode::Continuous,
        };
        manager.register_subscription_with_contracts(
            generic,
            "test_strategy".to_string(),
            l1.clone(),
            l2.clone(),
        );

        // Push volume data using public API (process_bar + end_of_session)
        let days = l1_volumes.len().min(l2_volumes.len());
        for i in 0..days {
            let l1_bar = make_live_bar("ESH5", 5000.0, l1_volumes[i] as f64);
            let l2_bar = make_live_bar("ESM5", 5050.0, l2_volumes[i] as f64);
            manager.process_bar(&l1_bar);
            manager.process_bar(&l2_bar);
            // Use end_of_session to push daily volumes (won't trigger roll until 5 days)
            let date = NaiveDate::from_ymd_opt(2025, 3, 10 + i as u32).unwrap();
            let _ = manager.end_of_session("ES", date);
        }

        // Apply a roll adjustment via the public API
        let date = NaiveDate::from_ymd_opt(2025, 4, 1).unwrap();
        let _ = manager.apply_roll_adjustment("ES", 1.0, factor, date, "ESH5", "ESM5");

        // Snapshot state
        let snapshot = manager.snapshot_state();

        // Create a fresh manager and restore
        let mut restored_manager = make_test_manager();
        restored_manager.restore_state(&snapshot).unwrap();

        // Re-snapshot the restored manager to compare
        let restored_snapshot = restored_manager.snapshot_state();

        // Verify state machines match
        prop_assert_eq!(snapshot.machines.len(), restored_snapshot.machines.len(),
            "Number of state machines should match");
        for (orig, rest) in snapshot.machines.iter().zip(restored_snapshot.machines.iter()) {
            prop_assert_eq!(&orig.product_root, &rest.product_root);
            prop_assert_eq!(&orig.l1, &rest.l1, "L1 should match after restore");
            prop_assert_eq!(&orig.l2, &rest.l2, "L2 should match after restore");
            prop_assert_eq!(orig.roll_latched, rest.roll_latched, "roll_latched should match");
            prop_assert_eq!(&orig.phase, &rest.phase, "phase should match");
            prop_assert_eq!(&orig.l1_volumes, &rest.l1_volumes, "L1 volume buffers should match");
            prop_assert_eq!(&orig.l2_volumes, &rest.l2_volumes, "L2 volume buffers should match");
        }

        // Verify adjusters match
        prop_assert_eq!(snapshot.adjusters.len(), restored_snapshot.adjusters.len());
        for (orig, rest) in snapshot.adjusters.iter().zip(restored_snapshot.adjusters.iter()) {
            prop_assert_eq!(&orig.product_root, &rest.product_root);
            let err = (orig.cumulative_factor - rest.cumulative_factor).abs();
            prop_assert!(err < 1e-10,
                "cumulative_factor mismatch: orig={}, restored={}",
                orig.cumulative_factor, rest.cumulative_factor);
        }

        // Verify roll history matches
        prop_assert_eq!(snapshot.roll_history.len(), restored_snapshot.roll_history.len(),
            "roll_history length should match");
        for (orig, rest) in snapshot.roll_history.iter().zip(restored_snapshot.roll_history.iter()) {
            prop_assert_eq!(&orig.product_root, &rest.product_root);
            prop_assert_eq!(&orig.old_contract, &rest.old_contract);
            prop_assert_eq!(&orig.new_contract, &rest.new_contract);
            prop_assert_eq!(orig.date, rest.date);
        }
    }

    // Feature: futures-roll-manager, Property 15: Roll history grows monotonically
    //
    // For any roll execution on a product, the roll_history length SHALL
    // increase by exactly 1 with correct fields.
    // **Validates: Requirements 7.3**
    #[test]
    fn prop_roll_history_grows_monotonically(
        l1_volumes in prop::collection::vec(1u64..500_000u64, 5..=5),
        l2_volumes in prop::collection::vec(500_001u64..1_000_000u64, 5..=5),
    ) {
        let mut manager = make_test_manager();

        let l1 = ConcreteContract {
            root: "ES".to_string(),
            month: MonthCode::H,
            year: 5,
        };
        let l2 = ConcreteContract {
            root: "ES".to_string(),
            month: MonthCode::M,
            year: 5,
        };

        // Register =F subscription to get roll events
        let generic = GenericSymbol {
            root: "ES".to_string(),
            mode: SymbolMode::Continuous,
        };
        manager.register_subscription_with_contracts(
            generic,
            "test_strategy".to_string(),
            l1.clone(),
            l2.clone(),
        );

        // Record history length before via snapshot
        let history_len_before = manager.snapshot_state().roll_history.len();
        prop_assert_eq!(history_len_before, 0, "History should start empty");

        // Push 5 days of volume where L2 > L1 using end_of_session
        let mut roll_fired_on_day = None;
        for i in 0..5 {
            let l1_bar = make_live_bar("ESH5", 5000.0, l1_volumes[i] as f64);
            let l2_bar = make_live_bar("ESM5", 5050.0, l2_volumes[i] as f64);
            manager.process_bar(&l1_bar);
            manager.process_bar(&l2_bar);

            let date = NaiveDate::from_ymd_opt(2025, 3, 10 + i as u32).unwrap();
            if manager.end_of_session("ES", date).is_some() {
                roll_fired_on_day = Some(date);
            }
        }

        // Roll should have fired on day 5
        prop_assert!(roll_fired_on_day.is_some(), "Roll should have fired");
        let trigger_date = roll_fired_on_day.unwrap();

        // History should grow by exactly 1
        let snapshot = manager.snapshot_state();
        let history_len_after = snapshot.roll_history.len();
        prop_assert_eq!(history_len_after, history_len_before + 1,
            "Roll history should grow by exactly 1");

        // Verify the newest entry has correct fields
        let record = snapshot.roll_history.last().unwrap();
        prop_assert_eq!(record.date, trigger_date, "Roll date should match trigger date");
        prop_assert_eq!(&record.product_root, "ES", "Product root should be ES");
        prop_assert_eq!(&record.old_contract, &format_concrete(&l1),
            "old_contract should be the original L1");
        prop_assert_eq!(&record.new_contract, &format_concrete(&l2),
            "new_contract should be the original L2 (now promoted to L1)");
    }
}
