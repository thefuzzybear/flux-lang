// Feature: flux-fmt-syntax-highlight, Property 4: Brace Placement
//!
//! Property-based tests verifying that the formatter always places braces correctly:
//! 1. Every `{` appears at the end of a line (trimmed line ends with `{`)
//! 2. Every `}` appears at the start of a line (trimmed line starts with `}`)
//! 3. `} elif` and `} else` patterns: a line starts with indentation followed by `} elif` or `} else`
//! 4. No opening brace `{` appears on its own line (it's always after a declaration)
//! 5. No closing brace `}` appears in the middle of a line (except `} elif`/`} else` patterns)
//!
//! **Validates: Requirements 3.3, 3.4, 3.13**

use proptest::prelude::*;

use flux_compiler::lexer;
use flux_compiler::parser;
use flux_cli::formatter::Formatter;

// =============================================================================
// Generators for Flux programs with block constructs
// =============================================================================

/// Generate a valid Flux identifier (not a keyword)
fn ident_strategy() -> impl Strategy<Value = String> {
    prop::string::string_regex("[a-z][a-z0-9_]{1,7}")
        .unwrap()
        .prop_filter("must not be a keyword", |s| {
            !matches!(
                s.as_str(),
                "strategy" | "params" | "state" | "on"
                    | "if" | "elif" | "else" | "for" | "while"
                    | "return" | "from" | "import" | "and" | "or"
                    | "not" | "true" | "false" | "null" | "in"
                    | "bar"
            )
        })
}

/// Generate a simple expression (integer literal, identifier, or simple binary op)
fn simple_expr_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        (1..100i64).prop_map(|n| n.to_string()),
        Just("close".to_string()),
        Just("open".to_string()),
        Just("volume".to_string()),
        (1..50i64, 1..50i64).prop_map(|(a, b)| format!("{} + {}", a, b)),
        (1..50i64, 1..50i64).prop_map(|(a, b)| format!("{} > {}", a, b)),
    ]
}

/// Generate a simple assignment statement
fn assignment_strategy() -> impl Strategy<Value = String> {
    (ident_strategy(), simple_expr_strategy())
        .prop_map(|(name, expr)| format!("{} = {}", name, expr))
}

/// Generate a simple statement (assignment or function call)
fn simple_stmt_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        assignment_strategy(),
        Just("OPEN(symbol, 100.0)".to_string()),
        Just("CLOSE(symbol)".to_string()),
    ]
}

/// Generate an if statement (with optional elif/else)
fn if_stmt_strategy() -> impl Strategy<Value = String> {
    let base_if = (simple_expr_strategy(), simple_stmt_strategy()).prop_map(|(cond, body)| {
        format!("if {} {{\n{}\n}}", cond, body)
    });

    let with_else = (simple_expr_strategy(), simple_stmt_strategy(), simple_stmt_strategy())
        .prop_map(|(cond, body1, body2)| {
            format!("if {} {{\n{}\n}} else {{\n{}\n}}", cond, body1, body2)
        });

    let with_elif = (
        simple_expr_strategy(),
        simple_stmt_strategy(),
        simple_expr_strategy(),
        simple_stmt_strategy(),
        simple_stmt_strategy(),
    )
        .prop_map(|(cond1, body1, cond2, body2, body3)| {
            format!(
                "if {} {{\n{}\n}} elif {} {{\n{}\n}} else {{\n{}\n}}",
                cond1, body1, cond2, body2, body3
            )
        });

    prop_oneof![base_if, with_else, with_elif]
}

/// Generate a for loop
fn for_loop_strategy() -> impl Strategy<Value = String> {
    (ident_strategy(), simple_stmt_strategy()).prop_map(|(var, body)| {
        format!("for {} in prices {{\n{}\n}}", var, body)
    })
}

/// Generate a while loop
fn while_loop_strategy() -> impl Strategy<Value = String> {
    (simple_expr_strategy(), simple_stmt_strategy()).prop_map(|(cond, body)| {
        format!("while {} {{\n{}\n}}", cond, body)
    })
}

/// Generate statements for the on_bar body
fn on_bar_body_strategy() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec(
        prop_oneof![
            simple_stmt_strategy(),
            if_stmt_strategy(),
            for_loop_strategy(),
            while_loop_strategy(),
        ],
        1..4,
    )
}

/// Generate a params block
fn params_block_strategy() -> impl Strategy<Value = String> {
    prop::collection::vec(
        (ident_strategy(), 1..100i64).prop_map(|(name, val)| format!("    {} = {}", name, val)),
        1..4,
    )
    .prop_map(|params| {
        format!("params {{\n{}\n}}", params.join("\n"))
    })
}

/// Generate a state block
fn state_block_strategy() -> impl Strategy<Value = String> {
    prop::collection::vec(
        (ident_strategy(), 0..50i64).prop_map(|(name, val)| format!("    {} = {}", name, val)),
        1..4,
    )
    .prop_map(|vars| {
        format!("state {{\n{}\n}}", vars.join("\n"))
    })
}

/// Generate an on_bar block
fn on_bar_block_strategy() -> impl Strategy<Value = String> {
    on_bar_body_strategy().prop_map(|stmts| {
        let body = stmts
            .into_iter()
            .map(|s| {
                s.lines()
                    .map(|l| format!("    {}", l))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!("on bar {{\n{}\n}}", body)
    })
}

/// Generate a complete Flux strategy with various blocks
fn strategy_strategy() -> impl Strategy<Value = String> {
    (
        ident_strategy().prop_map(|s| {
            // Capitalize first letter for strategy name
            let mut chars = s.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().to_string() + chars.as_str(),
            }
        }),
        prop::option::of(params_block_strategy()),
        prop::option::of(state_block_strategy()),
        on_bar_block_strategy(),
    )
        .prop_map(|(name, params, state, on_bar)| {
            let mut blocks = Vec::new();
            if let Some(p) = params {
                blocks.push(indent_block(&p));
            }
            if let Some(s) = state {
                blocks.push(indent_block(&s));
            }
            blocks.push(indent_block(&on_bar));

            format!("strategy {} {{\n{}\n}}", name, blocks.join("\n\n"))
        })
}

/// Indent a block by 4 spaces (for strategy body)
fn indent_block(block: &str) -> String {
    block
        .lines()
        .map(|l| {
            if l.is_empty() {
                String::new()
            } else {
                format!("    {}", l)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// =============================================================================
// Helper: parse and format a Flux source string
// =============================================================================

fn format_source(source: &str) -> Option<String> {
    let tokens = lexer::lex_with_spans(source).ok()?;
    let ast = parser::parse(tokens).ok()?;
    Some(Formatter::format(&ast, source))
}

// =============================================================================
// Verification helpers
// =============================================================================

/// Check all brace placement properties on formatted output
fn verify_brace_placement(formatted: &str) -> Result<(), String> {
    for (line_num, line) in formatted.lines().enumerate() {
        let trimmed = line.trim();
        let line_num_display = line_num + 1;

        // Property 1: Every `{` that appears on a line must be at the end (trimmed)
        if trimmed.contains('{') && !trimmed.ends_with('{') {
            return Err(format!(
                "Line {}: Opening brace '{{' not at end of line: {:?}",
                line_num_display, line
            ));
        }

        // Property 4: No opening brace on its own line
        if trimmed == "{" {
            return Err(format!(
                "Line {}: Opening brace '{{' is on its own line: {:?}",
                line_num_display, line
            ));
        }

        // Property 2: Every `}` must be at the start of a trimmed line
        if trimmed.contains('}') && !trimmed.starts_with('}') {
            return Err(format!(
                "Line {}: Closing brace '}}' not at start of line: {:?}",
                line_num_display, line
            ));
        }

        // Property 5: No closing `}` in the middle of a line except for `} elif`/`} else`
        if trimmed.starts_with('}') && trimmed.len() > 1 {
            let after_brace = trimmed[1..].trim_start();
            if !after_brace.is_empty() {
                // Only `elif` and `else` are allowed after `}`
                if !after_brace.starts_with("elif") && !after_brace.starts_with("else") {
                    return Err(format!(
                        "Line {}: Unexpected content after '}}' (only 'elif'/'else' allowed): {:?}",
                        line_num_display, line
                    ));
                }
            }
        }

        // Property 3: `} elif` and `} else` patterns start with indentation + `} elif`/`} else`
        if trimmed.starts_with("} elif") || trimmed.starts_with("} else") {
            // Verify line only contains indentation before the `}`
            let leading_spaces = line.len() - line.trim_start().len();
            let prefix = &line[..leading_spaces];
            if prefix.chars().any(|c| c != ' ') {
                return Err(format!(
                    "Line {}: '}} elif'/'}} else' has non-space indentation: {:?}",
                    line_num_display, line
                ));
            }

            // For `} elif`, verify it ends with `{`
            if trimmed.starts_with("} elif") && !trimmed.ends_with('{') {
                return Err(format!(
                    "Line {}: '}} elif' line doesn't end with '{{': {:?}",
                    line_num_display, line
                ));
            }
            // For `} else`, it should end with `{`
            if trimmed.starts_with("} else") && !trimmed.ends_with('{') {
                return Err(format!(
                    "Line {}: '}} else' line doesn't end with '{{': {:?}",
                    line_num_display, line
                ));
            }
        }
    }

    Ok(())
}

// =============================================================================
// Property 4: Brace Placement
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 3.3, 3.4, 3.13**
    ///
    /// For any valid Flux program containing block constructs, the formatted output
    /// places every opening brace `{` at the end of a line, every closing brace `}`
    /// at the start of a line, and `} elif`/`} else` on the same line as the closing brace.
    #[test]
    fn prop_brace_placement_correct(source in strategy_strategy()) {
        let formatted = format_source(&source);
        prop_assume!(formatted.is_some(), "Source did not parse");
        let formatted = formatted.unwrap();
        let result = verify_brace_placement(&formatted);
        prop_assert!(
            result.is_ok(),
            "Brace placement violation in formatted output:\n{}\n\nFormatted:\n{}\n\nSource:\n{}",
            result.unwrap_err(),
            formatted,
            source
        );
    }

    /// **Validates: Requirements 3.3, 3.4, 3.13**
    ///
    /// Strategies with nested if/elif/else maintain correct brace placement.
    #[test]
    fn prop_brace_placement_nested_if_elif(
        cond1 in simple_expr_strategy(),
        cond2 in simple_expr_strategy(),
        cond3 in simple_expr_strategy(),
        stmt1 in simple_stmt_strategy(),
        stmt2 in simple_stmt_strategy(),
        stmt3 in simple_stmt_strategy(),
    ) {
        let source = format!(
            r#"strategy Test {{
    on bar {{
        if {} {{
            {}
        }} elif {} {{
            {}
        }} else {{
            {}
        }}
        if {} {{
            {}
        }}
    }}
}}"#,
            cond1, stmt1, cond2, stmt2, stmt3, cond3, stmt1
        );

        if let Some(formatted) = format_source(&source) {
            let result = verify_brace_placement(&formatted);
            prop_assert!(
                result.is_ok(),
                "Brace placement violation:\n{}\n\nFormatted:\n{}\n\nSource:\n{}",
                result.unwrap_err(),
                formatted,
                source
            );
        }
    }

    /// **Validates: Requirements 3.3, 3.4, 3.13**
    ///
    /// Strategies with for/while loops maintain correct brace placement.
    #[test]
    fn prop_brace_placement_loops(
        var in ident_strategy(),
        cond in simple_expr_strategy(),
        stmt1 in simple_stmt_strategy(),
        stmt2 in simple_stmt_strategy(),
    ) {
        let source = format!(
            r#"strategy Test {{
    state {{
        prices = []
    }}

    on bar {{
        for {} in prices {{
            {}
        }}
        while {} {{
            {}
        }}
    }}
}}"#,
            var, stmt1, cond, stmt2
        );

        if let Some(formatted) = format_source(&source) {
            let result = verify_brace_placement(&formatted);
            prop_assert!(
                result.is_ok(),
                "Brace placement violation:\n{}\n\nFormatted:\n{}\n\nSource:\n{}",
                result.unwrap_err(),
                formatted,
                source
            );
        }
    }

    /// **Validates: Requirements 3.3, 3.4, 3.13**
    ///
    /// Strategies with all block types (params, state, on_bar) and nested control flow
    /// maintain correct brace placement.
    #[test]
    fn prop_brace_placement_full_strategy(
        param_name in ident_strategy(),
        state_name in ident_strategy(),
        cond in simple_expr_strategy(),
        stmt1 in simple_stmt_strategy(),
        stmt2 in simple_stmt_strategy(),
    ) {
        let source = format!(
            r#"strategy FullTest {{
    params {{
        {} = 20
    }}

    state {{
        {} = 0
    }}

    on bar {{
        if {} {{
            {}
        }} else {{
            {}
        }}
    }}
}}"#,
            param_name, state_name, cond, stmt1, stmt2
        );

        if let Some(formatted) = format_source(&source) {
            let result = verify_brace_placement(&formatted);
            prop_assert!(
                result.is_ok(),
                "Brace placement violation:\n{}\n\nFormatted:\n{}\n\nSource:\n{}",
                result.unwrap_err(),
                formatted,
                source
            );
        }
    }
}
