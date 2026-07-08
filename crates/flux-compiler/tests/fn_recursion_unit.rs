//! Unit tests for recursion detection in user-defined functions.
//!
//! **Validates: Requirements 4.1, 4.2, 4.3**
//!
//! Tests that the typechecker correctly detects recursive function calls
//! (direct and mutual) and accepts acyclic call graphs.

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
// Requirement 4.1: Direct self-recursion detected
// ============================================================================

/// A function that calls itself directly is rejected with a recursion error.
#[test]
fn direct_self_recursion_produces_error() {
    let source = r#"
fn foo() {
    foo()
}

strategy Test {
    on bar {
        x = 1.0
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_err(), "Expected recursion error for direct self-recursion");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("recursive call detected") && msg.contains("'foo' calls itself"),
        "Error should report direct self-recursion, got: {}",
        msg
    );
}

// ============================================================================
// Requirement 4.2: Mutual recursion detected with cycle path
// ============================================================================

/// Two functions that call each other (A→B, B→A) are rejected with a cycle error.
#[test]
fn mutual_recursion_two_functions_produces_error() {
    let source = r#"
fn a() {
    b()
}

fn b() {
    a()
}

strategy Test {
    on bar {
        x = 1.0
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_err(), "Expected recursion error for mutual recursion");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("recursive call detected") && msg.contains("cycle"),
        "Error should report mutual recursion cycle, got: {}",
        msg
    );
    // The cycle path should mention both function names
    assert!(
        msg.contains("'a'") && msg.contains("'b'"),
        "Error should mention both functions in the cycle, got: {}",
        msg
    );
}

/// Three functions forming a cycle (A→B, B→C, C→A) are rejected.
#[test]
fn three_function_cycle_produces_error() {
    let source = r#"
fn a() {
    b()
}

fn b() {
    c()
}

fn c() {
    a()
}

strategy Test {
    on bar {
        x = 1.0
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_err(), "Expected recursion error for 3-function cycle");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("recursive call detected") && msg.contains("cycle"),
        "Error should report recursion cycle, got: {}",
        msg
    );
    // The cycle path should mention all three function names
    assert!(
        msg.contains("'a'") && msg.contains("'b'") && msg.contains("'c'"),
        "Error should mention all functions in the 3-function cycle, got: {}",
        msg
    );
}

// ============================================================================
// Requirement 4.3: Acyclic call chains accepted
// ============================================================================

/// An acyclic chain (A→B, B→C, C has no calls) is accepted without error.
#[test]
fn acyclic_chain_accepted() {
    let source = r#"
fn a() {
    b()
}

fn b() {
    c()
}

fn c() {
    x = 1.0
}

strategy Test {
    on bar {
        a()
    }
}
"#;
    let result = typecheck_source(source);
    assert!(
        result.is_ok(),
        "Expected acyclic chain to be accepted, got: {:?}",
        result.err()
    );
}

/// A diamond dependency (A→B, A→C, B→D, C→D) is accepted (no cycle).
#[test]
fn diamond_dependency_accepted() {
    let source = r#"
fn d() {
    x = 1.0
}

fn b() {
    d()
}

fn c() {
    d()
}

fn a() {
    b()
    c()
}

strategy Test {
    on bar {
        a()
    }
}
"#;
    let result = typecheck_source(source);
    assert!(
        result.is_ok(),
        "Expected diamond dependency to be accepted (no cycle), got: {:?}",
        result.err()
    );
}
