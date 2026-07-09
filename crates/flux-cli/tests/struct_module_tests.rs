//! Unit tests for struct module integration (Task 18.4).
//!
//! Validates:
//! - `from market::l1 import {Tick, Quote, Bar}` resolves correctly
//! - `from market::l2 import {Level, Book}` resolves correctly
//! - Using `Book` without importing it produces a helpful suggestion error
//!
//! Requirements: 20.1, 20.2, 20.4

use flux_cli::module_resolver::{resolve_modules, suggest_import_for_struct, is_stdlib_struct_module};

/// Helper: parse a source string into a Program AST.
fn parse_source(source: &str) -> flux_compiler::parser::ast::Program {
    let tokens = flux_compiler::lexer::lex_with_spans(source).expect("lex failed");
    flux_compiler::parser::parse(tokens).expect("parse failed")
}

/// Helper: find the workspace root (directory containing the top-level Cargo.toml with [workspace]).
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

// =============================================================================
// Task 18.4: Test that `from market::l1 import {Tick, Quote, Bar}` resolves correctly
// =============================================================================

#[test]
fn test_market_l1_import_tick_quote_bar_resolves() {
    let root = workspace_root();
    let source = r#"
from market::l1 import {Tick, Quote, Bar}

strategy TestStrategy {
    on bar {
        x = 1.0
    }
}
"#;
    let program = parse_source(source);

    // Resolve modules from the workspace root so it can find std/market/l1.flux
    let resolved = resolve_modules(program, root.as_path())
        .expect("module resolution should succeed for market::l1 imports");

    // After resolution, program.structs should contain Tick, Quote, and Bar
    let struct_names: Vec<&str> = resolved.structs.iter().map(|s| s.name.as_str()).collect();
    assert!(struct_names.contains(&"Tick"), "Tick should be in resolved structs, got: {:?}", struct_names);
    assert!(struct_names.contains(&"Quote"), "Quote should be in resolved structs, got: {:?}", struct_names);
    assert!(struct_names.contains(&"Bar"), "Bar should be in resolved structs, got: {:?}", struct_names);

    // market::l1 import should be consumed (not left in program.imports)
    assert!(
        resolved.imports.is_empty(),
        "stdlib struct imports should be consumed by resolver, remaining: {:?}",
        resolved.imports.iter().map(|i| &i.module_path).collect::<Vec<_>>()
    );
}

#[test]
fn test_market_l1_import_functions_resolves() {
    let root = workspace_root();
    let source = r#"
from market::l1 import {Quote, calc_spread, calc_mid}

strategy TestStrategy {
    on bar {
        x = 1.0
    }
}
"#;
    let program = parse_source(source);
    let resolved = resolve_modules(program, root.as_path())
        .expect("module resolution should succeed for market::l1 function imports");

    // Quote struct should be merged into program.structs
    let struct_names: Vec<&str> = resolved.structs.iter().map(|s| s.name.as_str()).collect();
    assert!(struct_names.contains(&"Quote"), "Quote should be in resolved structs");

    // calc_spread and calc_mid should be merged into program.functions
    let fn_names: Vec<&str> = resolved.functions.iter().map(|f| f.name.as_str()).collect();
    assert!(fn_names.contains(&"calc_spread"), "calc_spread should be in resolved functions");
    assert!(fn_names.contains(&"calc_mid"), "calc_mid should be in resolved functions");
}

// =============================================================================
// Task 18.4: Test that `from market::l2 import {Level, Book}` resolves correctly
// =============================================================================

#[test]
fn test_market_l2_import_level_book_resolves() {
    let root = workspace_root();
    let source = r#"
from market::l2 import {Level, Book}

strategy TestStrategy {
    on bar {
        x = 1.0
    }
}
"#;
    let program = parse_source(source);
    let resolved = resolve_modules(program, root.as_path())
        .expect("module resolution should succeed for market::l2 imports");

    // After resolution, program.structs should contain Level and Book
    let struct_names: Vec<&str> = resolved.structs.iter().map(|s| s.name.as_str()).collect();
    assert!(struct_names.contains(&"Level"), "Level should be in resolved structs, got: {:?}", struct_names);
    assert!(struct_names.contains(&"Book"), "Book should be in resolved structs, got: {:?}", struct_names);
}

#[test]
fn test_market_l2_book_depends_on_level() {
    let root = workspace_root();
    let source = r#"
from market::l2 import {Book}

strategy TestStrategy {
    on bar {
        x = 1.0
    }
}
"#;
    let program = parse_source(source);
    let resolved = resolve_modules(program, root.as_path())
        .expect("module resolution should succeed");

    // Importing Book should also pull in Level as a dependency
    let struct_names: Vec<&str> = resolved.structs.iter().map(|s| s.name.as_str()).collect();
    assert!(struct_names.contains(&"Book"), "Book should be in resolved structs");
    assert!(struct_names.contains(&"Level"), "Level should be automatically included as a dependency of Book");

    // Level should appear before Book (dependency order)
    let level_pos = struct_names.iter().position(|&n| n == "Level").unwrap();
    let book_pos = struct_names.iter().position(|&n| n == "Book").unwrap();
    assert!(level_pos < book_pos, "Level should be ordered before Book (dependency order)");
}

// =============================================================================
// Task 18.4: Test that using Book without importing it produces a helpful suggestion error
// =============================================================================

#[test]
fn test_unimported_struct_type_produces_suggestion_error() {
    // This tests the typechecker's error message when a struct type is used
    // without being imported (Requirement 20.4).
    let source = r#"
struct Container {
    content: Book
}

strategy TestStrategy {
    on bar {
        x = 1.0
    }
}
"#;
    let tokens = flux_compiler::lexer::lex_with_spans(source).expect("lex failed");
    let program = flux_compiler::parser::parse(tokens).expect("parse failed");
    let result = flux_compiler::typeck::check(program);

    assert!(result.is_err(), "Using Book without importing should produce an error");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("type 'Book' is not defined"),
        "Error should mention type is not defined, got: {}",
        err_msg
    );
    assert!(
        err_msg.contains("from market::l2 import {Book}"),
        "Error should suggest the correct import path, got: {}",
        err_msg
    );
}

#[test]
fn test_unimported_tick_produces_suggestion_error() {
    let source = r#"
fn process(t: Tick) -> f64 {
    return t.price
}

strategy TestStrategy {
    on bar {
        x = 1.0
    }
}
"#;
    let tokens = flux_compiler::lexer::lex_with_spans(source).expect("lex failed");
    let program = flux_compiler::parser::parse(tokens).expect("parse failed");
    let result = flux_compiler::typeck::check(program);

    assert!(result.is_err(), "Using Tick without importing should produce an error");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("type 'Tick' is not defined"),
        "Error should mention type is not defined, got: {}",
        err_msg
    );
    assert!(
        err_msg.contains("from market::l1 import {Tick}"),
        "Error should suggest the correct import path, got: {}",
        err_msg
    );
}

// =============================================================================
// Additional integration: struct-typed arguments at call sites (Task 18.2)
// =============================================================================

#[test]
fn test_struct_typed_function_args_work_with_imports() {
    let root = workspace_root();
    let source = r#"
from market::l1 import {Quote, calc_spread}

strategy TestStrategy {
    on bar {
        q = Quote { bid = 100.0, bid_size = 10.0, ask = 101.0, ask_size = 5.0, timestamp = 0.0 }
        spread = calc_spread(q)
    }
}
"#;
    let program = parse_source(source);
    let resolved = resolve_modules(program, root.as_path())
        .expect("module resolution should succeed");

    // Now typecheck — this validates that calc_spread(q: Quote) works with Quote in scope
    let result = flux_compiler::typeck::check(resolved);
    assert!(
        result.is_ok(),
        "Typechecking should succeed when struct type and helper function are both imported: {:?}",
        result.err()
    );
}

#[test]
fn test_market_l2_functions_work_with_struct_imports() {
    let root = workspace_root();
    let source = r#"
from market::l2 import {Level, Book, book_spread_bps}

strategy TestStrategy {
    on bar {
        x = 1.0
    }
}
"#;
    let program = parse_source(source);
    let resolved = resolve_modules(program, root.as_path())
        .expect("module resolution should succeed");

    // Verify book_spread_bps function is available
    let fn_names: Vec<&str> = resolved.functions.iter().map(|f| f.name.as_str()).collect();
    assert!(fn_names.contains(&"book_spread_bps"), "book_spread_bps should be resolved");

    // Typecheck should pass
    let result = flux_compiler::typeck::check(resolved);
    assert!(
        result.is_ok(),
        "Typechecking should succeed with L2 struct and function imports: {:?}",
        result.err()
    );
}

// =============================================================================
// Helper function tests
// =============================================================================

#[test]
fn test_suggest_import_for_struct_known_types() {
    assert_eq!(suggest_import_for_struct("Tick"), Some("market::l1"));
    assert_eq!(suggest_import_for_struct("Quote"), Some("market::l1"));
    assert_eq!(suggest_import_for_struct("Bar"), Some("market::l1"));
    assert_eq!(suggest_import_for_struct("MarketSnapshot"), Some("market::l1"));
    assert_eq!(suggest_import_for_struct("Level"), Some("market::l2"));
    assert_eq!(suggest_import_for_struct("Book"), Some("market::l2"));
    assert_eq!(suggest_import_for_struct("QuoteWindow"), Some("collections::buffers"));
    assert_eq!(suggest_import_for_struct("BarWindow"), Some("collections::buffers"));
}

#[test]
fn test_suggest_import_for_struct_unknown_types() {
    assert_eq!(suggest_import_for_struct("UnknownType"), None);
    assert_eq!(suggest_import_for_struct("MyStruct"), None);
}

#[test]
fn test_is_stdlib_struct_module() {
    assert!(is_stdlib_struct_module("market::l1"));
    assert!(is_stdlib_struct_module("market::l2"));
    assert!(is_stdlib_struct_module("collections::buffers"));
    assert!(!is_stdlib_struct_module("indicators"));
    assert!(!is_stdlib_struct_module("helpers::common"));
    assert!(!is_stdlib_struct_module("unknown::module"));
}
