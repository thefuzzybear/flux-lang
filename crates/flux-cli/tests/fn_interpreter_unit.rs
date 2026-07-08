//! Unit tests for interpreter user-defined function execution.
//!
//! Each test compiles Flux source through the full pipeline (lex → parse → typecheck)
//! to produce a TypedProgram, then exercises the interpreter against bar data.
//!
//! **Validates: Requirements 5.1, 5.2, 5.3, 5.4, 5.5, 5.6, 5.7**

use flux_cli::interpreter::{Interpreter, Value};
use flux_compiler::lexer;
use flux_compiler::parser;
use flux_compiler::typeck;
use flux_runtime::BarContext;

/// Helper: compile Flux source through lex → parse → typecheck, returning an Interpreter.
fn compile_to_interpreter(source: &str) -> Interpreter {
    let tokens = lexer::lex_with_spans(source).expect("lexer failed");
    let ast = parser::parse(tokens).expect("parser failed");
    let typed_program = typeck::check(ast).expect("typechecker failed");
    Interpreter::new(&typed_program)
}

/// Helper: create a BarContext with specified values.
fn bar(symbol: &str, close: f64, open: f64) -> BarContext {
    BarContext {
        symbol: symbol.to_string(),
        close,
        open,
        high: close + 1.0,
        low: open - 1.0,
        volume: 1000.0,
        in_position: false,
    }
}

// =============================================================================
// Test: simple function call with parameters returns computed value
// Validates: Requirement 5.1
// =============================================================================

#[test]
fn test_simple_function_call_with_params_returns_computed_value() {
    let source = r#"
fn add(a, b) {
    return a + b
}

strategy Test {
    state {
        result = 0.0
    }
    on bar {
        result = add(3.0, 4.0)
    }
}
"#;

    let mut interp = compile_to_interpreter(source);
    let ctx = bar("AAPL", 100.0, 99.0);
    interp.on_bar(&ctx);

    // After running on_bar, state variable `result` should be 7.0
    match interp.state.get("result") {
        Some(Value::Float(f)) => assert!(
            (*f - 7.0).abs() < f64::EPSILON,
            "Expected 7.0, got {}",
            f
        ),
        other => panic!("Expected Float(7.0) in state, got {:?}", other),
    }
}

// =============================================================================
// Test: function accessing bar context variables (close, open) works
// Validates: Requirement 5.2
// =============================================================================

#[test]
fn test_function_accessing_bar_context_variables() {
    let source = r#"
fn spread() {
    return close - open
}

strategy Test {
    state {
        result = 0.0
    }
    on bar {
        result = spread()
    }
}
"#;

    let mut interp = compile_to_interpreter(source);
    let ctx = bar("AAPL", 105.0, 100.0);
    interp.on_bar(&ctx);

    match interp.state.get("result") {
        Some(Value::Float(f)) => assert!(
            (*f - 5.0).abs() < f64::EPSILON,
            "Expected 5.0, got {}",
            f
        ),
        other => panic!("Expected Float(5.0) in state, got {:?}", other),
    }
}

// =============================================================================
// Test: function with `return` exits early with correct value
// Validates: Requirement 5.3
// =============================================================================

#[test]
fn test_function_with_return_exits_early() {
    let source = r#"
fn early_exit(x) {
    if x > 10.0 {
        return 999.0
    }
    return 0.0 - 1.0
}

strategy Test {
    state {
        result = 0.0
    }
    on bar {
        result = early_exit(50.0)
    }
}
"#;

    let mut interp = compile_to_interpreter(source);
    let ctx = bar("AAPL", 100.0, 99.0);
    interp.on_bar(&ctx);

    // 50.0 > 10.0, so should return 999.0 (early exit)
    match interp.state.get("result") {
        Some(Value::Float(f)) => assert!(
            (*f - 999.0).abs() < f64::EPSILON,
            "Expected 999.0 (early return), got {}",
            f
        ),
        other => panic!("Expected Float(999.0) in state, got {:?}", other),
    }
}

// =============================================================================
// Test: function without `return` returns Null
// Validates: Requirement 5.4
// =============================================================================

#[test]
fn test_function_without_return_returns_null() {
    // A function that does computation but has no return statement.
    // The caller should receive Null (which becomes 0.0 or is unused).
    // We verify by checking that the function call doesn't error.
    let source = r#"
fn no_return(x) {
    y = x + 1.0
}

strategy Test {
    state {
        ran = 0.0
    }
    on bar {
        no_return(5.0)
        ran = 1.0
    }
}
"#;

    let mut interp = compile_to_interpreter(source);
    let ctx = bar("AAPL", 100.0, 99.0);
    interp.on_bar(&ctx);

    // The strategy should have run past the function call without error
    match interp.state.get("ran") {
        Some(Value::Float(f)) => assert!(
            (*f - 1.0).abs() < f64::EPSILON,
            "Expected ran=1.0, got {}",
            f
        ),
        other => panic!("Expected Float(1.0) in state, got {:?}", other),
    }
}

// =============================================================================
// Test: function emitting OPEN/CLOSE signal → signal appears in strategy output
// Validates: Requirement 5.5
// =============================================================================

#[test]
fn test_function_emitting_signals_propagates_to_output() {
    let source = r#"
fn open_position() {
    OPEN(symbol, 100.0)
}

strategy Test {
    on bar {
        open_position()
    }
}
"#;

    let mut interp = compile_to_interpreter(source);
    let ctx = bar("AAPL", 100.0, 99.0);
    let signals = interp.on_bar(&ctx);

    assert_eq!(signals.len(), 1, "Expected 1 signal, got {:?}", signals);
    match &signals[0] {
        flux_runtime::Signal::Open { symbol, qty } => {
            assert_eq!(symbol, "AAPL");
            assert!((*qty - 100.0).abs() < f64::EPSILON);
        }
        other => panic!("Expected Open signal, got {:?}", other),
    }
}

#[test]
fn test_function_emitting_close_signal() {
    let source = r#"
fn close_position() {
    CLOSE(symbol)
}

strategy Test {
    on bar {
        close_position()
    }
}
"#;

    let mut interp = compile_to_interpreter(source);
    interp.in_position = true;
    let ctx = bar("AAPL", 100.0, 99.0);
    let signals = interp.on_bar(&ctx);

    assert_eq!(signals.len(), 1, "Expected 1 signal, got {:?}", signals);
    match &signals[0] {
        flux_runtime::Signal::Close { symbol } => {
            assert_eq!(symbol, "AAPL");
        }
        other => panic!("Expected Close signal, got {:?}", other),
    }
}

// =============================================================================
// Test: function calling another function (chained call) works
// Validates: Requirement 5.6
// =============================================================================

#[test]
fn test_chained_function_calls() {
    let source = r#"
fn double(x) {
    return x * 2.0
}

fn quadruple(x) {
    return double(double(x))
}

strategy Test {
    state {
        result = 0.0
    }
    on bar {
        result = quadruple(3.0)
    }
}
"#;

    let mut interp = compile_to_interpreter(source);
    let ctx = bar("AAPL", 100.0, 99.0);
    interp.on_bar(&ctx);

    // quadruple(3.0) = double(double(3.0)) = double(6.0) = 12.0
    match interp.state.get("result") {
        Some(Value::Float(f)) => assert!(
            (*f - 12.0).abs() < f64::EPSILON,
            "Expected 12.0, got {}",
            f
        ),
        other => panic!("Expected Float(12.0) in state, got {:?}", other),
    }
}

// =============================================================================
// Test: call depth exceeding 64 produces stack overflow error
// Validates: Requirement 5.7
// =============================================================================

#[test]
fn test_call_depth_exceeding_limit_produces_stack_overflow() {
    // Create a chain of 65 functions: f0 calls f1, f1 calls f2, ... f64 returns.
    // This exceeds the default limit of 64.
    let mut source = String::new();

    // Generate 65 functions in a chain
    for i in 0..65 {
        if i < 64 {
            source.push_str(&format!(
                "fn f{}() {{\n    return f{}()\n}}\n\n",
                i,
                i + 1
            ));
        } else {
            source.push_str(&format!("fn f{}() {{\n    return 1.0\n}}\n\n", i));
        }
    }

    source.push_str(
        r#"
strategy Test {
    state {
        result = 0.0
    }
    on bar {
        result = f0()
    }
}
"#,
    );

    let mut interp = compile_to_interpreter(&source);
    let ctx = bar("AAPL", 100.0, 99.0);

    // on_bar should produce a warning (runtime error) and return empty signals.
    // The error is caught by the interpreter's on_bar handler and prints a warning.
    // We verify by checking that state is unchanged (the assignment failed due to error).
    let signals = interp.on_bar(&ctx);
    assert!(signals.is_empty(), "Expected no signals on stack overflow");

    // The state should remain at the initial value since the handler errored
    match interp.state.get("result") {
        Some(Value::Float(f)) => assert!(
            (*f - 0.0).abs() < f64::EPSILON,
            "Expected result=0.0 (unchanged due to error), got {}",
            f
        ),
        other => panic!("Expected Float(0.0) in state, got {:?}", other),
    }
}

#[test]
fn test_call_depth_at_limit_succeeds() {
    // Create a chain of exactly 64 functions (depth 0..63), which is at the limit but
    // should succeed because call_depth goes 0→1→2→...→63 (all < 64).
    let mut source = String::new();

    for i in 0..64 {
        if i < 63 {
            source.push_str(&format!(
                "fn f{}() {{\n    return f{}()\n}}\n\n",
                i,
                i + 1
            ));
        } else {
            source.push_str(&format!("fn f{}() {{\n    return 42.0\n}}\n\n", i));
        }
    }

    source.push_str(
        r#"
strategy Test {
    state {
        result = 0.0
    }
    on bar {
        result = f0()
    }
}
"#,
    );

    let mut interp = compile_to_interpreter(&source);
    let ctx = bar("AAPL", 100.0, 99.0);
    interp.on_bar(&ctx);

    // Chain of 64 functions (call_depth goes 0→63) should succeed
    match interp.state.get("result") {
        Some(Value::Float(f)) => assert!(
            (*f - 42.0).abs() < f64::EPSILON,
            "Expected 42.0, got {}",
            f
        ),
        other => panic!("Expected Float(42.0) in state, got {:?}", other),
    }
}

// =============================================================================
// Test: local variables inside function don't leak to caller scope
// Validates: Requirement 5.1, 5.6
// =============================================================================

#[test]
fn test_local_variables_dont_leak_to_caller() {
    let source = r#"
fn compute(x) {
    local_var = x * 10.0
    return local_var
}

strategy Test {
    state {
        result = 0.0
        leaked = 0.0
    }
    on bar {
        before = 42.0
        result = compute(5.0)
        if before == 42.0 {
            leaked = 0.0
        } else {
            leaked = 1.0
        }
    }
}
"#;

    let mut interp = compile_to_interpreter(source);
    let ctx = bar("AAPL", 100.0, 99.0);
    interp.on_bar(&ctx);

    // The function should return 50.0
    match interp.state.get("result") {
        Some(Value::Float(f)) => assert!(
            (*f - 50.0).abs() < f64::EPSILON,
            "Expected 50.0, got {}",
            f
        ),
        other => panic!("Expected Float(50.0) in state, got {:?}", other),
    }

    // The caller's `before` variable should remain 42.0 (leaked should be 0.0)
    match interp.state.get("leaked") {
        Some(Value::Float(f)) => assert!(
            (*f - 0.0).abs() < f64::EPSILON,
            "Expected leaked=0.0 (no leakage), got {}",
            f
        ),
        other => panic!("Expected Float(0.0) in state, got {:?}", other),
    }
}
