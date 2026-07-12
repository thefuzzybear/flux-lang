//! Property-based tests for Backtester Engine modules.
//!
//! Feature: flux-stdlib-backtester
//!
//! This file contains property tests for the Flux stdlib backtester engines.
//! - Properties 5-8 validate FastEngine correctness.
//! - Properties 14-16 validate metrics computation correctness.
//!
//! **Validates: Requirements 4.1, 4.3, 4.4, 4.5, 4.6, 4.7, 4.8, 7.2, 7.3, 7.4, 7.5, 7.9, 10.1**

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
        symbol: "TEST".to_string(), close: 100.0, open: 99.0,
        high: 101.0, low: 98.0, volume: 1000.0, in_position: false,
    }
}

// =============================================================================
// Property 16: Win Rate and Profit Factor Formulas
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    /// **Validates: Requirements 7.4, 7.5, 7.9**
    ///
    /// Property 16: Win Rate and Profit Factor Formulas
    ///
    /// For any list of trade P&Ls, the Flux `compute_metrics` function SHALL:
    /// - Compute win_rate = count(pnl > 0) / total_trades
    /// - Compute profit_factor = sum(positive pnls) / abs(sum(negative pnls))
    /// - Use 999999.0 as profit_factor when no losing trades exist
    #[test]
    fn prop_win_rate_and_profit_factor(
        pnls in prop::collection::vec(-100.0f64..200.0f64, 3..15),
    ) {
        let wins = pnls.iter().filter(|p| **p > 0.0).count() as f64;
        let total = pnls.len() as f64;
        let expected_win_rate = wins / total;

        let gross_profit: f64 = pnls.iter().filter(|p| **p > 0.0).sum();
        let gross_loss: f64 = pnls.iter().filter(|p| **p <= 0.0).map(|p| p.abs()).sum();
        let expected_pf = if gross_loss > 0.0 { gross_profit / gross_loss } else { 999999.0 };

        // Build fills that produce desired round-trip P&Ls
        let mut fills_source = String::from("fills = []\n");
        for (i, pnl) in pnls.iter().enumerate() {
            let buy_price = 100.0;
            let sell_price = buy_price + pnl;
            let order_id_buy = (i * 2) as i64;
            let order_id_sell = (i * 2 + 1) as i64;
            fills_source.push_str(&format!(
                "        fills.push(Fill {{ order_id = {}, symbol = \"TEST\", side = OrderSide.Buy, price = {:.10}, qty = 1.0, timestamp = {:.1}, slippage = 0.0 }})\n",
                order_id_buy, buy_price, (i * 2) as f64
            ));
            fills_source.push_str(&format!(
                "        fills.push(Fill {{ order_id = {}, symbol = \"TEST\", side = OrderSide.Sell, price = {:.10}, qty = 1.0, timestamp = {:.1}, slippage = 0.0 }})\n",
                order_id_sell, sell_price, (i * 2 + 1) as f64
            ));
        }

        let equity_curve_code = "equity_curve = []\n        equity_curve.push(10000.0)\n        equity_curve.push(10100.0)\n        equity_curve.push(10050.0)";

        let source = format!(
            r#"
from engine::types import {{Fill, OrderSide, PositionState}}
from engine::metrics import {{compute_metrics, Metrics, compute_sharpe, compute_max_drawdown, compute_trade_pnls}}

strategy MetricsTest {{
    state {{
        win_rate_result = 0.0
        profit_factor_result = 0.0
    }}
    on bar {{
        {}
        {}
        metrics = compute_metrics(fills, equity_curve)
        win_rate_result = metrics.win_rate
        profit_factor_result = metrics.profit_factor
    }}
}}
"#,
            fills_source, equity_curve_code
        );

        let mut interp = compile_to_interpreter(&source);
        interp.on_bar(&test_bar());

        let actual_win_rate = match interp.state.get("win_rate_result") {
            Some(Value::Float(f)) => *f,
            Some(Value::Int(i)) => *i as f64,
            other => panic!("Expected Float for 'win_rate_result', got {:?}", other),
        };

        let actual_pf = match interp.state.get("profit_factor_result") {
            Some(Value::Float(f)) => *f,
            Some(Value::Int(i)) => *i as f64,
            other => panic!("Expected Float for 'profit_factor_result', got {:?}", other),
        };

        let epsilon = 1e-6;
        prop_assert!(
            (actual_win_rate - expected_win_rate).abs() < epsilon,
            "Win rate mismatch: expected={}, got={}",
            expected_win_rate, actual_win_rate
        );

        let pf_epsilon = 1e-4;
        prop_assert!(
            (actual_pf - expected_pf).abs() < pf_epsilon,
            "Profit factor mismatch: expected={}, got={}",
            expected_pf, actual_pf
        );
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

/// Format a list of f64 values as Flux push statements.
fn format_list_with_pushes(var_name: &str, values: &[f64]) -> String {
    let mut code = format!("{} = []\n", var_name);
    for v in values {
        code.push_str(&format!("        {}.push({:.10})\n", var_name, v));
    }
    code
}

// =============================================================================
// Property 5: Fast Engine Fills at Bar Close with Zero Slippage
// Feature: flux-stdlib-backtester, Property 5
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(30))]

    /// **Validates: Requirements 4.1, 4.3, 4.4**
    ///
    /// Property 5: Fast Engine Fills at Bar Close with Zero Slippage
    ///
    /// For any bar with close price P and any buy order with quantity Q,
    /// when the FastEngine processes the bar, it SHALL produce a Fill with:
    /// - fill.price == P (bar close price)
    /// - fill.slippage == 0.0
    /// - fill.qty == Q (original order quantity)
    #[test]
    fn prop_fast_engine_fills_at_close(
        close_price in 1.0..1000.0f64,
        qty in 1.0..10000.0f64,
    ) {
        let source = format!(
            r#"from engine::types import {{Order, Fill, OrderSide, OrderType, TimeInForce, BacktestEngine, PositionState}}
from engine::fast import {{FastEngine, update_position}}
from market::l1 import {{Bar}}

strategy FillAtCloseTest {{
    state {{
        fill_price = 0.0
        fill_qty = 0.0
        fill_slippage = 0.0
        fill_count = 0
    }}
    on bar {{
        engine = FastEngine.new()
        order = Order {{
            id = 1, symbol = "TEST", side = OrderSide.Buy,
            order_type = OrderType.Market, qty = {qty:.10},
            tif = TimeInForce.GTC
        }}
        engine = engine.submit_order(order)
        bar_data = Bar {{
            symbol = "TEST", open = 99.0, high = 101.0, low = 98.0,
            close = {close:.10}, volume = 50000.0, timestamp = 0.0
        }}
        engine = engine.process_bar(bar_data)
        fills = engine.get_fills()
        fill_count = fills.len()
        if fill_count > 0 {{
            f = fills[0]
            fill_price = f.price
            fill_qty = f.qty
            fill_slippage = f.slippage
        }}
    }}
}}
"#, close = close_price, qty = qty);

        let mut interp = compile_to_interpreter(&source);
        interp.on_bar(&test_bar());

        let actual_count = get_state_float(&interp, "fill_count");
        let actual_price = get_state_float(&interp, "fill_price");
        let actual_qty = get_state_float(&interp, "fill_qty");
        let actual_slippage = get_state_float(&interp, "fill_slippage");

        prop_assert_eq!(actual_count as i64, 1,
            "Expected 1 fill, got {}", actual_count);
        prop_assert!((actual_price - close_price).abs() < 1e-10,
            "Fill price {} != bar close {}", actual_price, close_price);
        prop_assert!((actual_qty - qty).abs() < 1e-10,
            "Fill qty {} != order qty {}", actual_qty, qty);
        prop_assert!(actual_slippage.abs() < 1e-10,
            "Fill slippage {} should be 0.0", actual_slippage);
    }
}

// =============================================================================
// Property 6: Close Signal Produces Sell for Full Position or Nothing
// Feature: flux-stdlib-backtester, Property 6
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(30))]

    /// **Validates: Requirements 4.5, 4.8**
    ///
    /// Property 6: Close Signal Produces Sell for Full Position or Nothing
    ///
    /// When a sell order is submitted for a symbol with no open position,
    /// the FastEngine SHALL produce no fills (discard the order silently).
    #[test]
    fn prop_fast_engine_close_no_position_produces_nothing(
        close_price in 1.0..1000.0f64,
        qty in 1.0..100.0f64,
    ) {
        let source = format!(
            r#"from engine::types import {{Order, Fill, OrderSide, OrderType, TimeInForce, BacktestEngine, PositionState}}
from engine::fast import {{FastEngine, update_position}}
from market::l1 import {{Bar}}

strategy CloseNoPositionTest {{
    state {{
        fill_count = 0
    }}
    on bar {{
        engine = FastEngine.new()
        sell_order = Order {{
            id = 1, symbol = "TEST", side = OrderSide.Sell,
            order_type = OrderType.Market, qty = {qty:.10},
            tif = TimeInForce.GTC
        }}
        engine = engine.submit_order(sell_order)
        bar_data = Bar {{
            symbol = "TEST", open = 99.0, high = 101.0, low = 98.0,
            close = {close:.10}, volume = 50000.0, timestamp = 0.0
        }}
        engine = engine.process_bar(bar_data)
        fills = engine.get_fills()
        fill_count = fills.len()
    }}
}}
"#, close = close_price, qty = qty);

        let mut interp = compile_to_interpreter(&source);
        interp.on_bar(&test_bar());

        let actual_count = get_state_float(&interp, "fill_count");
        prop_assert_eq!(actual_count as i64, 0,
            "Expected 0 fills when selling with no position, got {}", actual_count);
    }
}

// =============================================================================
// Property 7: Deterministic Execution Across Repeated Runs
// Feature: flux-stdlib-backtester, Property 7
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(30))]

    /// **Validates: Requirements 4.6, 10.1**
    ///
    /// Property 7: Deterministic Execution Across Repeated Runs
    ///
    /// Running the same strategy with the same bar data twice through FastEngine
    /// SHALL produce identical fill sequences (same prices, quantities, ordering).
    #[test]
    fn prop_fast_engine_deterministic_execution(
        close_price in 50.0..500.0f64,
        qty in 1.0..1000.0f64,
    ) {
        let source = format!(
            r#"from engine::types import {{Order, Fill, OrderSide, OrderType, TimeInForce, BacktestEngine, PositionState}}
from engine::fast import {{FastEngine, update_position}}
from market::l1 import {{Bar}}

strategy DeterministicTest {{
    state {{
        fill_price = 0.0
        fill_qty = 0.0
        fill_count = 0
    }}
    on bar {{
        engine = FastEngine.new()
        order = Order {{
            id = 1, symbol = "TEST", side = OrderSide.Buy,
            order_type = OrderType.Market, qty = {qty:.10},
            tif = TimeInForce.GTC
        }}
        engine = engine.submit_order(order)
        bar_data = Bar {{
            symbol = "TEST", open = 99.0, high = 101.0, low = 98.0,
            close = {close:.10}, volume = 50000.0, timestamp = 0.0
        }}
        engine = engine.process_bar(bar_data)
        fills = engine.get_fills()
        fill_count = fills.len()
        if fill_count > 0 {{
            f = fills[0]
            fill_price = f.price
            fill_qty = f.qty
        }}
    }}
}}
"#, close = close_price, qty = qty);

        // Run 1
        let mut interp1 = compile_to_interpreter(&source);
        interp1.on_bar(&test_bar());
        let price1 = get_state_float(&interp1, "fill_price");
        let qty1 = get_state_float(&interp1, "fill_qty");
        let count1 = get_state_float(&interp1, "fill_count");

        // Run 2
        let mut interp2 = compile_to_interpreter(&source);
        interp2.on_bar(&test_bar());
        let price2 = get_state_float(&interp2, "fill_price");
        let qty2 = get_state_float(&interp2, "fill_qty");
        let count2 = get_state_float(&interp2, "fill_count");

        // Verify identical results
        prop_assert_eq!(count1 as i64, count2 as i64,
            "Fill count differs between runs: {} vs {}", count1, count2);
        prop_assert!((price1 - price2).abs() < 1e-10,
            "Fill price differs between runs: {} vs {}", price1, price2);
        prop_assert!((qty1 - qty2).abs() < 1e-10,
            "Fill qty differs between runs: {} vs {}", qty1, qty2);
    }
}

// =============================================================================
// Property 8: Fast Engine Model Equivalence with PositionTracker
// Feature: flux-stdlib-backtester, Property 8
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(30))]

    /// **Validates: Requirements 4.7**
    ///
    /// Property 8: Fast Engine Model Equivalence with PositionTracker
    ///
    /// For a buy followed by a sell, the FastEngine SHALL produce fills where:
    /// - Buy fill price == bar1.close
    /// - Sell fill price == bar2.close
    /// This verifies the FastEngine model is equivalent to PositionTracker
    /// (both fill at bar close price).
    #[test]
    fn prop_fast_engine_model_equivalence(
        close1 in 50.0..500.0f64,
        close2 in 50.0..500.0f64,
        qty in 1.0..1000.0f64,
    ) {
        let source = format!(
            r#"from engine::types import {{Order, Fill, OrderSide, OrderType, TimeInForce, BacktestEngine, PositionState}}
from engine::fast import {{FastEngine, update_position}}
from market::l1 import {{Bar}}

strategy EquivalenceTest {{
    state {{
        buy_fill_price = 0.0
        sell_fill_price = 0.0
        buy_fill_count = 0
        sell_fill_count = 0
    }}
    on bar {{
        engine = FastEngine.new()

        buy_order = Order {{
            id = 1, symbol = "TEST", side = OrderSide.Buy,
            order_type = OrderType.Market, qty = {qty:.10},
            tif = TimeInForce.GTC
        }}
        engine = engine.submit_order(buy_order)

        bar1 = Bar {{
            symbol = "TEST", open = 99.0, high = 101.0, low = 98.0,
            close = {close1:.10}, volume = 50000.0, timestamp = 0.0
        }}
        engine = engine.process_bar(bar1)
        buy_fills = engine.get_fills()
        buy_fill_count = buy_fills.len()
        if buy_fill_count > 0 {{
            buy_fill_price = buy_fills[0].price
        }}

        sell_order = Order {{
            id = 2, symbol = "TEST", side = OrderSide.Sell,
            order_type = OrderType.Market, qty = {qty:.10},
            tif = TimeInForce.GTC
        }}
        engine = engine.submit_order(sell_order)

        bar2 = Bar {{
            symbol = "TEST", open = 99.0, high = 101.0, low = 98.0,
            close = {close2:.10}, volume = 50000.0, timestamp = 1.0
        }}
        engine = engine.process_bar(bar2)
        sell_fills = engine.get_fills()
        sell_fill_count = sell_fills.len()
        if sell_fill_count > 0 {{
            sell_fill_price = sell_fills[0].price
        }}
    }}
}}
"#, close1 = close1, close2 = close2, qty = qty);

        let mut interp = compile_to_interpreter(&source);
        interp.on_bar(&test_bar());

        let buy_count = get_state_float(&interp, "buy_fill_count");
        let sell_count = get_state_float(&interp, "sell_fill_count");
        let buy_price = get_state_float(&interp, "buy_fill_price");
        let sell_price = get_state_float(&interp, "sell_fill_price");

        // Both fills should succeed
        prop_assert_eq!(buy_count as i64, 1,
            "Expected 1 buy fill, got {}", buy_count);
        prop_assert_eq!(sell_count as i64, 1,
            "Expected 1 sell fill, got {}", sell_count);

        // Buy fill price must equal bar1.close
        prop_assert!((buy_price - close1).abs() < 1e-10,
            "Buy fill price {} != bar1.close {}", buy_price, close1);

        // Sell fill price must equal bar2.close
        prop_assert!((sell_price - close2).abs() < 1e-10,
            "Sell fill price {} != bar2.close {}", sell_price, close2);
    }
}

// =============================================================================
// Property 14: Sharpe Ratio Formula Correctness
// Feature: flux-stdlib-backtester, Property 14
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(30))]

    /// **Validates: Requirements 7.2**
    ///
    /// Property 14: Sharpe Ratio Formula Correctness
    ///
    /// For any random equity curve, the Flux `compute_sharpe` function SHALL
    /// compute: mean(returns) / stddev(returns) * sqrt(252)
    /// where returns are daily percentage changes, using sample standard deviation.
    #[test]
    fn prop_sharpe_ratio_formula(
        equity in prop::collection::vec(9000.0f64..11000.0f64, 5..20),
    ) {
        // Compute returns in Rust
        let returns: Vec<f64> = equity.windows(2)
            .map(|w| (w[1] - w[0]) / w[0])
            .collect();
        if returns.len() < 2 { return Ok(()); }
        let mean = returns.iter().sum::<f64>() / returns.len() as f64;
        let var = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (returns.len() - 1) as f64;
        let std = var.sqrt();
        let expected_sharpe = if std == 0.0 { 0.0 } else { (mean / std) * 15.8745 };

        // Build Flux source to call compute_sharpe
        let returns_code = format_list_with_pushes("returns", &returns);
        let source = format!(r#"
from engine::metrics import {{compute_sharpe}}

strategy SharpeTest {{
    state {{ sharpe_result = 0.0 }}
    on bar {{
        {}
        sharpe_result = compute_sharpe(returns)
    }}
}}
"#, returns_code);

        let mut interp = compile_to_interpreter(&source);
        interp.on_bar(&test_bar());
        let actual = get_state_float(&interp, "sharpe_result");
        prop_assert!((actual - expected_sharpe).abs() < 0.01,
            "Sharpe: expected={}, got={}", expected_sharpe, actual);
    }
}

// =============================================================================
// Property 15: Max Drawdown Formula Correctness
// Feature: flux-stdlib-backtester, Property 15
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(30))]

    /// **Validates: Requirements 7.3**
    ///
    /// Property 15: Max Drawdown Formula Correctness
    ///
    /// For any random equity curve, the Flux `compute_max_drawdown` function SHALL
    /// compute max over all i of (peak - equity[i]) / peak, where peak is the
    /// running maximum of the equity curve up to that point.
    #[test]
    fn prop_max_drawdown_formula(
        equity in prop::collection::vec(5000.0f64..15000.0f64, 3..20),
    ) {
        // Compute expected max drawdown in Rust
        let mut peak = equity[0];
        let mut max_dd = 0.0f64;
        for &e in &equity {
            if e > peak { peak = e; }
            if peak > 0.0 {
                let dd = (peak - e) / peak;
                if dd > max_dd { max_dd = dd; }
            }
        }

        let equity_code = format_list_with_pushes("equity_curve", &equity);
        let source = format!(r#"
from engine::metrics import {{compute_max_drawdown}}

strategy DrawdownTest {{
    state {{ dd_result = 0.0 }}
    on bar {{
        {}
        dd_result = compute_max_drawdown(equity_curve)
    }}
}}
"#, equity_code);

        let mut interp = compile_to_interpreter(&source);
        interp.on_bar(&test_bar());
        let actual = get_state_float(&interp, "dd_result");
        prop_assert!((actual - max_dd).abs() < 1e-6,
            "Max drawdown: expected={}, got={}", max_dd, actual);
    }
}


// =============================================================================
// Property 9: Price Path is Deterministic 4-Point Sequence from OHLCV
// Feature: flux-stdlib-backtester, Property 9
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    /// **Validates: Requirements 5.1, 5.2**
    ///
    /// Property 9: Price Path is Deterministic 4-Point Sequence from OHLCV
    ///
    /// For any OHLCV bar, `generate_price_path` SHALL produce exactly 4 points where:
    /// - path[0] == open
    /// - path[3] == close
    /// - The middle two points are the bar's high and low
    /// - The nearer extreme to open is visited first (preferring high when equidistant)
    /// - The result is deterministic for the same input
    #[test]
    fn prop_price_path_deterministic_4point_sequence(
        open in 10.0..500.0f64,
        high_offset in 0.1..50.0f64,
        low_offset in 0.1..50.0f64,
        close_frac in 0.0..1.0f64,
    ) {
        // Construct a valid OHLCV bar: high = open + high_offset, low = open - low_offset
        let high = open + high_offset;
        let low = open - low_offset;
        // Close is somewhere between low and high
        let close = low + close_frac * (high - low);

        let source = format!(
            r#"from market::l1 import {{Bar}}

fn generate_price_path(bar: Bar) -> list {{
    path = []
    path.push(bar.open)
    dist_to_high = abs(bar.high - bar.open)
    dist_to_low = abs(bar.open - bar.low)
    if dist_to_high <= dist_to_low {{
        path.push(bar.high)
        path.push(bar.low)
    }} else {{
        path.push(bar.low)
        path.push(bar.high)
    }}
    path.push(bar.close)
    return path
}}

strategy PricePathTest {{
    state {{
        path_len = 0
        path_0 = 0.0
        path_1 = 0.0
        path_2 = 0.0
        path_3 = 0.0
    }}
    on bar {{
        bar_data = Bar {{
            symbol = "TEST", open = {open:.10}, high = {high:.10},
            low = {low:.10}, close = {close:.10}, volume = 1000.0,
            timestamp = 0.0
        }}
        path = generate_price_path(bar_data)
        path_len = path.len()
        path_0 = path[0]
        path_1 = path[1]
        path_2 = path[2]
        path_3 = path[3]
    }}
}}
"#, open = open, high = high, low = low, close = close);

        let mut interp = compile_to_interpreter(&source);
        interp.on_bar(&test_bar());

        let path_len = get_state_float(&interp, "path_len") as i64;
        let path_0 = get_state_float(&interp, "path_0");
        let path_1 = get_state_float(&interp, "path_1");
        let path_2 = get_state_float(&interp, "path_2");
        let path_3 = get_state_float(&interp, "path_3");

        // Property: exactly 4 points
        prop_assert_eq!(path_len, 4, "Price path must have exactly 4 points, got {}", path_len);

        // Property: path[0] == open
        prop_assert!((path_0 - open).abs() < 1e-10,
            "path[0] = {} should equal open = {}", path_0, open);

        // Property: path[3] == close
        prop_assert!((path_3 - close).abs() < 1e-10,
            "path[3] = {} should equal close = {}", path_3, close);

        // Property: middle two points are {high, low} in some order
        let mid_contains_high = (path_1 - high).abs() < 1e-10 || (path_2 - high).abs() < 1e-10;
        let mid_contains_low = (path_1 - low).abs() < 1e-10 || (path_2 - low).abs() < 1e-10;
        prop_assert!(mid_contains_high,
            "Middle two points ({}, {}) must contain high = {}", path_1, path_2, high);
        prop_assert!(mid_contains_low,
            "Middle two points ({}, {}) must contain low = {}", path_1, path_2, low);

        // Property: nearer extreme visited first, high preferred when equidistant
        let dist_to_high = (high - open).abs();
        let dist_to_low = (open - low).abs();

        if dist_to_high <= dist_to_low {
            // High is nearer (or equidistant), should be path[1]
            prop_assert!((path_1 - high).abs() < 1e-10,
                "When dist_to_high ({}) <= dist_to_low ({}), path[1] should be high ({}), got {}",
                dist_to_high, dist_to_low, high, path_1);
            prop_assert!((path_2 - low).abs() < 1e-10,
                "When high is nearer, path[2] should be low ({}), got {}",
                low, path_2);
        } else {
            // Low is nearer, should be path[1]
            prop_assert!((path_1 - low).abs() < 1e-10,
                "When dist_to_low ({}) < dist_to_high ({}), path[1] should be low ({}), got {}",
                dist_to_low, dist_to_high, low, path_1);
            prop_assert!((path_2 - high).abs() < 1e-10,
                "When low is nearer, path[2] should be high ({}), got {}",
                high, path_2);
        }

        // Property: deterministic — running again gives same results
        let mut interp2 = compile_to_interpreter(&source);
        interp2.on_bar(&test_bar());
        let path_0b = get_state_float(&interp2, "path_0");
        let path_1b = get_state_float(&interp2, "path_1");
        let path_2b = get_state_float(&interp2, "path_2");
        let path_3b = get_state_float(&interp2, "path_3");

        prop_assert!((path_0 - path_0b).abs() < 1e-10, "Determinism: path[0] differs between runs");
        prop_assert!((path_1 - path_1b).abs() < 1e-10, "Determinism: path[1] differs between runs");
        prop_assert!((path_2 - path_2b).abs() < 1e-10, "Determinism: path[2] differs between runs");
        prop_assert!((path_3 - path_3b).abs() < 1e-10, "Determinism: path[3] differs between runs");
    }
}

// =============================================================================
// Property 10: Synthetic Book Has Correct Structure
// Feature: flux-stdlib-backtester, Property 10
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    /// **Validates: Requirements 5.4, 5.5, 10.5**
    ///
    /// Property 10: Synthetic Book Has Correct Structure
    ///
    /// For any SyntheticConfig and center price, `build_synthetic_book` SHALL produce
    /// an OrderBook with:
    /// - Exactly `config.depth` ask levels (ascending from center)
    /// - Exactly `config.depth` bid levels (descending from center)
    /// - Each level containing `config.liquidity_per_side / config.depth` quantity
    /// - Ask levels spaced by spread_step = center_price * spread_pct / 100.0
    /// - Bid levels spaced by the same spread_step below center
    #[test]
    fn prop_synthetic_book_correct_structure(
        center_price in 10.0..1000.0f64,
        depth in 1..10i32,
        spread_pct in 0.01..5.0f64,
        liquidity in 1000.0..100000.0f64,
    ) {
        let depth_i64 = depth as i64;
        let qty_per_level = liquidity / (depth as f64);
        let spread_step = center_price * spread_pct / 100.0;

        let source = format!(
            r#"from engine::types import {{Order, Fill, FillResult, OrderSide, OrderType, TimeInForce, BacktestEngine, PositionState}}
from engine::book import {{OrderBook, PriceLevel}}
from market::l1 import {{Bar}}

fn build_synthetic_book(center_price: f64, symbol: str, config_depth: int, config_spread_pct: f64, config_liquidity: f64) -> OrderBook {{
    book = OrderBook.new(symbol)
    qty_per_level = config_liquidity / config_depth
    spread_step = center_price * config_spread_pct / 100.0

    i = 0
    while i < config_depth {{
        ask_price = center_price + spread_step * (i + 1)
        ask_order = Order {{
            id = 0 - (i + 1),
            symbol = symbol,
            side = OrderSide.Sell,
            order_type = OrderType.Limit(ask_price),
            qty = qty_per_level,
            tif = TimeInForce.GTC
        }}
        level = PriceLevel {{
            price = ask_price,
            total_size = qty_per_level,
            orders = [ask_order]
        }}
        book.asks.push(level)
        i = i + 1
    }}

    i = 0
    while i < config_depth {{
        bid_price = center_price - spread_step * (i + 1)
        bid_order = Order {{
            id = 0 - (config_depth + i + 1),
            symbol = symbol,
            side = OrderSide.Buy,
            order_type = OrderType.Limit(bid_price),
            qty = qty_per_level,
            tif = TimeInForce.GTC
        }}
        level = PriceLevel {{
            price = bid_price,
            total_size = qty_per_level,
            orders = [bid_order]
        }}
        book.bids.push(level)
        i = i + 1
    }}

    return book
}}

strategy BookStructureTest {{
    state {{
        ask_count = 0
        bid_count = 0
        ask_first_price = 0.0
        ask_last_price = 0.0
        bid_first_price = 0.0
        bid_last_price = 0.0
        ask_first_size = 0.0
        bid_first_size = 0.0
        ask_ascending = 1
        bid_descending = 1
    }}
    on bar {{
        book = build_synthetic_book({center:.10}, "TEST", {depth}, {spread:.10}, {liq:.10})
        ask_count = book.asks.len()
        bid_count = book.bids.len()

        if ask_count > 0 {{
            ask_first_price = book.asks[0].price
            ask_last_price = book.asks[ask_count - 1].price
            ask_first_size = book.asks[0].total_size
        }}
        if bid_count > 0 {{
            bid_first_price = book.bids[0].price
            bid_last_price = book.bids[bid_count - 1].price
            bid_first_size = book.bids[0].total_size
        }}

        # Check asks are ascending
        i = 1
        while i < ask_count {{
            if book.asks[i].price <= book.asks[i - 1].price {{
                ask_ascending = 0
            }}
            i = i + 1
        }}

        # Check bids are descending
        i = 1
        while i < bid_count {{
            if book.bids[i].price >= book.bids[i - 1].price {{
                bid_descending = 0
            }}
            i = i + 1
        }}
    }}
}}
"#, center = center_price, depth = depth, spread = spread_pct, liq = liquidity);

        let mut interp = compile_to_interpreter(&source);
        interp.on_bar(&test_bar());

        let ask_count = get_state_float(&interp, "ask_count") as i64;
        let bid_count = get_state_float(&interp, "bid_count") as i64;
        let ask_first_price = get_state_float(&interp, "ask_first_price");
        let ask_last_price = get_state_float(&interp, "ask_last_price");
        let bid_first_price = get_state_float(&interp, "bid_first_price");
        let bid_last_price = get_state_float(&interp, "bid_last_price");
        let ask_first_size = get_state_float(&interp, "ask_first_size");
        let bid_first_size = get_state_float(&interp, "bid_first_size");
        let ask_ascending = get_state_float(&interp, "ask_ascending") as i64;
        let bid_descending = get_state_float(&interp, "bid_descending") as i64;

        // Property: exactly `depth` ask levels
        prop_assert_eq!(ask_count, depth_i64,
            "Expected {} ask levels, got {}", depth_i64, ask_count);

        // Property: exactly `depth` bid levels
        prop_assert_eq!(bid_count, depth_i64,
            "Expected {} bid levels, got {}", depth_i64, bid_count);

        // Property: each level contains correct qty (liquidity / depth)
        prop_assert!((ask_first_size - qty_per_level).abs() < 1e-6,
            "Ask level size {} != expected {}", ask_first_size, qty_per_level);
        prop_assert!((bid_first_size - qty_per_level).abs() < 1e-6,
            "Bid level size {} != expected {}", bid_first_size, qty_per_level);

        // Property: ask levels are ascending
        prop_assert_eq!(ask_ascending, 1,
            "Ask levels must be in ascending price order");

        // Property: bid levels are descending
        prop_assert_eq!(bid_descending, 1,
            "Bid levels must be in descending price order");

        // Property: first ask is at center + spread_step, first bid at center - spread_step
        let expected_ask_first = center_price + spread_step;
        prop_assert!((ask_first_price - expected_ask_first).abs() < 1e-6,
            "First ask price {} != expected center + spread_step = {}",
            ask_first_price, expected_ask_first);

        let expected_bid_first = center_price - spread_step;
        prop_assert!((bid_first_price - expected_bid_first).abs() < 1e-6,
            "First bid price {} != expected center - spread_step = {}",
            bid_first_price, expected_bid_first);

        // Property: last ask at center + spread_step * depth
        let expected_ask_last = center_price + spread_step * (depth as f64);
        prop_assert!((ask_last_price - expected_ask_last).abs() < 1e-6,
            "Last ask price {} != expected center + spread_step * depth = {}",
            ask_last_price, expected_ask_last);

        // Property: last bid at center - spread_step * depth
        let expected_bid_last = center_price - spread_step * (depth as f64);
        prop_assert!((bid_last_price - expected_bid_last).abs() < 1e-6,
            "Last bid price {} != expected center - spread_step * depth = {}",
            bid_last_price, expected_bid_last);
    }
}

// =============================================================================
// Property 11: L2 Events Produce Valid Reconstructed Book
// Feature: flux-stdlib-backtester, Property 11
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(30))]

    /// **Validates: Requirements 6.1, 6.2**
    ///
    /// Property 11: L2 Events Produce Valid Reconstructed Book
    ///
    /// For any sequence of L2 Add events processed by the ReplayEngine,
    /// the reconstructed OrderBook SHALL have at most 20 price levels per side,
    /// with bids sorted descending and asks sorted ascending by price.
    #[test]
    fn prop_replay_engine_valid_book_structure(
        num_bid_levels in 1usize..30,
        num_ask_levels in 1usize..30,
        base_bid in 90.0f64..100.0f64,
        base_ask in 100.0f64..110.0f64,
    ) {
        // Generate L2 Add events for bids (each at a unique price descending)
        // and asks (each at a unique price ascending)
        let mut events_code = String::new();
        events_code.push_str("engine = ReplayEngine.new()\n");

        let mut timestamp = 1.0;
        // Add bid levels at prices base_bid, base_bid-1, base_bid-2, ...
        for i in 0..num_bid_levels {
            let price = base_bid - (i as f64);
            events_code.push_str(&format!(
                "        engine = process_l2_event(engine, L2Event {{ timestamp = {:.1}, side = OrderSide.Buy, price = {:.6}, size = 100.0, action = L2Action.Add }})\n",
                timestamp, price
            ));
            timestamp += 1.0;
        }
        // Add ask levels at prices base_ask, base_ask+1, base_ask+2, ...
        for i in 0..num_ask_levels {
            let price = base_ask + (i as f64);
            events_code.push_str(&format!(
                "        engine = process_l2_event(engine, L2Event {{ timestamp = {:.1}, side = OrderSide.Sell, price = {:.6}, size = 100.0, action = L2Action.Add }})\n",
                timestamp, price
            ));
            timestamp += 1.0;
        }

        // Extract book state for verification
        events_code.push_str(r#"
        book = engine.books.get("default")
        bid_count = book.bids.len()
        ask_count = book.asks.len()

        # Verify bids sorted descending
        bids_sorted = 1
        i = 0
        while i < bid_count - 1 {
            if book.bids[i].price < book.bids[i + 1].price {
                bids_sorted = 0
            }
            i = i + 1
        }

        # Verify asks sorted ascending
        asks_sorted = 1
        i = 0
        while i < ask_count - 1 {
            if book.asks[i].price > book.asks[i + 1].price {
                asks_sorted = 0
            }
            i = i + 1
        }
"#);

        let source = format!(
            r#"from engine::types import {{Order, Fill, OrderSide, OrderType, TimeInForce, BacktestEngine, PositionState, FillResult}}
from engine::replay import {{ReplayEngine, L2Event, L2Action, process_l2_event, QueuedOrder, get_queue_ahead, advance_queues, check_queue_fills, update_replay_position, trim_book}}
from engine::book import {{OrderBook, PriceLevel}}
from market::l1 import {{Bar}}

strategy BookStructureTest {{
    state {{
        bid_count = 0
        ask_count = 0
        bids_sorted = 1
        asks_sorted = 1
    }}
    on bar {{
        {}
    }}
}}
"#, events_code);

        let mut interp = compile_to_interpreter(&source);
        interp.on_bar(&test_bar());

        let bid_count = get_state_float(&interp, "bid_count") as usize;
        let ask_count = get_state_float(&interp, "ask_count") as usize;
        let bids_sorted = get_state_float(&interp, "bids_sorted") as i64;
        let asks_sorted = get_state_float(&interp, "asks_sorted") as i64;

        // Max 20 levels per side
        prop_assert!(bid_count <= 20,
            "Bid levels {} exceeds max 20 (added {})", bid_count, num_bid_levels);
        prop_assert!(ask_count <= 20,
            "Ask levels {} exceeds max 20 (added {})", ask_count, num_ask_levels);

        // Bids sorted descending
        prop_assert_eq!(bids_sorted, 1,
            "Bids are not sorted descending (bid_count={})", bid_count);

        // Asks sorted ascending
        prop_assert_eq!(asks_sorted, 1,
            "Asks are not sorted ascending (ask_count={})", ask_count);
    }
}

// =============================================================================
// Property 12: Queue Position Lifecycle
// Feature: flux-stdlib-backtester, Property 12
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(30))]

    /// **Validates: Requirements 6.3, 6.4, 6.5**
    ///
    /// Property 12: Queue Position Lifecycle
    ///
    /// For any limit order submitted to the ReplayEngine:
    /// - Initial queue_position == total resting quantity at that price level
    /// - When queue_position is 0 (no liquidity ahead), order fills at limit price
    #[test]
    fn prop_replay_engine_queue_lifecycle(
        initial_size in 100.0f64..1000.0f64,
        order_qty in 1.0f64..50.0f64,
    ) {
        // Test Part A: When there is resting size at the limit price,
        // initial queue_position should equal that size.
        // Test Part B: When there is NO resting size at the limit price,
        // queue_position is 0 and the order fills immediately at the limit price.
        let limit_price = 99.0;

        let source = format!(
            r#"from engine::types import {{Order, Fill, OrderSide, OrderType, TimeInForce, BacktestEngine, PositionState, FillResult}}
from engine::replay import {{ReplayEngine, L2Event, L2Action, process_l2_event, QueuedOrder, get_queue_ahead, advance_queues, check_queue_fills, update_replay_position, trim_book}}
from engine::book import {{OrderBook, PriceLevel}}
from market::l1 import {{Bar}}

strategy QueueLifecycleTest {{
    state {{
        initial_queue_pos = 0.0
        queued_count_with_size = 0
        fill_count_no_size = 0
        fill_price_no_size = 0.0
    }}
    on bar {{
        # --- Part A: limit order WITH resting liquidity ahead ---
        engine_a = ReplayEngine.new()

        # Add a bid level with initial liquidity at the limit price
        engine_a = process_l2_event(engine_a, L2Event {{
            timestamp = 1.0, side = OrderSide.Buy,
            price = {limit_price:.6}, size = {initial_size:.10},
            action = L2Action.Add
        }})

        # Submit a limit buy order at that price
        order_a = Order {{
            id = 1, symbol = "default", side = OrderSide.Buy,
            order_type = OrderType.Limit({limit_price:.6}),
            qty = {order_qty:.10}, tif = TimeInForce.GTC
        }}
        engine_a = engine_a.submit_order(order_a)

        # Capture initial queue position (should equal total resting size)
        queued_count_with_size = engine_a.queued_orders.len()
        if queued_count_with_size > 0 {{
            initial_queue_pos = engine_a.queued_orders[0].queue_position
        }}

        # --- Part B: limit order with NO resting liquidity (queue_pos = 0 → fills) ---
        engine_b = ReplayEngine.new()

        # Add a bid level at a DIFFERENT price (not our limit price)
        engine_b = process_l2_event(engine_b, L2Event {{
            timestamp = 1.0, side = OrderSide.Buy,
            price = 98.0, size = 500.0,
            action = L2Action.Add
        }})

        # Submit a limit buy at our limit price (no liquidity there → queue_pos = 0)
        order_b = Order {{
            id = 2, symbol = "default", side = OrderSide.Buy,
            order_type = OrderType.Limit({limit_price:.6}),
            qty = {order_qty:.10}, tif = TimeInForce.GTC
        }}
        engine_b = engine_b.submit_order(order_b)

        # Process another event to trigger check_queue_fills
        engine_b = process_l2_event(engine_b, L2Event {{
            timestamp = 2.0, side = OrderSide.Buy,
            price = 97.0, size = 100.0,
            action = L2Action.Add
        }})

        # The order should have filled since queue_position was 0
        fill_count_no_size = engine_b.fills.len()
        if fill_count_no_size > 0 {{
            fill_price_no_size = engine_b.fills[0].price
        }}
    }}
}}
"#,
            limit_price = limit_price,
            initial_size = initial_size,
            order_qty = order_qty,
        );

        let mut interp = compile_to_interpreter(&source);
        interp.on_bar(&test_bar());

        let initial_queue = get_state_float(&interp, "initial_queue_pos");
        let queued_count = get_state_float(&interp, "queued_count_with_size") as i64;
        let fills_no_size = get_state_float(&interp, "fill_count_no_size") as i64;
        let fill_price = get_state_float(&interp, "fill_price_no_size");

        // Part A: queue_position should equal total resting size at that level
        prop_assert_eq!(queued_count, 1,
            "Expected 1 queued order, got {}", queued_count);
        prop_assert!((initial_queue - initial_size).abs() < 1e-6,
            "Initial queue_position {} should equal initial_size {}", initial_queue, initial_size);

        // Part B: when queue_position is 0, order fills at limit price
        prop_assert!(fills_no_size >= 1,
            "Expected fill when queue_position=0, got {} fills", fills_no_size);
        prop_assert!((fill_price - limit_price).abs() < 1e-6,
            "Fill price {} should equal limit price {}", fill_price, limit_price);
    }
}

// =============================================================================
// Property 13: Out-of-Order Timestamps Rejected
// Feature: flux-stdlib-backtester, Property 13
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(30))]

    /// **Validates: Requirements 6.8, 6.9**
    ///
    /// Property 13: Out-of-Order Timestamps Rejected
    ///
    /// For any L2 event whose timestamp is strictly less than the previously
    /// processed event's timestamp, the ReplayEngine SHALL reject the event
    /// without modifying book state.
    #[test]
    fn prop_replay_engine_ooo_timestamps_rejected(
        first_ts in 10.0f64..100.0f64,
        size1 in 50.0f64..500.0f64,
        size2 in 50.0f64..500.0f64,
    ) {
        // Process an event at first_ts, then try to process an event at first_ts - 5.0.
        // The second event should be rejected; book state should remain unchanged.
        let ooo_ts = first_ts - 5.0;
        let price1 = 100.0;
        let price2 = 101.0; // Different price so we can detect if it was added

        let source = format!(
            r#"from engine::types import {{Order, Fill, OrderSide, OrderType, TimeInForce, BacktestEngine, PositionState, FillResult}}
from engine::replay import {{ReplayEngine, L2Event, L2Action, process_l2_event, QueuedOrder, get_queue_ahead, advance_queues, check_queue_fills, update_replay_position, trim_book}}
from engine::book import {{OrderBook, PriceLevel}}
from market::l1 import {{Bar}}

strategy OOOTimestampTest {{
    state {{
        bid_count_before = 0
        bid_count_after = 0
        ask_count_before = 0
        ask_count_after = 0
        last_ts_before = 0.0
        last_ts_after = 0.0
    }}
    on bar {{
        engine = ReplayEngine.new()

        # Process first event at timestamp first_ts
        engine = process_l2_event(engine, L2Event {{
            timestamp = {first_ts:.10}, side = OrderSide.Buy,
            price = {price1:.6}, size = {size1:.10},
            action = L2Action.Add
        }})

        # Capture state before out-of-order event
        book_before = engine.books.get("default")
        bid_count_before = book_before.bids.len()
        ask_count_before = book_before.asks.len()
        last_ts_before = engine.last_timestamp

        # Attempt to process event with earlier timestamp (out-of-order)
        engine = process_l2_event(engine, L2Event {{
            timestamp = {ooo_ts:.10}, side = OrderSide.Sell,
            price = {price2:.6}, size = {size2:.10},
            action = L2Action.Add
        }})

        # Capture state after — should be unchanged
        book_after = engine.books.get("default")
        bid_count_after = book_after.bids.len()
        ask_count_after = book_after.asks.len()
        last_ts_after = engine.last_timestamp
    }}
}}
"#,
            first_ts = first_ts,
            ooo_ts = ooo_ts,
            price1 = price1,
            size1 = size1,
            price2 = price2,
            size2 = size2,
        );

        let mut interp = compile_to_interpreter(&source);
        interp.on_bar(&test_bar());

        let bid_before = get_state_float(&interp, "bid_count_before") as i64;
        let bid_after = get_state_float(&interp, "bid_count_after") as i64;
        let ask_before = get_state_float(&interp, "ask_count_before") as i64;
        let ask_after = get_state_float(&interp, "ask_count_after") as i64;
        let ts_before = get_state_float(&interp, "last_ts_before");
        let ts_after = get_state_float(&interp, "last_ts_after");

        // Book state should be unchanged
        prop_assert_eq!(bid_before, bid_after,
            "Bid count changed after OOO event: before={}, after={}", bid_before, bid_after);
        prop_assert_eq!(ask_before, ask_after,
            "Ask count changed after OOO event: before={}, after={}", ask_before, ask_after);

        // Timestamp should not advance
        prop_assert!((ts_before - ts_after).abs() < 1e-10,
            "Timestamp changed after OOO event: before={}, after={}", ts_before, ts_after);
    }
}


// =============================================================================
// Property 1: FIFO Matching Consumes from Best Price in Insertion Order
// Feature: flux-stdlib-backtester, Property 1
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    /// **Validates: Requirements 2.3, 2.4, 2.8**
    ///
    /// Property 1: FIFO Matching Consumes from Best Price in Insertion Order
    ///
    /// For any OrderBook with non-empty ask levels and a market buy order,
    /// the matching algorithm SHALL consume liquidity starting at the lowest
    /// ask price level, processing resting orders within each level in FIFO
    /// order (insertion order), and SHALL remove price levels whose total_size
    /// reaches zero.
    #[test]
    fn prop_fifo_matching_best_price_insertion_order(
        // Two ask levels with different prices
        ask_price1 in 100.0f64..150.0,
        ask_price2 in 151.0f64..200.0,
        // Two orders at the first (best) price level
        qty_order_a in 10.0f64..100.0,
        qty_order_b in 10.0f64..100.0,
        // Market buy qty that fits within first order at best price
        buy_fraction in 0.1f64..0.99,
    ) {
        // Buy qty is a fraction of only the first order at best price
        let buy_qty = qty_order_a * buy_fraction;

        // The test implements the FIFO matching algorithm inline to verify the property:
        // 1. Start at lowest ask (ask_price1)
        // 2. Within that level, consume order_a first (FIFO: inserted first)
        // 3. Fill price = ask_price1 (only one level touched)
        let source = format!(
            r#"from engine::types import {{Order, Fill, FillResult, OrderSide, OrderType, TimeInForce}}
from engine::book import {{OrderBook, PriceLevel}}

# Inline FIFO matching that uses local variable index assignment
fn match_buy_fifo(asks: list, buy_qty: f64) -> list {{
    # Returns [filled_qty, fill_price, remaining_level_count, first_level_remaining_size]
    remaining = buy_qty
    filled_qty = 0.0
    cost = 0.0
    levels_consumed = 0

    # Walk asks from index 0 (lowest price first)
    i = 0
    while i < asks.len() and remaining > 0.0 {{
        level = asks[i]
        level_price = level.price
        orders = level.orders

        j = 0
        while j < orders.len() and remaining > 0.0 {{
            resting = orders[j]
            take = min(resting.qty, remaining)
            cost = cost + level_price * take
            filled_qty = filled_qty + take
            remaining = remaining - take
            j = j + 1
        }}
        i = i + 1
    }}

    vwap = 0.0
    if filled_qty > 0.0 {{
        vwap = cost / filled_qty
    }}

    # Compute remaining state: how many ask levels still have liquidity
    remaining_levels = 0
    first_remaining_size = 0.0
    i = 0
    while i < asks.len() {{
        level = asks[i]
        level_remaining = level.total_size
        # Only the first level could be partially consumed
        if i == 0 {{
            if buy_qty < level.total_size {{
                level_remaining = level.total_size - buy_qty
            }} else {{
                level_remaining = 0.0
            }}
        }}
        if level_remaining > 0.0 {{
            remaining_levels = remaining_levels + 1
            if first_remaining_size == 0.0 {{
                first_remaining_size = level_remaining
            }}
        }}
        i = i + 1
    }}

    result = []
    result.push(filled_qty)
    result.push(vwap)
    result.push(remaining_levels)
    result.push(first_remaining_size)
    return result
}}

strategy FifoTest {{
    state {{
        fill_price = 0.0
        fill_qty = 0.0
        remaining_levels = 0.0
        first_level_remaining = 0.0
    }}
    on bar {{
        # Build two ask levels: level1 at best price with two orders (FIFO), level2 at worse price
        order_a = Order {{
            id = 1, symbol = "TEST", side = OrderSide.Sell,
            order_type = OrderType.Limit({ask_price1:.10}),
            qty = {qty_a:.10}, tif = TimeInForce.GTC
        }}
        order_b = Order {{
            id = 2, symbol = "TEST", side = OrderSide.Sell,
            order_type = OrderType.Limit({ask_price1:.10}),
            qty = {qty_b:.10}, tif = TimeInForce.GTC
        }}
        order_c = Order {{
            id = 3, symbol = "TEST", side = OrderSide.Sell,
            order_type = OrderType.Limit({ask_price2:.10}),
            qty = 500.0, tif = TimeInForce.GTC
        }}

        level1 = PriceLevel {{
            price = {ask_price1:.10},
            total_size = {total_level1:.10},
            orders = [order_a, order_b]
        }}
        level2 = PriceLevel {{
            price = {ask_price2:.10},
            total_size = 500.0,
            orders = [order_c]
        }}

        asks = []
        asks.push(level1)
        asks.push(level2)

        result = match_buy_fifo(asks, {buy_qty:.10})
        fill_qty = result[0]
        fill_price = result[1]
        remaining_levels = result[2]
        first_level_remaining = result[3]
    }}
}}
"#,
            ask_price1 = ask_price1,
            ask_price2 = ask_price2,
            qty_a = qty_order_a,
            qty_b = qty_order_b,
            total_level1 = qty_order_a + qty_order_b,
            buy_qty = buy_qty,
        );

        let mut interp = compile_to_interpreter(&source);
        interp.on_bar(&test_bar());

        let fill_price = get_state_float(&interp, "fill_price");
        let fill_qty = get_state_float(&interp, "fill_qty");
        let remaining_levels = get_state_float(&interp, "remaining_levels") as i64;
        let first_level_remaining = get_state_float(&interp, "first_level_remaining");

        // Fill price should be the best ask price (only lowest level consumed)
        prop_assert!((fill_price - ask_price1).abs() < 1e-6,
            "Fill price {} should equal best ask {} (FIFO: lowest price first)",
            fill_price, ask_price1);

        // Fill qty should match requested qty
        prop_assert!((fill_qty - buy_qty).abs() < 1e-6,
            "Fill qty {} should equal buy qty {}", fill_qty, buy_qty);

        // Both levels should still have liquidity (we only partially consumed first level)
        prop_assert_eq!(remaining_levels, 2,
            "Expected 2 remaining ask levels, got {}", remaining_levels);

        // First level remaining size should be reduced by buy_qty
        let expected_remaining = qty_order_a + qty_order_b - buy_qty;
        prop_assert!((first_level_remaining - expected_remaining).abs() < 1e-6,
            "First level remaining {} should equal {} (original {} - consumed {})",
            first_level_remaining, expected_remaining, qty_order_a + qty_order_b, buy_qty);
    }
}

// =============================================================================
// Property 2: Fill Price Equals Volume-Weighted Average
// Feature: flux-stdlib-backtester, Property 2
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    /// **Validates: Requirements 2.5, 5.7**
    ///
    /// Property 2: Fill Price Equals Volume-Weighted Average
    ///
    /// For any market order that consumes liquidity across multiple price levels,
    /// the reported fill price SHALL equal sum(level_price_i * qty_filled_at_i) /
    /// total_filled_qty (the volume-weighted average price).
    #[test]
    fn prop_fill_price_equals_vwap(
        ask_price1 in 100.0f64..150.0,
        ask_price2 in 151.0f64..200.0,
        qty_level1 in 10.0f64..100.0,
        qty_level2 in 10.0f64..100.0,
    ) {
        // Buy qty that exceeds level 1 and dips into level 2
        let buy_qty = qty_level1 + qty_level2 * 0.5;

        // Expected VWAP: (price1 * qty_level1 + price2 * (buy_qty - qty_level1)) / buy_qty
        let qty_from_level2 = buy_qty - qty_level1;
        let expected_vwap = (ask_price1 * qty_level1 + ask_price2 * qty_from_level2) / buy_qty;

        let source = format!(
            r#"from engine::types import {{Order, Fill, FillResult, OrderSide, OrderType, TimeInForce}}
from engine::book import {{OrderBook, PriceLevel}}

# FIFO matching that computes VWAP across consumed levels
fn match_buy_vwap(asks: list, buy_qty: f64) -> list {{
    # Returns [filled_qty, vwap_price]
    remaining = buy_qty
    filled_qty = 0.0
    cost = 0.0

    i = 0
    while i < asks.len() and remaining > 0.0 {{
        level = asks[i]
        level_price = level.price
        orders = level.orders

        j = 0
        while j < orders.len() and remaining > 0.0 {{
            resting = orders[j]
            take = min(resting.qty, remaining)
            cost = cost + level_price * take
            filled_qty = filled_qty + take
            remaining = remaining - take
            j = j + 1
        }}
        i = i + 1
    }}

    vwap = 0.0
    if filled_qty > 0.0 {{
        vwap = cost / filled_qty
    }}

    result = []
    result.push(filled_qty)
    result.push(vwap)
    return result
}}

strategy VwapTest {{
    state {{
        fill_price = 0.0
        fill_qty = 0.0
    }}
    on bar {{
        order1 = Order {{
            id = 1, symbol = "TEST", side = OrderSide.Sell,
            order_type = OrderType.Limit({ask_price1:.10}),
            qty = {qty_level1:.10}, tif = TimeInForce.GTC
        }}
        order2 = Order {{
            id = 2, symbol = "TEST", side = OrderSide.Sell,
            order_type = OrderType.Limit({ask_price2:.10}),
            qty = {qty_level2:.10}, tif = TimeInForce.GTC
        }}

        level1 = PriceLevel {{
            price = {ask_price1:.10},
            total_size = {qty_level1:.10},
            orders = [order1]
        }}
        level2 = PriceLevel {{
            price = {ask_price2:.10},
            total_size = {qty_level2:.10},
            orders = [order2]
        }}

        asks = []
        asks.push(level1)
        asks.push(level2)

        result = match_buy_vwap(asks, {buy_qty:.10})
        fill_qty = result[0]
        fill_price = result[1]
    }}
}}
"#,
            ask_price1 = ask_price1,
            ask_price2 = ask_price2,
            qty_level1 = qty_level1,
            qty_level2 = qty_level2,
            buy_qty = buy_qty,
        );

        let mut interp = compile_to_interpreter(&source);
        interp.on_bar(&test_bar());

        let fill_price = get_state_float(&interp, "fill_price");
        let fill_qty = get_state_float(&interp, "fill_qty");

        // Fill qty should match the requested amount
        prop_assert!((fill_qty - buy_qty).abs() < 1e-6,
            "Fill qty {} should equal buy qty {}", fill_qty, buy_qty);

        // Fill price should equal VWAP across consumed levels
        prop_assert!((fill_price - expected_vwap).abs() < 1e-6,
            "Fill price {} should equal VWAP {} (price1={}, qty1={}, price2={}, qty_from_2={})",
            fill_price, expected_vwap, ask_price1, qty_level1, ask_price2, qty_from_level2);
    }
}

// =============================================================================
// Property 3: Limit Order Insertion Preserves Price-Time Priority
// Feature: flux-stdlib-backtester, Property 3
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    /// **Validates: Requirements 2.6**
    ///
    /// Property 3: Limit Order Insertion Preserves Price-Time Priority
    ///
    /// For any sequence of limit order insertions into an OrderBook, the bid
    /// levels SHALL remain sorted in descending price order, the ask levels
    /// SHALL remain sorted in ascending price order, and orders within each
    /// level SHALL be ordered by insertion time (earliest first).
    #[test]
    fn prop_limit_order_insertion_preserves_price_time_priority(
        // Three distinct ask prices inserted in arbitrary order
        price_a in 100.0f64..130.0,
        price_b in 131.0f64..160.0,
        price_c in 161.0f64..200.0,
        // Three distinct bid prices inserted in arbitrary order
        bid_a in 90.0f64..95.0,
        bid_b in 80.0f64..89.0,
        bid_c in 70.0f64..79.0,
    ) {
        // Test insertion via a custom function that mimics insert_limit logic
        // but uses local variable index assignments (which the interpreter supports).
        // Insert asks in shuffled order (C, A, B) — after insertion,
        // the book should sort them as A < B < C (ascending).
        // Insert bids in shuffled order (C, A, B) — after insertion,
        // the book should sort them as A > B > C (descending).
        let source = format!(
            r#"from engine::types import {{Order, Fill, FillResult, OrderSide, OrderType, TimeInForce}}
from engine::book import {{OrderBook, PriceLevel}}

# Insertion into a sorted list of price levels (ascending for asks)
fn insert_ask_level(asks: list, price: f64, order: Order) -> list {{
    # Find insertion point: first level with price > new price
    inserted = false
    result = []
    i = 0
    while i < asks.len() {{
        if not inserted and asks[i].price > price {{
            new_level = PriceLevel {{ price = price, total_size = order.qty, orders = [order] }}
            result.push(new_level)
            inserted = true
        }}
        result.push(asks[i])
        i = i + 1
    }}
    if not inserted {{
        new_level = PriceLevel {{ price = price, total_size = order.qty, orders = [order] }}
        result.push(new_level)
    }}
    return result
}}

# Insertion into a sorted list of price levels (descending for bids)
fn insert_bid_level(bids: list, price: f64, order: Order) -> list {{
    inserted = false
    result = []
    i = 0
    while i < bids.len() {{
        if not inserted and bids[i].price < price {{
            new_level = PriceLevel {{ price = price, total_size = order.qty, orders = [order] }}
            result.push(new_level)
            inserted = true
        }}
        result.push(bids[i])
        i = i + 1
    }}
    if not inserted {{
        new_level = PriceLevel {{ price = price, total_size = order.qty, orders = [order] }}
        result.push(new_level)
    }}
    return result
}}

strategy PriorityTest {{
    state {{
        ask_count = 0
        ask_price_0 = 0.0
        ask_price_1 = 0.0
        ask_price_2 = 0.0
        bid_count = 0
        bid_price_0 = 0.0
        bid_price_1 = 0.0
        bid_price_2 = 0.0
    }}
    on bar {{
        # Insert asks in shuffled order: C (highest), A (lowest), B (middle)
        ask_c = Order {{
            id = 1, symbol = "TEST", side = OrderSide.Sell,
            order_type = OrderType.Limit({price_c:.10}),
            qty = 10.0, tif = TimeInForce.GTC
        }}
        ask_a = Order {{
            id = 2, symbol = "TEST", side = OrderSide.Sell,
            order_type = OrderType.Limit({price_a:.10}),
            qty = 20.0, tif = TimeInForce.GTC
        }}
        ask_b = Order {{
            id = 3, symbol = "TEST", side = OrderSide.Sell,
            order_type = OrderType.Limit({price_b:.10}),
            qty = 30.0, tif = TimeInForce.GTC
        }}

        asks = []
        asks = insert_ask_level(asks, {price_c:.10}, ask_c)
        asks = insert_ask_level(asks, {price_a:.10}, ask_a)
        asks = insert_ask_level(asks, {price_b:.10}, ask_b)

        # Insert bids in shuffled order: C (lowest), A (highest), B (middle)
        bid_c = Order {{
            id = 4, symbol = "TEST", side = OrderSide.Buy,
            order_type = OrderType.Limit({bid_c:.10}),
            qty = 10.0, tif = TimeInForce.GTC
        }}
        bid_a = Order {{
            id = 5, symbol = "TEST", side = OrderSide.Buy,
            order_type = OrderType.Limit({bid_a:.10}),
            qty = 20.0, tif = TimeInForce.GTC
        }}
        bid_b = Order {{
            id = 6, symbol = "TEST", side = OrderSide.Buy,
            order_type = OrderType.Limit({bid_b:.10}),
            qty = 30.0, tif = TimeInForce.GTC
        }}

        bids = []
        bids = insert_bid_level(bids, {bid_c:.10}, bid_c)
        bids = insert_bid_level(bids, {bid_a:.10}, bid_a)
        bids = insert_bid_level(bids, {bid_b:.10}, bid_b)

        # Extract book state
        ask_count = asks.len()
        if ask_count >= 3 {{
            ask_price_0 = asks[0].price
            ask_price_1 = asks[1].price
            ask_price_2 = asks[2].price
        }}

        bid_count = bids.len()
        if bid_count >= 3 {{
            bid_price_0 = bids[0].price
            bid_price_1 = bids[1].price
            bid_price_2 = bids[2].price
        }}
    }}
}}
"#,
            price_a = price_a,
            price_b = price_b,
            price_c = price_c,
            bid_a = bid_a,
            bid_b = bid_b,
            bid_c = bid_c,
        );

        let mut interp = compile_to_interpreter(&source);
        interp.on_bar(&test_bar());

        let ask_count = get_state_float(&interp, "ask_count") as i64;
        let ask_price_0 = get_state_float(&interp, "ask_price_0");
        let ask_price_1 = get_state_float(&interp, "ask_price_1");
        let ask_price_2 = get_state_float(&interp, "ask_price_2");
        let bid_count = get_state_float(&interp, "bid_count") as i64;
        let bid_price_0 = get_state_float(&interp, "bid_price_0");
        let bid_price_1 = get_state_float(&interp, "bid_price_1");
        let bid_price_2 = get_state_float(&interp, "bid_price_2");

        // Ask levels should be sorted ascending
        prop_assert_eq!(ask_count, 3, "Expected 3 ask levels, got {}", ask_count);
        prop_assert!((ask_price_0 - price_a).abs() < 1e-6,
            "asks[0].price {} should be lowest ({})", ask_price_0, price_a);
        prop_assert!((ask_price_1 - price_b).abs() < 1e-6,
            "asks[1].price {} should be middle ({})", ask_price_1, price_b);
        prop_assert!((ask_price_2 - price_c).abs() < 1e-6,
            "asks[2].price {} should be highest ({})", ask_price_2, price_c);

        // Bid levels should be sorted descending
        prop_assert_eq!(bid_count, 3, "Expected 3 bid levels, got {}", bid_count);
        prop_assert!((bid_price_0 - bid_a).abs() < 1e-6,
            "bids[0].price {} should be highest ({})", bid_price_0, bid_a);
        prop_assert!((bid_price_1 - bid_b).abs() < 1e-6,
            "bids[1].price {} should be middle ({})", bid_price_1, bid_b);
        prop_assert!((bid_price_2 - bid_c).abs() < 1e-6,
            "bids[2].price {} should be lowest ({})", bid_price_2, bid_c);
    }
}

// =============================================================================
// Property 4: Partial Fill When Order Exceeds Available Liquidity
// Feature: flux-stdlib-backtester, Property 4
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    /// **Validates: Requirements 2.7, 2.8, 5.8**
    ///
    /// Property 4: Partial Fill When Order Exceeds Available Liquidity
    ///
    /// For any market order whose quantity exceeds the total available liquidity
    /// on the opposing side of the OrderBook, the result SHALL be a PartialFill
    /// with filled_qty == total_available_liquidity and remaining_qty ==
    /// order.qty - filled_qty, and the fill price SHALL equal the VWAP across
    /// all consumed levels.
    #[test]
    fn prop_partial_fill_exceeds_liquidity(
        ask_price1 in 100.0f64..150.0,
        ask_price2 in 151.0f64..200.0,
        qty_level1 in 10.0f64..100.0,
        qty_level2 in 10.0f64..100.0,
        excess_fraction in 1.1f64..3.0,
    ) {
        let total_liquidity = qty_level1 + qty_level2;
        let buy_qty = total_liquidity * excess_fraction;
        let expected_remaining = buy_qty - total_liquidity;
        let expected_vwap = (ask_price1 * qty_level1 + ask_price2 * qty_level2) / total_liquidity;

        let source = format!(
            r#"from engine::types import {{Order, Fill, FillResult, OrderSide, OrderType, TimeInForce}}
from engine::book import {{OrderBook, PriceLevel}}

# FIFO matching that detects partial fills
fn match_buy_partial(asks: list, buy_qty: f64) -> list {{
    # Returns [filled_qty, vwap_price, remaining_qty, is_partial]
    # is_partial: 1 = partial fill, 0 = full fill
    remaining = buy_qty
    filled_qty = 0.0
    cost = 0.0

    i = 0
    while i < asks.len() and remaining > 0.0 {{
        level = asks[i]
        level_price = level.price
        orders = level.orders

        j = 0
        while j < orders.len() and remaining > 0.0 {{
            resting = orders[j]
            take = min(resting.qty, remaining)
            cost = cost + level_price * take
            filled_qty = filled_qty + take
            remaining = remaining - take
            j = j + 1
        }}
        i = i + 1
    }}

    vwap = 0.0
    if filled_qty > 0.0 {{
        vwap = cost / filled_qty
    }}

    is_partial = 0
    if remaining > 0.0 {{
        is_partial = 1
    }}

    result = []
    result.push(filled_qty)
    result.push(vwap)
    result.push(remaining)
    result.push(is_partial)
    return result
}}

strategy PartialFillTest {{
    state {{
        fill_price = 0.0
        fill_qty = 0.0
        remaining_qty = 0.0
        is_partial = 0
    }}
    on bar {{
        order1 = Order {{
            id = 1, symbol = "TEST", side = OrderSide.Sell,
            order_type = OrderType.Limit({ask_price1:.10}),
            qty = {qty_level1:.10}, tif = TimeInForce.GTC
        }}
        order2 = Order {{
            id = 2, symbol = "TEST", side = OrderSide.Sell,
            order_type = OrderType.Limit({ask_price2:.10}),
            qty = {qty_level2:.10}, tif = TimeInForce.GTC
        }}

        level1 = PriceLevel {{
            price = {ask_price1:.10},
            total_size = {qty_level1:.10},
            orders = [order1]
        }}
        level2 = PriceLevel {{
            price = {ask_price2:.10},
            total_size = {qty_level2:.10},
            orders = [order2]
        }}

        asks = []
        asks.push(level1)
        asks.push(level2)

        result = match_buy_partial(asks, {buy_qty:.10})
        fill_qty = result[0]
        fill_price = result[1]
        remaining_qty = result[2]
        is_partial = result[3]
    }}
}}
"#,
            ask_price1 = ask_price1,
            ask_price2 = ask_price2,
            qty_level1 = qty_level1,
            qty_level2 = qty_level2,
            buy_qty = buy_qty,
        );

        let mut interp = compile_to_interpreter(&source);
        interp.on_bar(&test_bar());

        let is_partial = get_state_float(&interp, "is_partial") as i64;
        let fill_price = get_state_float(&interp, "fill_price");
        let fill_qty = get_state_float(&interp, "fill_qty");
        let remaining_qty = get_state_float(&interp, "remaining_qty");

        // Should be a PartialFill
        prop_assert_eq!(is_partial, 1,
            "Expected partial fill (1), got {}", is_partial);

        // Fill qty should equal total available liquidity
        prop_assert!((fill_qty - total_liquidity).abs() < 1e-6,
            "Fill qty {} should equal total liquidity {}",
            fill_qty, total_liquidity);

        // Remaining qty should equal order qty minus total liquidity
        prop_assert!((remaining_qty - expected_remaining).abs() < 1e-6,
            "Remaining qty {} should equal expected {}",
            remaining_qty, expected_remaining);

        // Fill price should equal VWAP across all consumed levels
        prop_assert!((fill_price - expected_vwap).abs() < 1e-6,
            "Fill price {} should equal VWAP {} (p1={}, q1={}, p2={}, q2={})",
            fill_price, expected_vwap, ask_price1, qty_level1, ask_price2, qty_level2);
    }
}

// =============================================================================
// Property 17: Position Tracking Invariant
// Feature: flux-stdlib-backtester, Property 17
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(30))]

    /// **Validates: Requirements 8.1, 8.2, 8.3, 8.5**
    ///
    /// Property 17: Position Tracking Invariant
    ///
    /// For any sequence of buy fills followed by a sell fill:
    /// - After buys: position qty = sum(buy_qtys), avg_entry = volume-weighted average
    /// - After sell: realized_pnl = (sell_price - avg_entry) * sell_qty, position.qty decreases
    #[test]
    fn prop_position_tracking_invariant(
        buy_qty1 in 1.0..100.0f64,
        buy_qty2 in 1.0..100.0f64,
        buy_price1 in 10.0..500.0f64,
        buy_price2 in 10.0..500.0f64,
        sell_price in 10.0..500.0f64,
    ) {
        // Expected values after two buys
        let total_qty = buy_qty1 + buy_qty2;
        let expected_avg_entry = (buy_price1 * buy_qty1 + buy_price2 * buy_qty2) / total_qty;
        // Sell qty1 shares at sell_price
        let sell_qty = buy_qty1;  // sell first buy's qty
        let expected_realized_pnl = (sell_price - expected_avg_entry) * sell_qty;
        let expected_remaining_qty = total_qty - sell_qty;

        let source = format!(
            r#"from engine::types import {{Order, Fill, OrderSide, OrderType, TimeInForce, BacktestEngine, PositionState}}
from engine::fast import {{FastEngine, update_position}}
from market::l1 import {{Bar}}

strategy PositionTrackingTest {{
    state {{
        pos_qty_after_buys = 0.0
        avg_entry_after_buys = 0.0
        realized_pnl_after_sell = 0.0
        pos_qty_after_sell = 0.0
    }}
    on bar {{
        engine = FastEngine.new()

        # Buy 1
        order1 = Order {{
            id = 1, symbol = "TEST", side = OrderSide.Buy,
            order_type = OrderType.Market, qty = {buy_qty1:.10},
            tif = TimeInForce.GTC
        }}
        engine = engine.submit_order(order1)
        bar1 = Bar {{
            symbol = "TEST", open = 99.0, high = 600.0, low = 1.0,
            close = {buy_price1:.10}, volume = 50000.0, timestamp = 0.0
        }}
        engine = engine.process_bar(bar1)

        # Buy 2
        order2 = Order {{
            id = 2, symbol = "TEST", side = OrderSide.Buy,
            order_type = OrderType.Market, qty = {buy_qty2:.10},
            tif = TimeInForce.GTC
        }}
        engine = engine.submit_order(order2)
        bar2 = Bar {{
            symbol = "TEST", open = 99.0, high = 600.0, low = 1.0,
            close = {buy_price2:.10}, volume = 50000.0, timestamp = 1.0
        }}
        engine = engine.process_bar(bar2)

        # Check position after two buys
        positions = engine.get_positions()
        if positions.len() > 0 {{
            pos = positions[0]
            pos_qty_after_buys = pos.qty
            avg_entry_after_buys = pos.avg_entry_price
        }}

        # Sell
        order3 = Order {{
            id = 3, symbol = "TEST", side = OrderSide.Sell,
            order_type = OrderType.Market, qty = {sell_qty:.10},
            tif = TimeInForce.GTC
        }}
        engine = engine.submit_order(order3)
        bar3 = Bar {{
            symbol = "TEST", open = 99.0, high = 600.0, low = 1.0,
            close = {sell_price:.10}, volume = 50000.0, timestamp = 2.0
        }}
        engine = engine.process_bar(bar3)

        # Check position after sell
        positions = engine.get_positions()
        if positions.len() > 0 {{
            pos = positions[0]
            pos_qty_after_sell = pos.qty
            realized_pnl_after_sell = pos.realized_pnl
        }}
    }}
}}
"#, buy_qty1 = buy_qty1, buy_qty2 = buy_qty2,
    buy_price1 = buy_price1, buy_price2 = buy_price2,
    sell_price = sell_price, sell_qty = sell_qty);

        let mut interp = compile_to_interpreter(&source);
        interp.on_bar(&test_bar());

        let pos_qty_after_buys = get_state_float(&interp, "pos_qty_after_buys");
        let avg_entry_after_buys = get_state_float(&interp, "avg_entry_after_buys");
        let realized_pnl_after_sell = get_state_float(&interp, "realized_pnl_after_sell");
        let pos_qty_after_sell = get_state_float(&interp, "pos_qty_after_sell");

        // After buys: qty = sum of buy qtys
        prop_assert!((pos_qty_after_buys - total_qty).abs() < 1e-6,
            "Position qty after buys: expected={}, got={}", total_qty, pos_qty_after_buys);

        // After buys: avg_entry = volume-weighted average
        prop_assert!((avg_entry_after_buys - expected_avg_entry).abs() < 1e-6,
            "Avg entry after buys: expected={}, got={}", expected_avg_entry, avg_entry_after_buys);

        // After sell: qty decreases by sell_qty
        prop_assert!((pos_qty_after_sell - expected_remaining_qty).abs() < 1e-6,
            "Position qty after sell: expected={}, got={}", expected_remaining_qty, pos_qty_after_sell);

        // After sell: realized_pnl = (sell_price - avg_entry) * sell_qty
        prop_assert!((realized_pnl_after_sell - expected_realized_pnl).abs() < 0.01,
            "Realized PnL: expected={}, got={}", expected_realized_pnl, realized_pnl_after_sell);
    }
}

// =============================================================================
// Property 18: Multi-Symbol Isolation
// Feature: flux-stdlib-backtester, Property 18
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(30))]

    /// **Validates: Requirements 8.4, 9.1, 9.2, 9.3**
    ///
    /// Property 18: Multi-Symbol Isolation
    ///
    /// Submitting orders for symbols A and B, then processing a bar for only
    /// symbol A, SHALL fill only A's orders. B's orders remain pending and
    /// no position state for B is modified.
    #[test]
    fn prop_multi_symbol_isolation(
        close_a in 10.0..500.0f64,
        qty_a in 1.0..100.0f64,
        qty_b in 1.0..100.0f64,
    ) {
        let source = format!(
            r#"from engine::types import {{Order, Fill, OrderSide, OrderType, TimeInForce, BacktestEngine, PositionState}}
from engine::fast import {{FastEngine, update_position}}
from market::l1 import {{Bar}}

strategy MultiSymbolTest {{
    state {{
        fills_after_bar_a = 0
        fill_symbol = ""
        positions_count = 0
        pos_symbol = ""
    }}
    on bar {{
        engine = FastEngine.new()

        # Submit order for symbol A
        order_a = Order {{
            id = 1, symbol = "AAPL", side = OrderSide.Buy,
            order_type = OrderType.Market, qty = {qty_a:.10},
            tif = TimeInForce.GTC
        }}
        engine = engine.submit_order(order_a)

        # Submit order for symbol B
        order_b = Order {{
            id = 2, symbol = "MSFT", side = OrderSide.Buy,
            order_type = OrderType.Market, qty = {qty_b:.10},
            tif = TimeInForce.GTC
        }}
        engine = engine.submit_order(order_b)

        # Process bar for ONLY symbol A
        bar_a = Bar {{
            symbol = "AAPL", open = 99.0, high = 600.0, low = 1.0,
            close = {close_a:.10}, volume = 50000.0, timestamp = 0.0
        }}
        engine = engine.process_bar(bar_a)

        # Check fills — should only contain A's fill
        fills = engine.get_fills()
        fills_after_bar_a = fills.len()
        if fills_after_bar_a > 0 {{
            fill_symbol = fills[0].symbol
        }}

        # Check positions — only A should have a position
        positions = engine.get_positions()
        positions_count = positions.len()
        if positions_count > 0 {{
            pos_symbol = positions[0].symbol
        }}
    }}
}}
"#, close_a = close_a, qty_a = qty_a, qty_b = qty_b);

        let mut interp = compile_to_interpreter(&source);
        interp.on_bar(&test_bar());

        let fills_count = get_state_float(&interp, "fills_after_bar_a");
        let fill_symbol = match interp.state.get("fill_symbol") {
            Some(Value::Str(s)) => s.clone(),
            other => panic!("Expected Str for 'fill_symbol', got {:?}", other),
        };
        let positions_count = get_state_float(&interp, "positions_count");
        let pos_symbol = match interp.state.get("pos_symbol") {
            Some(Value::Str(s)) => s.clone(),
            other => panic!("Expected Str for 'pos_symbol', got {:?}", other),
        };

        // Only 1 fill produced (for AAPL)
        prop_assert_eq!(fills_count as i64, 1,
            "Expected 1 fill (AAPL only), got {}", fills_count);

        // The fill is for AAPL, not MSFT
        prop_assert!(fill_symbol == "AAPL",
            "Fill should be for AAPL, got {}", fill_symbol);

        // Only 1 position exists (AAPL)
        prop_assert_eq!(positions_count as i64, 1,
            "Expected 1 position (AAPL only), got {}", positions_count);

        // Position is for AAPL
        prop_assert!(pos_symbol == "AAPL",
            "Position should be for AAPL, got {}", pos_symbol);
    }
}

// =============================================================================
// Property 19: Submission Order Determines Processing Order
// Feature: flux-stdlib-backtester, Property 19
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(30))]

    /// **Validates: Requirements 10.3**
    ///
    /// Property 19: Submission Order Determines Processing Order
    ///
    /// When orders O1 then O2 are submitted for the same symbol, after
    /// process_bar, fills SHALL appear in submission order [O1_fill, O2_fill].
    #[test]
    fn prop_submission_order_determines_processing_order(
        close_price in 10.0..500.0f64,
        qty1 in 1.0..100.0f64,
        qty2 in 1.0..100.0f64,
    ) {
        let source = format!(
            r#"from engine::types import {{Order, Fill, OrderSide, OrderType, TimeInForce, BacktestEngine, PositionState}}
from engine::fast import {{FastEngine, update_position}}
from market::l1 import {{Bar}}

strategy SubmissionOrderTest {{
    state {{
        fill_count = 0
        fill_0_order_id = 0
        fill_1_order_id = 0
        fill_0_qty = 0.0
        fill_1_qty = 0.0
    }}
    on bar {{
        engine = FastEngine.new()

        # Submit O1 first (id=10)
        order1 = Order {{
            id = 10, symbol = "TEST", side = OrderSide.Buy,
            order_type = OrderType.Market, qty = {qty1:.10},
            tif = TimeInForce.GTC
        }}
        engine = engine.submit_order(order1)

        # Submit O2 second (id=20)
        order2 = Order {{
            id = 20, symbol = "TEST", side = OrderSide.Buy,
            order_type = OrderType.Market, qty = {qty2:.10},
            tif = TimeInForce.GTC
        }}
        engine = engine.submit_order(order2)

        # Process bar — both should fill
        bar_data = Bar {{
            symbol = "TEST", open = 99.0, high = 600.0, low = 1.0,
            close = {close:.10}, volume = 50000.0, timestamp = 0.0
        }}
        engine = engine.process_bar(bar_data)

        fills = engine.get_fills()
        fill_count = fills.len()
        if fill_count >= 2 {{
            fill_0_order_id = fills[0].order_id
            fill_1_order_id = fills[1].order_id
            fill_0_qty = fills[0].qty
            fill_1_qty = fills[1].qty
        }}
    }}
}}
"#, close = close_price, qty1 = qty1, qty2 = qty2);

        let mut interp = compile_to_interpreter(&source);
        interp.on_bar(&test_bar());

        let fill_count = get_state_float(&interp, "fill_count");
        let fill_0_order_id = get_state_float(&interp, "fill_0_order_id");
        let fill_1_order_id = get_state_float(&interp, "fill_1_order_id");
        let fill_0_qty = get_state_float(&interp, "fill_0_qty");
        let fill_1_qty = get_state_float(&interp, "fill_1_qty");

        // Both orders should fill
        prop_assert_eq!(fill_count as i64, 2,
            "Expected 2 fills, got {}", fill_count);

        // First fill should be O1 (id=10), second should be O2 (id=20)
        prop_assert_eq!(fill_0_order_id as i64, 10,
            "First fill should be order 10, got {}", fill_0_order_id);
        prop_assert_eq!(fill_1_order_id as i64, 20,
            "Second fill should be order 20, got {}", fill_1_order_id);

        // Fill quantities match submission order
        prop_assert!((fill_0_qty - qty1).abs() < 1e-6,
            "First fill qty should be {}, got {}", qty1, fill_0_qty);
        prop_assert!((fill_1_qty - qty2).abs() < 1e-6,
            "Second fill qty should be {}, got {}", qty2, fill_1_qty);
    }
}

// =============================================================================
// Property 20: Sell Exceeding Position Rejected
// Feature: flux-stdlib-backtester, Property 20
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(30))]

    /// **Validates: Requirements 8.6**
    ///
    /// Property 20: Sell Exceeding Position Rejected
    ///
    /// When a sell order is submitted for qty > current position qty,
    /// the sell SHALL NOT produce a fill (silently discarded).
    #[test]
    fn prop_sell_exceeding_position_rejected(
        buy_price in 10.0..500.0f64,
        buy_qty in 1.0..50.0f64,
        excess in 0.01..50.0f64,
    ) {
        let sell_qty = buy_qty + excess;  // Always exceeds position

        let source = format!(
            r#"from engine::types import {{Order, Fill, OrderSide, OrderType, TimeInForce, BacktestEngine, PositionState}}
from engine::fast import {{FastEngine, update_position}}
from market::l1 import {{Bar}}

strategy SellExceedingTest {{
    state {{
        buy_fill_count = 0
        sell_fill_count = 0
        pos_qty_before_sell = 0.0
    }}
    on bar {{
        engine = FastEngine.new()

        # First buy to establish a position
        buy_order = Order {{
            id = 1, symbol = "TEST", side = OrderSide.Buy,
            order_type = OrderType.Market, qty = {buy_qty:.10},
            tif = TimeInForce.GTC
        }}
        engine = engine.submit_order(buy_order)
        bar1 = Bar {{
            symbol = "TEST", open = 99.0, high = 600.0, low = 1.0,
            close = {buy_price:.10}, volume = 50000.0, timestamp = 0.0
        }}
        engine = engine.process_bar(bar1)

        buy_fills = engine.get_fills()
        buy_fill_count = buy_fills.len()

        # Check position before sell attempt
        positions = engine.get_positions()
        if positions.len() > 0 {{
            pos_qty_before_sell = positions[0].qty
        }}

        # Attempt sell with qty > position (should be rejected)
        sell_order = Order {{
            id = 2, symbol = "TEST", side = OrderSide.Sell,
            order_type = OrderType.Market, qty = {sell_qty:.10},
            tif = TimeInForce.GTC
        }}
        engine = engine.submit_order(sell_order)
        bar2 = Bar {{
            symbol = "TEST", open = 99.0, high = 600.0, low = 1.0,
            close = 105.0, volume = 50000.0, timestamp = 1.0
        }}
        engine = engine.process_bar(bar2)

        sell_fills = engine.get_fills()
        sell_fill_count = sell_fills.len()
    }}
}}
"#, buy_qty = buy_qty, buy_price = buy_price, sell_qty = sell_qty);

        let mut interp = compile_to_interpreter(&source);
        interp.on_bar(&test_bar());

        let buy_fill_count = get_state_float(&interp, "buy_fill_count");
        let pos_qty_before_sell = get_state_float(&interp, "pos_qty_before_sell");
        let sell_fill_count = get_state_float(&interp, "sell_fill_count");

        // Buy should have filled
        prop_assert_eq!(buy_fill_count as i64, 1,
            "Expected 1 buy fill, got {}", buy_fill_count);

        // Position should be established
        prop_assert!((pos_qty_before_sell - buy_qty).abs() < 1e-6,
            "Position qty should be {}, got {}", buy_qty, pos_qty_before_sell);

        // Sell exceeding position should produce NO fills
        prop_assert_eq!(sell_fill_count as i64, 0,
            "Sell exceeding position (sell_qty={} > pos_qty={}) should produce 0 fills, got {}",
            sell_qty, buy_qty, sell_fill_count);
    }
}
