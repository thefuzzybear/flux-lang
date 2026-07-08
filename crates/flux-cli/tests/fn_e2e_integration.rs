//! End-to-end integration tests for user-defined functions.
//!
//! These tests exercise the FULL pipeline:
//! - Interpret path: source → lex → parse → typecheck → interpret (backtest)
//! - Codegen path: source → lex → parse → typecheck → codegen (Rust code)
//! - Error path: source → lex → parse → typecheck → meaningful error
//!
//! **Validates: Requirements 1.1–7.5 (cross-cutting)**

use flux_cli::interpreter::{Interpreter, Value};
use flux_compiler::compile;
use flux_compiler::error::CompileError;
use flux_compiler::lexer;
use flux_compiler::parser;
use flux_compiler::typeck;
use flux_runtime::{BarContext, Signal};

// =============================================================================
// Helpers
// =============================================================================

/// Compile source through lex → parse → typecheck, returning an Interpreter.
fn compile_to_interpreter(source: &str) -> Interpreter {
    let tokens = lexer::lex_with_spans(source).expect("lexer failed");
    let ast = parser::parse(tokens).expect("parser failed");
    let typed_program = typeck::check(ast).expect("typechecker failed");
    Interpreter::new(&typed_program)
}

/// Attempt to compile source through lex → parse → typecheck. Returns the error if any.
fn compile_to_error(source: &str) -> CompileError {
    let tokens = lexer::lex_with_spans(source).expect("lexer failed");
    let ast = parser::parse(tokens).expect("parser failed");
    typeck::check(ast).expect_err("expected typechecker error, but got Ok")
}

/// Create a BarContext with given values.
fn bar(symbol: &str, close: f64, open: f64, high: f64, low: f64, volume: f64) -> BarContext {
    BarContext {
        symbol: symbol.to_string(),
        close,
        open,
        high,
        low,
        volume,
        in_position: false,
    }
}

/// Create a simple BarContext for quick tests.
fn simple_bar(symbol: &str, close: f64, open: f64) -> BarContext {
    bar(symbol, close, open, close + 1.0, open - 1.0, 1000.0)
}

// =============================================================================
// Test: Full pipeline (lex → parse → typecheck → interpret)
// Strategy with user-defined function, backtest produces correct signals
// Validates: Requirements 1.1, 2.1, 3.1, 5.1, 5.2, 5.5
// =============================================================================

#[test]
fn e2e_interpret_strategy_with_user_function_produces_correct_signals() {
    let source = r#"
fn should_open(price, threshold) {
    if price > threshold {
        return 1.0
    }
    return 0.0
}

strategy MeanReversion {
    params {
        entry_threshold = 100.0
    }

    on bar {
        signal = should_open(close, entry_threshold)
        if signal > 0.5 and not in_position {
            OPEN(symbol, 50.0)
        }
    }
}
"#;

    let mut interp = compile_to_interpreter(source);

    // Bar with close=95 (below threshold 100) → no signal
    let ctx1 = simple_bar("AAPL", 95.0, 94.0);
    let signals1 = interp.on_bar(&ctx1);
    assert!(
        signals1.is_empty(),
        "Expected no signals when close < threshold, got {:?}",
        signals1
    );

    // Bar with close=105 (above threshold 100) → OPEN signal
    let ctx2 = simple_bar("AAPL", 105.0, 100.0);
    let signals2 = interp.on_bar(&ctx2);
    assert_eq!(
        signals2.len(),
        1,
        "Expected 1 signal when close > threshold, got {:?}",
        signals2
    );
    match &signals2[0] {
        Signal::Open { symbol, qty } => {
            assert_eq!(symbol, "AAPL");
            assert!((qty - 50.0).abs() < f64::EPSILON);
        }
        other => panic!("Expected Open signal, got {:?}", other),
    }
}

// =============================================================================
// Test: Full pipeline (lex → parse → typecheck → codegen)
// Strategy with user-defined function, generated Rust code checks out
// Validates: Requirements 1.1, 2.1, 3.1, 6.1, 6.2, 6.3
// =============================================================================

#[test]
fn e2e_codegen_strategy_with_user_function_generates_valid_rust() {
    let source = r#"
fn calculate_signal(price, threshold) {
    if price > threshold {
        return 1.0
    }
    return 0.0
}

strategy Momentum {
    params {
        threshold = 100.0
    }

    on bar {
        sig = calculate_signal(close, threshold)
        if sig > 0.5 and not in_position {
            OPEN(symbol, 100.0)
        }
    }
}
"#;

    let output = compile(source).expect("full compilation should succeed");

    // The generated Rust code should contain:
    // 1. A function definition for calculate_signal
    assert!(
        output.contains("fn calculate_signal("),
        "Generated code should contain fn calculate_signal. Got:\n{}",
        output
    );

    // 2. The function should have f64 params (price, threshold)
    assert!(
        output.contains("price: f64") && output.contains("threshold: f64"),
        "Function params should be typed as f64. Got:\n{}",
        output
    );

    // 3. The function should have ctx: &BarContext since it accesses close transitively
    //    (actually, calculate_signal only uses its params — no bar context needed)
    //    But the call site in on_bar uses `close`, which is resolved from ctx there.
    //    The function itself only uses price/threshold, so it should NOT need ctx.
    let fn_line = output
        .lines()
        .find(|l| l.contains("fn calculate_signal("))
        .expect("should find fn calculate_signal");
    assert!(
        !fn_line.contains("ctx"),
        "Pure function (using only params) should not have ctx. Line: {}",
        fn_line
    );

    // 4. A struct definition for the strategy
    assert!(
        output.contains("struct Momentum"),
        "Generated code should contain struct Momentum. Got:\n{}",
        output
    );

    // 5. An on_bar implementation
    assert!(
        output.contains("on_bar"),
        "Generated code should contain on_bar method. Got:\n{}",
        output
    );

    // 6. The function is emitted before the struct
    let fn_pos = output.find("fn calculate_signal(").unwrap();
    let struct_pos = output.find("struct Momentum").unwrap();
    assert!(
        fn_pos < struct_pos,
        "Function should appear before strategy struct"
    );
}

// =============================================================================
// Test: Multi-function strategy
// Function A calls function B, both access bar context
// Validates: Requirements 5.2, 5.6, 6.3, 6.5
// =============================================================================

#[test]
fn e2e_interpret_multi_function_a_calls_b_both_access_bar_context() {
    let source = r#"
fn spread() {
    return close - open
}

fn is_bullish(threshold) {
    s = spread()
    if s > threshold {
        return 1.0
    }
    return 0.0
}

strategy MultiFn {
    params {
        bull_threshold = 2.0
    }

    state {
        result = 0.0
    }

    on bar {
        result = is_bullish(bull_threshold)
        if result > 0.5 and not in_position {
            OPEN(symbol, 100.0)
        }
    }
}
"#;

    let mut interp = compile_to_interpreter(source);

    // Bar with spread = 105 - 100 = 5.0 > 2.0 threshold → bullish
    let ctx1 = simple_bar("TSLA", 105.0, 100.0);
    let signals1 = interp.on_bar(&ctx1);
    assert_eq!(
        signals1.len(),
        1,
        "Expected OPEN signal when spread > threshold, got {:?}",
        signals1
    );
    match &signals1[0] {
        Signal::Open { symbol, qty } => {
            assert_eq!(symbol, "TSLA");
            assert!((qty - 100.0).abs() < f64::EPSILON);
        }
        other => panic!("Expected Open signal, got {:?}", other),
    }

    // Verify state was updated
    match interp.state.get("result") {
        Some(Value::Float(f)) => assert!(
            (*f - 1.0).abs() < f64::EPSILON,
            "Expected result=1.0, got {}",
            f
        ),
        other => panic!("Expected Float(1.0) in state, got {:?}", other),
    }

    // Bar with spread = 101 - 100 = 1.0 < 2.0 threshold → not bullish
    // Reset in_position to false for this test
    interp.in_position = false;
    let ctx2 = simple_bar("TSLA", 101.0, 100.0);
    let signals2 = interp.on_bar(&ctx2);
    assert!(
        signals2.is_empty(),
        "Expected no signals when spread < threshold, got {:?}",
        signals2
    );

    match interp.state.get("result") {
        Some(Value::Float(f)) => assert!(
            (*f - 0.0).abs() < f64::EPSILON,
            "Expected result=0.0, got {}",
            f
        ),
        other => panic!("Expected Float(0.0) in state, got {:?}", other),
    }
}

#[test]
fn e2e_codegen_multi_function_both_access_bar_context() {
    let source = r#"
fn spread() {
    return close - open
}

fn is_bullish(threshold) {
    s = spread()
    if s > threshold {
        return 1.0
    }
    return 0.0
}

strategy MultiFn {
    params {
        bull_threshold = 2.0
    }

    on bar {
        result = is_bullish(bull_threshold)
        if result > 0.5 and not in_position {
            OPEN(symbol, 100.0)
        }
    }
}
"#;

    let output = compile(source).expect("compilation should succeed");

    // spread() accesses close and open → needs ctx
    let spread_line = output
        .lines()
        .find(|l| l.contains("fn spread("))
        .expect("should find fn spread");
    assert!(
        spread_line.contains("ctx: &BarContext"),
        "spread() should have ctx param since it accesses close/open. Line: {}",
        spread_line
    );

    // is_bullish calls spread which needs ctx → is_bullish also needs ctx (transitive)
    let bullish_line = output
        .lines()
        .find(|l| l.contains("fn is_bullish("))
        .expect("should find fn is_bullish");
    assert!(
        bullish_line.contains("ctx: &BarContext"),
        "is_bullish() should have ctx transitively. Line: {}",
        bullish_line
    );

    // is_bullish's call to spread should forward ctx
    assert!(
        output.contains("spread(ctx)"),
        "is_bullish should forward ctx to spread(). Got:\n{}",
        output
    );
}

// =============================================================================
// Test: Function emitting signals
// Verify signals appear in backtest output
// Validates: Requirements 5.5, 7.1
// =============================================================================

#[test]
fn e2e_interpret_function_emitting_signals_appear_in_backtest() {
    let source = r#"
fn open_if_cheap(threshold, qty) {
    if close < threshold {
        OPEN(symbol, qty)
    }
}

fn close_if_expensive(threshold) {
    if close > threshold {
        CLOSE(symbol)
    }
}

strategy SignalFn {
    params {
        buy_level = 100.0
        sell_level = 150.0
        position_size = 200.0
    }

    on bar {
        if not in_position {
            open_if_cheap(buy_level, position_size)
        }
        if in_position {
            close_if_expensive(sell_level)
        }
    }
}
"#;

    let mut interp = compile_to_interpreter(source);

    // Bar 1: close=95 < buy_level=100 → OPEN signal
    let ctx1 = simple_bar("MSFT", 95.0, 94.0);
    let signals1 = interp.on_bar(&ctx1);
    assert_eq!(signals1.len(), 1, "Expected OPEN signal, got {:?}", signals1);
    match &signals1[0] {
        Signal::Open { symbol, qty } => {
            assert_eq!(symbol, "MSFT");
            assert!((qty - 200.0).abs() < f64::EPSILON);
        }
        other => panic!("Expected Open signal, got {:?}", other),
    }

    // After OPEN, in_position becomes true (handled by on_bar)
    assert!(interp.in_position, "in_position should be true after OPEN");

    // Bar 2: close=120, in_position=true, but 120 < sell_level=150 → no CLOSE
    let ctx2 = simple_bar("MSFT", 120.0, 118.0);
    let signals2 = interp.on_bar(&ctx2);
    assert!(
        signals2.is_empty(),
        "Expected no signals (price not at sell level), got {:?}",
        signals2
    );

    // Bar 3: close=160 > sell_level=150 → CLOSE signal
    let ctx3 = simple_bar("MSFT", 160.0, 155.0);
    let signals3 = interp.on_bar(&ctx3);
    assert_eq!(
        signals3.len(),
        1,
        "Expected CLOSE signal, got {:?}",
        signals3
    );
    match &signals3[0] {
        Signal::Close { symbol } => {
            assert_eq!(symbol, "MSFT");
        }
        other => panic!("Expected Close signal, got {:?}", other),
    }

    // After CLOSE, in_position becomes false
    assert!(!interp.in_position, "in_position should be false after CLOSE");
}

// =============================================================================
// Test: Error pipeline — recursion → meaningful error with correct span
// Validates: Requirements 4.1, 7.1, 7.4
// =============================================================================

#[test]
fn e2e_error_pipeline_recursion_produces_meaningful_error() {
    let source = r#"
fn infinite() {
    return infinite()
}

strategy Test {
    on bar {
        x = infinite()
    }
}
"#;

    let err = compile_to_error(source);
    match &err {
        CompileError::Type(msg) => {
            // Should mention recursion
            assert!(
                msg.contains("recursive") || msg.contains("recursion") || msg.contains("cycle"),
                "Expected recursion error, got: {}",
                msg
            );
            // Should mention the function name
            assert!(
                msg.contains("infinite"),
                "Error should mention function name 'infinite', got: {}",
                msg
            );
            // Should include span info (at byte N:)
            assert!(
                msg.contains("at byte "),
                "Error should include byte offset span info, got: {}",
                msg
            );
        }
        other => panic!("Expected CompileError::Type for recursion, got: {:?}", other),
    }
}

#[test]
fn e2e_error_pipeline_mutual_recursion_produces_meaningful_error() {
    let source = r#"
fn ping() {
    return pong()
}

fn pong() {
    return ping()
}

strategy Test {
    on bar {
        x = ping()
    }
}
"#;

    let err = compile_to_error(source);
    match &err {
        CompileError::Type(msg) => {
            assert!(
                msg.contains("recursive") || msg.contains("recursion") || msg.contains("cycle"),
                "Expected recursion/cycle error, got: {}",
                msg
            );
            // Should mention at least one of the functions in the cycle
            assert!(
                msg.contains("ping") || msg.contains("pong"),
                "Error should mention function(s) in cycle, got: {}",
                msg
            );
        }
        other => panic!("Expected CompileError::Type for mutual recursion, got: {:?}", other),
    }
}

// =============================================================================
// Test: Error pipeline — state access in function → correct error message
// Validates: Requirements 3.8, 7.5
// =============================================================================

#[test]
fn e2e_error_pipeline_state_access_in_function_produces_error() {
    let source = r#"
fn bad_fn() {
    return count + 1.0
}

strategy Test {
    state {
        count = 0.0
    }

    on bar {
        x = bad_fn()
    }
}
"#;

    let err = compile_to_error(source);
    match &err {
        CompileError::Type(msg) => {
            // Should mention that functions cannot access state
            assert!(
                msg.contains("state") || msg.contains("cannot access"),
                "Expected state access error, got: {}",
                msg
            );
            // Should mention the variable name
            assert!(
                msg.contains("count"),
                "Error should mention state variable name 'count', got: {}",
                msg
            );
        }
        other => panic!(
            "Expected CompileError::Type for state access, got: {:?}",
            other
        ),
    }
}

// =============================================================================
// Test: Error pipeline — arity mismatch → error with expected/actual counts
// Validates: Requirements 3.3, 7.2
// =============================================================================

#[test]
fn e2e_error_pipeline_arity_mismatch_too_few_args() {
    let source = r#"
fn add(a, b, c) {
    return a + b + c
}

strategy Test {
    on bar {
        x = add(1.0)
    }
}
"#;

    let err = compile_to_error(source);
    match &err {
        CompileError::Type(msg) => {
            // Should mention function name
            assert!(
                msg.contains("add"),
                "Error should mention function name 'add', got: {}",
                msg
            );
            // Should mention expected count (3)
            assert!(
                msg.contains('3'),
                "Error should mention expected param count 3, got: {}",
                msg
            );
            // Should mention actual count (1)
            assert!(
                msg.contains('1'),
                "Error should mention actual arg count 1, got: {}",
                msg
            );
        }
        other => panic!(
            "Expected CompileError::Type for arity mismatch, got: {:?}",
            other
        ),
    }
}

#[test]
fn e2e_error_pipeline_arity_mismatch_too_many_args() {
    let source = r#"
fn single(x) {
    return x * 2.0
}

strategy Test {
    on bar {
        y = single(1.0, 2.0, 3.0)
    }
}
"#;

    let err = compile_to_error(source);
    match &err {
        CompileError::Type(msg) => {
            // Should mention function name
            assert!(
                msg.contains("single"),
                "Error should mention function name 'single', got: {}",
                msg
            );
            // Should mention expected count (1)
            assert!(
                msg.contains('1'),
                "Error should mention expected param count 1, got: {}",
                msg
            );
            // Should mention actual count (3)
            assert!(
                msg.contains('3'),
                "Error should mention actual arg count 3, got: {}",
                msg
            );
        }
        other => panic!(
            "Expected CompileError::Type for arity mismatch, got: {:?}",
            other
        ),
    }
}

// =============================================================================
// Additional integration tests: multi-bar backtest with user functions
// =============================================================================

#[test]
fn e2e_interpret_multi_bar_backtest_with_function_based_strategy() {
    // A more realistic strategy: function computes a moving-average-like check
    // and emits signals over multiple bars.
    let source = r#"
fn price_above_level(level) {
    if close > level {
        return 1.0
    }
    return 0.0
}

fn price_below_level(level) {
    if close < level {
        return 1.0
    }
    return 0.0
}

strategy Breakout {
    params {
        upper = 110.0
        lower = 90.0
        size = 100.0
    }

    on bar {
        if not in_position {
            above = price_above_level(upper)
            if above > 0.5 {
                OPEN(symbol, size)
            }
        }
        if in_position {
            below = price_below_level(lower)
            if below > 0.5 {
                CLOSE(symbol)
            }
        }
    }
}
"#;

    let mut interp = compile_to_interpreter(source);
    let mut all_signals: Vec<Signal> = Vec::new();

    // Simulate 5 bars
    let bars = vec![
        simple_bar("SPY", 100.0, 99.0),  // no signal (between levels)
        simple_bar("SPY", 112.0, 108.0), // OPEN (above 110)
        simple_bar("SPY", 105.0, 103.0), // no signal (in position, above 90)
        simple_bar("SPY", 85.0, 88.0),   // CLOSE (below 90)
        simple_bar("SPY", 115.0, 112.0), // OPEN again (above 110, not in position)
    ];

    for ctx in &bars {
        let signals = interp.on_bar(ctx);
        all_signals.extend(signals);
    }

    // Expected: OPEN, CLOSE, OPEN (3 signals total)
    assert_eq!(
        all_signals.len(),
        3,
        "Expected 3 signals (OPEN, CLOSE, OPEN) across 5 bars, got {:?}",
        all_signals
    );

    // Signal 1: OPEN
    assert!(
        matches!(&all_signals[0], Signal::Open { symbol, qty } if symbol == "SPY" && (*qty - 100.0).abs() < f64::EPSILON),
        "Signal 1 should be OPEN(SPY, 100.0), got {:?}",
        all_signals[0]
    );

    // Signal 2: CLOSE
    assert!(
        matches!(&all_signals[1], Signal::Close { symbol } if symbol == "SPY"),
        "Signal 2 should be CLOSE(SPY), got {:?}",
        all_signals[1]
    );

    // Signal 3: OPEN again
    assert!(
        matches!(&all_signals[2], Signal::Open { symbol, qty } if symbol == "SPY" && (*qty - 100.0).abs() < f64::EPSILON),
        "Signal 3 should be OPEN(SPY, 100.0), got {:?}",
        all_signals[2]
    );
}

#[test]
fn e2e_codegen_function_emitting_signals_has_signals_param() {
    let source = r#"
fn emit_open(qty) {
    OPEN(symbol, qty)
}

fn emit_close() {
    CLOSE(symbol)
}

strategy SignalEmitter {
    on bar {
        if not in_position {
            emit_open(100.0)
        }
        if in_position {
            emit_close()
        }
    }
}
"#;

    let output = compile(source).expect("compilation should succeed");

    // emit_open uses `symbol` (bar context) and emits signals
    let emit_open_line = output
        .lines()
        .find(|l| l.contains("fn emit_open("))
        .expect("should find fn emit_open");
    assert!(
        emit_open_line.contains("signals: &mut Vec<Signal>"),
        "emit_open should have signals param. Line: {}",
        emit_open_line
    );
    assert!(
        emit_open_line.contains("ctx: &BarContext"),
        "emit_open should have ctx param (uses symbol). Line: {}",
        emit_open_line
    );

    // emit_close uses `symbol` (bar context) and emits signals
    let emit_close_line = output
        .lines()
        .find(|l| l.contains("fn emit_close("))
        .expect("should find fn emit_close");
    assert!(
        emit_close_line.contains("signals: &mut Vec<Signal>"),
        "emit_close should have signals param. Line: {}",
        emit_close_line
    );
    assert!(
        emit_close_line.contains("ctx: &BarContext"),
        "emit_close should have ctx param (uses symbol). Line: {}",
        emit_close_line
    );
}
