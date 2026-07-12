//! Property-based tests for Position Tracking and Multi-Symbol Isolation.
//!
//! Feature: flux-stdlib-backtester
//!
//! This file contains property tests for:
//! - Property 17: Position Tracking Invariant
//! - Property 18: Multi-Symbol Isolation
//! - Property 19: Submission Order Determines Processing Order
//! - Property 20: Sell Exceeding Position Rejected
//!
//! **Validates: Requirements 8.1, 8.2, 8.3, 8.4, 8.5, 8.6, 9.1, 9.2, 9.3, 10.3**

use proptest::prelude::*;

use flux_cli::interpreter::{Interpreter, Value};
use flux_cli::module_resolver::resolve_modules;
use flux_compiler::lexer;
use flux_compiler::parser;
use flux_compiler::typeck;

// =============================================================================
// Helpers
// =============================================================================

/// Find the workspace root (directory containing the top-level Cargo.toml with [workspace]).
fn workspace_root() -> std::path::PathBuf {
    let mut dir = std::env::current_dir().unwrap();
    loop {
        let cargo_path = dir.join("Cargo.toml");
        if cargo_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&cargo_path) {
                if content.contains("[workspace]") {
                    return dir;
                }
            }
        }
        if !dir.pop() {
            panic!("could not find workspace root");
        }
    }
}

/// Compile Flux source through lex, parse, resolve_modules, typecheck.
fn compile_to_interpreter(source: &str) -> Interpreter {
    let root = workspace_root();
    let tokens = lexer::lex_with_spans(source).expect("lexer failed");
    let ast = parser::parse(tokens).expect("parser failed");
    let resolved = resolve_modules(ast, root.as_path()).expect("resolve failed");
    let typed = typeck::check(resolved).expect("typeck failed");
    Interpreter::new(&typed)
}

/// Create a BarContext for triggering on_bar.
fn test_bar() -> flux_runtime::BarContext {
    flux_runtime::BarContext {
        symbol: "TEST".to_string(),
        close: 100.0,
        open: 99.0,
        high: 101.0,
        low: 98.0,
        volume: 1000.0,
        in_position: false,
    }
}

/// Extract a float from interpreter state.
fn get_state_float(interp: &Interpreter, name: &str) -> f64 {
    match interp.state.get(name) {
        Some(Value::Float(f)) => *f,
        Some(Value::Int(i)) => *i as f64,
        other => panic!("Expected Float/Int in '{}', got {:?}", name, other),
    }
}

// =============================================================================
// Property 17: Position Tracking Invariant
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(30))]

    /// **Validates: Requirements 8.1, 8.2**
    ///
    /// Property 17: Position Tracking Invariant
    ///
    /// Buy qty Q1 at price P1, then verify position qty == Q1 and avg_entry == P1.
    /// Buy another qty Q2 at price P2, verify new_avg == (P1*Q1 + P2*Q2) / (Q1+Q2).
    #[test]
    fn prop_position_tracking_invariant(
        q1 in 1.0..500.0f64,
        p1 in 10.0..500.0f64,
        q2 in 1.0..500.0f64,
        p2 in 10.0..500.0f64,
    ) {
        let expected_total_qty = q1 + q2;
        let expected_avg = (p1 * q1 + p2 * q2) / expected_total_qty;

        // We track position directly via update_position by reading the fills and
        // computing position state in Flux, storing results in state vars.
        let source = format!(
            r#"from engine::types import {{Order, Fill, OrderSide, OrderType, TimeInForce, BacktestEngine, PositionState}}
from engine::fast import {{FastEngine, update_position}}
from market::l1 import {{Bar}}

strategy PositionTrackingTest {{
    state {{
        pos_qty = 0.0
        pos_avg = 0.0
        pos_qty_after_first = 0.0
        pos_avg_after_first = 0.0
    }}
    on bar {{
        engine = FastEngine.new()

        # Buy Q1 at close=P1
        buy1 = Order {{ id = 1, symbol = "TEST", side = OrderSide.Buy, order_type = OrderType.Market, qty = {q1:.10}, tif = TimeInForce.GTC }}
        engine = engine.submit_order(buy1)
        bar1 = Bar {{ symbol = "TEST", open = {p1_open:.10}, high = {p1_high:.10}, low = {p1_low:.10}, close = {p1:.10}, volume = 1000.0, timestamp = 0.0 }}
        engine = engine.process_bar(bar1)

        # Check position after first buy via the positions HashMap directly
        if engine.positions.contains_key("TEST") {{
            pos = engine.positions.get("TEST")
            pos_qty_after_first = pos.qty
            pos_avg_after_first = pos.avg_entry_price
        }}

        # Buy Q2 at close=P2
        buy2 = Order {{ id = 2, symbol = "TEST", side = OrderSide.Buy, order_type = OrderType.Market, qty = {q2:.10}, tif = TimeInForce.GTC }}
        engine = engine.submit_order(buy2)
        bar2 = Bar {{ symbol = "TEST", open = {p2_open:.10}, high = {p2_high:.10}, low = {p2_low:.10}, close = {p2:.10}, volume = 1000.0, timestamp = 1.0 }}
        engine = engine.process_bar(bar2)

        # Check position after second buy
        if engine.positions.contains_key("TEST") {{
            pos = engine.positions.get("TEST")
            pos_qty = pos.qty
            pos_avg = pos.avg_entry_price
        }}
    }}
}}
"#,
            q1 = q1, p1 = p1, q2 = q2, p2 = p2,
            p1_open = p1 - 1.0, p1_high = p1 + 1.0, p1_low = p1 - 2.0,
            p2_open = p2 - 1.0, p2_high = p2 + 1.0, p2_low = p2 - 2.0,
        );

        let mut interp = compile_to_interpreter(&source);
        interp.on_bar(&test_bar());

        // After first buy: qty == Q1, avg == P1
        let actual_qty_first = get_state_float(&interp, "pos_qty_after_first");
        let actual_avg_first = get_state_float(&interp, "pos_avg_after_first");

        prop_assert!(
            (actual_qty_first - q1).abs() < 1e-6,
            "After first buy: qty {} != expected {}", actual_qty_first, q1
        );
        prop_assert!(
            (actual_avg_first - p1).abs() < 1e-6,
            "After first buy: avg_entry {} != expected {}", actual_avg_first, p1
        );

        // After second buy: qty == Q1+Q2, avg == VWAP
        let actual_qty = get_state_float(&interp, "pos_qty");
        let actual_avg = get_state_float(&interp, "pos_avg");

        prop_assert!(
            (actual_qty - expected_total_qty).abs() < 1e-6,
            "After second buy: qty {} != expected {}", actual_qty, expected_total_qty
        );
        prop_assert!(
            (actual_avg - expected_avg).abs() < 1e-6,
            "After second buy: avg_entry {} != expected VWAP {}", actual_avg, expected_avg
        );
    }
}

// =============================================================================
// Property 18: Multi-Symbol Isolation
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(30))]

    /// **Validates: Requirements 9.1, 9.2, 9.3**
    ///
    /// Property 18: Multi-Symbol Isolation
    ///
    /// Submit orders for "AAPL" and "MSFT". Process bar for "AAPL" only.
    /// Verify only AAPL orders fill, MSFT orders remain pending (no fills for MSFT).
    #[test]
    fn prop_multi_symbol_isolation(
        aapl_qty in 1.0..500.0f64,
        msft_qty in 1.0..500.0f64,
        aapl_close in 50.0..300.0f64,
    ) {
        let source = format!(
            r#"from engine::types import {{Order, Fill, OrderSide, OrderType, TimeInForce, BacktestEngine, PositionState}}
from engine::fast import {{FastEngine, update_position}}
from market::l1 import {{Bar}}

strategy MultiSymbolTest {{
    state {{
        fill_count = 0
        aapl_fill_qty = 0.0
        aapl_fill_price = 0.0
        has_aapl_pos = 0
        has_msft_pos = 0
    }}
    on bar {{
        engine = FastEngine.new()

        # Submit buy for AAPL
        aapl_order = Order {{ id = 1, symbol = "AAPL", side = OrderSide.Buy, order_type = OrderType.Market, qty = {aapl_qty:.10}, tif = TimeInForce.GTC }}
        engine = engine.submit_order(aapl_order)

        # Submit buy for MSFT
        msft_order = Order {{ id = 2, symbol = "MSFT", side = OrderSide.Buy, order_type = OrderType.Market, qty = {msft_qty:.10}, tif = TimeInForce.GTC }}
        engine = engine.submit_order(msft_order)

        # Process bar only for AAPL
        aapl_bar = Bar {{ symbol = "AAPL", open = {aapl_open:.10}, high = {aapl_high:.10}, low = {aapl_low:.10}, close = {aapl_close:.10}, volume = 5000.0, timestamp = 0.0 }}
        engine = engine.process_bar(aapl_bar)

        fills = engine.get_fills()
        fill_count = fills.len()
        if fill_count > 0 {{
            f = fills[0]
            aapl_fill_qty = f.qty
            aapl_fill_price = f.price
        }}

        # Check positions directly via HashMap
        if engine.positions.contains_key("AAPL") {{
            has_aapl_pos = 1
        }}
        if engine.positions.contains_key("MSFT") {{
            has_msft_pos = 1
        }}
    }}
}}
"#,
            aapl_qty = aapl_qty,
            msft_qty = msft_qty,
            aapl_close = aapl_close,
            aapl_open = aapl_close - 1.0,
            aapl_high = aapl_close + 1.0,
            aapl_low = aapl_close - 2.0,
        );

        let mut interp = compile_to_interpreter(&source);
        interp.on_bar(&test_bar());

        // Only 1 fill (AAPL), not 2
        let fill_count = get_state_float(&interp, "fill_count") as i64;
        prop_assert_eq!(fill_count, 1,
            "Expected exactly 1 fill (AAPL only), got {}", fill_count);

        // The fill should be for AAPL qty and price
        let actual_aapl_qty = get_state_float(&interp, "aapl_fill_qty");
        let actual_aapl_price = get_state_float(&interp, "aapl_fill_price");

        prop_assert!(
            (actual_aapl_qty - aapl_qty).abs() < 1e-6,
            "AAPL fill qty {} != expected {}", actual_aapl_qty, aapl_qty
        );
        prop_assert!(
            (actual_aapl_price - aapl_close).abs() < 1e-6,
            "AAPL fill price {} != expected close {}", actual_aapl_price, aapl_close
        );

        // Only AAPL position should exist
        let has_aapl = get_state_float(&interp, "has_aapl_pos") as i64;
        let has_msft = get_state_float(&interp, "has_msft_pos") as i64;
        prop_assert_eq!(has_aapl, 1,
            "Expected AAPL position to exist");
        prop_assert_eq!(has_msft, 0,
            "Expected MSFT position NOT to exist (bar not processed)");
    }
}

// =============================================================================
// Property 19: Submission Order Determines Processing Order
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(30))]

    /// **Validates: Requirements 10.3**
    ///
    /// Property 19: Submission Order Determines Processing Order
    ///
    /// Submit orders A, B, C in that order. Process bar. Verify fills come back
    /// in order A, B, C (submission order = processing order).
    #[test]
    fn prop_submission_order_determines_processing_order(
        qty_a in 10.0..100.0f64,
        qty_b in 10.0..100.0f64,
        qty_c in 10.0..100.0f64,
        close_price in 50.0..300.0f64,
    ) {
        let source = format!(
            r#"from engine::types import {{Order, Fill, OrderSide, OrderType, TimeInForce, BacktestEngine, PositionState}}
from engine::fast import {{FastEngine, update_position}}
from market::l1 import {{Bar}}

strategy OrderDeterminismTest {{
    state {{
        fill_count = 0
        fill_id_0 = 0
        fill_id_1 = 0
        fill_id_2 = 0
        fill_qty_0 = 0.0
        fill_qty_1 = 0.0
        fill_qty_2 = 0.0
    }}
    on bar {{
        engine = FastEngine.new()

        # Submit orders A (id=10), B (id=20), C (id=30) in that order
        order_a = Order {{ id = 10, symbol = "TEST", side = OrderSide.Buy, order_type = OrderType.Market, qty = {qty_a:.10}, tif = TimeInForce.GTC }}
        engine = engine.submit_order(order_a)

        order_b = Order {{ id = 20, symbol = "TEST", side = OrderSide.Buy, order_type = OrderType.Market, qty = {qty_b:.10}, tif = TimeInForce.GTC }}
        engine = engine.submit_order(order_b)

        order_c = Order {{ id = 30, symbol = "TEST", side = OrderSide.Buy, order_type = OrderType.Market, qty = {qty_c:.10}, tif = TimeInForce.GTC }}
        engine = engine.submit_order(order_c)

        # Process a single bar
        bar_data = Bar {{ symbol = "TEST", open = {open:.10}, high = {high:.10}, low = {low:.10}, close = {close:.10}, volume = 10000.0, timestamp = 0.0 }}
        engine = engine.process_bar(bar_data)

        fills = engine.get_fills()
        fill_count = fills.len()
        if fill_count >= 3 {{
            fill_id_0 = fills[0].order_id
            fill_id_1 = fills[1].order_id
            fill_id_2 = fills[2].order_id
            fill_qty_0 = fills[0].qty
            fill_qty_1 = fills[1].qty
            fill_qty_2 = fills[2].qty
        }}
    }}
}}
"#,
            qty_a = qty_a, qty_b = qty_b, qty_c = qty_c,
            close = close_price,
            open = close_price - 1.0,
            high = close_price + 1.0,
            low = close_price - 2.0,
        );

        let mut interp = compile_to_interpreter(&source);
        interp.on_bar(&test_bar());

        let fill_count = get_state_float(&interp, "fill_count") as i64;
        prop_assert_eq!(fill_count, 3,
            "Expected 3 fills, got {}", fill_count);

        // Verify fills are in submission order: A=10, B=20, C=30
        let id_0 = get_state_float(&interp, "fill_id_0") as i64;
        let id_1 = get_state_float(&interp, "fill_id_1") as i64;
        let id_2 = get_state_float(&interp, "fill_id_2") as i64;

        prop_assert_eq!(id_0, 10, "First fill should be order A (id=10), got id={}", id_0);
        prop_assert_eq!(id_1, 20, "Second fill should be order B (id=20), got id={}", id_1);
        prop_assert_eq!(id_2, 30, "Third fill should be order C (id=30), got id={}", id_2);

        // Verify quantities match submission order
        let actual_qty_0 = get_state_float(&interp, "fill_qty_0");
        let actual_qty_1 = get_state_float(&interp, "fill_qty_1");
        let actual_qty_2 = get_state_float(&interp, "fill_qty_2");

        prop_assert!(
            (actual_qty_0 - qty_a).abs() < 1e-6,
            "Fill 0 qty {} != order A qty {}", actual_qty_0, qty_a
        );
        prop_assert!(
            (actual_qty_1 - qty_b).abs() < 1e-6,
            "Fill 1 qty {} != order B qty {}", actual_qty_1, qty_b
        );
        prop_assert!(
            (actual_qty_2 - qty_c).abs() < 1e-6,
            "Fill 2 qty {} != order C qty {}", actual_qty_2, qty_c
        );
    }
}

// =============================================================================
// Property 20: Sell Exceeding Position Rejected
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(30))]

    /// **Validates: Requirements 8.6**
    ///
    /// Property 20: Sell Exceeding Position Rejected
    ///
    /// Open a position of qty Q (where Q < 100). Submit a sell order for qty 100
    /// (exceeds position). Verify no sell fill is produced (order rejected/discarded).
    #[test]
    fn prop_sell_exceeding_position_rejected(
        buy_qty in 10.0..90.0f64,
        buy_price in 50.0..300.0f64,
        sell_price in 50.0..300.0f64,
    ) {
        // sell_qty is always 100.0, which exceeds buy_qty (10..90)
        let sell_qty = 100.0;

        let source = format!(
            r#"from engine::types import {{Order, Fill, OrderSide, OrderType, TimeInForce, BacktestEngine, PositionState}}
from engine::fast import {{FastEngine, update_position}}
from market::l1 import {{Bar}}

strategy SellExceedsPositionTest {{
    state {{
        buy_fill_count = 0
        sell_fill_count = 0
        final_pos_qty = 0.0
    }}
    on bar {{
        engine = FastEngine.new()

        # First: buy to establish position
        buy_order = Order {{ id = 1, symbol = "TEST", side = OrderSide.Buy, order_type = OrderType.Market, qty = {buy_qty:.10}, tif = TimeInForce.GTC }}
        engine = engine.submit_order(buy_order)
        bar1 = Bar {{ symbol = "TEST", open = {buy_open:.10}, high = {buy_high:.10}, low = {buy_low:.10}, close = {buy_price:.10}, volume = 1000.0, timestamp = 0.0 }}
        engine = engine.process_bar(bar1)

        buy_fills = engine.get_fills()
        buy_fill_count = buy_fills.len()

        # Now try to sell more than we hold
        sell_order = Order {{ id = 2, symbol = "TEST", side = OrderSide.Sell, order_type = OrderType.Market, qty = {sell_qty:.10}, tif = TimeInForce.GTC }}
        engine = engine.submit_order(sell_order)
        bar2 = Bar {{ symbol = "TEST", open = {sell_open:.10}, high = {sell_high:.10}, low = {sell_low:.10}, close = {sell_price:.10}, volume = 1000.0, timestamp = 1.0 }}
        engine = engine.process_bar(bar2)

        sell_fills = engine.get_fills()
        sell_fill_count = sell_fills.len()

        # Check final position — should still be the original buy qty (sell was rejected)
        if engine.positions.contains_key("TEST") {{
            pos = engine.positions.get("TEST")
            final_pos_qty = pos.qty
        }}
    }}
}}
"#,
            buy_qty = buy_qty,
            buy_price = buy_price,
            buy_open = buy_price - 1.0,
            buy_high = buy_price + 1.0,
            buy_low = buy_price - 2.0,
            sell_qty = sell_qty,
            sell_price = sell_price,
            sell_open = sell_price - 1.0,
            sell_high = sell_price + 1.0,
            sell_low = sell_price - 2.0,
        );

        let mut interp = compile_to_interpreter(&source);
        interp.on_bar(&test_bar());

        // Buy should have filled (1 fill)
        let buy_fills = get_state_float(&interp, "buy_fill_count") as i64;
        prop_assert_eq!(buy_fills, 1,
            "Expected 1 buy fill, got {}", buy_fills);

        // Sell should have been rejected (0 fills) because sell_qty > position qty
        let sell_fills = get_state_float(&interp, "sell_fill_count") as i64;
        prop_assert_eq!(sell_fills, 0,
            "Expected 0 sell fills (sell qty {} exceeds position {}), got {}",
            sell_qty, buy_qty, sell_fills);

        // Position should remain unchanged (original buy qty)
        let final_pos_qty = get_state_float(&interp, "final_pos_qty");
        prop_assert!(
            (final_pos_qty - buy_qty).abs() < 1e-6,
            "Position qty should remain {} after rejected sell, but got {}",
            buy_qty, final_pos_qty
        );
    }
}
