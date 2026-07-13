//! Unit tests for typechecker: state access rejection and return type inference.
//!
//! **Validates: Requirements 3.6, 3.7, 3.8, 7.5**
//!
//! Tests that:
//! - Functions cannot access state variables (produces specific error)
//! - Functions CAN access bar context variables (close, open, etc.)
//! - Return type is inferred from `return expr`
//! - Return type is Null when no `return` statement is present
//! - Functions can access their own params (params are not state)

use flux_compiler::error::CompileError;
use flux_compiler::lexer::lex_with_spans;
use flux_compiler::parser::parse;
use flux_compiler::typeck::{check, FluxType};

/// Helper: lex, parse, and typecheck a complete Flux source string.
fn typecheck_source(source: &str) -> Result<flux_compiler::typeck::TypedProgram, CompileError> {
    let tokens = lex_with_spans(source)?;
    let ast = parse(tokens)?;
    check(ast)
}

// ============================================================================
// Requirement 3.8, 7.5: State access in function body → error
// ============================================================================

/// A function that accesses a state variable produces a specific error naming that variable.
#[test]
fn function_accessing_state_var_produces_error() {
    let source = r#"
fn bad_fn() {
    return count
}

strategy Test {
    state {
        count = 0
    }

    on bar {
        bad_fn()
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_err(), "Expected error when function accesses state variable");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("functions cannot access state variable 'count'"),
        "Error should name the state variable, got: {}",
        msg
    );
}

/// A function accessing a different state variable names that variable in the error.
#[test]
fn function_accessing_different_state_var_names_it() {
    let source = r#"
fn compute() {
    return total_pnl
}

strategy Test {
    state {
        total_pnl = 0.0
    }

    on bar {
        compute()
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_err(), "Expected error when function accesses state variable");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("functions cannot access state variable 'total_pnl'"),
        "Error should name 'total_pnl', got: {}",
        msg
    );
}

/// A function that assigns to a state variable is also rejected.
#[test]
fn function_assigning_state_var_produces_error() {
    let source = r#"
fn increment() {
    count = count + 1
}

strategy Test {
    state {
        count = 0
    }

    on bar {
        increment()
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_err(), "Expected error when function accesses (assigns) state variable");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("functions cannot access state variable 'count'"),
        "Error should mention state variable 'count', got: {}",
        msg
    );
}

// ============================================================================
// Requirement 3.8: Bar context variables ARE accessible in functions
// ============================================================================

/// A function accessing bar context variable `close` is accepted.
#[test]
fn function_accessing_close_is_accepted() {
    let source = r#"
fn get_close() {
    return close
}

strategy Test {
    on bar {
        get_close()
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_ok(), "Expected Ok for function accessing close, got: {:?}", result.err());
}

/// A function accessing bar context variable `open` is accepted.
#[test]
fn function_accessing_open_is_accepted() {
    let source = r#"
fn get_open() {
    return open
}

strategy Test {
    on bar {
        get_open()
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_ok(), "Expected Ok for function accessing open, got: {:?}", result.err());
}

/// A function accessing multiple bar context variables (high, low, volume) is accepted.
#[test]
fn function_accessing_multiple_bar_context_vars_is_accepted() {
    let source = r#"
fn compute_range() {
    price_range = high - low
    return price_range
}

strategy Test {
    on bar {
        compute_range()
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_ok(), "Expected Ok for function accessing high/low, got: {:?}", result.err());
}

/// A function accessing `in_position` bar context is accepted.
#[test]
fn function_accessing_in_position_is_accepted() {
    let source = r#"
fn should_trade() {
    if not in_position {
        return 1.0
    }
    return 0.0
}

strategy Test {
    on bar {
        should_trade()
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_ok(), "Expected Ok for function accessing in_position, got: {:?}", result.err());
}

// ============================================================================
// Requirement 3.7: Return type inferred from `return expr`
// ============================================================================

/// A function with `return <float expr>` has return type Float.
#[test]
fn function_return_float_expr_has_float_return_type() {
    let source = r#"
fn compute(x) {
    return x + 1.0
}

strategy Test {
    on bar {
        compute(close)
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_ok(), "Expected Ok, got: {:?}", result.err());
    let program = result.unwrap();
    assert_eq!(program.functions.len(), 1);
    assert_eq!(program.functions[0].return_type, FluxType::Float);
}

/// A function with `return <int literal>` has return type Int.
#[test]
fn function_return_int_literal_has_int_return_type() {
    let source = r#"
fn get_count() {
    return 42
}

strategy Test {
    on bar {
        get_count()
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_ok(), "Expected Ok, got: {:?}", result.err());
    let program = result.unwrap();
    assert_eq!(program.functions.len(), 1);
    assert_eq!(program.functions[0].return_type, FluxType::Int);
}

/// A function with `return <bool expr>` has return type Bool.
#[test]
fn function_return_bool_expr_has_bool_return_type() {
    let source = r#"
fn is_positive(x) {
    return x > 0.0
}

strategy Test {
    on bar {
        is_positive(close)
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_ok(), "Expected Ok, got: {:?}", result.err());
    let program = result.unwrap();
    assert_eq!(program.functions.len(), 1);
    assert_eq!(program.functions[0].return_type, FluxType::Bool);
}

// ============================================================================
// Requirement 3.6: Function without `return` has return type Null
// ============================================================================

/// A function without any `return` statement has return type Null.
#[test]
fn function_without_return_has_null_return_type() {
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
    assert!(result.is_ok(), "Expected Ok, got: {:?}", result.err());
    let program = result.unwrap();
    assert_eq!(program.functions.len(), 1);
    assert_eq!(program.functions[0].return_type, FluxType::Null);
}

/// A function with only assignments (no return) has return type Null.
#[test]
fn function_with_only_assignments_has_null_return_type() {
    let source = r#"
fn process(a, b) {
    x = a + b
    y = x * 2.0
}

strategy Test {
    on bar {
        process(close, open)
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_ok(), "Expected Ok, got: {:?}", result.err());
    let program = result.unwrap();
    assert_eq!(program.functions.len(), 1);
    assert_eq!(program.functions[0].return_type, FluxType::Null);
}

// ============================================================================
// Params are not state: functions CAN access their own parameters
// ============================================================================

/// A function using its own parameters is accepted (params are not state).
#[test]
fn function_accessing_params_is_accepted() {
    let source = r#"
fn add(x, y) {
    return x + y
}

strategy Test {
    state {
        count = 0
    }

    on bar {
        add(1.0, 2.0)
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_ok(), "Expected Ok for function accessing its own params, got: {:?}", result.err());
}

/// A function whose parameter has the same name as a state variable uses
/// the parameter (shadow), not the state variable — no error.
#[test]
fn function_param_shadows_state_var_is_accepted() {
    let source = r#"
fn use_count(count) {
    return count + 1.0
}

strategy Test {
    state {
        count = 0
    }

    on bar {
        use_count(5.0)
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_ok(), "Expected Ok when param shadows state var, got: {:?}", result.err());
}
