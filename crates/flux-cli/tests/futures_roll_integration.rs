//! Integration tests for the Futures Roll Manager.
//!
//! Tests the full pipeline from raw contract bars through to synthetic bar emission,
//! position roll via RollEvent, state persistence across restart, and non-futures bypass.
//!
//! **Validates: Requirements 1.5, 1.6, 5.1, 5.2, 5.3, 5.4, 6.1, 6.5, 7.4, 7.5, 9.1, 9.2, 9.5**

use std::sync::Arc;

use chrono::NaiveDate;
use flux_cli::live::connector::LiveBar;
use flux_cli::live::futures_roll::*;
use flux_cli::live::market_calendar::MarketCalendar;
use flux_cli::live::product_registry::ProductRegistry;

// =============================================================================
// Test Helpers
// =============================================================================

/// Create a FuturesRollManager with an empty product registry and a basic CME calendar.
fn make_test_manager() -> FuturesRollManager {
    let registry = Arc::new(ProductRegistry::from_entries(&[]));
    let calendar = Arc::new(
        MarketCalendar::from_toml(
            r#"
[[session]]
exchange = "CME"
open = "09:30"
close = "16:00"
timezone = "US/Eastern"
"#,
        )
        .unwrap(),
    );
    FuturesRollManager::new(registry, calendar)
}

/// Create a LiveBar with the given symbol, close price, and volume.
/// Open/High/Low are derived from close for simplicity.
fn make_live_bar(symbol: &str, close: f64, volume: f64) -> LiveBar {
    LiveBar {
        bar: flux_runtime::BarContext {
            symbol: symbol.to_string(),
            open: close - 10.0,
            high: close + 5.0,
            low: close - 15.0,
            close,
            volume,
            in_position: false,
        },
        connector_id: "test".to_string(),
        received_at: chrono::Utc::now(),
    }
}

// =============================================================================
// 6.1 — Full pipeline synthetic bar emission
// =============================================================================

/// Validates: Requirements 1.5, 1.6, 6.1, 6.5, 9.1, 9.2
#[test]
fn test_full_pipeline_synthetic_bar_emission_no_adjustment() {
    let mut manager = make_test_manager();

    // Register ES with explicit L1=ESH5, L2=ESM5
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

    // Register =F (Continuous) subscription
    manager.register_subscription_with_contracts(
        GenericSymbol {
            root: "ES".to_string(),
            mode: SymbolMode::Continuous,
        },
        "test_strategy".to_string(),
        l1.clone(),
        l2.clone(),
    );

    // Register =1 (NthMonth(1)) subscription
    manager.register_subscription_with_contracts(
        GenericSymbol {
            root: "ES".to_string(),
            mode: SymbolMode::NthMonth(1),
        },
        "test_strategy".to_string(),
        l1.clone(),
        l2.clone(),
    );

    // Feed a raw ESH5 bar through process_bar
    let raw_bar = make_live_bar("ESH5", 5000.0, 15000.0);
    let result = manager.process_bar(&raw_bar);

    // With no adjustments applied yet (factor=1.0), both bars should be emitted
    assert_eq!(
        result.bars.len(),
        2,
        "Expected 2 synthetic bars (ES=F and ES=1)"
    );

    // Find ES=F bar (adjusted — but factor=1.0, so prices are same as raw)
    let es_f_bar = result
        .bars
        .iter()
        .find(|b| b.bar.symbol == "ES=F")
        .expect("ES=F synthetic bar should be emitted");
    assert_eq!(es_f_bar.bar.close, 5000.0);
    assert_eq!(es_f_bar.bar.open, 4990.0);
    assert_eq!(es_f_bar.bar.high, 5005.0);
    assert_eq!(es_f_bar.bar.low, 4985.0);

    // Find ES=1 bar (unadjusted — always raw prices)
    let es_1_bar = result
        .bars
        .iter()
        .find(|b| b.bar.symbol == "ES=1")
        .expect("ES=1 synthetic bar should be emitted");
    assert_eq!(es_1_bar.bar.close, 5000.0);
    assert_eq!(es_1_bar.bar.open, 4990.0);
    assert_eq!(es_1_bar.bar.high, 5005.0);
    assert_eq!(es_1_bar.bar.low, 4985.0);
    assert_eq!(es_1_bar.bar.volume, 15000.0);

    // No roll event should fire from a simple bar
    assert!(result.roll_event.is_none());
}

/// After applying a roll adjustment (factor != 1.0), =F bars get adjusted prices.
/// Validates: Requirements 1.5, 6.5, 9.1, 9.2
#[test]
fn test_full_pipeline_adjusted_prices_after_roll() {
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

    // Register =F subscription
    manager.register_subscription_with_contracts(
        GenericSymbol {
            root: "ES".to_string(),
            mode: SymbolMode::Continuous,
        },
        "test_strategy".to_string(),
        l1.clone(),
        l2.clone(),
    );

    // Register =1 subscription
    manager.register_subscription_with_contracts(
        GenericSymbol {
            root: "ES".to_string(),
            mode: SymbolMode::NthMonth(1),
        },
        "test_strategy".to_string(),
        l1.clone(),
        l2.clone(),
    );

    // Apply a roll adjustment: old_close=5000, new_close=5100 → ratio=1.02
    let date = NaiveDate::from_ymd_opt(2025, 3, 14).unwrap();
    let ratio = manager
        .apply_roll_adjustment("ES", 5000.0, 5100.0, date, "ESH5", "ESM5")
        .unwrap();
    assert!((ratio - 1.02).abs() < 1e-10);

    // Now feed a raw ESH5 bar — =F should get adjusted prices
    let raw_bar = make_live_bar("ESH5", 5000.0, 10000.0);
    let result = manager.process_bar(&raw_bar);

    // ES=F bar should have adjusted prices (raw * 1.02)
    let es_f_bar = result
        .bars
        .iter()
        .find(|b| b.bar.symbol == "ES=F")
        .expect("ES=F bar emitted");
    assert!(
        (es_f_bar.bar.close - 5000.0 * 1.02).abs() < 1e-10,
        "ES=F close should be adjusted: expected {}, got {}",
        5000.0 * 1.02,
        es_f_bar.bar.close
    );
    assert!(
        (es_f_bar.bar.open - 4990.0 * 1.02).abs() < 1e-10,
        "ES=F open should be adjusted"
    );
    assert!(
        (es_f_bar.bar.high - 5005.0 * 1.02).abs() < 1e-10,
        "ES=F high should be adjusted"
    );
    assert!(
        (es_f_bar.bar.low - 4985.0 * 1.02).abs() < 1e-10,
        "ES=F low should be adjusted"
    );
    // Volume adjusted: raw_volume / factor
    assert!(
        (es_f_bar.bar.volume - 10000.0 / 1.02).abs() < 1e-6,
        "ES=F volume should be adjusted (divided by factor)"
    );

    // ES=1 bar should have unadjusted (raw) prices
    let es_1_bar = result
        .bars
        .iter()
        .find(|b| b.bar.symbol == "ES=1")
        .expect("ES=1 bar emitted");
    assert_eq!(es_1_bar.bar.close, 5000.0, "ES=1 close should be raw");
    assert_eq!(es_1_bar.bar.volume, 10000.0, "ES=1 volume should be raw");
}

// =============================================================================
// 6.2 — Position roll via BrokerAdapter (RollEvent emission)
// =============================================================================

/// Validates: Requirements 5.1, 5.2, 5.3, 5.4
#[test]
fn test_position_roll_via_volume_crossover() {
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

    // Register =F subscription (only continuous mode triggers position rolls)
    manager.register_subscription_with_contracts(
        GenericSymbol {
            root: "ES".to_string(),
            mode: SymbolMode::Continuous,
        },
        "test_strategy".to_string(),
        l1.clone(),
        l2.clone(),
    );

    // Feed 5 days of volume data where L2 volume > L1 volume
    // This triggers the crossover condition (avg(L2) > avg(L1) after 5 full days)
    for day in 0..5 {
        // L1 gets low volume
        let l1_bar = make_live_bar("ESH5", 5000.0, 1000.0);
        manager.process_bar(&l1_bar);

        // L2 gets high volume
        let l2_bar = make_live_bar("ESM5", 5100.0, 5000.0);
        manager.process_bar(&l2_bar);

        // End of session — push volumes into buffers (but don't trigger roll until day 5)
        let date = NaiveDate::from_ymd_opt(2025, 3, 10 + day).unwrap();
        // We call end_of_session but only the last call (day 5) should trigger the roll
        // because buffers need 5 full days
        let event = manager.end_of_session("ES", date);

        if day < 4 {
            // Buffers not yet full — no roll event
            assert!(
                event.is_none(),
                "Roll should not trigger before 5 days (day {})",
                day
            );
        } else {
            // Day 5: buffers are full and L2 avg > L1 avg → roll triggers
            let roll_event = event.expect("Roll should trigger on day 5 (buffers full, L2 > L1)");

            // Verify RollEvent fields
            assert_eq!(roll_event.product_root, "ES");
            assert_eq!(roll_event.old_contract.root, "ES");
            assert_eq!(roll_event.old_contract.month, MonthCode::H);
            assert_eq!(roll_event.old_contract.year, 5);
            assert_eq!(roll_event.new_contract.root, "ES");
            assert_eq!(roll_event.new_contract.month, MonthCode::M);
            assert_eq!(roll_event.new_contract.year, 5);

            // use_calendar_spread should be true
            assert!(
                roll_event.use_calendar_spread,
                "Roll event should indicate calendar spread execution"
            );

            // Position qty and direction are placeholders (filled by harness)
            // They should be present as default values
            assert_eq!(
                roll_event.position_qty, 0.0,
                "position_qty is placeholder (filled by harness from position tracker)"
            );
            // direction is a placeholder Side::Buy (filled by harness)
            assert_eq!(
                roll_event.direction,
                flux_cli::live::broker::Side::Buy,
                "direction is placeholder (filled by harness)"
            );
        }
    }
}

/// Verify that after roll, L1 is promoted and new L2 is computed.
/// Validates: Requirements 5.1, 5.2
#[test]
fn test_roll_event_contract_promotion() {
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

    manager.register_subscription_with_contracts(
        GenericSymbol {
            root: "ES".to_string(),
            mode: SymbolMode::Continuous,
        },
        "test_strategy".to_string(),
        l1.clone(),
        l2.clone(),
    );

    // Feed 5 days of crossover data
    for day in 0..5 {
        let l1_bar = make_live_bar("ESH5", 5000.0, 500.0);
        manager.process_bar(&l1_bar);
        let l2_bar = make_live_bar("ESM5", 5100.0, 3000.0);
        manager.process_bar(&l2_bar);

        let date = NaiveDate::from_ymd_opt(2025, 3, 10 + day).unwrap();
        manager.end_of_session("ES", date);
    }

    // After roll, verify mapping: L1 should now be ESM5, L2 should be ESU5
    let mapping = manager
        .current_mapping("ES")
        .expect("ES should have a mapping");
    assert_eq!(mapping.l1.month, MonthCode::M, "New L1 should be ESM5");
    assert_eq!(mapping.l1.year, 5);
    assert_eq!(mapping.l2.month, MonthCode::U, "New L2 should be ESU5");
    assert_eq!(mapping.l2.year, 5);
}

// =============================================================================
// 6.3 — State persistence across restart
// =============================================================================

/// Validates: Requirements 7.4, 7.5
#[test]
fn test_state_persistence_across_restart() {
    // --- Phase 1: Create manager, register subscriptions, push data, apply adjustment ---
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

    manager.register_subscription_with_contracts(
        GenericSymbol {
            root: "ES".to_string(),
            mode: SymbolMode::Continuous,
        },
        "test_strategy".to_string(),
        l1.clone(),
        l2.clone(),
    );

    // Push some volume data (3 days worth)
    for _ in 0..3 {
        let l1_bar = make_live_bar("ESH5", 5000.0, 2000.0);
        manager.process_bar(&l1_bar);
        let l2_bar = make_live_bar("ESM5", 5100.0, 1000.0);
        manager.process_bar(&l2_bar);

        let date = NaiveDate::from_ymd_opt(2025, 3, 10).unwrap();
        manager.end_of_session("ES", date);
    }

    // Apply an adjustment (simulating a previous roll)
    let date = NaiveDate::from_ymd_opt(2025, 3, 14).unwrap();
    manager
        .apply_roll_adjustment("ES", 4900.0, 5000.0, date, "ESZ4", "ESH5")
        .unwrap();

    // Take snapshot
    let snapshot = manager.snapshot_state();

    // --- Phase 2: Create a NEW manager and restore state ---
    let mut new_manager = make_test_manager();

    // Need to register subscriptions on new manager so it has the subscription list
    // (subscriptions are not part of the persisted state — they come from account config)
    new_manager.register_subscription_with_contracts(
        GenericSymbol {
            root: "ES".to_string(),
            mode: SymbolMode::Continuous,
        },
        "test_strategy".to_string(),
        l1.clone(),
        l2.clone(),
    );

    // Restore state
    new_manager
        .restore_state(&snapshot)
        .expect("State restoration should succeed");

    // Verify L1/L2 mappings match
    let mapping = new_manager
        .current_mapping("ES")
        .expect("ES mapping should exist after restore");
    assert_eq!(mapping.l1.root, "ES");
    assert_eq!(mapping.l1.month, MonthCode::H);
    assert_eq!(mapping.l1.year, 5);
    assert_eq!(mapping.l2.root, "ES");
    assert_eq!(mapping.l2.month, MonthCode::M);
    assert_eq!(mapping.l2.year, 5);

    // Verify cumulative factor matches (4900 → 5000 ratio = 5000/4900 ≈ 1.02040816...)
    let expected_factor = 5000.0 / 4900.0;
    assert!(
        (mapping.cumulative_adjustment_factor - expected_factor).abs() < 1e-10,
        "Cumulative factor should survive restart: expected {}, got {}",
        expected_factor,
        mapping.cumulative_adjustment_factor
    );

    // --- Phase 3: Verify continued operation ---
    // Push 2 more days of volume data into the restored manager
    for _ in 0..2 {
        let l1_bar = make_live_bar("ESH5", 5000.0, 3000.0);
        new_manager.process_bar(&l1_bar);
        let l2_bar = make_live_bar("ESM5", 5100.0, 1500.0);
        new_manager.process_bar(&l2_bar);

        let date = NaiveDate::from_ymd_opt(2025, 3, 15).unwrap();
        new_manager.end_of_session("ES", date);
    }

    // The volume buffers should now have 5 days total (3 restored + 2 new)
    // We can verify by checking that a crossover evaluation is possible
    // (buffers are full, meaning averages can be computed)
    // Since L1 volume (2000, 2000, 2000, 3000, 3000) > L2 volume (1000, 1000, 1000, 1500, 1500),
    // no roll should trigger (L1 > L2)
    let date = NaiveDate::from_ymd_opt(2025, 3, 16).unwrap();
    let event = new_manager.end_of_session("ES", date);
    // Even though we call end_of_session an extra time, we already pushed volumes via end_of_session above.
    // The key assertion: the manager is operational and doesn't crash or re-trigger old rolls.
    // No roll should fire because L1 avg > L2 avg.
    assert!(
        event.is_none(),
        "No roll should trigger when L1 volume > L2 volume"
    );

    // Verify adjusted prices work correctly with restored factor
    let raw_bar = make_live_bar("ESH5", 5000.0, 10000.0);
    let result = new_manager.process_bar(&raw_bar);
    let es_f_bar = result
        .bars
        .iter()
        .find(|b| b.bar.symbol == "ES=F")
        .expect("ES=F bar should be emitted from restored manager");
    assert!(
        (es_f_bar.bar.close - 5000.0 * expected_factor).abs() < 1e-6,
        "Adjusted price should use restored cumulative factor"
    );
}

// =============================================================================
// 6.4 — Non-futures bypass
// =============================================================================

/// Validates: Requirements 9.5
#[test]
fn test_non_futures_bar_passes_through_unmodified() {
    let mut manager = make_test_manager();

    // Register ES — so the manager is active for futures
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
    manager.register_subscription_with_contracts(
        GenericSymbol {
            root: "ES".to_string(),
            mode: SymbolMode::Continuous,
        },
        "test_strategy".to_string(),
        l1,
        l2,
    );

    // Feed a bar with symbol "AAPL" (not a futures symbol — can't parse as concrete contract)
    let aapl_bar = make_live_bar("AAPL", 185.50, 1200000.0);
    let result = manager.process_bar(&aapl_bar);

    // Assert: bar passes through unmodified
    assert_eq!(result.bars.len(), 1, "Non-futures bar should pass through");
    let output_bar = &result.bars[0];
    assert_eq!(output_bar.bar.symbol, "AAPL", "Symbol should be unchanged");
    assert_eq!(output_bar.bar.close, 185.50, "Close should be unchanged");
    assert_eq!(
        output_bar.bar.open,
        185.50 - 10.0,
        "Open should be unchanged"
    );
    assert_eq!(
        output_bar.bar.high,
        185.50 + 5.0,
        "High should be unchanged"
    );
    assert_eq!(
        output_bar.bar.low,
        185.50 - 15.0,
        "Low should be unchanged"
    );
    assert_eq!(
        output_bar.bar.volume, 1200000.0,
        "Volume should be unchanged"
    );
    assert!(result.roll_event.is_none(), "No roll event for non-futures");
}

/// Test the None case — when no FuturesRollManager exists in the harness,
/// bars pass through unchanged. We simulate this by using Option<FuturesRollManager>.
/// Validates: Requirements 9.5
#[test]
fn test_no_roll_manager_bar_passes_through() {
    // Simulate the harness pattern: Option<FuturesRollManager> = None
    let roll_manager: Option<FuturesRollManager> = None;

    let bar = make_live_bar("AAPL", 185.50, 1200000.0);

    // The harness dispatch pattern:
    let bars_to_dispatch = match roll_manager {
        Some(mut frm) => {
            let result = frm.process_bar(&bar);
            result.bars
        }
        None => vec![bar.clone()],
    };

    // Bar should be unchanged
    assert_eq!(bars_to_dispatch.len(), 1);
    let output = &bars_to_dispatch[0];
    assert_eq!(output.bar.symbol, "AAPL");
    assert_eq!(output.bar.close, 185.50);
    assert_eq!(output.bar.open, 185.50 - 10.0);
    assert_eq!(output.bar.high, 185.50 + 5.0);
    assert_eq!(output.bar.low, 185.50 - 15.0);
    assert_eq!(output.bar.volume, 1200000.0);
}

/// A futures-formatted symbol that isn't registered also passes through.
/// Validates: Requirements 9.5
#[test]
fn test_unregistered_futures_symbol_passes_through() {
    let mut manager = make_test_manager();

    // Register ES only
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
    manager.register_subscription_with_contracts(
        GenericSymbol {
            root: "ES".to_string(),
            mode: SymbolMode::Continuous,
        },
        "test_strategy".to_string(),
        l1,
        l2,
    );

    // Feed a bar for NQH5 — valid futures format but NQ is not registered
    let nq_bar = make_live_bar("NQH5", 18000.0, 50000.0);
    let result = manager.process_bar(&nq_bar);

    // Should pass through unmodified since NQ root has no state machine
    assert_eq!(
        result.bars.len(),
        1,
        "Unregistered futures symbol should pass through"
    );
    assert_eq!(result.bars[0].bar.symbol, "NQH5");
    assert_eq!(result.bars[0].bar.close, 18000.0);
    assert_eq!(result.bars[0].bar.volume, 50000.0);
    assert!(result.roll_event.is_none());
}
