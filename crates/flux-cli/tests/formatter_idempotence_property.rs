// Feature: flux-fmt-syntax-highlight, Property 2: Formatter Idempotence
//!
//! Property: For any valid Flux program, formatting the output of the formatter
//! a second time SHALL produce byte-for-byte identical output to the first
//! formatting. That is, `format(format(source)) == format(source)`.
//!
//! **Validates: Requirements 8.3, 3.14**

use proptest::prelude::*;

use flux_compiler::lexer::{self, Span};
use flux_compiler::parser::ast::{
    Assignment, BinOp, ElifBranch, EventHandler, Expr, ExprKind, ExprStmt, ForLoop, IfStmt,
    Import, Param, ParamsBlock, Program, ReturnStmt, StateBlock, StateVar,
    StrategyItem, Stmt, UnaryOp, WhileLoop,
};
// Import the AST Strategy struct with an alias to avoid conflict with proptest::Strategy trait
use flux_compiler::parser::ast::Strategy as FluxStrategy;
use flux_cli::formatter::Formatter;

// =============================================================================
// AST Generators — produce random valid Flux ASTs for idempotence testing
// =============================================================================

/// Reserved keywords that cannot be used as identifiers in Flux.
const FLUX_KEYWORDS: &[&str] = &[
    "strategy", "params", "state", "on", "if", "elif", "else", "for", "while",
    "return", "from", "import", "and", "or", "not", "true", "false", "null",
    "in", "bar",
];

/// Generate a valid Flux identifier (not a keyword).
fn arb_ident() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_]{1,8}"
        .prop_filter("must not be a keyword", |s| {
            !FLUX_KEYWORDS.contains(&s.as_str())
        })
}

/// Generate a simple expression (leaf nodes only, no recursion).
fn arb_simple_expr() -> impl Strategy<Value = Expr> {
    prop_oneof![
        // Integer literal
        (1i64..1000).prop_map(|v| Expr {
            kind: ExprKind::IntLiteral(v),
            span: Span::new(0, 0),
        }),
        // Float literal (finite values only, rounded for stable formatting)
        (1.0f64..1000.0).prop_map(|v| {
            let rounded = (v * 100.0).round() / 100.0;
            Expr {
                kind: ExprKind::FloatLiteral(rounded),
                span: Span::new(0, 0),
            }
        }),
        // Bool literal
        any::<bool>().prop_map(|v| Expr {
            kind: ExprKind::BoolLiteral(v),
            span: Span::new(0, 0),
        }),
        // Null literal
        Just(Expr {
            kind: ExprKind::NullLiteral,
            span: Span::new(0, 0),
        }),
        // Identifier
        arb_ident().prop_map(|name| Expr {
            kind: ExprKind::Ident(name),
            span: Span::new(0, 0),
        }),
        // String literal (simple ASCII, no problematic chars)
        "[a-zA-Z0-9_ ]{0,10}".prop_map(|s| Expr {
            kind: ExprKind::StringLiteral(s),
            span: Span::new(0, 0),
        }),
    ]
}

/// Generate an expression with limited recursion depth.
fn arb_expr(depth: u32) -> BoxedStrategy<Expr> {
    if depth == 0 {
        return arb_simple_expr().boxed();
    }

    let leaf = arb_simple_expr();
    let next_depth = depth - 1;

    prop_oneof![
        4 => leaf,
        // Binary operation
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
        // Unary operation
        1 => (arb_unaryop(), arb_expr(next_depth)).prop_map(|(op, operand)| Expr {
            kind: ExprKind::UnaryOp {
                op,
                operand: Box::new(operand),
            },
            span: Span::new(0, 0),
        }),
        // Function call
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
        // Method call
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
        // Member access
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
        // Index access
        1 => (arb_ident(), arb_expr(next_depth)).prop_map(|(obj, index)| Expr {
            kind: ExprKind::IndexAccess {
                object: Box::new(Expr {
                    kind: ExprKind::Ident(obj),
                    span: Span::new(0, 0),
                }),
                index: Box::new(index),
            },
            span: Span::new(0, 0),
        }),
        // List literal
        1 => proptest::collection::vec(arb_expr(next_depth), 0..4).prop_map(|elements| Expr {
            kind: ExprKind::ListLiteral(elements),
            span: Span::new(0, 0),
        }),
    ]
    .boxed()
}

/// Generate a random binary operator.
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

/// Generate a random unary operator.
fn arb_unaryop() -> impl Strategy<Value = UnaryOp> {
    prop_oneof![Just(UnaryOp::Neg), Just(UnaryOp::Not),]
}

/// Generate a non-return statement with limited depth.
/// Return statements are handled specially (only at end of blocks) to avoid
/// parser ambiguity where `return\n x = 1` is parsed as `return x` then fails on `=`.
fn arb_stmt(depth: u32) -> BoxedStrategy<Stmt> {
    if depth == 0 {
        // At depth 0, only generate simple assignments or function-call expression statements.
        // We avoid generating bare expression statements that could start with `[`
        // because the parser may merge them with the preceding line (ASI-like ambiguity).
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
            // Expression statement: only function/method calls (safe starts with identifier)
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
        // Expression statement: only function/method calls (starts with identifier, not `[`)
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
        // If statement
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

/// Generate a statement block (Vec<Stmt>) that may optionally end with a return.
/// This ensures return statements only appear at the end of a block, avoiding
/// the parser ambiguity where code following `return` is consumed as its expression.
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

/// Generate a random Param (parameter with default value).
fn arb_param() -> impl Strategy<Value = Param> {
    (arb_ident(), arb_simple_expr()).prop_map(|(name, default_value)| Param {
        name,
        default_value,
        span: Span::new(0, 0),
    })
}

/// Generate a random StateVar.
fn arb_state_var() -> impl Strategy<Value = StateVar> {
    (arb_ident(), arb_simple_expr()).prop_map(|(name, initial_value)| StateVar {
        name,
        initial_value,
        span: Span::new(0, 0),
    })
}

/// Generate a random StrategyItem.
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
        // EventHandler (uses arb_stmt_block so return is only at end)
        arb_stmt_block(2, 1, 5)
            .prop_map(|body| StrategyItem::EventHandler(EventHandler {
                event_name: "bar".to_string(),
                body,
                span: Span::new(0, 0),
            })),
    ]
}

/// Generate a random Program AST.
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
        // Strategy body items (1-4)
        proptest::collection::vec(arb_strategy_item(), 1..4),
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
            data_block: None,
            connector_block: None,
            strategy: FluxStrategy {
                name,
                body,
                span: Span::new(0, 0),
            },
            span: Span::new(0, 0),
        })
}

// =============================================================================
// Property Test: Formatter Idempotence
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 8.3, 3.14**
    ///
    /// For any valid Flux program, formatting the output of the formatter a second
    /// time produces byte-for-byte identical output to the first formatting.
    /// `format(format(source)) == format(source)`
    #[test]
    fn prop_formatter_idempotence(program in arb_program()) {
        // Step 1: Format the generated AST to get a valid source string.
        // We use an empty source for comments (generated ASTs have no comments).
        let first_format = Formatter::format(&program, "");

        // Step 2: Parse the formatted output to get a new AST.
        let tokens = lexer::lex_with_spans(&first_format)
            .expect("Formatted output should lex successfully");
        let ast2 = flux_compiler::parser::parse(tokens)
            .expect("Formatted output should parse successfully");

        // Step 3: Format the re-parsed AST a second time.
        let second_format = Formatter::format(&ast2, &first_format);

        // Step 4: Assert byte-for-byte equality.
        prop_assert_eq!(
            &first_format,
            &second_format,
            "Formatter idempotence violated: format(format(source)) != format(source)\n\
             First format ({} bytes):\n{}\n\
             Second format ({} bytes):\n{}",
            first_format.len(),
            first_format,
            second_format.len(),
            second_format,
        );
    }
}
