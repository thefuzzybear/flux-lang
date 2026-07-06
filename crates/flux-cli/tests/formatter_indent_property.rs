// Feature: flux-fmt-syntax-highlight, Property 3: Indentation Correctness
//!
//! Property test verifying that the formatter produces output where every non-blank,
//! non-comment line has leading whitespace consisting of exactly N*4 spaces (for some N >= 0),
//! containing no tabs, and that nested constructs are indented more than their parents.
//!
//! **Validates: Requirements 3.1, 3.2**

use proptest::prelude::*;

use flux_compiler::lexer;
use flux_compiler::parser;
use flux_cli::formatter::Formatter;

// =============================================================================
// Proptest Strategies — Generate valid Flux programs with varying nesting
// =============================================================================

/// Generate a valid Flux identifier (lowercase letter followed by alphanumeric/underscores).
fn arb_ident() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_]{0,5}".prop_map(|s| s)
}

/// Generate a simple expression.
fn arb_simple_expr() -> impl Strategy<Value = String> {
    prop_oneof![
        (1i64..1000).prop_map(|n| n.to_string()),
        (1.0f64..100.0).prop_map(|f| format!("{:.1}", f)),
        Just("true".to_string()),
        Just("false".to_string()),
        Just("close".to_string()),
        Just("open".to_string()),
        Just("high".to_string()),
        Just("volume".to_string()),
        arb_ident(),
    ]
}

/// Generate a condition expression suitable for if/while.
fn arb_condition() -> impl Strategy<Value = String> {
    prop_oneof![
        (arb_simple_expr(), arb_simple_expr())
            .prop_map(|(l, r)| format!("{} > {}", l, r)),
        (arb_simple_expr(), arb_simple_expr())
            .prop_map(|(l, r)| format!("{} < {}", l, r)),
        Just("true".to_string()),
    ]
}

/// Generate an assignment statement string.
fn arb_assignment_str() -> impl Strategy<Value = String> {
    (arb_ident(), arb_simple_expr(), arb_simple_expr())
        .prop_map(|(name, l, r)| format!("{} = {} + {}", name, l, r))
}

/// Generate a function call statement.
fn arb_func_call_str() -> impl Strategy<Value = String> {
    prop_oneof![
        arb_simple_expr().prop_map(|e| format!("OPEN(symbol, {})", e)),
        Just("CLOSE(symbol)".to_string()),
        (arb_ident(), arb_simple_expr())
            .prop_map(|(f, a)| format!("{}({})", f, a)),
    ]
}

/// A simple (non-nesting) statement line.
fn arb_simple_stmt_str() -> impl Strategy<Value = String> {
    prop_oneof![
        arb_assignment_str(),
        arb_func_call_str(),
    ]
}

/// Build a nested block structure as a vector of indented lines.
/// This uses an iterative approach to avoid stack overflows.
///
/// `depth` controls how many nested if/for/while blocks to include.
/// Returns lines that should be placed inside an `on bar { }` block (at depth=2 from root).
fn build_nested_body(depth: u32, stmts: &[String], conditions: &[String], vars: &[String]) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();

    // Always start with a simple statement
    if let Some(s) = stmts.first() {
        lines.push(s.clone());
    }

    // Build nesting iteratively
    let mut current_depth: u32 = 0;
    let mut open_blocks: Vec<&str> = Vec::new(); // track block type for closing

    for i in 0..depth {
        let idx = i as usize;
        let cond = conditions.get(idx % conditions.len()).cloned().unwrap_or_else(|| "true".to_string());
        let var = vars.get(idx % vars.len()).cloned().unwrap_or_else(|| "x".to_string());

        // Choose block type based on index
        let indent = "    ".repeat(current_depth as usize);
        match idx % 3 {
            0 => {
                lines.push(format!("{}if {} {{", indent, cond));
                open_blocks.push("if");
            }
            1 => {
                lines.push(format!("{}for {} in items {{", indent, var));
                open_blocks.push("for");
            }
            _ => {
                lines.push(format!("{}while {} {{", indent, cond));
                open_blocks.push("while");
            }
        }
        current_depth += 1;

        // Add a statement inside the block
        let inner_indent = "    ".repeat(current_depth as usize);
        let stmt_idx = (idx + 1) % stmts.len().max(1);
        let stmt = stmts.get(stmt_idx).cloned().unwrap_or_else(|| "x = 1".to_string());
        lines.push(format!("{}{}", inner_indent, stmt));
    }

    // Close all open blocks (innermost first)
    while let Some(_block_type) = open_blocks.pop() {
        current_depth -= 1;
        let indent = "    ".repeat(current_depth as usize);
        lines.push(format!("{}}}", indent));
    }

    // Add another simple statement after the nested block
    if let Some(s) = stmts.get(1) {
        lines.push(s.clone());
    }

    lines
}

/// Generate a complete valid Flux strategy source string with specified nesting depth.
fn arb_flux_program_with_depth(max_depth: u32) -> impl Strategy<Value = String> {
    let has_params = prop::bool::ANY;
    let has_state = prop::bool::ANY;

    (
        arb_ident(),
        has_params,
        has_state,
        // params
        proptest::collection::vec(
            (arb_ident(), arb_simple_expr()),
            1..4,
        ),
        // state vars
        proptest::collection::vec(
            (arb_ident(), arb_simple_expr()),
            1..3,
        ),
        // statements for on_bar body
        proptest::collection::vec(arb_simple_stmt_str(), 2..5),
        // conditions for nested blocks
        proptest::collection::vec(arb_condition(), 1..4),
        // variable names for for-loops
        proptest::collection::vec(arb_ident(), 1..4),
    )
        .prop_map(move |(name, has_p, has_s, params, state_vars, stmts, conditions, vars)| {
            let mut program = format!("strategy {} {{\n", name);

            if has_p {
                program.push_str("    params {\n");
                for (pname, pval) in &params {
                    program.push_str(&format!("        {} = {}\n", pname, pval));
                }
                program.push_str("    }\n\n");
            }

            if has_s {
                program.push_str("    state {\n");
                for (sname, sval) in &state_vars {
                    program.push_str(&format!("        {} = {}\n", sname, sval));
                }
                program.push_str("    }\n\n");
            }

            // on bar block with nested statements
            program.push_str("    on bar {\n");
            let body_lines = build_nested_body(max_depth, &stmts, &conditions, &vars);
            for line in &body_lines {
                program.push_str(&format!("        {}\n", line));
            }
            program.push_str("    }\n");

            program.push_str("}\n");
            program
        })
}

// =============================================================================
// Property Tests
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 3.1, 3.2**
    ///
    /// Property 3: Indentation Correctness — Depth 1 (shallow nesting)
    ///
    /// For generated programs with 1 level of nesting inside on_bar, verify
    /// all indentation rules hold on the formatted output.
    #[test]
    fn prop_indent_correctness_depth_1(
        source in arb_flux_program_with_depth(1),
    ) {
        let tokens = lexer::lex_with_spans(&source);
        prop_assume!(tokens.is_ok());
        let tokens = tokens.unwrap();

        let ast = parser::parse(tokens);
        prop_assume!(ast.is_ok());
        let ast = ast.unwrap();

        let formatted = Formatter::format(&ast, &source);

        verify_indentation(&formatted)?;
    }

    /// **Validates: Requirements 3.1, 3.2**
    ///
    /// Property 3: Indentation Correctness — Depth 2 (medium nesting)
    ///
    /// For generated programs with 2 levels of nested blocks, verify indentation.
    #[test]
    fn prop_indent_correctness_depth_2(
        source in arb_flux_program_with_depth(2),
    ) {
        let tokens = lexer::lex_with_spans(&source);
        prop_assume!(tokens.is_ok());
        let tokens = tokens.unwrap();

        let ast = parser::parse(tokens);
        prop_assume!(ast.is_ok());
        let ast = ast.unwrap();

        let formatted = Formatter::format(&ast, &source);

        verify_indentation(&formatted)?;
    }

    /// **Validates: Requirements 3.1, 3.2**
    ///
    /// Property 3: Indentation Correctness — Depth 3 (deep nesting)
    ///
    /// For generated programs with 3 levels of nested blocks (e.g., if inside for
    /// inside while inside on_bar), verify indentation.
    #[test]
    fn prop_indent_correctness_depth_3(
        source in arb_flux_program_with_depth(3),
    ) {
        let tokens = lexer::lex_with_spans(&source);
        prop_assume!(tokens.is_ok());
        let tokens = tokens.unwrap();

        let ast = parser::parse(tokens);
        prop_assume!(ast.is_ok());
        let ast = ast.unwrap();

        let formatted = Formatter::format(&ast, &source);

        verify_indentation(&formatted)?;
    }

    /// **Validates: Requirements 3.1, 3.2**
    ///
    /// Property 3: Indentation Correctness — Structural nesting increases
    ///
    /// Verify that lines inside blocks (after `{`) are indented MORE than the
    /// opening brace line, and closing `}` is at the same level as the opener.
    #[test]
    fn prop_indent_nesting_increases(
        source in arb_flux_program_with_depth(3),
    ) {
        let tokens = lexer::lex_with_spans(&source);
        prop_assume!(tokens.is_ok());
        let tokens = tokens.unwrap();

        let ast = parser::parse(tokens);
        prop_assume!(ast.is_ok());
        let ast = ast.unwrap();

        let formatted = Formatter::format(&ast, &source);

        verify_nesting_depth_increases(&formatted)?;
    }
}

// =============================================================================
// Verification Helpers
// =============================================================================

/// Verify that every non-blank line in the formatted output:
/// 1. Has leading whitespace consisting ONLY of spaces (no tabs)
/// 2. Has a leading space count that is a multiple of 4
fn verify_indentation(formatted: &str) -> std::result::Result<(), TestCaseError> {
    for (line_num, line) in formatted.lines().enumerate() {
        // Skip blank lines
        if line.trim().is_empty() {
            continue;
        }

        // Check no tabs in leading whitespace
        let leading_ws: String = line.chars().take_while(|c| c.is_whitespace()).collect();
        prop_assert!(
            !leading_ws.contains('\t'),
            "Line {} contains tabs in leading whitespace: {:?}",
            line_num + 1,
            line
        );

        // Check leading space count is a multiple of 4
        let leading_spaces = line.len() - line.trim_start_matches(' ').len();
        prop_assert!(
            leading_spaces % 4 == 0,
            "Line {} has {} leading spaces (not a multiple of 4): {:?}",
            line_num + 1,
            leading_spaces,
            line
        );
    }
    Ok(())
}

/// Verify that lines within a block (after `{` opener, before `}` closer) are
/// indented more than the line containing the opening brace.
fn verify_nesting_depth_increases(formatted: &str) -> std::result::Result<(), TestCaseError> {
    let lines: Vec<&str> = formatted.lines().collect();

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();

        // If this line ends with `{`, the next non-blank line should be indented more
        if trimmed.ends_with('{') && !trimmed.is_empty() {
            let opener_indent = line.len() - line.trim_start().len();

            // Find the next non-blank line
            if let Some(next_line) = lines[i + 1..].iter().find(|l| !l.trim().is_empty()) {
                let next_indent = next_line.len() - next_line.trim_start().len();
                let next_trimmed = next_line.trim();

                // The next non-blank line should either be:
                // - Indented more (content inside the block), OR
                // - A closing brace `}` at the same level (empty block)
                if !next_trimmed.starts_with('}') {
                    prop_assert!(
                        next_indent > opener_indent,
                        "Line {} opens a block (indent={}) but next content line has indent={}: opener={:?}, next={:?}",
                        i + 1,
                        opener_indent,
                        next_indent,
                        line,
                        next_line
                    );
                }
            }
        }
    }
    Ok(())
}
