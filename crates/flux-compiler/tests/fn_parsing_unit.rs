//! Unit tests for `fn` definition parsing
//!
//! **Validates: Requirements 2.1, 2.2, 2.3, 2.4, 2.5, 2.6**
//!
//! Tests that the parser correctly handles function definitions at the top level,
//! including parameter lists, body statements, error cases, and ordering.

use flux_compiler::error::CompileError;
use flux_compiler::lexer::lex_with_spans;
use flux_compiler::parser::{parse, ExprKind, Stmt};

/// Helper: lex and parse a complete Flux source string.
fn parse_source(source: &str) -> Result<flux_compiler::parser::Program, CompileError> {
    let tokens = lex_with_spans(source)?;
    parse(tokens)
}

/// Helper: extract plain parameter names from an `FnDef` for assertions.
fn param_names(fn_def: &flux_compiler::parser::ast::FnDef) -> Vec<&str> {
    fn_def.params.iter().map(|p| p.name.as_str()).collect()
}

/// Minimal strategy block appended to function definitions for valid programs.
const STRATEGY_SUFFIX: &str = "\nstrategy Test {\n    on bar {\n    }\n}\n";

// ============================================================================
// Happy path tests
// ============================================================================

/// Test minimal: `fn foo() {}` → FnDef with name "foo", empty params, empty body
#[test]
fn minimal_fn_def_empty_params_empty_body() {
    let source = format!("fn foo() {{}}{}", STRATEGY_SUFFIX);
    let program = parse_source(&source).expect("should parse successfully");

    assert_eq!(program.functions.len(), 1);
    let f = &program.functions[0];
    assert_eq!(f.name, "foo");
    assert!(f.params.is_empty());
    assert!(f.body.is_empty());
}

/// Test with params: `fn bar(x, y, z) { return x }` → 3 params, return statement in body
#[test]
fn fn_def_with_params_and_return() {
    let source = format!("fn bar(x, y, z) {{ return x }}{}", STRATEGY_SUFFIX);
    let program = parse_source(&source).expect("should parse successfully");

    assert_eq!(program.functions.len(), 1);
    let f = &program.functions[0];
    assert_eq!(f.name, "bar");
    assert_eq!(param_names(f), vec!["x", "y", "z"]);
    assert_eq!(f.body.len(), 1);
    assert!(matches!(f.body[0], Stmt::Return(_)));
}

/// Test trailing comma: `fn f(a, b,) {}` → 2 params
#[test]
fn fn_def_trailing_comma_in_params() {
    let source = format!("fn f(a, b,) {{}}{}", STRATEGY_SUFFIX);
    let program = parse_source(&source).expect("should parse successfully");

    assert_eq!(program.functions.len(), 1);
    let f = &program.functions[0];
    assert_eq!(f.name, "f");
    assert_eq!(param_names(f), vec!["a", "b"]);
    assert!(f.body.is_empty());
}

/// Test body with multiple statements: assignment, if, function call, return
#[test]
fn fn_def_body_with_multiple_statements() {
    let source = format!(
        r#"fn compute(x, threshold) {{
    result = x + 1.0
    if result > threshold {{
        OPEN(symbol, 100.0)
    }}
    sma(x, 20)
    return result
}}{}"#,
        STRATEGY_SUFFIX
    );
    let program = parse_source(&source).expect("should parse successfully");

    assert_eq!(program.functions.len(), 1);
    let f = &program.functions[0];
    assert_eq!(f.name, "compute");
    assert_eq!(param_names(f), vec!["x", "threshold"]);
    assert_eq!(f.body.len(), 4);

    // Statement 1: assignment
    assert!(matches!(f.body[0], Stmt::Assignment(_)));
    // Statement 2: if
    assert!(matches!(f.body[1], Stmt::If(_)));
    // Statement 3: expression statement (function call)
    assert!(matches!(f.body[2], Stmt::Expr(_)));
    if let Stmt::Expr(ref expr_stmt) = f.body[2] {
        assert!(matches!(expr_stmt.expr.kind, ExprKind::FunctionCall { .. }));
    }
    // Statement 4: return
    assert!(matches!(f.body[3], Stmt::Return(_)));
}

/// Test multiple functions in one file parsed in order
#[test]
fn multiple_functions_parsed_in_order() {
    let source = format!(
        "fn alpha() {{}}\nfn beta(x) {{ return x }}\nfn gamma(a, b) {{}}{}",
        STRATEGY_SUFFIX
    );
    let program = parse_source(&source).expect("should parse successfully");

    assert_eq!(program.functions.len(), 3);
    assert_eq!(program.functions[0].name, "alpha");
    assert_eq!(program.functions[1].name, "beta");
    assert_eq!(program.functions[2].name, "gamma");

    // Verify params
    assert!(program.functions[0].params.is_empty());
    assert_eq!(param_names(&program.functions[1]), vec!["x"]);
    assert_eq!(param_names(&program.functions[2]), vec!["a", "b"]);
}

// ============================================================================
// Error tests
// ============================================================================

/// Test error: `fn` inside strategy body produces parse error
#[test]
fn fn_inside_strategy_body_is_error() {
    let source = r#"strategy Test {
    fn inner() {}
    on bar {
    }
}"#;
    let result = parse_source(source);
    assert!(result.is_err(), "fn inside strategy body should be rejected");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("expected") || msg.contains("error") || msg.contains("unexpected"),
        "Error message should indicate parse failure, got: {}",
        msg
    );
}

/// Test error: nested `fn` inside another function body produces parse error
#[test]
fn nested_fn_inside_function_body_is_error() {
    let source = format!(
        "fn outer() {{ fn inner() {{}} }}{}",
        STRATEGY_SUFFIX
    );
    let result = parse_source(&source);
    assert!(result.is_err(), "nested fn should be rejected");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("expected") || msg.contains("error") || msg.contains("unexpected"),
        "Error message should indicate parse failure, got: {}",
        msg
    );
}

/// Test error: missing opening paren after function name
#[test]
fn fn_missing_open_paren_is_error() {
    let source = format!("fn foo {{}}{}", STRATEGY_SUFFIX);
    let result = parse_source(&source);
    assert!(result.is_err(), "missing open paren should be rejected");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("(") || msg.contains("expected") || msg.contains("paren"),
        "Error should mention missing paren, got: {}",
        msg
    );
}

/// Test error: missing closing brace on function body
#[test]
fn fn_missing_close_brace_is_error() {
    let source = format!("fn foo() {{{}", STRATEGY_SUFFIX);
    let result = parse_source(&source);
    assert!(result.is_err(), "missing close brace should be rejected");
}

// ============================================================================
// Struct type annotations on function signatures (flux-structs Task 2.6)
// **Validates: Requirements 5.1, 5.2**
// ============================================================================

use flux_compiler::parser::ast::TypeAnnotation;

/// `fn calc_spread(q: Quote) -> f64 { ... }` — typed param and return type.
#[test]
fn fn_def_with_typed_param_and_return_type() {
    let source = format!(
        "fn calc_spread(q: Quote) -> f64 {{ return q.ask - q.bid }}{}",
        STRATEGY_SUFFIX
    );
    let program = parse_source(&source).expect("should parse successfully");

    assert_eq!(program.functions.len(), 1);
    let f = &program.functions[0];
    assert_eq!(f.name, "calc_spread");
    assert_eq!(f.params.len(), 1);
    assert_eq!(f.params[0].name, "q");
    assert_eq!(
        f.params[0].param_type,
        Some(TypeAnnotation::Named("Quote".to_string()))
    );
    assert_eq!(f.return_type, Some(TypeAnnotation::F64));
}

/// Untyped functions must still parse exactly as before (backward compatibility).
#[test]
fn fn_def_untyped_params_have_none_type_and_no_return_type() {
    let source = format!("fn add(a, b) {{ return a + b }}{}", STRATEGY_SUFFIX);
    let program = parse_source(&source).expect("should parse successfully");

    assert_eq!(program.functions.len(), 1);
    let f = &program.functions[0];
    assert_eq!(param_names(f), vec!["a", "b"]);
    assert!(f.params.iter().all(|p| p.param_type.is_none()));
    assert!(f.return_type.is_none());
}

/// Mixed typed and untyped params in the same parameter list.
#[test]
fn fn_def_mixed_typed_and_untyped_params() {
    let source = format!(
        "fn mix(a: f64, b, c: int) {{ return a }}{}",
        STRATEGY_SUFFIX
    );
    let program = parse_source(&source).expect("should parse successfully");

    let f = &program.functions[0];
    assert_eq!(f.params.len(), 3);
    assert_eq!(f.params[0].name, "a");
    assert_eq!(f.params[0].param_type, Some(TypeAnnotation::F64));
    assert_eq!(f.params[1].name, "b");
    assert_eq!(f.params[1].param_type, None);
    assert_eq!(f.params[2].name, "c");
    assert_eq!(f.params[2].param_type, Some(TypeAnnotation::Int));
}

/// Return type without any typed params: `fn f() -> bool { ... }`.
#[test]
fn fn_def_return_type_only() {
    let source = format!("fn is_ready() -> bool {{ return true }}{}", STRATEGY_SUFFIX);
    let program = parse_source(&source).expect("should parse successfully");

    let f = &program.functions[0];
    assert!(f.params.is_empty());
    assert_eq!(f.return_type, Some(TypeAnnotation::Bool));
}

/// Struct-typed return type: `fn make_quote() -> Quote { ... }`.
#[test]
fn fn_def_struct_named_return_type() {
    let source = format!(
        "fn make_quote() -> Quote {{ return q }}{}",
        STRATEGY_SUFFIX
    );
    let program = parse_source(&source).expect("should parse successfully");

    let f = &program.functions[0];
    assert_eq!(
        f.return_type,
        Some(TypeAnnotation::Named("Quote".to_string()))
    );
}
