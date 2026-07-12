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
