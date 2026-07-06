// Feature: flux-fmt-syntax-highlight, Property 9: Colorization Token Coverage
//!
//! Property-based tests verifying that for any valid Flux program, applying the
//! ANSI renderer produces output where every token span is wrapped with the correct
//! category ANSI codes, string literals receive a single consistent color, numeric
//! literals are distinct from identifiers, and no token span contains mixed color codes.
//!
//! **Validates: Requirements 2.4, 2.6**

use flux_compiler::lexer::{lex_with_spans, Span};
use flux_compiler::parser::ast::{
    Assignment, BinOp, EventHandler, Expr, ExprKind, ExprStmt, ForLoop, IfStmt, Import, Param,
    ParamsBlock, Program, StateBlock, StateVar, Stmt, Strategy as AstStrategy,
    StrategyItem, UnaryOp, WhileLoop,
};
use flux_compiler::parser::pretty_print_program;

use flux_cli::formatter::ansi::{classify_token, colorize, ColorMode, ColorTheme, TokenCategory};
use flux_cli::formatter::Formatter;

use proptest::prelude::*;
use regex::Regex;

// ============================================================================
// ANSI Helpers
// ============================================================================

/// Strip all ANSI escape codes from a string.
fn strip_ansi(s: &str) -> String {
    let re = Regex::new(r"\x1b\[[0-9;]*m").unwrap();
    re.replace_all(s, "").to_string()
}

/// Extract all ANSI-wrapped spans from colorized output.
/// Returns a vec of (prefix_code, token_text, suffix_code) tuples.
fn extract_ansi_spans(s: &str) -> Vec<(String, String, String)> {
    let re = Regex::new(r"(\x1b\[[0-9;]*m)(.*?)(\x1b\[0m)").unwrap();
    re.captures_iter(s)
        .map(|cap| {
            (
                cap[1].to_string(),
                cap[2].to_string(),
                cap[3].to_string(),
            )
        })
        .collect()
}

/// Check if a string contains any ANSI escape code.
fn contains_ansi(s: &str) -> bool {
    let re = Regex::new(r"\x1b\[[0-9;]*m").unwrap();
    re.is_match(s)
}

/// Get the expected ANSI prefix for a token category.
fn expected_prefix(category: TokenCategory) -> &'static str {
    match category {
        TokenCategory::Keyword => "\x1b[1;34m",
        TokenCategory::Identifier => "",
        TokenCategory::IntegerLiteral => "\x1b[36m",
        TokenCategory::FloatLiteral => "\x1b[36m",
        TokenCategory::StringLiteral => "\x1b[32m",
        TokenCategory::Comment => "\x1b[2;37m",
        TokenCategory::Operator => "\x1b[33m",
        TokenCategory::Delimiter => "",
        TokenCategory::SignalFunction => "\x1b[1;35m",
        TokenCategory::BooleanLiteral => "\x1b[1;34m",
    }
}

// ============================================================================
// AST Generators (same pattern as formatter_property.rs)
// ============================================================================

fn dummy_span() -> Span {
    Span::new(0, 0)
}

fn is_keyword(s: &str) -> bool {
    matches!(
        s,
        "strategy" | "params" | "state" | "on" | "if" | "elif" | "else" | "for" | "while"
            | "return" | "from" | "import" | "and" | "or" | "not" | "true" | "false" | "null"
            | "in"
    )
}

/// Valid identifier: lowercase alpha start, not a keyword, not signal functions
fn arb_ident() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9]{0,5}".prop_filter("not keyword or on_ prefix", |s| {
        !is_keyword(s) && !s.starts_with("on_")
    })
}

/// Signal function names
fn arb_signal_function() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("OPEN".to_string()),
        Just("CLOSE".to_string()),
        Just("CLOSE_QTY".to_string()),
    ]
}

/// Event name
fn arb_event_name() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9]{0,5}".prop_filter("not keyword", |s| !is_keyword(s))
}

/// Module path segment
fn arb_module_segment() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9]{0,5}".prop_filter("not keyword", |s| !is_keyword(s))
}

fn arb_binop() -> impl Strategy<Value = BinOp> {
    prop_oneof![
        Just(BinOp::Add),
        Just(BinOp::Sub),
        Just(BinOp::Mul),
        Just(BinOp::Div),
        Just(BinOp::Mod),
        Just(BinOp::Eq),
        Just(BinOp::Ne),
        Just(BinOp::Lt),
        Just(BinOp::Le),
        Just(BinOp::Gt),
        Just(BinOp::Ge),
        Just(BinOp::And),
        Just(BinOp::Or),
    ]
}

fn arb_leaf_expr() -> impl Strategy<Value = Expr> {
    prop_oneof![
        // Integer literal
        (1i64..1000).prop_map(|v| Expr {
            kind: ExprKind::IntLiteral(v),
            span: dummy_span(),
        }),
        // Float literal
        (1u32..999, 1u32..99).prop_map(|(i, d)| {
            let s = format!("{}.{}", i, d);
            let f: f64 = s.parse().unwrap();
            Expr {
                kind: ExprKind::FloatLiteral(f),
                span: dummy_span(),
            }
        }),
        // Bool literal
        any::<bool>().prop_map(|b| Expr {
            kind: ExprKind::BoolLiteral(b),
            span: dummy_span(),
        }),
        // Identifier
        arb_ident().prop_map(|s| Expr {
            kind: ExprKind::Ident(s),
            span: dummy_span(),
        }),
        // Null literal
        Just(Expr {
            kind: ExprKind::NullLiteral,
            span: dummy_span(),
        }),
        // String literal (simple printable ASCII)
        "[a-zA-Z0-9 ]{1,8}".prop_map(|s| Expr {
            kind: ExprKind::StringLiteral(s),
            span: dummy_span(),
        }),
    ]
}

fn arb_expr() -> impl Strategy<Value = Expr> {
    arb_leaf_expr().prop_recursive(2, 10, 3, |inner| {
        prop_oneof![
            // Binary op
            (inner.clone(), arb_binop(), inner.clone()).prop_map(|(l, op, r)| Expr {
                kind: ExprKind::BinaryOp {
                    left: Box::new(l),
                    op,
                    right: Box::new(r),
                },
                span: dummy_span(),
            }),
            // Unary neg
            inner.clone().prop_map(|e| Expr {
                kind: ExprKind::UnaryOp {
                    op: UnaryOp::Neg,
                    operand: Box::new(e),
                },
                span: dummy_span(),
            }),
            // Unary not
            inner.clone().prop_map(|e| Expr {
                kind: ExprKind::UnaryOp {
                    op: UnaryOp::Not,
                    operand: Box::new(e),
                },
                span: dummy_span(),
            }),
            // Function call (regular)
            (arb_ident(), proptest::collection::vec(inner.clone(), 1..3)).prop_map(
                |(name, args)| Expr {
                    kind: ExprKind::FunctionCall {
                        function: Box::new(Expr {
                            kind: ExprKind::Ident(name),
                            span: dummy_span(),
                        }),
                        args,
                    },
                    span: dummy_span(),
                }
            ),
            // Signal function call
            (arb_signal_function(), proptest::collection::vec(inner.clone(), 1..3)).prop_map(
                |(name, args)| Expr {
                    kind: ExprKind::FunctionCall {
                        function: Box::new(Expr {
                            kind: ExprKind::Ident(name),
                            span: dummy_span(),
                        }),
                        args,
                    },
                    span: dummy_span(),
                }
            ),
            // List literal
            proptest::collection::vec(inner, 1..3).prop_map(|elems| Expr {
                kind: ExprKind::ListLiteral(elems),
                span: dummy_span(),
            }),
        ]
    })
}

fn arb_stmt() -> impl Strategy<Value = Stmt> {
    prop_oneof![
        // Assignment
        (arb_ident(), arb_expr()).prop_map(|(name, value)| {
            Stmt::Assignment(Assignment {
                target: Expr {
                    kind: ExprKind::Ident(name),
                    span: dummy_span(),
                },
                value,
                span: dummy_span(),
            })
        }),
        // Expression statement (function call)
        (arb_ident(), proptest::collection::vec(arb_leaf_expr(), 1..3)).prop_map(|(name, args)| {
            Stmt::Expr(ExprStmt {
                expr: Expr {
                    kind: ExprKind::FunctionCall {
                        function: Box::new(Expr {
                            kind: ExprKind::Ident(name),
                            span: dummy_span(),
                        }),
                        args,
                    },
                    span: dummy_span(),
                },
                span: dummy_span(),
            })
        }),
        // Signal function expression statement
        (
            arb_signal_function(),
            proptest::collection::vec(arb_leaf_expr(), 1..3)
        )
            .prop_map(|(name, args)| {
                Stmt::Expr(ExprStmt {
                    expr: Expr {
                        kind: ExprKind::FunctionCall {
                            function: Box::new(Expr {
                                kind: ExprKind::Ident(name),
                                span: dummy_span(),
                            }),
                            args,
                        },
                        span: dummy_span(),
                    },
                    span: dummy_span(),
                })
            }),
        // If statement
        (arb_leaf_expr(), arb_ident(), arb_leaf_expr()).prop_map(|(cond, name, val)| {
            Stmt::If(IfStmt {
                condition: cond,
                body: vec![Stmt::Assignment(Assignment {
                    target: Expr {
                        kind: ExprKind::Ident(name),
                        span: dummy_span(),
                    },
                    value: val,
                    span: dummy_span(),
                })],
                elif_branches: vec![],
                else_body: None,
                span: dummy_span(),
            })
        }),
        // For loop
        (arb_ident(), arb_ident(), arb_ident(), arb_leaf_expr()).prop_map(
            |(var, iter_name, assign_name, val)| {
                Stmt::For(ForLoop {
                    variable: var,
                    iterable: Expr {
                        kind: ExprKind::Ident(iter_name),
                        span: dummy_span(),
                    },
                    body: vec![Stmt::Assignment(Assignment {
                        target: Expr {
                            kind: ExprKind::Ident(assign_name),
                            span: dummy_span(),
                        },
                        value: val,
                        span: dummy_span(),
                    })],
                    span: dummy_span(),
                })
            }
        ),
        // While loop
        (arb_leaf_expr(), arb_ident(), arb_leaf_expr()).prop_map(|(cond, name, val)| {
            Stmt::While(WhileLoop {
                condition: cond,
                body: vec![Stmt::Assignment(Assignment {
                    target: Expr {
                        kind: ExprKind::Ident(name),
                        span: dummy_span(),
                    },
                    value: val,
                    span: dummy_span(),
                })],
                span: dummy_span(),
            })
        }),
    ]
}

fn arb_strategy_item() -> impl Strategy<Value = StrategyItem> {
    prop_oneof![
        // Params block
        proptest::collection::vec(
            (arb_ident(), arb_leaf_expr()).prop_map(|(name, value)| Param {
                name,
                default_value: value,
                span: dummy_span(),
            }),
            1..3,
        )
        .prop_map(|params| StrategyItem::ParamsBlock(ParamsBlock {
            params,
            span: dummy_span(),
        })),
        // State block
        proptest::collection::vec(
            (arb_ident(), arb_leaf_expr()).prop_map(|(name, value)| StateVar {
                name,
                initial_value: value,
                span: dummy_span(),
            }),
            1..3,
        )
        .prop_map(|variables| StrategyItem::StateBlock(StateBlock {
            variables,
            span: dummy_span(),
        })),
        // Event handler
        (arb_event_name(), proptest::collection::vec(arb_stmt(), 1..4)).prop_map(
            |(event_name, body)| {
                StrategyItem::EventHandler(EventHandler {
                    event_name,
                    body,
                    span: dummy_span(),
                })
            }
        ),
    ]
}

fn arb_import() -> impl Strategy<Value = Import> {
    (
        proptest::collection::vec(arb_module_segment(), 1..3),
        proptest::collection::vec(arb_ident(), 1..3),
    )
        .prop_map(|(segments, names)| Import {
            module_path: segments.join("."),
            names,
            span: dummy_span(),
        })
}

/// Generate a valid Flux program AST with diverse token types including
/// signal functions, string literals, numerics, booleans, and keywords.
fn arb_program() -> impl Strategy<Value = Program> {
    (
        proptest::collection::vec(arb_import(), 0..2),
        arb_ident(),
        proptest::collection::vec(arb_strategy_item(), 1..4),
    )
        .prop_map(|(imports, name, body)| Program {
            imports,
            strategy: AstStrategy {
                name,
                body,
                span: dummy_span(),
            },
            span: dummy_span(),
        })
}

// ============================================================================
// Property Test: Colorization Token Coverage
// ============================================================================

// Feature: flux-fmt-syntax-highlight, Property 9: Colorization Token Coverage
// **Validates: Requirements 2.4, 2.6**
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Property 9a: Stripped colorized output equals the original formatted source.
    ///
    /// For any valid Flux program, colorizing the formatted source and then
    /// stripping all ANSI codes produces the exact same text as the original
    /// formatted source.
    #[test]
    fn prop_colorize_stripped_equals_source(program in arb_program()) {
        let initial_source = pretty_print_program(&program);

        // Lex, parse, and format through the formatter
        let tokens = lex_with_spans(&initial_source).expect("Generated source should lex");
        let parsed_ast = flux_compiler::parser::parse(tokens)
            .expect("Generated source should parse");
        let formatted = Formatter::format(&parsed_ast, &initial_source);

        // Colorize with Always mode
        let theme = ColorTheme::default_theme();
        let colorized = colorize(&formatted, &theme, ColorMode::Always);

        // Strip ANSI codes and compare
        let stripped = strip_ansi(&colorized);
        prop_assert_eq!(
            &stripped,
            &formatted,
            "Stripped colorized output must equal original formatted source.\n\
             Formatted ({} bytes):\n{}\n\n\
             Colorized ({} bytes):\n{:?}\n\n\
             Stripped ({} bytes):\n{}",
            formatted.len(),
            formatted,
            colorized.len(),
            colorized,
            stripped.len(),
            stripped
        );
    }

    /// Property 9b: Keywords are colored with bold blue.
    ///
    /// For any valid Flux program, every keyword token in the colorized output
    /// is wrapped with the bold blue ANSI code (`\x1b[1;34m...\x1b[0m`).
    #[test]
    fn prop_colorize_keywords_bold_blue(program in arb_program()) {
        let initial_source = pretty_print_program(&program);
        let tokens = lex_with_spans(&initial_source).expect("Generated source should lex");
        let parsed_ast = flux_compiler::parser::parse(tokens)
            .expect("Generated source should parse");
        let formatted = Formatter::format(&parsed_ast, &initial_source);

        let theme = ColorTheme::default_theme();
        let colorized = colorize(&formatted, &theme, ColorMode::Always);

        // Re-lex the formatted source to get token spans
        let spanned_tokens = lex_with_spans(&formatted).expect("Formatted source should lex");

        for st in &spanned_tokens {
            let category = classify_token(&st.token);
            if category == TokenCategory::Keyword {
                let token_text = &formatted[st.span.start..st.span.end];
                let expected_colored = format!("\x1b[1;34m{}\x1b[0m", token_text);
                prop_assert!(
                    colorized.contains(&expected_colored),
                    "Keyword '{}' should be wrapped with bold blue.\nColorized: {:?}",
                    token_text,
                    colorized
                );
            }
        }
    }

    /// Property 9c: Integer literals are colored with cyan.
    ///
    /// For any valid Flux program containing integer literals, every integer
    /// token is wrapped with cyan ANSI codes (`\x1b[36m...\x1b[0m`).
    #[test]
    fn prop_colorize_integers_cyan(program in arb_program()) {
        let initial_source = pretty_print_program(&program);
        let tokens = lex_with_spans(&initial_source).expect("Generated source should lex");
        let parsed_ast = flux_compiler::parser::parse(tokens)
            .expect("Generated source should parse");
        let formatted = Formatter::format(&parsed_ast, &initial_source);

        let theme = ColorTheme::default_theme();
        let colorized = colorize(&formatted, &theme, ColorMode::Always);

        let spanned_tokens = lex_with_spans(&formatted).expect("Formatted source should lex");

        for st in &spanned_tokens {
            let category = classify_token(&st.token);
            if category == TokenCategory::IntegerLiteral {
                let token_text = &formatted[st.span.start..st.span.end];
                let expected_colored = format!("\x1b[36m{}\x1b[0m", token_text);
                prop_assert!(
                    colorized.contains(&expected_colored),
                    "Integer '{}' should be wrapped with cyan.\nColorized: {:?}",
                    token_text,
                    colorized
                );
            }
        }
    }

    /// Property 9d: Float literals are colored with cyan.
    ///
    /// For any valid Flux program containing float literals, every float
    /// token is wrapped with cyan ANSI codes (`\x1b[36m...\x1b[0m`).
    #[test]
    fn prop_colorize_floats_cyan(program in arb_program()) {
        let initial_source = pretty_print_program(&program);
        let tokens = lex_with_spans(&initial_source).expect("Generated source should lex");
        let parsed_ast = flux_compiler::parser::parse(tokens)
            .expect("Generated source should parse");
        let formatted = Formatter::format(&parsed_ast, &initial_source);

        let theme = ColorTheme::default_theme();
        let colorized = colorize(&formatted, &theme, ColorMode::Always);

        let spanned_tokens = lex_with_spans(&formatted).expect("Formatted source should lex");

        for st in &spanned_tokens {
            let category = classify_token(&st.token);
            if category == TokenCategory::FloatLiteral {
                let token_text = &formatted[st.span.start..st.span.end];
                let expected_colored = format!("\x1b[36m{}\x1b[0m", token_text);
                prop_assert!(
                    colorized.contains(&expected_colored),
                    "Float '{}' should be wrapped with cyan.\nColorized: {:?}",
                    token_text,
                    colorized
                );
            }
        }
    }

    /// Property 9e: String literals are colored with green.
    ///
    /// For any valid Flux program containing string literals, every string
    /// token (including quotes) is wrapped with green ANSI codes (`\x1b[32m...\x1b[0m`).
    #[test]
    fn prop_colorize_strings_green(program in arb_program()) {
        let initial_source = pretty_print_program(&program);
        let tokens = lex_with_spans(&initial_source).expect("Generated source should lex");
        let parsed_ast = flux_compiler::parser::parse(tokens)
            .expect("Generated source should parse");
        let formatted = Formatter::format(&parsed_ast, &initial_source);

        let theme = ColorTheme::default_theme();
        let colorized = colorize(&formatted, &theme, ColorMode::Always);

        let spanned_tokens = lex_with_spans(&formatted).expect("Formatted source should lex");

        for st in &spanned_tokens {
            let category = classify_token(&st.token);
            if category == TokenCategory::StringLiteral {
                let token_text = &formatted[st.span.start..st.span.end];
                let expected_colored = format!("\x1b[32m{}\x1b[0m", token_text);
                prop_assert!(
                    colorized.contains(&expected_colored),
                    "String '{}' should be wrapped with green.\nColorized: {:?}",
                    token_text,
                    colorized
                );
            }
        }
    }

    /// Property 9f: Signal functions are colored with bold magenta.
    ///
    /// For any valid Flux program containing signal function calls (OPEN, CLOSE, CLOSE_QTY),
    /// the signal function name is wrapped with bold magenta codes (`\x1b[1;35m...\x1b[0m`).
    #[test]
    fn prop_colorize_signal_functions_bold_magenta(program in arb_program()) {
        let initial_source = pretty_print_program(&program);
        let tokens = lex_with_spans(&initial_source).expect("Generated source should lex");
        let parsed_ast = flux_compiler::parser::parse(tokens)
            .expect("Generated source should parse");
        let formatted = Formatter::format(&parsed_ast, &initial_source);

        let theme = ColorTheme::default_theme();
        let colorized = colorize(&formatted, &theme, ColorMode::Always);

        let spanned_tokens = lex_with_spans(&formatted).expect("Formatted source should lex");

        for st in &spanned_tokens {
            let category = classify_token(&st.token);
            if category == TokenCategory::SignalFunction {
                let token_text = &formatted[st.span.start..st.span.end];
                let expected_colored = format!("\x1b[1;35m{}\x1b[0m", token_text);
                prop_assert!(
                    colorized.contains(&expected_colored),
                    "Signal function '{}' should be wrapped with bold magenta.\nColorized: {:?}",
                    token_text,
                    colorized
                );
            }
        }
    }

    /// Property 9g: No token has mixed ANSI codes from different categories.
    ///
    /// For any valid Flux program, within each colorized token span, there is
    /// at most ONE ANSI prefix code. A token should never be partially in one
    /// color and partially in another.
    #[test]
    fn prop_colorize_no_mixed_codes_within_token(program in arb_program()) {
        let initial_source = pretty_print_program(&program);
        let tokens = lex_with_spans(&initial_source).expect("Generated source should lex");
        let parsed_ast = flux_compiler::parser::parse(tokens)
            .expect("Generated source should parse");
        let formatted = Formatter::format(&parsed_ast, &initial_source);

        let theme = ColorTheme::default_theme();
        let colorized = colorize(&formatted, &theme, ColorMode::Always);

        // Extract all ANSI-wrapped spans
        let spans = extract_ansi_spans(&colorized);

        for (prefix, text, _suffix) in &spans {
            // The text within an ANSI span should not itself contain ANSI codes
            prop_assert!(
                !contains_ansi(text),
                "Token text within ANSI span should not contain nested ANSI codes.\n\
                 Prefix: {:?}, Text: {:?}",
                prefix,
                text
            );
        }
    }

    /// Property 9h: Numerics (int and float) have a distinct color from identifiers.
    ///
    /// For any valid Flux program, integer and float literals use cyan (`\x1b[36m`)
    /// while identifiers are unstyled (no ANSI code). This ensures numeric tokens
    /// are visually distinct from identifier tokens.
    #[test]
    fn prop_colorize_numerics_distinct_from_identifiers(program in arb_program()) {
        let theme = ColorTheme::default_theme();

        // Verify at theme level that integer/float prefix differs from identifier prefix
        let int_prefix = expected_prefix(TokenCategory::IntegerLiteral);
        let float_prefix = expected_prefix(TokenCategory::FloatLiteral);
        let ident_prefix = expected_prefix(TokenCategory::Identifier);

        prop_assert_ne!(
            int_prefix, ident_prefix,
            "Integer color must differ from identifier color"
        );
        prop_assert_ne!(
            float_prefix, ident_prefix,
            "Float color must differ from identifier color"
        );

        // Also verify in the actual colorized output
        let initial_source = pretty_print_program(&program);
        let tokens = lex_with_spans(&initial_source).expect("Generated source should lex");
        let parsed_ast = flux_compiler::parser::parse(tokens)
            .expect("Generated source should parse");
        let formatted = Formatter::format(&parsed_ast, &initial_source);
        let colorized = colorize(&formatted, &theme, ColorMode::Always);

        let spanned_tokens = lex_with_spans(&formatted).expect("Formatted source should lex");

        // Verify that integers in the output actually have the cyan prefix
        for st in &spanned_tokens {
            let category = classify_token(&st.token);
            if category == TokenCategory::IntegerLiteral || category == TokenCategory::FloatLiteral {
                let token_text = &formatted[st.span.start..st.span.end];
                let expected_colored = format!("\x1b[36m{}\x1b[0m", token_text);
                prop_assert!(
                    colorized.contains(&expected_colored),
                    "Numeric token '{}' should have cyan color, distinct from identifiers.\n\
                     Colorized: {:?}",
                    token_text,
                    colorized
                );
            }
        }
    }
}
