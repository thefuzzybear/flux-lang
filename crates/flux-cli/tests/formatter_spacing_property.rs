// Feature: flux-fmt-syntax-highlight, Property 5: Spacing Normalization
//!
//! Property-based tests verifying that the formatter produces output with:
//! - Exactly one space on each side of every binary operator
//! - Zero spaces between function/method name and opening parenthesis
//! - Exactly one space after every comma in argument lists
//! - Exactly one space on each side of the assignment operator
//!
//! **Validates: Requirements 3.8, 3.9, 3.10**

use proptest::prelude::*;
use regex::Regex;

use flux_compiler::lexer;
use flux_compiler::parser;
use flux_cli::formatter::Formatter;

// =============================================================================
// Generators
// =============================================================================

/// Generate a valid Flux identifier that doesn't conflict with keywords.
fn ident_strategy() -> impl Strategy<Value = String> {
    prop::sample::select(vec![
        "alpha", "beta", "gamma", "delta", "val", "res", "tmp", "acc",
        "price", "qty", "sig", "avg", "cnt", "idx", "total", "ratio",
    ])
    .prop_map(|s| s.to_string())
}

/// Generate an integer literal.
fn int_lit_strategy() -> impl Strategy<Value = String> {
    (1i64..1000).prop_map(|n| n.to_string())
}

/// Generate a float literal.
fn float_lit_strategy() -> impl Strategy<Value = String> {
    (1.0f64..100.0).prop_map(|f| format!("{:.1}", f))
}

/// Generate a simple atom (identifier or literal).
fn atom_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        ident_strategy(),
        int_lit_strategy(),
        float_lit_strategy(),
    ]
}

/// Generate a binary operator string.
fn binop_strategy() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec![
        "+", "-", "*", "/", "%", "==", "!=", "<", "<=", ">", ">=", "and", "or",
    ])
}

/// Generate a binary expression (possibly nested up to depth 2).
fn binary_expr_strategy() -> impl Strategy<Value = String> {
    // Simple binary: atom op atom
    let simple_binary = (atom_strategy(), binop_strategy(), atom_strategy())
        .prop_map(|(l, op, r)| format!("{} {} {}", l, op, r));

    // Nested: (atom op atom) op atom
    let nested_binary = (
        atom_strategy(),
        binop_strategy(),
        atom_strategy(),
        binop_strategy(),
        atom_strategy(),
    )
        .prop_map(|(a, op1, b, op2, c)| format!("{} {} {} {} {}", a, op1, b, op2, c));

    prop_oneof![simple_binary, nested_binary]
}

/// Generate a function call with 0-4 arguments.
fn func_call_strategy() -> impl Strategy<Value = String> {
    let func_names = prop::sample::select(vec![
        "sma", "ema", "stddev", "compute", "calc", "process",
    ]);

    let args = proptest::collection::vec(atom_strategy(), 0..=4)
        .prop_map(|args| args.join(", "));

    (func_names, args).prop_map(|(name, args)| format!("{}({})", name, args))
}

/// Generate a method call: receiver.method(args)
fn method_call_strategy() -> impl Strategy<Value = String> {
    let methods = prop::sample::select(vec![
        "append", "length", "get", "set", "update",
    ]);

    let args = proptest::collection::vec(atom_strategy(), 0..=3)
        .prop_map(|args| args.join(", "));

    (ident_strategy(), methods, args)
        .prop_map(|(recv, method, args)| format!("{}.{}({})", recv, method, args))
}

/// Generate a complex expression that may combine binary ops with function calls.
fn complex_expr_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        // function call result in binary op
        (func_call_strategy(), binop_strategy(), atom_strategy())
            .prop_map(|(f, op, a)| format!("{} {} {}", f, op, a)),
        // binary op inside function call args
        (
            prop::sample::select(vec!["sma", "ema", "calc", "process"]),
            binary_expr_strategy(),
            atom_strategy(),
        )
            .prop_map(|(name, expr, extra)| format!("{}({}, {})", name, expr, extra)),
        // method call with binary expression arg
        (ident_strategy(), prop::sample::select(vec!["get", "set", "update"]), binary_expr_strategy())
            .prop_map(|(recv, method, expr)| format!("{}.{}({})", recv, method, expr)),
    ]
}

/// Generate a full expression (any of the above types).
fn expr_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => binary_expr_strategy(),
        2 => func_call_strategy(),
        2 => method_call_strategy(),
        2 => complex_expr_strategy(),
        1 => atom_strategy(),
    ]
}

/// Generate a full Flux program wrapping the expression in a strategy body.
fn flux_program_strategy() -> impl Strategy<Value = String> {
    proptest::collection::vec(expr_strategy(), 1..=4).prop_map(|exprs| {
        let mut body = String::new();
        for (i, expr) in exprs.iter().enumerate() {
            let var = format!("v{}", i);
            body.push_str(&format!("        {} = {}\n", var, expr));
        }
        format!(
            "strategy TestStrat {{\n    on bar {{\n{}\
            }}\n}}\n",
            body
        )
    })
}

// =============================================================================
// Verification helpers
// =============================================================================



/// Check that binary operators have exactly one space on each side.
/// Accounts for unary minus (preceded by `=`, `(`, `,`, or start of expression context).
fn check_binary_op_spacing(formatted: &str) -> Result<(), String> {
    for line in formatted.lines() {
        let trimmed = line.trim();
        // Skip comment lines
        if trimmed.starts_with('#') {
            continue;
        }

        // Check word-based operators (and, or) — must have spaces around them
        // Use word-boundary matching
        let and_re = Regex::new(r"[^ ]\band\b|\band\b[^ ]").unwrap();
        if let Some(m) = and_re.find(trimmed) {
            // Check it's truly missing a space (not part of a larger word)
            let ctx = &trimmed[m.start().saturating_sub(3)..std::cmp::min(trimmed.len(), m.end() + 3)];
            return Err(format!(
                "Operator 'and' not properly spaced on line: '{}' (context: '{}')",
                trimmed, ctx
            ));
        }

        let or_re = Regex::new(r"[^ ]\bor\b|\bor\b[^ ]").unwrap();
        if let Some(m) = or_re.find(trimmed) {
            let ctx = &trimmed[m.start().saturating_sub(3)..std::cmp::min(trimmed.len(), m.end() + 3)];
            return Err(format!(
                "Operator 'or' not properly spaced on line: '{}' (context: '{}')",
                trimmed, ctx
            ));
        }

        // Check symbolic binary operators
        for &op in &["+", "*", "/", "%", "==", "!=", "<=", ">="] {
            check_symbolic_op_spacing(trimmed, op)?;
        }

        // Special handling for `-` (could be unary) and `<`, `>` (could overlap with `<=`, `>=`)
        check_minus_spacing(trimmed)?;
        check_angle_bracket_spacing(trimmed)?;
    }
    Ok(())
}

/// Check a single symbolic operator has spaces around it.
fn check_symbolic_op_spacing(line: &str, op: &str) -> Result<(), String> {
    let mut search_from = 0;
    while let Some(pos) = line[search_from..].find(op) {
        let abs_pos = search_from + pos;

        // Skip if this is part of a longer operator
        if op == "=" && is_part_of_multi_char_op(line, abs_pos, &["==", "!=", "<=", ">="]) {
            search_from = abs_pos + op.len();
            continue;
        }
        if op == "<" && abs_pos + 1 < line.len() && &line[abs_pos..abs_pos + 2] == "<=" {
            search_from = abs_pos + 2;
            continue;
        }
        if op == ">" && abs_pos + 1 < line.len() && &line[abs_pos..abs_pos + 2] == ">=" {
            search_from = abs_pos + 2;
            continue;
        }

        // Check space before
        if abs_pos > 0 {
            let before = line.as_bytes()[abs_pos - 1];
            if before != b' ' {
                return Err(format!(
                    "Missing space before '{}' at position {} on line: '{}'",
                    op, abs_pos, line
                ));
            }
        }

        // Check space after
        let after_pos = abs_pos + op.len();
        if after_pos < line.len() {
            let after = line.as_bytes()[after_pos];
            if after != b' ' {
                return Err(format!(
                    "Missing space after '{}' at position {} on line: '{}'",
                    op, abs_pos, line
                ));
            }
        }

        search_from = abs_pos + op.len();
    }
    Ok(())
}

/// Check if a `=` at a given position is part of a multi-char operator.
fn is_part_of_multi_char_op(line: &str, pos: usize, ops: &[&str]) -> bool {
    for &op in ops {
        let op_len = op.len();
        // Check if this position is within any multi-char operator occurrence
        for start in pos.saturating_sub(op_len - 1)..=pos {
            if start + op_len <= line.len() && &line[start..start + op_len] == op {
                return true;
            }
        }
    }
    false
}

/// Check `-` spacing. Unary minus won't have a space before it (it follows `=`, `(`, `,`, or space + op).
/// Binary minus must have one space on each side.
fn check_minus_spacing(line: &str) -> Result<(), String> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'-' {
            // Skip if inside a multi-char operator (not applicable for `-`)
            // Determine if unary or binary:
            // Unary if preceded by: `=`, `(`, `,`, `[`, or another operator, or start of expr
            let is_unary = if i == 0 {
                true
            } else {
                let prev_non_space = line[..i].trim_end().as_bytes().last().copied();
                matches!(
                    prev_non_space,
                    Some(b'=') | Some(b'(') | Some(b',') | Some(b'[') | None
                )
            };

            if !is_unary {
                // Binary minus — check spaces
                if i > 0 && bytes[i - 1] != b' ' {
                    return Err(format!(
                        "Missing space before binary '-' at position {} on line: '{}'",
                        i, line
                    ));
                }
                if i + 1 < bytes.len() && bytes[i + 1] != b' ' {
                    return Err(format!(
                        "Missing space after binary '-' at position {} on line: '{}'",
                        i, line
                    ));
                }
            }
        }
        i += 1;
    }
    Ok(())
}

/// Check `<` and `>` spacing (excluding `<=` and `>=` which are handled separately).
fn check_angle_bracket_spacing(line: &str) -> Result<(), String> {
    let bytes = line.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'<' && (i + 1 >= bytes.len() || bytes[i + 1] != b'=') {
            // Standalone `<`
            if i > 0 && bytes[i - 1] != b' ' {
                return Err(format!(
                    "Missing space before '<' at position {} on line: '{}'",
                    i, line
                ));
            }
            if i + 1 < bytes.len() && bytes[i + 1] != b' ' {
                return Err(format!(
                    "Missing space after '<' at position {} on line: '{}'",
                    i, line
                ));
            }
        }
        if bytes[i] == b'>' && (i == 0 || bytes[i - 1] != b'!') && (i + 1 >= bytes.len() || bytes[i + 1] != b'=') {
            // Standalone `>` (not part of `>=`)
            // Also check it's not preceded by something that makes it `!=` — that's not applicable to >
            if i > 0 && bytes[i - 1] != b' ' {
                return Err(format!(
                    "Missing space before '>' at position {} on line: '{}'",
                    i, line
                ));
            }
            if i + 1 < bytes.len() && bytes[i + 1] != b' ' {
                return Err(format!(
                    "Missing space after '>' at position {} on line: '{}'",
                    i, line
                ));
            }
        }
    }
    Ok(())
}

/// Check that there is no space between function/method name and `(`.
fn check_no_space_before_paren(formatted: &str) -> Result<(), String> {
    // Pattern: identifier followed by space then `(` indicates bad spacing.
    // Valid: `sma(`, `obj.method(`
    // Invalid: `sma (`, `obj.method (`
    // Exceptions: control flow keywords like `if (`, `while (`, `for (` — but Flux
    // doesn't use parens after these, so we don't need to exclude them.
    let bad_pattern = Regex::new(r"[a-zA-Z_][a-zA-Z0-9_]*\s+\(").unwrap();

    for line in formatted.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }
        if let Some(m) = bad_pattern.find(trimmed) {
            let matched = &trimmed[m.start()..m.end()];
            // Exclude keywords that legitimately precede `{` but not `(`
            // In Flux formatted output, we should never see `keyword (` patterns
            // because if/while/for don't use parens. But let's be safe:
            let word_end = matched.find(|c: char| c.is_whitespace()).unwrap_or(matched.len());
            let word = &matched[..word_end];
            if matches!(word, "if" | "elif" | "while" | "for" | "return" | "not" | "and" | "or") {
                continue;
            }
            return Err(format!(
                "Space found between function name and '(' on line: '{}' (match: '{}')",
                trimmed, matched
            ));
        }
    }
    Ok(())
}

/// Check commas have exactly one space after them (and no space before).
fn check_comma_spacing(formatted: &str) -> Result<(), String> {
    for line in formatted.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }

        let bytes = trimmed.as_bytes();
        for i in 0..bytes.len() {
            if bytes[i] == b',' {
                // No space before comma
                if i > 0 && bytes[i - 1] == b' ' {
                    return Err(format!(
                        "Space before comma at position {} on line: '{}'",
                        i, trimmed
                    ));
                }
                // Exactly one space after comma (if not at end of line)
                if i + 1 < bytes.len() {
                    if bytes[i + 1] != b' ' {
                        return Err(format!(
                            "Missing space after comma at position {} on line: '{}'",
                            i, trimmed
                        ));
                    }
                    // Check no double space
                    if i + 2 < bytes.len() && bytes[i + 2] == b' ' {
                        return Err(format!(
                            "Multiple spaces after comma at position {} on line: '{}'",
                            i, trimmed
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Check assignment operator `=` has exactly one space on each side.
/// Must distinguish from `==`, `!=`, `<=`, `>=`.
fn check_assignment_spacing(formatted: &str) -> Result<(), String> {
    for line in formatted.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }
        // Skip lines that are structural (strategy, params, etc.)
        if trimmed.starts_with("strategy ")
            || trimmed.starts_with("params")
            || trimmed.starts_with("state")
            || trimmed.starts_with("on ")
            || trimmed == "}"
            || trimmed.starts_with("}")
            || trimmed.starts_with("from ")
            || trimmed.starts_with("if ")
            || trimmed.starts_with("} elif")
            || trimmed.starts_with("} else")
            || trimmed.starts_with("for ")
            || trimmed.starts_with("while ")
            || trimmed.starts_with("return")
        {
            continue;
        }

        let bytes = trimmed.as_bytes();
        for i in 0..bytes.len() {
            if bytes[i] != b'=' {
                continue;
            }
            // Skip if part of ==
            if i + 1 < bytes.len() && bytes[i + 1] == b'=' {
                continue;
            }
            // Skip if part of !=, <=, >=
            if i > 0 && matches!(bytes[i - 1], b'!' | b'<' | b'>') {
                continue;
            }
            // Skip if preceded by another `=` (second char of `==`)
            if i > 0 && bytes[i - 1] == b'=' {
                continue;
            }

            // This is a standalone assignment `=`
            // Check space before
            if i > 0 && bytes[i - 1] != b' ' {
                return Err(format!(
                    "Missing space before '=' at position {} on line: '{}'",
                    i, trimmed
                ));
            }
            // Check space after
            if i + 1 < bytes.len() && bytes[i + 1] != b' ' {
                return Err(format!(
                    "Missing space after '=' at position {} on line: '{}'",
                    i, trimmed
                ));
            }
        }
    }
    Ok(())
}

// =============================================================================
// Property Tests
// =============================================================================

/// Helper: parse and format a Flux source string.
fn format_source(source: &str) -> Option<String> {
    let tokens = lexer::lex_with_spans(source).ok()?;
    let ast = parser::parse(tokens).ok()?;
    Some(Formatter::format(&ast, source))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 3.8, 3.9, 3.10**
    ///
    /// For any generated Flux program with binary expressions and function calls,
    /// the formatted output has exactly one space around binary operators,
    /// no space between function name and `(`, and one space after commas.
    #[test]
    fn prop_spacing_normalization(source in flux_program_strategy()) {
        // Only test sources that parse successfully
        if let Some(formatted) = format_source(&source) {
            // Check binary operator spacing
            prop_assert!(
                check_binary_op_spacing(&formatted).is_ok(),
                "Binary op spacing failed: {}",
                check_binary_op_spacing(&formatted).unwrap_err()
            );

            // Check no space before parenthesis in function calls
            prop_assert!(
                check_no_space_before_paren(&formatted).is_ok(),
                "Function call spacing failed: {}",
                check_no_space_before_paren(&formatted).unwrap_err()
            );

            // Check comma spacing
            prop_assert!(
                check_comma_spacing(&formatted).is_ok(),
                "Comma spacing failed: {}",
                check_comma_spacing(&formatted).unwrap_err()
            );

            // Check assignment spacing
            prop_assert!(
                check_assignment_spacing(&formatted).is_ok(),
                "Assignment spacing failed: {}",
                check_assignment_spacing(&formatted).unwrap_err()
            );
        }
    }
}
