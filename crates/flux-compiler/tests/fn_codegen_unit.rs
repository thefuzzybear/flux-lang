//! Unit tests for codegen function emission
//!
//! **Validates: Requirements 6.1, 6.2, 6.3, 6.4, 6.5, 6.6**
//!
//! Tests that the code generator correctly emits Rust `fn` definitions for
//! user-defined functions, with appropriate parameter threading for bar
//! context and signals based on function body content and call graph.

use flux_compiler::compile;

/// Test: function with no ctx/signals → plain `fn name(params) { ... }`
///
/// A pure function that only uses its parameters should not receive
/// ctx or signals parameters.
#[test]
fn pure_function_no_ctx_no_signals() {
    let source = r#"
fn add(x, y) {
    return x + y
}

strategy Test {
    on bar {
        z = add(1.0, 2.0)
    }
}
"#;
    let output = compile(source).expect("compilation should succeed");

    // Should emit fn add(x: f64, y: f64) -> f64 { ... }
    assert!(
        output.contains("fn add(x: f64, y: f64) -> f64"),
        "Expected pure function signature without ctx/signals. Got:\n{}",
        output
    );
    // Should NOT contain ctx or signals in the add function signature
    // Find the line with "fn add" and check it doesn't have ctx or signals
    let add_line = output
        .lines()
        .find(|l| l.contains("fn add("))
        .expect("should find fn add");
    assert!(
        !add_line.contains("ctx"),
        "Pure function should not have ctx param. Line: {}",
        add_line
    );
    assert!(
        !add_line.contains("signals"),
        "Pure function should not have signals param. Line: {}",
        add_line
    );
}

/// Test: function accessing `close` → gets `ctx: &BarContext` param, body uses `ctx.close`
///
/// A function that references market data variables should receive
/// ctx: &BarContext and resolve those variables through ctx.
#[test]
fn function_accessing_close_gets_ctx_param() {
    let source = r#"
fn spread() {
    return close
}

strategy Test {
    on bar {
        s = spread()
    }
}
"#;
    let output = compile(source).expect("compilation should succeed");

    // Should emit fn spread(ctx: &BarContext) -> f64 { ... }
    assert!(
        output.contains("fn spread(ctx: &BarContext)"),
        "Function accessing close should get ctx param. Got:\n{}",
        output
    );
    // Body should use ctx.close
    assert!(
        output.contains("ctx.close"),
        "Function body should resolve close as ctx.close. Got:\n{}",
        output
    );
}

/// Test: function emitting OPEN → gets `signals: &mut Vec<Signal>` param
///
/// A function that emits signals should receive signals parameter.
#[test]
fn function_emitting_open_gets_signals_param() {
    let source = r#"
fn emit_buy() {
    OPEN(symbol, 100.0)
}

strategy Test {
    on bar {
        emit_buy()
    }
}
"#;
    let output = compile(source).expect("compilation should succeed");

    // Should have signals param (and ctx because it uses `symbol`)
    let fn_line = output
        .lines()
        .find(|l| l.contains("fn emit_buy("))
        .expect("should find fn emit_buy");
    assert!(
        fn_line.contains("signals: &mut Vec<Signal>"),
        "Function emitting signal should get signals param. Line: {}",
        fn_line
    );
    // Body should contain signals.push(Signal::open(...))
    assert!(
        output.contains("signals.push(Signal::open("),
        "Function body should push signal. Got:\n{}",
        output
    );
}

/// Test: function calling another function that needs ctx → caller also gets ctx, forwards it
///
/// Transitive context propagation: if a function calls another function
/// that needs ctx, the caller should also receive ctx and forward it.
#[test]
fn caller_forwards_ctx_to_callee() {
    let source = r#"
fn get_price() {
    return close
}

fn wrapper() {
    return get_price()
}

strategy Test {
    on bar {
        p = wrapper()
    }
}
"#;
    let output = compile(source).expect("compilation should succeed");

    // get_price should have ctx
    assert!(
        output.contains("fn get_price(ctx: &BarContext)"),
        "get_price should have ctx param. Got:\n{}",
        output
    );

    // wrapper should also have ctx (transitive)
    let wrapper_line = output
        .lines()
        .find(|l| l.contains("fn wrapper("))
        .expect("should find fn wrapper");
    assert!(
        wrapper_line.contains("ctx: &BarContext"),
        "wrapper should get ctx transitively. Line: {}",
        wrapper_line
    );

    // wrapper's call to get_price should forward ctx
    assert!(
        output.contains("get_price(ctx)"),
        "wrapper should forward ctx to get_price. Got:\n{}",
        output
    );
}

/// Test: function emitted before strategy struct in output
///
/// User-defined functions should appear in the generated code before
/// the strategy struct definition.
#[test]
fn function_emitted_before_strategy_struct() {
    let source = r#"
fn helper(x) {
    return x
}

strategy MyStrat {
    on bar {
        v = helper(1.0)
    }
}
"#;
    let output = compile(source).expect("compilation should succeed");

    let fn_pos = output
        .find("fn helper(")
        .expect("should find fn helper in output");
    let struct_pos = output
        .find("pub struct MyStrat")
        .expect("should find pub struct MyStrat in output");
    assert!(
        fn_pos < struct_pos,
        "Function should appear before strategy struct. fn_pos={}, struct_pos={}",
        fn_pos,
        struct_pos
    );
}

/// Test: return statement emits `return value;`
///
/// The return statement should emit a proper Rust return statement.
#[test]
fn return_statement_emits_return_value() {
    let source = r#"
fn add(x, y) {
    return x + y
}

strategy Test {
    on bar {
        z = add(1.0, 2.0)
    }
}
"#;
    let output = compile(source).expect("compilation should succeed");

    // Should contain "return (x + y);" or "return x + y;"
    // The codegen emits binary ops, so let's check for "return" with the expression
    assert!(
        output.contains("return "),
        "Should emit return statement. Got:\n{}",
        output
    );
    // The return statement should end with a semicolon
    let return_line = output
        .lines()
        .find(|l| l.trim().starts_with("return "))
        .expect("should find a return statement line");
    assert!(
        return_line.trim().ends_with(';'),
        "Return statement should end with semicolon. Line: {}",
        return_line
    );
    // The return expression should contain the binary operation
    assert!(
        return_line.contains("+"),
        "Return should contain the addition. Line: {}",
        return_line
    );
}
