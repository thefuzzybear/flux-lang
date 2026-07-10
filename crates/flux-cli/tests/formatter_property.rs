// Feature: flux-fmt-syntax-highlight, Property 1: AST Round-Trip Preservation
//!
//! Property-based test verifying that for any valid Flux Program AST,
//! formatting the AST and re-parsing produces a structurally identical AST
//! (ignoring Span values).
//!
//! **Validates: Requirements 8.1, 8.2, 8.4**

use flux_compiler::lexer::{lex_with_spans, Span};
use flux_compiler::parser::ast::{
    Assignment, BinOp, EventHandler, Expr, ExprKind, ExprStmt, ForLoop, IfStmt, Import, Param,
    ParamsBlock, Program, ReturnStmt, StateBlock, StateVar, Stmt,
    Strategy as AstStrategy, StrategyItem, UnaryOp, WhileLoop,
};
use flux_compiler::parser::{parse, pretty_print_program};

use proptest::prelude::*;

// ============================================================================
// Helpers
// ============================================================================

fn dummy_span() -> Span {
    Span::new(0, 0)
}

fn is_keyword(s: &str) -> bool {
    matches!(
        s,
        "strategy" | "params" | "state" | "on" | "if" | "elif" | "else" | "for" | "while"
            | "return" | "from" | "import" | "and" | "or" | "not" | "true" | "false" | "null"
            | "in" | "fn" | "struct" | "data" | "connector"
    )
}

// ============================================================================
// AST Generators
// ============================================================================

/// Valid identifier: lowercase alpha start, not a keyword, doesn't start with "on_"
fn arb_ident() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9]{0,5}".prop_filter("not keyword or on_ prefix", |s| {
        !is_keyword(s) && !s.starts_with("on_")
    })
}

/// Event name (the part after "on_" / "on ")
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

fn arb_unaryop() -> impl Strategy<Value = UnaryOp> {
    prop_oneof![Just(UnaryOp::Neg), Just(UnaryOp::Not)]
}

fn arb_leaf_expr() -> impl Strategy<Value = Expr> {
    prop_oneof![
        // Integer literal
        (1i64..1000).prop_map(|v| Expr {
            kind: ExprKind::IntLiteral(v),
            span: dummy_span(),
        }),
        // Float literal: integer.decimal form for clean round-trip
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
        // String literal (printable ASCII, no problematic escaping)
        "[a-zA-Z0-9 ]{0,10}".prop_map(|s| Expr {
            kind: ExprKind::StringLiteral(s),
            span: dummy_span(),
        }),
    ]
}

fn arb_expr() -> impl Strategy<Value = Expr> {
    arb_leaf_expr().prop_recursive(3, 16, 4, |inner| {
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
            // Unary op
            (arb_unaryop(), inner.clone()).prop_map(|(op, e)| Expr {
                kind: ExprKind::UnaryOp {
                    op,
                    operand: Box::new(e),
                },
                span: dummy_span(),
            }),
            // Function call
            (
                arb_ident(),
                proptest::collection::vec(inner.clone(), 0..3)
            )
                .prop_map(|(name, args)| Expr {
                    kind: ExprKind::FunctionCall {
                        function: Box::new(Expr {
                            kind: ExprKind::Ident(name),
                            span: dummy_span(),
                        }),
                        args,
                    },
                    span: dummy_span(),
                }),
            // List literal
            proptest::collection::vec(inner, 0..3).prop_map(|elems| Expr {
                kind: ExprKind::ListLiteral(elems),
                span: dummy_span(),
            }),
        ]
    })
}

fn arb_stmt() -> impl Strategy<Value = Stmt> {
    prop_oneof![
        // Assignment: ident = expr
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
        // Return with value
        arb_leaf_expr().prop_map(|e| {
            Stmt::Return(ReturnStmt {
                value: Some(e),
                span: dummy_span(),
            })
        }),
        // Expression statement (function call)
        (arb_ident(), proptest::collection::vec(arb_leaf_expr(), 0..3)).prop_map(|(name, args)| {
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
        // If statement (simple, no elif/else)
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

/// Generate a list of statements where bare `return` (no value) is only
/// placed at the end to avoid parse ambiguity.
fn arb_stmts(count: std::ops::Range<usize>) -> impl Strategy<Value = Vec<Stmt>> {
    (proptest::collection::vec(arb_stmt(), count), proptest::bool::ANY)
        .prop_map(|(stmts, append_bare_return)| {
            let mut result = stmts;
            if append_bare_return {
                result.push(Stmt::Return(ReturnStmt {
                    value: None,
                    span: dummy_span(),
                }));
            }
            result
        })
        .prop_filter("must have at least one statement", |v| !v.is_empty())
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
            1..4,
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
            1..4,
        )
        .prop_map(|variables| StrategyItem::StateBlock(StateBlock {
            variables,
            span: dummy_span(),
        })),
        // Event handler
        (arb_event_name(), arb_stmts(1..4)).prop_map(|(event_name, body)| {
            StrategyItem::EventHandler(EventHandler {
                event_name,
                body,
                span: dummy_span(),
            })
        }),
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

fn arb_program() -> impl Strategy<Value = Program> {
    (
        proptest::collection::vec(arb_import(), 0..2),
        arb_ident(),
        proptest::collection::vec(arb_strategy_item(), 1..3),
    )
        .prop_map(|(imports, name, body)| Program {
            structs: vec![], enums: vec![],
            imports,
            functions: vec![],
            impl_blocks: vec![],
            data_block: None,
            connector_block: None,
            strategy: AstStrategy {
                name,
                body,
                span: dummy_span(),
            },
            span: dummy_span(),
        })
}

// ============================================================================
// Span-ignoring structural equality
// ============================================================================

fn programs_eq(a: &Program, b: &Program) -> bool {
    a.imports.len() == b.imports.len()
        && a.imports
            .iter()
            .zip(b.imports.iter())
            .all(|(ia, ib)| ia.module_path == ib.module_path && ia.names == ib.names)
        && a.strategy.name == b.strategy.name
        && a.strategy.body.len() == b.strategy.body.len()
        && a.strategy
            .body
            .iter()
            .zip(b.strategy.body.iter())
            .all(|(ia, ib)| items_eq(ia, ib))
}

fn items_eq(a: &StrategyItem, b: &StrategyItem) -> bool {
    match (a, b) {
        (StrategyItem::Property(pa), StrategyItem::Property(pb)) => {
            pa.name == pb.name && exprs_eq(&pa.value, &pb.value)
        }
        (StrategyItem::ParamsBlock(pa), StrategyItem::ParamsBlock(pb)) => {
            pa.params.len() == pb.params.len()
                && pa
                    .params
                    .iter()
                    .zip(pb.params.iter())
                    .all(|(a, b)| a.name == b.name && exprs_eq(&a.default_value, &b.default_value))
        }
        (StrategyItem::StateBlock(sa), StrategyItem::StateBlock(sb)) => {
            sa.variables.len() == sb.variables.len()
                && sa
                    .variables
                    .iter()
                    .zip(sb.variables.iter())
                    .all(|(a, b)| a.name == b.name && exprs_eq(&a.initial_value, &b.initial_value))
        }
        (StrategyItem::EventHandler(ha), StrategyItem::EventHandler(hb)) => {
            ha.event_name == hb.event_name
                && ha.body.len() == hb.body.len()
                && ha
                    .body
                    .iter()
                    .zip(hb.body.iter())
                    .all(|(a, b)| stmts_eq(a, b))
        }
        _ => false,
    }
}

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
        (ExprKind::FloatLiteral(va), ExprKind::FloatLiteral(vb)) => {
            // Compare floats by their string representation to handle
            // formatting edge cases (e.g., 1.0 vs 1)
            va == vb || format!("{}", va) == format!("{}", vb)
        }
        (ExprKind::StringLiteral(sa), ExprKind::StringLiteral(sb)) => sa == sb,
        (ExprKind::BoolLiteral(ba), ExprKind::BoolLiteral(bb)) => ba == bb,
        (ExprKind::NullLiteral, ExprKind::NullLiteral) => true,
        (ExprKind::Ident(na), ExprKind::Ident(nb)) => na == nb,
        (ExprKind::ListLiteral(ea), ExprKind::ListLiteral(eb)) => {
            ea.len() == eb.len()
                && ea.iter().zip(eb.iter()).all(|(a, b)| exprs_eq(a, b))
        }
        (
            ExprKind::BinaryOp {
                left: la,
                op: opa,
                right: ra,
            },
            ExprKind::BinaryOp {
                left: lb,
                op: opb,
                right: rb,
            },
        ) => opa == opb && exprs_eq(la, lb) && exprs_eq(ra, rb),
        (
            ExprKind::UnaryOp {
                op: opa,
                operand: ea,
            },
            ExprKind::UnaryOp {
                op: opb,
                operand: eb,
            },
        ) => opa == opb && exprs_eq(ea, eb),
        (
            ExprKind::FunctionCall {
                function: fa,
                args: aa,
            },
            ExprKind::FunctionCall {
                function: fb,
                args: ab,
            },
        ) => {
            exprs_eq(fa, fb)
                && aa.len() == ab.len()
                && aa.iter().zip(ab.iter()).all(|(a, b)| exprs_eq(a, b))
        }
        (
            ExprKind::MethodCall {
                receiver: ra,
                method: ma,
                args: aa,
            },
            ExprKind::MethodCall {
                receiver: rb,
                method: mb,
                args: ab,
            },
        ) => {
            ma == mb
                && exprs_eq(ra, rb)
                && aa.len() == ab.len()
                && aa.iter().zip(ab.iter()).all(|(a, b)| exprs_eq(a, b))
        }
        (
            ExprKind::MemberAccess {
                object: oa,
                field: fa,
            },
            ExprKind::MemberAccess {
                object: ob,
                field: fb,
            },
        ) => fa == fb && exprs_eq(oa, ob),
        (
            ExprKind::IndexAccess {
                object: oa,
                index: ia,
            },
            ExprKind::IndexAccess {
                object: ob,
                index: ib,
            },
        ) => exprs_eq(oa, ob) && exprs_eq(ia, ib),
        _ => false,
    }
}

// ============================================================================
// Property Test: AST Round-Trip Preservation
// ============================================================================

// Feature: flux-fmt-syntax-highlight, Property 1: AST Round-Trip Preservation
// **Validates: Requirements 8.1, 8.2, 8.4**
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// For any valid Flux program, formatting the AST and re-parsing the
    /// formatted output SHALL produce a structurally identical AST
    /// (field-by-field equality of all nodes, excluding Span values).
    #[test]
    fn prop_ast_round_trip_preservation(program in arb_program()) {
        // Step 1: Pretty-print the generated AST to get a valid source string.
        // This uses the parser's pretty-printer which is known to produce parseable output.
        let initial_source = pretty_print_program(&program);

        // Step 2: Lex and parse the source to get a properly-spanned AST.
        let tokens = lex_with_spans(&initial_source).expect(
            &format!("Generated source should lex.\nSource:\n{}", initial_source)
        );
        let parsed_ast = parse(tokens).expect(
            &format!("Generated source should parse.\nSource:\n{}", initial_source)
        );

        // Step 3: Format the parsed AST using the Formatter under test.
        let formatted_source = flux_cli::formatter::Formatter::format(&parsed_ast, &initial_source);

        // Step 4: Re-parse the formatted output.
        let tokens2 = lex_with_spans(&formatted_source).expect(
            &format!(
                "Formatted output should lex.\nFormatted:\n{}\nOriginal source:\n{}",
                formatted_source, initial_source
            )
        );
        let reparsed_ast = parse(tokens2).expect(
            &format!(
                "Formatted output should parse.\nFormatted:\n{}\nOriginal source:\n{}",
                formatted_source, initial_source
            )
        );

        // Step 5: Compare the original generated AST with the reparsed AST,
        // ignoring all Span values.
        prop_assert!(
            programs_eq(&program, &reparsed_ast),
            "AST round-trip preservation failed!\n\
             Original AST (generated): {:?}\n\n\
             Pretty-printed source:\n{}\n\n\
             Formatted source:\n{}\n\n\
             Reparsed AST: {:?}",
            program,
            initial_source,
            formatted_source,
            reparsed_ast
        );
    }
}
