//! Property-based tests for the Broker Adapter module.
//!
//! Tests correctness properties defined in the broker-adapter design document.

use proptest::prelude::*;

use flux_cli::live::broker::{
    BrokerAdapter, Order, OrderId, Side,
    execution::{translate_signal, ExecutionPolicy, AdaptiveUrgency, aggressive_limit_price, DeduplicationGuard, parse_execution_policy},
    mock::MockBrokerAdapter,
};
use flux_runtime::Signal;

// =============================================================================
// Strategies (Generators)
// =============================================================================

/// Strategy that generates a random ExecutionPolicy variant.
fn arb_execution_policy() -> impl Strategy<Value = ExecutionPolicy> {
    prop_oneof![
        Just(ExecutionPolicy::Market),
        (-10i32..10i32).prop_map(|offset_ticks| ExecutionPolicy::AggressiveLimit { offset_ticks }),
        (1.0..10000.0f64).prop_map(|price| ExecutionPolicy::Limit { price }),
        Just(ExecutionPolicy::MarketOnClose),
        (1.0..10000.0f64).prop_map(|price| ExecutionPolicy::LimitOnClose { price }),
        (1.0..10000.0f64).prop_map(|trigger_price| ExecutionPolicy::Stop { trigger_price }),
        (1.0..10000.0f64, 1.0..10000.0f64).prop_map(|(trigger_price, limit_price)| {
            ExecutionPolicy::StopLimit { trigger_price, limit_price }
        }),
        (0.01..100.0f64).prop_map(|trail_amount| ExecutionPolicy::TrailingStop { trail_amount }),
        (0.01..50.0f64).prop_map(|trail_pct| ExecutionPolicy::TrailingStopPct { trail_pct }),
        prop_oneof![
            Just(AdaptiveUrgency::Patient),
            Just(AdaptiveUrgency::Normal),
            Just(AdaptiveUrgency::Urgent),
        ].prop_map(|urgency| ExecutionPolicy::Adaptive { urgency }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // Feature: broker-adapter, Property 8: Mock Adapter State Consistency
    /// **Validates: Requirements 9.6**
    ///
    /// For any sequence of orders submitted to MockBrokerAdapter (with ImmediateFill behavior),
    /// querying `get_positions()` SHALL reflect the net position changes from all filled orders,
    /// and `get_open_orders()` SHALL return an empty list (since all are immediately filled).
    #[test]
    fn prop_mock_adapter_state_consistency(
        num_buys in 0..10u32,
        num_sells in 0..10u32,
        symbol in "[A-Z]{1,3}",
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mock = MockBrokerAdapter::new();
            // mock defaults to ImmediateFill

            // Submit buy orders
            for i in 0..num_buys {
                let order = Order {
                    id: OrderId(format!("buy_{}", i)),
                    symbol: symbol.clone(),
                    side: Side::Buy,
                    contracts: 1,
                    execution: ExecutionPolicy::Market,
                    last_price: 100.0,
                    tick_size: 0.25,
                };
                mock.submit_order(&order).await.unwrap();
            }

            // Submit sell orders
            for i in 0..num_sells {
                let order = Order {
                    id: OrderId(format!("sell_{}", i)),
                    symbol: symbol.clone(),
                    side: Side::Sell,
                    contracts: 1,
                    execution: ExecutionPolicy::Market,
                    last_price: 100.0,
                    tick_size: 0.25,
                };
                mock.submit_order(&order).await.unwrap();
            }

            // All immediately filled → open_orders should be empty
            let open = mock.get_open_orders().await.unwrap();
            prop_assert!(open.is_empty(), "Expected no open orders with ImmediateFill, got {}", open.len());

            // Net position should reflect buys - sells
            let positions = mock.get_positions().await.unwrap();
            let expected_net = num_buys as f64 - num_sells as f64;
            if expected_net == 0.0 && num_buys == 0 && num_sells == 0 {
                prop_assert!(positions.is_empty(), "Expected no positions when no orders submitted");
            } else {
                // If there was at least one order, there should be a position
                prop_assert!(!positions.is_empty(), "Expected a position after submitting orders");
                let pos = &positions[0];
                prop_assert_eq!(&pos.symbol, &symbol);
                prop_assert!(
                    (pos.qty - expected_net).abs() < 1e-10,
                    "Expected net position {} but got {} for symbol {}",
                    expected_net, pos.qty, symbol
                );
            }

            Ok(())
        })?;
    }

    // Feature: broker-adapter, Property 3: AggressiveLimit Price Calculation
    /// **Validates: Requirements 3.7**
    ///
    /// For any combination of (side, last_price, offset_ticks, tick_size) where
    /// last_price > 0 and tick_size > 0, the aggressive limit price SHALL equal:
    /// - `last_price + (offset_ticks * tick_size)` for Buy orders
    /// - `last_price - (offset_ticks * tick_size)` for Sell orders
    #[test]
    fn prop_aggressive_limit_price_calculation(
        last_price in 1.0..10000.0f64,
        offset_ticks in 1..100i32,
        tick_size in 0.01..1.0f64,
    ) {
        // Buy: last_price + (offset_ticks * tick_size)
        let buy_price = aggressive_limit_price(Side::Buy, last_price, offset_ticks, tick_size);
        let expected_buy = last_price + (offset_ticks as f64 * tick_size);
        prop_assert!((buy_price - expected_buy).abs() < 1e-10,
            "Buy price mismatch: got {}, expected {}", buy_price, expected_buy);

        // Sell: last_price - (offset_ticks * tick_size)
        let sell_price = aggressive_limit_price(Side::Sell, last_price, offset_ticks, tick_size);
        let expected_sell = last_price - (offset_ticks as f64 * tick_size);
        prop_assert!((sell_price - expected_sell).abs() < 1e-10,
            "Sell price mismatch: got {}, expected {}", sell_price, expected_sell);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // Feature: broker-adapter, Property 2: Sub-Unit Quantities Are Sized Out
    /// **Validates: Requirements 3.2**
    ///
    /// For any Signal with quantity in the range (0.0, 1.0), translating the signal
    /// SHALL return None (the signal is "sized out") regardless of the ExecutionPolicy,
    /// symbol, or other parameters.
    #[test]
    fn prop_sub_unit_quantities_sized_out(
        symbol in "[A-Z]{1,5}",
        qty in 0.01..1.0f64,
    ) {
        // Open signal with sub-unit qty should be sized out
        let signal = Signal::Open { symbol: symbol.clone(), qty };
        let result = translate_signal(&signal, &ExecutionPolicy::Market, "acc", "strat", 1, 100.0, 0.25, 0.0);
        prop_assert!(result.is_none(), "Open with qty={} should be sized out (None), got Some", qty);

        // Short signal with sub-unit qty should be sized out
        let signal = Signal::Short { symbol: symbol.clone(), qty };
        let result = translate_signal(&signal, &ExecutionPolicy::Market, "acc", "strat", 1, 100.0, 0.25, 0.0);
        prop_assert!(result.is_none(), "Short with qty={} should be sized out (None), got Some", qty);

        // CloseQty signal with sub-unit qty should be sized out
        let signal = Signal::CloseQty { symbol, qty };
        let result = translate_signal(&signal, &ExecutionPolicy::Market, "acc", "strat", 1, 100.0, 0.25, 0.0);
        prop_assert!(result.is_none(), "CloseQty with qty={} should be sized out (None), got Some", qty);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // Feature: broker-adapter, Property 4: OrderId Determinism
    /// **Validates: Requirements 7.1**
    ///
    /// For any (account, strategy, symbol, bar_index) tuple, generating the OrderId
    /// SHALL always produce the string "{account}_{strategy}_{symbol}_{bar_index}",
    /// and generating it twice with the same inputs SHALL produce equal values.
    #[test]
    fn prop_order_id_determinism(
        account in "[a-z]{3,10}",
        strategy in "[a-z]{3,10}",
        symbol in "[A-Z]{1,5}",
        bar_index in 0..100000u64,
    ) {
        let signal = Signal::Open { symbol: symbol.clone(), qty: 5.0 };
        let policy = ExecutionPolicy::Market;

        let order1 = translate_signal(&signal, &policy, &account, &strategy, bar_index, 100.0, 0.25, 0.0);
        let order2 = translate_signal(&signal, &policy, &account, &strategy, bar_index, 100.0, 0.25, 0.0);

        let o1 = order1.unwrap();
        let o2 = order2.unwrap();

        // Idempotent: same inputs produce same OrderId
        prop_assert_eq!(&o1.id, &o2.id);

        // Format: {account}_{strategy}_{symbol}_{bar_index}
        let expected = format!("{}_{}_{}_{}", account, strategy, symbol, bar_index);
        prop_assert_eq!(&o1.id.0, &expected);
    }
}


// =============================================================================
// Property 1: Signal Translation Preserves Intent
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // Feature: broker-adapter, Property 1: Signal Translation Preserves Intent
    // **Validates: Requirements 3.1, 3.3, 3.4, 3.5, 3.6, 3.8**

    /// Signal::Open produces an Order with Side::Buy, preserved symbol, contracts == floor(qty), contracts > 0
    #[test]
    fn prop_open_signal_preserves_intent(
        symbol in "[A-Z]{1,5}",
        qty in 1.0..10000.0f64,
        policy in arb_execution_policy(),
        last_price in 1.0..10000.0f64,
        tick_size in 0.01..10.0f64,
        bar_index in 0u64..1000000,
    ) {
        let signal = Signal::Open { symbol: symbol.clone(), qty };
        let result = translate_signal(
            &signal,
            &policy,
            "acct",
            "strat",
            bar_index,
            last_price,
            tick_size,
            0.0, // current_position_qty irrelevant for Open
        );

        let order = result.expect("qty >= 1.0 should always produce Some");
        prop_assert_eq!(&order.symbol, &symbol, "symbol must be preserved");
        prop_assert_eq!(order.side, Side::Buy, "Open signal must produce Buy side");
        prop_assert_eq!(order.contracts, qty.floor() as u32, "contracts must equal floor(qty)");
        prop_assert!(order.contracts > 0, "contracts must be > 0 for qty >= 1.0");
    }

    /// Signal::Short produces an Order with Side::Sell, preserved symbol, contracts == floor(qty), contracts > 0
    #[test]
    fn prop_short_signal_preserves_intent(
        symbol in "[A-Z]{1,5}",
        qty in 1.0..10000.0f64,
        policy in arb_execution_policy(),
        last_price in 1.0..10000.0f64,
        tick_size in 0.01..10.0f64,
        bar_index in 0u64..1000000,
    ) {
        let signal = Signal::Short { symbol: symbol.clone(), qty };
        let result = translate_signal(
            &signal,
            &policy,
            "acct",
            "strat",
            bar_index,
            last_price,
            tick_size,
            0.0, // current_position_qty irrelevant for Short
        );

        let order = result.expect("qty >= 1.0 should always produce Some");
        prop_assert_eq!(&order.symbol, &symbol, "symbol must be preserved");
        prop_assert_eq!(order.side, Side::Sell, "Short signal must produce Sell side");
        prop_assert_eq!(order.contracts, qty.floor() as u32, "contracts must equal floor(qty)");
        prop_assert!(order.contracts > 0, "contracts must be > 0 for qty >= 1.0");
    }

    /// Signal::Close produces an Order with Side::Sell, preserved symbol, contracts == floor(current_position_qty), contracts > 0
    #[test]
    fn prop_close_signal_preserves_intent(
        symbol in "[A-Z]{1,5}",
        current_position_qty in 1.0..10000.0f64,
        policy in arb_execution_policy(),
        last_price in 1.0..10000.0f64,
        tick_size in 0.01..10.0f64,
        bar_index in 0u64..1000000,
    ) {
        let signal = Signal::Close { symbol: symbol.clone() };
        let result = translate_signal(
            &signal,
            &policy,
            "acct",
            "strat",
            bar_index,
            last_price,
            tick_size,
            current_position_qty,
        );

        let order = result.expect("current_position_qty >= 1.0 should always produce Some");
        prop_assert_eq!(&order.symbol, &symbol, "symbol must be preserved");
        prop_assert_eq!(order.side, Side::Sell, "Close signal must produce Sell side");
        prop_assert_eq!(order.contracts, current_position_qty.abs().floor() as u32, "contracts must equal floor(abs(current_position_qty))");
        prop_assert!(order.contracts > 0, "contracts must be > 0 for current_position_qty >= 1.0");
    }

    /// Signal::CloseQty produces an Order with Side::Sell, preserved symbol, contracts == floor(qty), contracts > 0
    #[test]
    fn prop_close_qty_signal_preserves_intent(
        symbol in "[A-Z]{1,5}",
        qty in 1.0..10000.0f64,
        policy in arb_execution_policy(),
        last_price in 1.0..10000.0f64,
        tick_size in 0.01..10.0f64,
        bar_index in 0u64..1000000,
    ) {
        let signal = Signal::CloseQty { symbol: symbol.clone(), qty };
        let result = translate_signal(
            &signal,
            &policy,
            "acct",
            "strat",
            bar_index,
            last_price,
            tick_size,
            0.0, // current_position_qty irrelevant for CloseQty
        );

        let order = result.expect("qty >= 1.0 should always produce Some");
        prop_assert_eq!(&order.symbol, &symbol, "symbol must be preserved");
        prop_assert_eq!(order.side, Side::Sell, "CloseQty signal must produce Sell side");
        prop_assert_eq!(order.contracts, qty.floor() as u32, "contracts must equal floor(qty)");
        prop_assert!(order.contracts > 0, "contracts must be > 0 for qty >= 1.0");
    }
}


// =============================================================================
// Property 5: Deduplication Guard Rejects Duplicates
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // Feature: broker-adapter, Property 5: Deduplication Guard Rejects Duplicates
    /// **Validates: Requirements 7.5**
    ///
    /// For any sequence of Order submissions, if an OrderId has been marked as submitted,
    /// attempting to submit an order with the same OrderId SHALL be identified as a duplicate.
    /// Conversely, an OrderId that has never been submitted SHALL not be identified as a duplicate.
    #[test]
    fn prop_dedup_guard_rejects_duplicates(
        ids in proptest::collection::vec("[a-z]{3,8}_[A-Z]{1,4}_[0-9]{1,5}", 1..20),
        dup_indices in proptest::collection::vec(0..20usize, 0..5),
    ) {
        let mut guard = DeduplicationGuard::new();

        // Submit each unique ID — first time should NOT be duplicate
        for id_str in &ids {
            let order_id = OrderId(id_str.clone());
            prop_assert!(!guard.is_duplicate(&order_id),
                "First submission of {} should not be duplicate", id_str);
            guard.mark_submitted(order_id);
        }

        // Re-submit some IDs — should all be duplicates
        for &idx in &dup_indices {
            if idx < ids.len() {
                let order_id = OrderId(ids[idx].clone());
                prop_assert!(guard.is_duplicate(&order_id),
                    "Re-submission of {} should be duplicate", ids[idx]);
            }
        }
    }
}


// =============================================================================
// Property 6: Session Gate Consistency
// =============================================================================

use chrono::{DateTime, Utc, NaiveDate, NaiveTime, TimeZone};
use chrono_tz::Tz;
use flux_cli::live::market_calendar::MarketCalendar;
use flux_cli::live::broker::execution::check_session_gate;

const TEST_CALENDAR_TOML: &str = r#"
[[session]]
exchange = "CME"
open = "09:30"
close = "16:00"
timezone = "US/Eastern"

[[session]]
exchange = "CBOT"
open = "09:30"
close = "16:00"
timezone = "US/Eastern"
"#;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // Feature: broker-adapter, Property 6: Session Gate Consistency
    /// **Validates: Requirements 8.1, 8.2**
    ///
    /// For any timestamp (hour, minute) on a known trading day, the session gate
    /// decision SHALL match the expected RTH window: Ok when within [open, close],
    /// Err(SessionClosed) when outside.
    #[test]
    fn prop_session_gate_consistency(
        hour in 0u32..24,
        minute in 0u32..60,
    ) {
        let calendar = MarketCalendar::from_toml(TEST_CALENDAR_TOML).unwrap();

        // Use a known trading day (Wednesday, 2024-03-13)
        let tz: Tz = "US/Eastern".parse().unwrap();
        let date = NaiveDate::from_ymd_opt(2024, 3, 13).unwrap();
        let time = NaiveTime::from_hms_opt(hour, minute, 0).unwrap();
        let local_dt = tz.from_local_datetime(&date.and_time(time)).earliest().unwrap();
        let utc_dt: DateTime<Utc> = local_dt.with_timezone(&Utc);

        let result = check_session_gate(&calendar, "CME", utc_dt);

        let open = NaiveTime::from_hms_opt(9, 30, 0).unwrap();
        let close = NaiveTime::from_hms_opt(16, 0, 0).unwrap();

        if time >= open && time <= close {
            prop_assert!(result.is_ok(), "Time {:02}:{:02} is within RTH but gate rejected", hour, minute);
        } else {
            prop_assert!(result.is_err(), "Time {:02}:{:02} is outside RTH but gate allowed", hour, minute);
        }
    }
}


// =============================================================================
// Property 7: Execution Policy Parsing Round-Trip
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // Feature: broker-adapter, Property 7: Execution Policy Parsing Round-Trip
    /// **Validates: Requirements 2.2**
    ///
    /// For any valid execution policy string from the set {"market", "aggressive_limit",
    /// "limit", "market_on_close", "limit_on_close", "stop", "stop_limit", "trailing_stop",
    /// "trailing_stop_pct", "adaptive"}, parsing the string SHALL produce the corresponding
    /// ExecutionPolicy variant, and unknown strings SHALL default to Market.
    #[test]
    fn prop_execution_policy_parsing_round_trip(
        policy_idx in 0..10usize,
        offset_ticks in 1..10i32,
    ) {
        let valid_policies = [
            "market", "aggressive_limit", "limit", "market_on_close",
            "limit_on_close", "stop", "stop_limit", "trailing_stop",
            "trailing_stop_pct", "adaptive",
        ];
        let policy_str = valid_policies[policy_idx];

        let parsed = parse_execution_policy(policy_str, Some(offset_ticks));

        // Verify correct variant is produced
        match policy_str {
            "market" => prop_assert_eq!(parsed, ExecutionPolicy::Market),
            "aggressive_limit" => prop_assert_eq!(parsed, ExecutionPolicy::AggressiveLimit { offset_ticks }),
            "limit" => {
                let is_limit = matches!(parsed, ExecutionPolicy::Limit { .. });
                prop_assert!(is_limit, "expected Limit variant");
            }
            "market_on_close" => prop_assert_eq!(parsed, ExecutionPolicy::MarketOnClose),
            "limit_on_close" => {
                let is_loc = matches!(parsed, ExecutionPolicy::LimitOnClose { .. });
                prop_assert!(is_loc, "expected LimitOnClose variant");
            }
            "stop" => {
                let is_stop = matches!(parsed, ExecutionPolicy::Stop { .. });
                prop_assert!(is_stop, "expected Stop variant");
            }
            "stop_limit" => {
                let is_sl = matches!(parsed, ExecutionPolicy::StopLimit { .. });
                prop_assert!(is_sl, "expected StopLimit variant");
            }
            "trailing_stop" => {
                let is_ts = matches!(parsed, ExecutionPolicy::TrailingStop { .. });
                prop_assert!(is_ts, "expected TrailingStop variant");
            }
            "trailing_stop_pct" => {
                let is_tsp = matches!(parsed, ExecutionPolicy::TrailingStopPct { .. });
                prop_assert!(is_tsp, "expected TrailingStopPct variant");
            }
            "adaptive" => {
                let is_adaptive = matches!(parsed, ExecutionPolicy::Adaptive { .. });
                prop_assert!(is_adaptive, "expected Adaptive variant");
            }
            _ => prop_assert!(false, "unexpected policy string"),
        }

        // Also verify unknown strings default to Market
        let unknown = parse_execution_policy("invalid_unknown_policy", None);
        prop_assert_eq!(unknown, ExecutionPolicy::Market);
    }
}
