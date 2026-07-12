//! Property-based tests for SyntheticEngine modules.
//!
//! Feature: flux-stdlib-backtester
//!
//! This file contains property tests for the Flux stdlib Synthetic Engine.
//! - Property 9 validates price path generation from OHLCV bars.
//! - Property 10 validates synthetic book structure.
//!
//! **Validates: Requirements 5.1, 5.2, 5.4, 5.5, 10.5**

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

/// Extract a float from interpreter state.
fn get_state_float(interp: &Interpreter, name: &str) -> f64 {
    match interp.state.get(name) {
        Some(Value::Float(f)) => *f,
        Some(Value::Int(i)) => *i as f64,
        other => panic!("Expected Float/Int in '{}', got {:?}", name, other),
    }
}

// =============================================================================
// Property 9: Price Path is Deterministic 4-Point Sequence from OHLCV
// Feature: flux-stdlib-backtester, Property 9
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    /// **Validates: Requirements 5.1, 5.2, 10.5**
    ///
    /// Property 9: Price Path is Deterministic 4-Point Sequence from OHLCV
    ///
    /// For any OHLCV bar, `generate_price_path` SHALL produce exactly 4 points where:
    /// - path[0] == open
    /// - path[3] == close
    /// - The middle two points are the bar's high and low with the nearer extreme
    ///   to open visited first (preferring high when equidistant).
    #[test]
    fn prop_price_path_deterministic_4_point_sequence(
        open in 10.0..500.0f64,
        close in 10.0..500.0f64,
        high_delta in 0.0..50.0f64,
        low_delta in 0.0..50.0f64,
    ) {
        // high must be >= max(open, close), low must be <= min(open, close)
        let high = f64::max(open, close) + high_delta;
        let low = f64::min(open, close) - low_delta;

        let source = format!(
            r#"from engine::synthetic import {{generate_price_path}}
from market::l1 import {{Bar}}

strategy PricePathTest {{
    state {{
        path_len = 0
        path_0 = 0.0
        path_1 = 0.0
        path_2 = 0.0
        path_3 = 0.0
    }}
    on bar {{
        bar_data = Bar {{ open = {open:.10}, high = {high:.10}, low = {low:.10}, close = {close:.10}, volume = 1000.0, timestamp = 0.0, symbol = "TEST" }}
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

        let path_len = get_state_float(&interp, "path_len");
        let path_0 = get_state_float(&interp, "path_0");
        let path_1 = get_state_float(&interp, "path_1");
        let path_2 = get_state_float(&interp, "path_2");
        let path_3 = get_state_float(&interp, "path_3");

        // Path has exactly 4 elements
        prop_assert_eq!(path_len as i64, 4,
            "Expected path length 4, got {}", path_len);

        // Path[0] == open
        prop_assert!((path_0 - open).abs() < 1e-10,
            "path[0]={} should equal open={}", path_0, open);

        // Path[3] == close
        prop_assert!((path_3 - close).abs() < 1e-10,
            "path[3]={} should equal close={}", path_3, close);

        // Determine expected ordering of middle points
        let dist_to_high = (high - open).abs();
        let dist_to_low = (open - low).abs();

        if dist_to_high <= dist_to_low {
            // High is nearer or equidistant — prefer High first
            prop_assert!((path_1 - high).abs() < 1e-10,
                "When high nearer (dist_high={}, dist_low={}): path[1]={} should be high={}",
                dist_to_high, dist_to_low, path_1, high);
            prop_assert!((path_2 - low).abs() < 1e-10,
                "When high nearer (dist_high={}, dist_low={}): path[2]={} should be low={}",
                dist_to_high, dist_to_low, path_2, low);
        } else {
            // Low is nearer
            prop_assert!((path_1 - low).abs() < 1e-10,
                "When low nearer (dist_high={}, dist_low={}): path[1]={} should be low={}",
                dist_to_high, dist_to_low, path_1, low);
            prop_assert!((path_2 - high).abs() < 1e-10,
                "When low nearer (dist_high={}, dist_low={}): path[2]={} should be high={}",
                dist_to_high, dist_to_low, path_2, high);
        }
    }
}

// =============================================================================
// Property 10: Synthetic Book Has Correct Structure
// Feature: flux-stdlib-backtester, Property 10
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(30))]

    /// **Validates: Requirements 5.4, 5.5, 10.5**
    ///
    /// Property 10: Synthetic Book Has Correct Structure
    ///
    /// For any SyntheticConfig and center price, `build_synthetic_book` SHALL produce
    /// an OrderBook with exactly `config.depth` ask levels (ascending) and
    /// `config.depth` bid levels (descending), each containing
    /// `config.liquidity_per_side / config.depth` quantity.
    #[test]
    fn prop_synthetic_book_correct_structure(
        center_price in 10.0..1000.0f64,
        depth in 1..10i32,
        spread_pct in 0.01..5.0f64,
        liquidity in 1000.0..100000.0f64,
    ) {
        let expected_qty_per_level = liquidity / (depth as f64);
        let spread_step = center_price * spread_pct / 100.0;

        // We store depth+1 ask prices and depth+1 bid prices to verify spacing
        // We limit to 10 depth, so max items needed is 10
        let source = format!(
            r#"from engine::types import {{Order, Fill, FillResult, OrderSide, OrderType, TimeInForce}}
from engine::book import {{OrderBook, PriceLevel}}
from engine::synthetic import {{build_synthetic_book, SyntheticConfig}}
from market::l1 import {{Bar}}

strategy BookStructureTest {{
    state {{
        ask_count = 0
        bid_count = 0
        ask_price_0 = 0.0
        ask_price_1 = 0.0
        ask_qty_0 = 0.0
        bid_price_0 = 0.0
        bid_price_1 = 0.0
        bid_qty_0 = 0.0
        asks_ascending = 1
        bids_descending = 1
        qty_uniform = 1
    }}
    on bar {{
        config = SyntheticConfig {{ depth = {depth}, spread_pct = {spread_pct:.10}, liquidity_per_side = {liquidity:.10} }}
        book = build_synthetic_book({center:.10}, "TEST", config)

        ask_count = book.asks.len()
        bid_count = book.bids.len()

        # Check first ask and bid prices and quantities
        if ask_count > 0 {{
            ask_price_0 = book.asks[0].price
            ask_qty_0 = book.asks[0].total_size
        }}
        if ask_count > 1 {{
            ask_price_1 = book.asks[1].price
        }}
        if bid_count > 0 {{
            bid_price_0 = book.bids[0].price
            bid_qty_0 = book.bids[0].total_size
        }}
        if bid_count > 1 {{
            bid_price_1 = book.bids[1].price
        }}

        # Verify asks are ascending
        i = 0
        while i < ask_count - 1 {{
            if book.asks[i].price >= book.asks[i + 1].price {{
                asks_ascending = 0
            }}
            i = i + 1
        }}

        # Verify bids are descending
        i = 0
        while i < bid_count - 1 {{
            if book.bids[i].price <= book.bids[i + 1].price {{
                bids_descending = 0
            }}
            i = i + 1
        }}

        # Verify uniform quantity across all levels
        expected_qty = {liquidity:.10} / {depth}
        i = 0
        while i < ask_count {{
            diff = book.asks[i].total_size - expected_qty
            if diff > 0.01 or diff < 0.0 - 0.01 {{
                qty_uniform = 0
            }}
            i = i + 1
        }}
        i = 0
        while i < bid_count {{
            diff = book.bids[i].total_size - expected_qty
            if diff > 0.01 or diff < 0.0 - 0.01 {{
                qty_uniform = 0
            }}
            i = i + 1
        }}
    }}
}}
"#, depth = depth, spread_pct = spread_pct, liquidity = liquidity, center = center_price);

        let mut interp = compile_to_interpreter(&source);
        interp.on_bar(&test_bar());

        let ask_count = get_state_float(&interp, "ask_count");
        let bid_count = get_state_float(&interp, "bid_count");
        let ask_price_0 = get_state_float(&interp, "ask_price_0");
        let ask_price_1 = get_state_float(&interp, "ask_price_1");
        let ask_qty_0 = get_state_float(&interp, "ask_qty_0");
        let bid_price_0 = get_state_float(&interp, "bid_price_0");
        let bid_price_1 = get_state_float(&interp, "bid_price_1");
        let bid_qty_0 = get_state_float(&interp, "bid_qty_0");
        let asks_ascending = get_state_float(&interp, "asks_ascending");
        let bids_descending = get_state_float(&interp, "bids_descending");
        let qty_uniform = get_state_float(&interp, "qty_uniform");

        // Book has exactly `depth` ask levels
        prop_assert_eq!(ask_count as i64, depth as i64,
            "Expected {} ask levels, got {}", depth, ask_count);

        // Book has exactly `depth` bid levels
        prop_assert_eq!(bid_count as i64, depth as i64,
            "Expected {} bid levels, got {}", depth, bid_count);

        // Each level has qty == liquidity_per_side / depth
        prop_assert!((ask_qty_0 - expected_qty_per_level).abs() < 0.01,
            "Ask level 0 qty={} should be {}", ask_qty_0, expected_qty_per_level);
        prop_assert!((bid_qty_0 - expected_qty_per_level).abs() < 0.01,
            "Bid level 0 qty={} should be {}", bid_qty_0, expected_qty_per_level);
        prop_assert_eq!(qty_uniform as i64, 1,
            "All levels should have uniform qty = {}", expected_qty_per_level);

        // Ask prices are ascending from center + spread_step
        let expected_ask_0 = center_price + spread_step;
        prop_assert!((ask_price_0 - expected_ask_0).abs() < 1e-6,
            "Ask[0] price={} should be center + spread_step = {}",
            ask_price_0, expected_ask_0);

        if depth > 1 {
            let expected_ask_1 = center_price + spread_step * 2.0;
            prop_assert!((ask_price_1 - expected_ask_1).abs() < 1e-6,
                "Ask[1] price={} should be center + 2*spread_step = {}",
                ask_price_1, expected_ask_1);
        }

        prop_assert_eq!(asks_ascending as i64, 1,
            "Ask prices should be strictly ascending");

        // Bid prices are descending from center - spread_step
        let expected_bid_0 = center_price - spread_step;
        prop_assert!((bid_price_0 - expected_bid_0).abs() < 1e-6,
            "Bid[0] price={} should be center - spread_step = {}",
            bid_price_0, expected_bid_0);

        if depth > 1 {
            let expected_bid_1 = center_price - spread_step * 2.0;
            prop_assert!((bid_price_1 - expected_bid_1).abs() < 1e-6,
                "Bid[1] price={} should be center - 2*spread_step = {}",
                bid_price_1, expected_bid_1);
        }

        prop_assert_eq!(bids_descending as i64, 1,
            "Bid prices should be strictly descending");
    }
}
