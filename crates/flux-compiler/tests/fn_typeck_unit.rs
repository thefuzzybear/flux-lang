//! Unit tests for typechecker: function registration, arity checking, and body validation.
//!
//! **Validates: Requirements 3.1, 3.2, 3.3, 3.4, 3.5, 7.2, 7.3**
//!
//! Tests that the typechecker correctly registers user-defined functions,
//! validates call arity, detects duplicate definitions, reports undefined
//! function calls, and type-checks function bodies.

use flux_compiler::error::CompileError;
use flux_compiler::lexer::lex_with_spans;
use flux_compiler::parser::parse;
use flux_compiler::typeck::check;

/// Helper: lex, parse, and typecheck a complete Flux source string.
fn typecheck_source(source: &str) -> Result<flux_compiler::typeck::TypedProgram, CompileError> {
    let tokens = lex_with_spans(source)?;
    let ast = parse(tokens)?;
    check(ast)
}

// ============================================================================
// Requirement 3.1, 3.2: Function registered, callable from strategy body
// ============================================================================

/// A single user-defined function registered by the typechecker can be called
/// from the strategy body with the correct number of arguments.
#[test]
fn single_function_registered_callable_from_strategy() {
    let source = r#"
fn foo(x) {
    return x
}

strategy Test {
    on bar {
        foo(1.0)
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_ok(), "Expected Ok, got: {:?}", result.err());
}

/// A function with multiple parameters can be called with matching arguments.
#[test]
fn function_with_multiple_params_callable() {
    let source = r#"
fn add(a, b, c) {
    return a + b + c
}

strategy Test {
    on bar {
        add(1.0, 2.0, 3.0)
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_ok(), "Expected Ok, got: {:?}", result.err());
}

/// A zero-parameter function can be called with no arguments.
#[test]
fn zero_param_function_callable() {
    let source = r#"
fn get_value() {
    return 42.0
}

strategy Test {
    on bar {
        get_value()
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_ok(), "Expected Ok, got: {:?}", result.err());
}

// ============================================================================
// Requirement 3.3, 7.2: Arity mismatch (too few args)
// ============================================================================

/// Calling a function with too few arguments produces an arity error.
#[test]
fn arity_mismatch_too_few_args() {
    let source = r#"
fn foo(x, y) {
    return x + y
}

strategy Test {
    on bar {
        foo(1.0)
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_err(), "Expected arity error for too few args");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("'foo' expects 2 arguments, found 1"),
        "Error should report arity mismatch with name/expected/actual, got: {}",
        msg
    );
}

/// Calling a 3-param function with 0 arguments produces arity error.
#[test]
fn arity_mismatch_zero_args_for_three_params() {
    let source = r#"
fn compute(a, b, c) {
    return a + b + c
}

strategy Test {
    on bar {
        compute()
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_err(), "Expected arity error for zero args");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("'compute' expects 3 arguments, found 0"),
        "Error should report arity mismatch, got: {}",
        msg
    );
}

// ============================================================================
// Requirement 3.3, 7.2: Arity mismatch (too many args)
// ============================================================================

/// Calling a function with too many arguments produces an arity error.
#[test]
fn arity_mismatch_too_many_args() {
    let source = r#"
fn foo(x, y) {
    return x + y
}

strategy Test {
    on bar {
        foo(1.0, 2.0, 3.0)
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_err(), "Expected arity error for too many args");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("'foo' expects 2 arguments, found 3"),
        "Error should report arity mismatch with name/expected/actual, got: {}",
        msg
    );
}

/// Calling a zero-param function with arguments produces arity error.
#[test]
fn arity_mismatch_args_for_zero_params() {
    let source = r#"
fn no_args() {
    return 1.0
}

strategy Test {
    on bar {
        no_args(5.0)
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_err(), "Expected arity error for extra args on zero-param fn");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("'no_args' expects 0 arguments, found 1"),
        "Error should report arity mismatch, got: {}",
        msg
    );
}

// ============================================================================
// Requirement 3.5: Duplicate function name produces error
// ============================================================================

/// Defining two functions with the same name produces a duplicate error.
#[test]
fn duplicate_function_name_produces_error() {
    let source = r#"
fn foo(x) {
    return x
}

fn foo(y) {
    return y
}

strategy Test {
    on bar {
        foo(1.0)
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_err(), "Expected duplicate function error");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("duplicate function definition 'foo'"),
        "Error should report duplicate function name, got: {}",
        msg
    );
}

// ============================================================================
// Requirement 3.4, 7.3: Calling undefined function produces error
// ============================================================================

/// Calling an undefined function produces an error.
#[test]
fn calling_undefined_function_produces_error() {
    let source = r#"
strategy Test {
    on bar {
        undefined_func(1.0)
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_err(), "Expected undefined function error");
    let err = result.unwrap_err();
    let msg = err.to_string();
    // The typechecker reports undefined identifiers
    assert!(
        msg.contains("undefined") && msg.contains("undefined_func"),
        "Error should report the undefined function name, got: {}",
        msg
    );
}

/// Calling a function with a slightly misspelled name produces error mentioning the name.
#[test]
fn calling_misspelled_function_produces_error() {
    let source = r#"
fn calculate(x) {
    return x
}

strategy Test {
    on bar {
        calculat(1.0)
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_err(), "Expected error for misspelled function name");
    let err = result.unwrap_err();
    let msg = err.to_string();
    // Should mention the undefined name
    assert!(
        msg.contains("calculat"),
        "Error should mention the misspelled name, got: {}",
        msg
    );
}

// ============================================================================
// Requirement 3.1, 3.6, 3.7: Function body type-checks successfully
// ============================================================================

/// A function body with valid statements (assignment, arithmetic, return) type-checks.
#[test]
fn function_body_with_valid_statements_typechecks() {
    let source = r#"
fn compute(x, y) {
    sum = x + y
    diff = x - y
    product = sum * diff
    return product
}

strategy Test {
    on bar {
        compute(close, open)
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_ok(), "Expected Ok for valid function body, got: {:?}", result.err());
}

/// A function body accessing bar context variables type-checks.
#[test]
fn function_body_accessing_bar_context_typechecks() {
    let source = r#"
fn check_price() {
    avg = (high + low) / 2.0
    return avg
}

strategy Test {
    on bar {
        check_price()
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_ok(), "Expected Ok for function accessing bar context, got: {:?}", result.err());
}

/// A function body calling built-in math functions type-checks.
#[test]
fn function_body_calling_builtins_typechecks() {
    let source = r#"
fn compute(price, factor) {
    return abs(price) + sqrt(factor)
}

strategy Test {
    on bar {
        compute(close, 2.0)
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_ok(), "Expected Ok for function calling builtins, got: {:?}", result.err());
}

/// A function without a return statement type-checks successfully.
#[test]
fn function_without_return_typechecks() {
    let source = r#"
fn side_effect(x) {
    y = x + 1.0
}

strategy Test {
    on bar {
        side_effect(close)
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_ok(), "Expected Ok for function without return, got: {:?}", result.err());
}

/// A function with conditional logic type-checks.
#[test]
fn function_with_if_else_typechecks() {
    let source = r#"
fn clamp(value, min_val, max_val) {
    if value < min_val {
        return min_val
    }
    if value > max_val {
        return max_val
    }
    return value
}

strategy Test {
    on bar {
        clamp(close, 100.0, 200.0)
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_ok(), "Expected Ok for function with if/else, got: {:?}", result.err());
}

/// A function calling another user-defined function type-checks.
#[test]
fn function_calling_another_user_function_typechecks() {
    let source = r#"
fn helper(x) {
    return x * 2.0
}

fn main_calc(price) {
    doubled = helper(price)
    return doubled + 1.0
}

strategy Test {
    on bar {
        main_calc(close)
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_ok(), "Expected Ok for function calling another fn, got: {:?}", result.err());
}
