//! Property-based tests for for-loop parse round-trip.
//!
//! Feature: for-loop-iteration, Property 2: For-loop parse round-trip
//!
//! Generates valid `for var in expr { body }` source strings with random variable
//! names, simple iterable expressions, and 0-3 body statements, then verifies:
//! parse → pretty-print → parse again → AST equivalence.
//!
//! **Validates: Requirements 2.1, 2.5, 2.6, 2.7, 8.1, 8.2, 8.3**

#[cfg(test)]
mod tests {
    use crate::lexer::lex_with_spans;
    use crate::parser::ast::{Expr, ExprKind, Program, Stmt, StrategyItem};
    use crate::parser::{parse, pretty_print_program};
    use proptest::prelude::*;

    // ========================================================================
    // Helpers
    // ========================================================================

    /// Flux keywords that cannot be used as variable names.
    const KEYWORDS: &[&str] = &[
        "for", "in", "if", "else", "elif", "fn", "return", "strategy", "params", "state", "on",
        "from", "import", "and", "or", "not", "true", "false", "null", "data", "connector",
        "struct", "enum", "match", "self", "impl", "trait", "while",
    ];

    fn is_keyword(s: &str) -> bool {
        KEYWORDS.contains(&s)
    }

    // ========================================================================
    // Generators
    // ========================================================================

    /// Generate a valid identifier: lowercase alpha start, 1-6 chars, not a keyword,
    /// doesn't start with "on_" (which triggers event handler parsing).
    fn arb_var_name() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9_]{0,5}".prop_filter("not keyword or on_ prefix", |s| {
            !is_keyword(s) && !s.starts_with("on_")
        })
    }

    /// Generate a simple iterable expression (identifier or function call).
    fn arb_iterable_source() -> impl Strategy<Value = String> {
        prop_oneof![
            // Simple identifiers that are valid iterables
            Just("items".to_string()),
            Just("values".to_string()),
            Just("elements".to_string()),
            Just("results".to_string()),
            Just("collection".to_string()),
            // Function calls like range(0, 10)
            (0i64..50, 1i64..100).prop_map(|(start, end)| format!("range({}, {})", start, start + end)),
            // List literals
            Just("[1, 2, 3]".to_string()),
            Just("[10, 20]".to_string()),
        ]
    }

    /// Generate a simple body statement as source text.
    /// Uses assignments like `x = 1` or `y = 2.0`.
    fn arb_body_stmt_source() -> impl Strategy<Value = String> {
        prop_oneof![
            // Simple int assignment
            (arb_var_name(), 1i64..1000).prop_map(|(name, val)| format!("    {} = {}", name, val)),
            // Simple float assignment
            (arb_var_name(), 1u32..99, 1u32..99)
                .prop_map(|(name, i, d)| format!("    {} = {}.{}", name, i, d)),
            // Expression statement (just an identifier)
            arb_var_name().prop_map(|name| format!("    {}", name)),
        ]
    }

    /// Generate a complete for-loop source string embedded in a minimal strategy program.
    fn arb_for_loop_program_source() -> impl Strategy<Value = String> {
        (
            arb_var_name(),
            arb_iterable_source(),
            proptest::collection::vec(arb_body_stmt_source(), 0..4),
        )
            .prop_map(|(var, iterable, body_stmts)| {
                let body = if body_stmts.is_empty() {
                    // Need at least one statement in the body for a valid for-loop
                    "    x = 1".to_string()
                } else {
                    body_stmts.join("\n")
                };
                format!(
                    "strategy T {{\n    on_bar {{\n        for {} in {} {{\n{}\n        }}\n    }}\n}}",
                    var, iterable, indent_body(&body, 2)
                )
            })
    }

    /// Generate a nested for-loop source to validate Requirement 2.6.
    fn arb_nested_for_loop_program_source() -> impl Strategy<Value = String> {
        (
            arb_var_name(),
            arb_var_name(),
            arb_iterable_source(),
            arb_iterable_source(),
            arb_var_name(),
            1i64..100,
        )
            .prop_filter("outer and inner vars must differ", |(outer, inner, _, _, _, _)| {
                outer != inner
            })
            .prop_map(|(outer_var, inner_var, outer_iter, inner_iter, assign_var, val)| {
                format!(
                    "strategy T {{\n    on_bar {{\n        for {} in {} {{\n            for {} in {} {{\n                {} = {}\n            }}\n        }}\n    }}\n}}",
                    outer_var, outer_iter, inner_var, inner_iter, assign_var, val
                )
            })
    }

    /// Indent body lines by additional levels (each level = 4 spaces).
    fn indent_body(body: &str, extra_levels: usize) -> String {
        let prefix = "    ".repeat(extra_levels);
        body.lines()
            .map(|line| format!("{}{}", prefix, line))
            .collect::<Vec<_>>()
            .join("\n")
    }

    // ========================================================================
    // Span-ignoring structural equality
    // ========================================================================

    fn stmts_eq(a: &Stmt, b: &Stmt) -> bool {
        match (a, b) {
            (Stmt::Assignment(aa), Stmt::Assignment(ab)) => {
                exprs_eq(&aa.target, &ab.target) && exprs_eq(&aa.value, &ab.value)
            }
            (Stmt::If(ia), Stmt::If(ib)) => {
                exprs_eq(&ia.condition, &ib.condition)
                    && ia.body.len() == ib.body.len()
                    && ia
                        .body
                        .iter()
                        .zip(ib.body.iter())
                        .all(|(a, b)| stmts_eq(a, b))
                    && ia.elif_branches.len() == ib.elif_branches.len()
                    && ia
                        .elif_branches
                        .iter()
                        .zip(ib.elif_branches.iter())
                        .all(|(a, b)| {
                            exprs_eq(&a.condition, &b.condition)
                                && a.body.len() == b.body.len()
                                && a.body
                                    .iter()
                                    .zip(b.body.iter())
                                    .all(|(a, b)| stmts_eq(a, b))
                        })
                    && match (&ia.else_body, &ib.else_body) {
                        (None, None) => true,
                        (Some(ea), Some(eb)) => {
                            ea.len() == eb.len()
                                && ea.iter().zip(eb.iter()).all(|(a, b)| stmts_eq(a, b))
                        }
                        _ => false,
                    }
            }
            (Stmt::For(fa), Stmt::For(fb)) => {
                fa.variable == fb.variable
                    && exprs_eq(&fa.iterable, &fb.iterable)
                    && fa.body.len() == fb.body.len()
                    && fa
                        .body
                        .iter()
                        .zip(fb.body.iter())
                        .all(|(a, b)| stmts_eq(a, b))
            }
            (Stmt::While(wa), Stmt::While(wb)) => {
                exprs_eq(&wa.condition, &wb.condition)
                    && wa.body.len() == wb.body.len()
                    && wa
                        .body
                        .iter()
                        .zip(wb.body.iter())
                        .all(|(a, b)| stmts_eq(a, b))
            }
            (Stmt::Return(ra), Stmt::Return(rb)) => match (&ra.value, &rb.value) {
                (None, None) => true,
                (Some(a), Some(b)) => exprs_eq(a, b),
                _ => false,
            },
            (Stmt::Expr(ea), Stmt::Expr(eb)) => exprs_eq(&ea.expr, &eb.expr),
            _ => false,
        }
    }

    fn exprs_eq(a: &Expr, b: &Expr) -> bool {
        match (&a.kind, &b.kind) {
            (ExprKind::IntLiteral(va), ExprKind::IntLiteral(vb)) => va == vb,
            (ExprKind::FloatLiteral(va), ExprKind::FloatLiteral(vb)) => va == vb,
            (ExprKind::StringLiteral(sa), ExprKind::StringLiteral(sb)) => sa == sb,
            (ExprKind::BoolLiteral(ba), ExprKind::BoolLiteral(bb)) => ba == bb,
            (ExprKind::NullLiteral, ExprKind::NullLiteral) => true,
            (ExprKind::Ident(na), ExprKind::Ident(nb)) => na == nb,
            (ExprKind::ListLiteral(ea), ExprKind::ListLiteral(eb)) => {
                ea.len() == eb.len()
                    && ea.iter().zip(eb.iter()).all(|(a, b)| exprs_eq(a, b))
            }
            (
                ExprKind::BinaryOp { left: la, op: opa, right: ra },
                ExprKind::BinaryOp { left: lb, op: opb, right: rb },
            ) => opa == opb && exprs_eq(la, lb) && exprs_eq(ra, rb),
            (
                ExprKind::UnaryOp { op: opa, operand: ea },
                ExprKind::UnaryOp { op: opb, operand: eb },
            ) => opa == opb && exprs_eq(ea, eb),
            (
                ExprKind::FunctionCall { function: fa, args: aa },
                ExprKind::FunctionCall { function: fb, args: ab },
            ) => {
                exprs_eq(fa, fb)
                    && aa.len() == ab.len()
                    && aa.iter().zip(ab.iter()).all(|(a, b)| exprs_eq(a, b))
            }
            (
                ExprKind::MethodCall { receiver: ra, method: ma, args: aa },
                ExprKind::MethodCall { receiver: rb, method: mb, args: ab },
            ) => {
                ma == mb
                    && exprs_eq(ra, rb)
                    && aa.len() == ab.len()
                    && aa.iter().zip(ab.iter()).all(|(a, b)| exprs_eq(a, b))
            }
            (
                ExprKind::MemberAccess { object: oa, field: fa },
                ExprKind::MemberAccess { object: ob, field: fb },
            ) => fa == fb && exprs_eq(oa, ob),
            (
                ExprKind::IndexAccess { object: oa, index: ia },
                ExprKind::IndexAccess { object: ob, index: ib },
            ) => exprs_eq(oa, ob) && exprs_eq(ia, ib),
            _ => false,
        }
    }

    /// Extract the for-loop statements from the first event handler in a program.
    fn extract_event_body(program: &Program) -> &[Stmt] {
        for item in &program.strategy.body {
            if let StrategyItem::EventHandler(handler) = item {
                return &handler.body;
            }
        }
        &[]
    }

    // ========================================================================
    // Property Tests
    // ========================================================================

    // Feature: for-loop-iteration, Property 2: For-loop parse round-trip
    // **Validates: Requirements 2.1, 2.5, 2.6, 2.7, 8.1, 8.2, 8.3**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// For any valid for-loop source string, parse → pretty-print → parse
        /// produces an equivalent AST (ignoring spans).
        #[test]
        fn prop_for_loop_parse_round_trip(source in arb_for_loop_program_source()) {
            // First parse
            let tokens1 = lex_with_spans(&source).expect(
                &format!("Generated source should lex successfully.\nSource:\n{}", source)
            );
            let program1 = parse(tokens1).expect(
                &format!("Generated source should parse successfully.\nSource:\n{}", source)
            );

            // Pretty-print
            let printed = pretty_print_program(&program1);

            // Second parse
            let tokens2 = lex_with_spans(&printed).expect(
                &format!("Pretty-printed source should lex successfully.\nPrinted:\n{}", printed)
            );
            let program2 = parse(tokens2).expect(
                &format!("Pretty-printed source should parse successfully.\nPrinted:\n{}", printed)
            );

            // Compare event handler bodies (which contain the for-loop)
            let body1 = extract_event_body(&program1);
            let body2 = extract_event_body(&program2);

            prop_assert_eq!(body1.len(), body2.len(),
                "Body statement count mismatch.\nSource:\n{}\nPrinted:\n{}\nBody1: {:?}\nBody2: {:?}",
                source, printed, body1, body2
            );

            for (s1, s2) in body1.iter().zip(body2.iter()) {
                prop_assert!(stmts_eq(s1, s2),
                    "For-loop round-trip failed!\nOriginal source:\n{}\nPretty-printed:\n{}\nStmt1: {:?}\nStmt2: {:?}",
                    source, printed, s1, s2
                );
            }
        }

        /// Nested for-loops also round-trip correctly (validates Requirement 2.6).
        #[test]
        fn prop_nested_for_loop_round_trip(source in arb_nested_for_loop_program_source()) {
            // First parse
            let tokens1 = lex_with_spans(&source).expect(
                &format!("Nested for-loop source should lex.\nSource:\n{}", source)
            );
            let program1 = parse(tokens1).expect(
                &format!("Nested for-loop source should parse.\nSource:\n{}", source)
            );

            // Pretty-print
            let printed = pretty_print_program(&program1);

            // Second parse
            let tokens2 = lex_with_spans(&printed).expect(
                &format!("Pretty-printed nested source should lex.\nPrinted:\n{}", printed)
            );
            let program2 = parse(tokens2).expect(
                &format!("Pretty-printed nested source should parse.\nPrinted:\n{}", printed)
            );

            // Compare event handler bodies
            let body1 = extract_event_body(&program1);
            let body2 = extract_event_body(&program2);

            prop_assert_eq!(body1.len(), body2.len(),
                "Nested body count mismatch.\nSource:\n{}\nPrinted:\n{}",
                source, printed
            );

            for (s1, s2) in body1.iter().zip(body2.iter()) {
                prop_assert!(stmts_eq(s1, s2),
                    "Nested for-loop round-trip failed!\nSource:\n{}\nPrinted:\n{}\nStmt1: {:?}\nStmt2: {:?}",
                    source, printed, s1, s2
                );
            }
        }
    }
}
