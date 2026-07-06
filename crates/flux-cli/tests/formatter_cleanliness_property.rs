// Feature: flux-fmt-syntax-highlight, Property 6: Output Cleanliness
//!
//! Property: For any valid Flux program, the formatted output SHALL contain
//! no trailing whitespace on any line, no consecutive blank lines (at most one
//! blank line between any two non-blank lines), and SHALL end with exactly one
//! newline character.
//!
//! **Validates: Requirements 3.11, 3.12, 3.7**

use proptest::prelude::*;

use flux_compiler::lexer::Span;
use flux_compiler::parser::ast::{
    Assignment, BinOp, ElifBranch, EventHandler, Expr, ExprKind, ExprStmt, ForLoop, IfStmt,
    Import, Param, ParamsBlock, Program, ReturnStmt, StateBlock, StateVar, Strategy as FluxStrategy,
    StrategyItem, Stmt, UnaryOp, WhileLoop,
};
use flux_cli::formatter::Formatter;

// =============================================================================
// AST Generators
// =============================================================================

const FLUX_KEYWORDS: &[&str] = &[
    "strategy", "params", "state", "on", "if", "elif", "else", "for", "while",
    "return", "from", "import", "and", "or", "not", "true", "false", "null",
    "in", "bar",
];

fn arb_ident() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_]{1,8}".prop_filter("must not be a keyword", |s| {
        !FLUX_KEYWORDS.contains(&s.as_str())
    })
}

fn arb_simple_expr() -> impl Strategy<Value = Expr> {
    prop_oneof![
        (1i64..1000).prop_map(|v| Expr {
            kind: ExprKind::IntLiteral(v),
            span: Span::new(0, 0),
        }),
        (1.0f64..1000.0).prop_map(|v| {
            let rounded = (v * 100.0).round() / 100.0;
            Expr {
                kind: ExprKind::FloatLiteral(rounded),
                span: Span::new(0, 0),
            }
        }),
        any::<bool>().prop_map(|v| Expr {
            kind: ExprKind::BoolLiteral(v),
            span: Span::new(0, 0),
        }),
        Just(Expr {
            kind: ExprKind::NullLiteral,
            span: Span::new(0, 0),
        }),
        arb_ident().prop_map(|name| Expr {
            kind: ExprKind::Ident(name),
            span: Span::new(0, 0),
        }),
        "[a-zA-Z0-9_ ]{0,10}".prop_map(|s| Expr {
            kind: ExprKind::StringLiteral(s),
            span: Span::new(0, 0),
        }),
    ]
}

fn arb_expr(depth: u32) -> BoxedStrategy<Expr> {
    if depth == 0 {
        return arb_simple_expr().boxed();
    }

    let next_depth = depth - 1;

    prop_oneof![
        4 => arb_simple_expr(),
        2 => (arb_expr(next_depth), arb_binop(), arb_expr(next_depth)).prop_map(
            |(left, op, right)| Expr {
                kind: ExprKind::BinaryOp {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                },
                span: Span::new(0, 0),
            }
        ),
        1 => (arb_unaryop(), arb_expr(next_depth)).prop_map(|(op, operand)| Expr {
            kind: ExprKind::UnaryOp {
                op,
                operand: Box::new(operand),
            },
            span: Span::new(0, 0),
        }),
        1 => (arb_ident(), proptest::collection::vec(arb_expr(next_depth), 0..3))
            .prop_map(|(name, args)| Expr {
                kind: ExprKind::FunctionCall {
                    function: Box::new(Expr {
                        kind: ExprKind::Ident(name),
                        span: Span::new(0, 0),
                    }),
                    args,
                },
                span: Span::new(0, 0),
            }),
        1 => (arb_ident(), arb_ident(), proptest::collection::vec(arb_expr(next_depth), 0..3))
            .prop_map(|(receiver, method, args)| Expr {
                kind: ExprKind::MethodCall {
                    receiver: Box::new(Expr {
                        kind: ExprKind::Ident(receiver),
                        span: Span::new(0, 0),
                    }),
                    method,
                    args,
                },
                span: Span::new(0, 0),
            }),
        1 => (arb_ident(), arb_ident()).prop_map(|(obj, field)| Expr {
            kind: ExprKind::MemberAccess {
                object: Box::new(Expr {
                    kind: ExprKind::Ident(obj),
                    span: Span::new(0, 0),
                }),
                field,
            },
            span: Span::new(0, 0),
        }),
        1 => proptest::collection::vec(arb_expr(next_depth), 0..4).prop_map(|elements| Expr {
            kind: ExprKind::ListLiteral(elements),
            span: Span::new(0, 0),
        }),
    ]
    .boxed()
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

fn arb_unaryop() -> impl Strategy<Value = UnaryOp> {
    prop_oneof![Just(UnaryOp::Neg), Just(UnaryOp::Not)]
}

fn arb_stmt(depth: u32) -> BoxedStrategy<Stmt> {
    if depth == 0 {
        return prop_oneof![
            (arb_ident(), arb_expr(1)).prop_map(|(name, value)| {
                Stmt::Assignment(Assignment {
                    target: Expr {
                        kind: ExprKind::Ident(name),
                        span: Span::new(0, 0),
                    },
                    value,
                    span: Span::new(0, 0),
                })
            }),
            (arb_ident(), proptest::collection::vec(arb_simple_expr(), 0..3))
                .prop_map(|(name, args)| {
                    Stmt::Expr(ExprStmt {
                        expr: Expr {
                            kind: ExprKind::FunctionCall {
                                function: Box::new(Expr {
                                    kind: ExprKind::Ident(name),
                                    span: Span::new(0, 0),
                                }),
                                args,
                            },
                            span: Span::new(0, 0),
                        },
                        span: Span::new(0, 0),
                    })
                }),
        ]
        .boxed();
    }

    let next_depth = depth - 1;

    prop_oneof![
        // Assignment
        3 => (arb_ident(), arb_expr(1)).prop_map(|(name, value)| {
            Stmt::Assignment(Assignment {
                target: Expr {
                    kind: ExprKind::Ident(name),
                    span: Span::new(0, 0),
                },
                value,
                span: Span::new(0, 0),
            })
        }),
        // Expression statement (function call)
        2 => (arb_ident(), proptest::collection::vec(arb_simple_expr(), 0..3))
            .prop_map(|(name, args)| {
                Stmt::Expr(ExprStmt {
                    expr: Expr {
                        kind: ExprKind::FunctionCall {
                            function: Box::new(Expr {
                                kind: ExprKind::Ident(name),
                                span: Span::new(0, 0),
                            }),
                            args,
                        },
                        span: Span::new(0, 0),
                    },
                    span: Span::new(0, 0),
                })
            }),
        // If with optional elif/else
        2 => (
            arb_expr(1),
            proptest::collection::vec(arb_stmt(next_depth), 1..3),
            proptest::collection::vec(
                (arb_expr(1), proptest::collection::vec(arb_stmt(next_depth), 1..2)),
                0..2,
            ),
            proptest::option::of(proptest::collection::vec(arb_stmt(next_depth), 1..2)),
        )
            .prop_map(|(cond, body, elifs, else_body)| {
                Stmt::If(IfStmt {
                    condition: cond,
                    body,
                    elif_branches: elifs
                        .into_iter()
                        .map(|(c, b)| ElifBranch {
                            condition: c,
                            body: b,
                            span: Span::new(0, 0),
                        })
                        .collect(),
                    else_body,
                    span: Span::new(0, 0),
                })
            }),
        // For loop
        1 => (
            arb_ident(),
            arb_expr(1),
            proptest::collection::vec(arb_stmt(next_depth), 1..3),
        )
            .prop_map(|(var, iterable, body)| {
                Stmt::For(ForLoop {
                    variable: var,
                    iterable,
                    body,
                    span: Span::new(0, 0),
                })
            }),
        // While loop
        1 => (
            arb_expr(1),
            proptest::collection::vec(arb_stmt(next_depth), 1..3),
        )
            .prop_map(|(cond, body)| {
                Stmt::While(WhileLoop {
                    condition: cond,
                    body,
                    span: Span::new(0, 0),
                })
            }),
    ]
    .boxed()
}

fn arb_stmt_block(depth: u32, min_len: usize, max_len: usize) -> BoxedStrategy<Vec<Stmt>> {
    (
        proptest::collection::vec(arb_stmt(depth), min_len..max_len),
        proptest::option::weighted(0.2, arb_expr(1)),
    )
        .prop_map(|(mut stmts, opt_return_expr)| {
            if let Some(ret_val) = opt_return_expr {
                stmts.push(Stmt::Return(ReturnStmt {
                    value: Some(ret_val),
                    span: Span::new(0, 0),
                }));
            }
            stmts
        })
        .boxed()
}

fn arb_param() -> impl Strategy<Value = Param> {
    (arb_ident(), arb_simple_expr()).prop_map(|(name, default_value)| Param {
        name,
        default_value,
        span: Span::new(0, 0),
    })
}

fn arb_state_var() -> impl Strategy<Value = StateVar> {
    (arb_ident(), arb_simple_expr()).prop_map(|(name, initial_value)| StateVar {
        name,
        initial_value,
        span: Span::new(0, 0),
    })
}

fn arb_strategy_item() -> impl Strategy<Value = StrategyItem> {
    prop_oneof![
        // ParamsBlock
        proptest::collection::vec(arb_param(), 1..4)
            .prop_map(|params| StrategyItem::ParamsBlock(ParamsBlock {
                params,
                span: Span::new(0, 0),
            })),
        // StateBlock
        proptest::collection::vec(arb_state_var(), 1..4)
            .prop_map(|variables| StrategyItem::StateBlock(StateBlock {
                variables,
                span: Span::new(0, 0),
            })),
        // EventHandler
        arb_stmt_block(2, 1, 5)
            .prop_map(|body| StrategyItem::EventHandler(EventHandler {
                event_name: "bar".to_string(),
                body,
                span: Span::new(0, 0),
            })),
    ]
}

fn arb_program() -> impl Strategy<Value = Program> {
    (
        // Optional imports (0-2)
        proptest::collection::vec(
            (
                "[a-z]{3,8}(\\.[a-z]{3,8}){0,2}".prop_filter("valid module path", |s| {
                    s.split('.').all(|part| !FLUX_KEYWORDS.contains(&part))
                }),
                proptest::collection::vec(arb_ident(), 1..4),
            ),
            0..3,
        ),
        // Strategy name
        "[A-Z][a-zA-Z]{2,10}".prop_filter("must not be a keyword", |s| {
            !FLUX_KEYWORDS.contains(&s.to_lowercase().as_str())
        }),
        // Strategy body items (1-5 for more variety)
        proptest::collection::vec(arb_strategy_item(), 1..5),
    )
        .prop_map(|(imports, name, body)| Program {
            imports: imports
                .into_iter()
                .map(|(module_path, names)| Import {
                    module_path,
                    names,
                    span: Span::new(0, 0),
                })
                .collect(),
            strategy: FluxStrategy {
                name,
                body,
                span: Span::new(0, 0),
            },
            span: Span::new(0, 0),
        })
}

// =============================================================================
// Property Test: Output Cleanliness
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 3.11, 3.12, 3.7**
    ///
    /// For any valid Flux program, the formatted output:
    /// 1. Contains no trailing whitespace on any line
    /// 2. Contains no consecutive blank lines
    /// 3. Ends with exactly one newline character
    #[test]
    fn prop_output_cleanliness(program in arb_program()) {
        let formatted = Formatter::format(&program, "");

        // Property 1: No trailing whitespace on any line
        for (line_num, line) in formatted.lines().enumerate() {
            let trimmed_end = line.trim_end();
            prop_assert_eq!(
                line, trimmed_end,
                "Line {} has trailing whitespace.\nLine content: {:?}\nFull output:\n{}",
                line_num + 1, line, formatted
            );
        }

        // Property 2: No consecutive blank lines
        let lines: Vec<&str> = formatted.lines().collect();
        for i in 0..lines.len().saturating_sub(1) {
            let both_blank = lines[i].is_empty() && lines[i + 1].is_empty();
            prop_assert!(
                !both_blank,
                "Consecutive blank lines found at lines {} and {}.\nFull output:\n{}",
                i + 1, i + 2, formatted
            );
        }

        // Property 3: Exactly one trailing newline
        prop_assert!(
            formatted.ends_with('\n'),
            "Output does not end with a newline.\nFull output:\n{:?}",
            formatted
        );
        prop_assert!(
            !formatted.ends_with("\n\n"),
            "Output ends with multiple newlines.\nFull output:\n{:?}",
            formatted
        );
    }
}
