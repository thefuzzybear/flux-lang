//! Property-based tests for the `range()` built-in function.
//!
//! Feature: for-loop-iteration, Property 6: Range function produces correct sequence
//!
//! Validates that `range(start, end)` returns:
//! - `[start, start+1, ..., end-1]` when `start < end`
//! - An empty list when `start >= end`
//!
//! **Validates: Requirements 6.1, 6.2, 6.3**

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
        symbol: "TEST".to_string(),
        close: 100.0,
        open: 99.0,
        high: 101.0,
        low: 98.0,
        volume: 1000.0,
        in_position: false,
    }
}

// =============================================================================
// Property 6: Range function produces correct sequence
// Feature: for-loop-iteration, Property 6: Range function produces correct sequence
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 6.1, 6.2, 6.3**
    ///
    /// Property 6: Range function produces correct sequence
    ///
    /// For any pair of integers (start, end), `range(start, end)` SHALL return
    /// a list equal to `[start, start+1, ..., end-1]` when `start < end`,
    /// and an empty list otherwise.
    #[test]
    fn prop_range_produces_correct_sequence(
        start in -100i16..100i16,
        end in -100i16..100i16,
    ) {
        let source = format!(
            r#"strategy RangeTest {{
    state {{
        result_len = 0
        result_correct = 1
    }}
    on bar {{
        result = range({start}, {end})
        result_len = result.len()
        i = 0
        while i < result_len {{
            expected_val = {start} + i
            if result[i] != expected_val {{
                result_correct = 0
            }}
            i = i + 1
        }}
    }}
}}
"#,
            start = start as i64,
            end = end as i64,
        );

        let mut interp = compile_to_interpreter(&source);
        interp.on_bar(&test_bar());

        // Check the length
        let actual_len = match interp.state.get("result_len") {
            Some(Value::Int(n)) => *n,
            other => panic!("Expected Int for 'result_len', got {:?}", other),
        };

        let expected_len = if (start as i64) < (end as i64) {
            (end as i64) - (start as i64)
        } else {
            0
        };

        prop_assert_eq!(
            actual_len, expected_len,
            "range({}, {}) length: expected {}, got {}",
            start, end, expected_len, actual_len
        );

        // Check that all elements are correct (validated inside Flux)
        let correct = match interp.state.get("result_correct") {
            Some(Value::Int(n)) => *n,
            other => panic!("Expected Int for 'result_correct', got {:?}", other),
        };

        prop_assert_eq!(
            correct, 1,
            "range({}, {}) produced incorrect element values",
            start, end
        );
    }
}
