//! Property-based tests for the Flux parser.
//!
//! Uses proptest to verify the Parse–Pretty-Print Round Trip property:
//! generating random valid AST nodes, pretty-printing them, lexing/parsing
//! the result, and comparing structurally (ignoring spans).

#[cfg(test)]
mod tests {
    use crate::lexer::{lex_with_spans, Span};
    use crate::parser::ast::{
        Assignment, BinOp, EventHandler, Expr, ExprKind, ExprStmt, FnDef, FnParam, ForLoop,
        IfStmt, Import, Param, ParamsBlock, Program, Property, ReturnStmt, StateBlock, StateVar,
        Stmt, Strategy as AstStrategy, StrategyItem, UnaryOp, WhileLoop,
    };
    use crate::parser::{parse, pretty_print_program};
    use proptest::prelude::*;

    // ========================================================================
    // Helpers
    // ========================================================================

    fn dummy_span() -> Span {
        Span::new(0, 0)
    }

    fn is_keyword(s: &str) -> bool {
        matches!(
            s,
            "strategy" | "params" | "state" | "on" | "if" | "elif" | "else"
                | "for" | "while" | "return" | "fn" | "from" | "import" | "and" | "or"
                | "not" | "true" | "false" | "null" | "in" | "data" | "connector"
        )
    }

    // ========================================================================
    // AST Generators
    // ========================================================================

    /// Valid identifier: lowercase alpha start, not a keyword, doesn't start with "on_"
    fn arb_ident() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9_]{0,6}".prop_filter("not keyword or on_ prefix", |s| {
            !is_keyword(s) && !s.starts_with("on_")
        })
    }

    /// Event name (the part after "on_")
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
            (1i64..1000).prop_map(|v| Expr {
                kind: ExprKind::IntLiteral(v),
                span: dummy_span(),
            }),
            // Float: use integer.decimal form to ensure clean round-trip
            (1u32..999, 1u32..99).prop_map(|(i, d)| {
                let s = format!("{}.{}", i, d);
                let f: f64 = s.parse().unwrap();
                Expr {
                    kind: ExprKind::FloatLiteral(f),
                    span: dummy_span(),
                }
            }),
            any::<bool>().prop_map(|b| Expr {
                kind: ExprKind::BoolLiteral(b),
                span: dummy_span(),
            }),
            arb_ident().prop_map(|s| Expr {
                kind: ExprKind::Ident(s),
                span: dummy_span(),
            }),
            Just(Expr {
                kind: ExprKind::NullLiteral,
                span: dummy_span(),
            }),
            // String literal (printable ASCII, no problematic chars)
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
                (arb_ident(), proptest::collection::vec(inner.clone(), 0..3)).prop_map(
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
            (arb_ident(), proptest::collection::vec(arb_leaf_expr(), 0..3)).prop_map(
                |(name, args)| {
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
                }
            ),
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
    /// placed at the end to avoid parse ambiguity (the parser would consume
    /// the next statement's leading expression as the return value).
    fn arb_stmts(count: std::ops::Range<usize>) -> impl Strategy<Value = Vec<Stmt>> {
        (
            proptest::collection::vec(arb_stmt(), count),
            proptest::bool::ANY,
        )
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
            // Property
            (arb_ident(), arb_expr()).prop_map(|(name, value)| {
                StrategyItem::Property(Property {
                    name,
                    value,
                    span: dummy_span(),
                })
            }),
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
            (arb_event_name(), arb_stmts(1..4)).prop_map(
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
            proptest::collection::vec(arb_module_segment(), 1..4),
            proptest::collection::vec(arb_ident(), 1..4),
        )
            .prop_map(|(segments, names)| Import {
                module_path: segments.join("."),
                names,
                span: dummy_span(),
            })
    }

    fn arb_program() -> impl Strategy<Value = Program> {
        (
            proptest::collection::vec(arb_import(), 0..3),
            arb_ident(),
            proptest::collection::vec(arb_strategy_item(), 1..4),
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

    /// Generate a valid function definition with random name, 0–8 params, and random body.
    /// Used for the FnDef round-trip property test.
    fn arb_fn_def() -> impl Strategy<Value = FnDef> {
        (
            arb_ident(),
            proptest::collection::vec(arb_ident(), 0..8),
            arb_fn_body_stmts(),
        )
            .prop_map(|(name, params, body)| {
                // Deduplicate params to avoid invalid duplicate parameter names
                let mut seen = std::collections::HashSet::new();
                let unique_params: Vec<FnParam> = params
                    .into_iter()
                    .filter(|p| seen.insert(p.clone()))
                    .map(|name| FnParam {
                        name,
                        param_type: None,
                        span: dummy_span(),
                    })
                    .collect();
                FnDef {
                    name,
                    params: unique_params,
                    return_type: None,
                    body,
                    span: dummy_span(),
                }
            })
    }

    /// Generate simple body statements suitable for functions:
    /// assignments (x = 1.0), return statements (return x), function calls (foo(x))
    fn arb_fn_body_stmts() -> impl Strategy<Value = Vec<Stmt>> {
        proptest::collection::vec(arb_fn_body_stmt(), 1..6)
    }

    fn arb_fn_body_stmt() -> impl Strategy<Value = Stmt> {
        prop_oneof![
            // Assignment: ident = expr
            (arb_ident(), arb_leaf_expr()).prop_map(|(name, value)| {
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
            (arb_ident(), proptest::collection::vec(arb_leaf_expr(), 0..3)).prop_map(
                |(name, args)| {
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
                }
            ),
        ]
    }

    // ========================================================================
    // Span-ignoring structural equality
    // ========================================================================

    fn programs_eq(a: &Program, b: &Program) -> bool {
        a.imports.len() == b.imports.len()
            && a.imports
                .iter()
                .zip(b.imports.iter())
                .all(|(ia, ib)| ia.module_path == ib.module_path && ia.names == ib.names)
            && a.functions.len() == b.functions.len()
            && a.functions
                .iter()
                .zip(b.functions.iter())
                .all(|(fa, fb)| fn_defs_eq(fa, fb))
            && a.strategy.name == b.strategy.name
            && a.strategy.body.len() == b.strategy.body.len()
            && a.strategy
                .body
                .iter()
                .zip(b.strategy.body.iter())
                .all(|(ia, ib)| items_eq(ia, ib))
    }

    fn fn_defs_eq(a: &FnDef, b: &FnDef) -> bool {
        a.name == b.name
            && a.params.len() == b.params.len()
            && a.params
                .iter()
                .zip(b.params.iter())
                .all(|(pa, pb)| pa.name == pb.name && pa.param_type == pb.param_type)
            && a.return_type == b.return_type
            && a.body.len() == b.body.len()
            && a.body
                .iter()
                .zip(b.body.iter())
                .all(|(sa, sb)| stmts_eq(sa, sb))
    }

    fn items_eq(a: &StrategyItem, b: &StrategyItem) -> bool {
        match (a, b) {
            (StrategyItem::Property(pa), StrategyItem::Property(pb)) => {
                pa.name == pb.name && exprs_eq(&pa.value, &pb.value)
            }
            (StrategyItem::ParamsBlock(pa), StrategyItem::ParamsBlock(pb)) => {
                pa.params.len() == pb.params.len()
                    && pa.params.iter().zip(pb.params.iter()).all(|(a, b)| {
                        a.name == b.name && exprs_eq(&a.default_value, &b.default_value)
                    })
            }
            (StrategyItem::StateBlock(sa), StrategyItem::StateBlock(sb)) => {
                sa.variables.len() == sb.variables.len()
                    && sa.variables.iter().zip(sb.variables.iter()).all(|(a, b)| {
                        a.name == b.name && exprs_eq(&a.initial_value, &b.initial_value)
                    })
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
                    && ia.body.iter().zip(ib.body.iter()).all(|(a, b)| stmts_eq(a, b))
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
                    && fa.body.iter().zip(fb.body.iter()).all(|(a, b)| stmts_eq(a, b))
            }
            (Stmt::While(wa), Stmt::While(wb)) => {
                exprs_eq(&wa.condition, &wb.condition)
                    && wa.body.len() == wb.body.len()
                    && wa.body.iter().zip(wb.body.iter()).all(|(a, b)| stmts_eq(a, b))
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

    // ========================================================================
    // Property Test
    // ========================================================================

    // Feature: flux-parser, Property 1: Parse–Pretty-Print Round Trip
    // **Validates: Requirements 25.2, 25.3, 25.4, 2.2, 5.3, 7.3, 9.4, 13.1, 22.1, 22.2, 22.5**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_parse_pretty_print_round_trip(program in arb_program()) {
            let source = pretty_print_program(&program);
            let tokens = lex_with_spans(&source).expect(
                &format!("Pretty-printed output should lex successfully.\nSource:\n{}", source)
            );
            let reparsed = parse(tokens).expect(
                &format!("Pretty-printed output should parse successfully.\nSource:\n{}", source)
            );
            prop_assert!(
                programs_eq(&program, &reparsed),
                "Round-trip failed!\nOriginal AST: {:?}\nPretty-printed:\n{}\nReparsed AST: {:?}",
                program, source, reparsed
            );
        }
    }

    // ========================================================================
    // Property: FnDef Round-Trip (parse → print → parse)
    // ========================================================================

    // Feature: flux-user-functions, Property 1: FnDef round-trip
    // **Validates: Requirements 2.7, 2.8**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_fn_def_round_trip(fn_def in arb_fn_def()) {
            // Build a minimal program containing just this FnDef + a dummy strategy
            let program = Program {
                structs: vec![], enums: vec![],
                imports: vec![],
                functions: vec![fn_def.clone()],
                impl_blocks: vec![],
                data_block: None,
                connector_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![StrategyItem::Property(Property {
                        name: "v".to_string(),
                        value: Expr {
                            kind: ExprKind::IntLiteral(1),
                            span: dummy_span(),
                        },
                        span: dummy_span(),
                    })],
                    span: dummy_span(),
                },
                span: dummy_span(),
            };

            let source = pretty_print_program(&program);
            let tokens = lex_with_spans(&source).expect(
                &format!("FnDef pretty-print should lex successfully.\nSource:\n{}", source)
            );
            let reparsed = parse(tokens).expect(
                &format!("FnDef pretty-print should parse successfully.\nSource:\n{}", source)
            );

            // Assert we got exactly one function back
            prop_assert_eq!(reparsed.functions.len(), 1,
                "Expected 1 function in reparsed program, got {}\nSource:\n{}",
                reparsed.functions.len(), source);

            // Assert structural equivalence of the FnDef (ignoring spans)
            let reparsed_fn = &reparsed.functions[0];
            prop_assert!(
                fn_defs_eq(&fn_def, reparsed_fn),
                "FnDef round-trip failed!\nOriginal: {:?}\nPretty-printed:\n{}\nReparsed: {:?}",
                fn_def, source, reparsed_fn
            );
        }
    }

    // ========================================================================
    // Span Collector (Recursive AST Visitor)
    // ========================================================================

    fn collect_all_spans(program: &Program) -> Vec<Span> {
        let mut spans = Vec::new();
        spans.push(program.span);

        for import in &program.imports {
            spans.push(import.span);
        }

        spans.push(program.strategy.span);
        for item in &program.strategy.body {
            collect_strategy_item_spans(&mut spans, item);
        }

        spans
    }

    fn collect_strategy_item_spans(spans: &mut Vec<Span>, item: &StrategyItem) {
        match item {
            StrategyItem::Property(p) => {
                spans.push(p.span);
                collect_expr_spans(spans, &p.value);
            }
            StrategyItem::ParamsBlock(b) => {
                spans.push(b.span);
                for param in &b.params {
                    spans.push(param.span);
                    collect_expr_spans(spans, &param.default_value);
                }
            }
            StrategyItem::StateBlock(b) => {
                spans.push(b.span);
                for var in &b.variables {
                    spans.push(var.span);
                    collect_expr_spans(spans, &var.initial_value);
                }
            }
            StrategyItem::EventHandler(h) => {
                spans.push(h.span);
                for stmt in &h.body {
                    collect_stmt_spans(spans, stmt);
                }
            }
        }
    }

    fn collect_stmt_spans(spans: &mut Vec<Span>, stmt: &Stmt) {
        match stmt {
            Stmt::Assignment(a) => {
                spans.push(a.span);
                collect_expr_spans(spans, &a.target);
                collect_expr_spans(spans, &a.value);
            }
            Stmt::If(i) => {
                spans.push(i.span);
                collect_expr_spans(spans, &i.condition);
                for s in &i.body {
                    collect_stmt_spans(spans, s);
                }
                for elif in &i.elif_branches {
                    spans.push(elif.span);
                    collect_expr_spans(spans, &elif.condition);
                    for s in &elif.body {
                        collect_stmt_spans(spans, s);
                    }
                }
                if let Some(else_body) = &i.else_body {
                    for s in else_body {
                        collect_stmt_spans(spans, s);
                    }
                }
            }
            Stmt::For(f) => {
                spans.push(f.span);
                collect_expr_spans(spans, &f.iterable);
                for s in &f.body {
                    collect_stmt_spans(spans, s);
                }
            }
            Stmt::While(w) => {
                spans.push(w.span);
                collect_expr_spans(spans, &w.condition);
                for s in &w.body {
                    collect_stmt_spans(spans, s);
                }
            }
            Stmt::Return(r) => {
                spans.push(r.span);
                if let Some(v) = &r.value {
                    collect_expr_spans(spans, v);
                }
            }
            Stmt::Expr(e) => {
                spans.push(e.span);
                collect_expr_spans(spans, &e.expr);
            }
        }
    }

    fn collect_expr_spans(spans: &mut Vec<Span>, expr: &Expr) {
        spans.push(expr.span);
        match &expr.kind {
            ExprKind::BinaryOp { left, right, .. } => {
                collect_expr_spans(spans, left);
                collect_expr_spans(spans, right);
            }
            ExprKind::UnaryOp { operand, .. } => {
                collect_expr_spans(spans, operand);
            }
            ExprKind::FunctionCall { function, args } => {
                collect_expr_spans(spans, function);
                for arg in args {
                    collect_expr_spans(spans, arg);
                }
            }
            ExprKind::MethodCall { receiver, args, .. } => {
                collect_expr_spans(spans, receiver);
                for arg in args {
                    collect_expr_spans(spans, arg);
                }
            }
            ExprKind::MemberAccess { object, .. } => {
                collect_expr_spans(spans, object);
            }
            ExprKind::IndexAccess { object, index } => {
                collect_expr_spans(spans, object);
                collect_expr_spans(spans, index);
            }
            ExprKind::ListLiteral(elems) => {
                for elem in elems {
                    collect_expr_spans(spans, elem);
                }
            }
            _ => {} // Leaf nodes - span already pushed above
        }
    }

    // ========================================================================
    // Property 2: Span Validity
    // ========================================================================

    // Feature: flux-parser, Property 2: Span Validity Invariant
    // **Validates: Requirements 23.1, 23.2, 23.3, 23.4**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_all_spans_valid(program in arb_program()) {
            let source = pretty_print_program(&program);
            let source_len = source.len();
            let tokens = lex_with_spans(&source).unwrap();
            let parsed = parse(tokens).unwrap();

            let spans = collect_all_spans(&parsed);
            for span in &spans {
                prop_assert!(span.start <= span.end,
                    "Invalid span: start {} > end {}", span.start, span.end);
                prop_assert!(span.end <= source_len,
                    "Span exceeds input: end {} > source_len {}", span.end, source_len);
            }
        }
    }

    // ========================================================================
    // Property 3: Operator Precedence Correctness
    // ========================================================================

    /// Convert a BinOp to its source string representation
    fn op_to_str(op: &BinOp) -> &'static str {
        match op {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Mod => "%",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::And => "and",
            BinOp::Or => "or",
        }
    }

    /// Extract the binary operator from an expression if it's a BinaryOp
    fn right_op(expr: &Expr) -> Option<BinOp> {
        match &expr.kind {
            ExprKind::BinaryOp { op, .. } => Some(*op),
            _ => None,
        }
    }

    /// Generate a pair of (lower_prec_op, higher_prec_op)
    fn arb_lower_higher_op_pair() -> impl Strategy<Value = (BinOp, BinOp)> {
        // Define operators by precedence level
        let level1 = vec![BinOp::Or];
        let level2 = vec![BinOp::And];
        let level3 = vec![BinOp::Eq, BinOp::Ne];
        let level4 = vec![BinOp::Lt, BinOp::Le, BinOp::Gt, BinOp::Ge];
        let level5 = vec![BinOp::Add, BinOp::Sub];
        let level6 = vec![BinOp::Mul, BinOp::Div, BinOp::Mod];

        let levels = vec![level1, level2, level3, level4, level5, level6];

        // Pick two different levels where lower_level < higher_level
        (0..5usize).prop_flat_map(move |lower_idx| {
            let levels_clone = levels.clone();
            (lower_idx + 1..6usize).prop_flat_map(move |higher_idx| {
                let lower_ops = levels_clone[lower_idx].clone();
                let higher_ops = levels_clone[higher_idx].clone();
                (
                    proptest::sample::select(lower_ops),
                    proptest::sample::select(higher_ops),
                )
            })
        })
    }

    // Feature: flux-parser, Property 3: Operator Precedence Correctness
    // **Validates: Requirements 13.1, 13.2, 13.3, 13.4, 13.5, 13.6**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_precedence_tree_structure(
            (op1, op2) in arb_lower_higher_op_pair(),
            a in arb_ident(),
            b in arb_ident(),
            c in arb_ident(),
        ) {
            // Build source: "strategy T { x = a OP1 b OP2 c }"
            let source = format!(
                "strategy T {{ x = {} {} {} {} {} }}",
                a, op_to_str(&op1), b, op_to_str(&op2), c
            );
            let tokens = lex_with_spans(&source).unwrap();
            let program = parse(tokens).unwrap();

            // Extract the expression from the strategy property
            let prop_value = match &program.strategy.body[0] {
                StrategyItem::Property(p) => &p.value,
                _ => panic!("Expected Property"),
            };

            // The AST should be: BinaryOp(a, OP1, BinaryOp(b, OP2, c))
            // i.e., OP1 is at the top, OP2 is deeper
            match &prop_value.kind {
                ExprKind::BinaryOp { left, op, right } => {
                    prop_assert_eq!(*op, op1,
                        "Top-level operator should be the lower-precedence one");
                    prop_assert_eq!(right_op(right), Some(op2),
                        "Right subtree should contain the higher-precedence operator");
                    // left should be a leaf (identifier)
                    prop_assert!(matches!(&left.kind, ExprKind::Ident(_)),
                        "Left should be an identifier");
                }
                _ => prop_assert!(false, "Expected BinaryOp at top level, got: {:?}", prop_value.kind),
            }
        }
    }

    // ========================================================================
    // Property 5: Error Format Consistency
    // ========================================================================

    // Feature: flux-parser, Property 5: Error Format Consistency
    // **Validates: Requirements 1.3, 24.1, 24.2, 24.3, 24.4**

    /// Generate invalid Flux source strings that should trigger parse errors
    fn arb_invalid_source() -> impl Strategy<Value = String> {
        prop_oneof![
            // Missing strategy keyword - just identifiers
            Just("x y z".to_string()),
            // Unclosed brace
            Just("strategy X {".to_string()),
            // Missing strategy name
            Just("strategy { }".to_string()),
            // Invalid token after strategy
            Just("strategy X { } extra".to_string()),
            // Empty import list
            Just("from m import {} strategy X {}".to_string()),
            // Missing expression in assignment
            Just("strategy X { on_bar { x = } }".to_string()),
            // Missing closing paren
            Just("strategy X { on_bar { f( } }".to_string()),
            // Missing closing bracket
            Just("strategy X { on_bar { x = [1, 2 } }".to_string()),
            // Random operator without context
            Just("strategy X { on_bar { = 5 } }".to_string()),
            // Missing `in` in for loop
            Just("strategy X { on_bar { for x items { } } }".to_string()),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_error_format_consistency(source in arb_invalid_source()) {
            let tokens = lex_with_spans(&source);

            // Skip if lexing fails (we want parse errors, not lex errors)
            if let Ok(tokens) = tokens {
                let result = parse(tokens);

                // Should be an error
                prop_assert!(result.is_err(),
                    "Expected parse error for input: {}", source);

                let err = result.unwrap_err();

                // Should be CompileError::Parser variant
                match &err {
                    crate::error::CompileError::Parser(msg) => {
                        // Should contain "at byte " followed by a number
                        prop_assert!(
                            msg.contains("at byte "),
                            "Error message should contain 'at byte ', got: {}",
                            msg
                        );

                        // Extract the byte offset and verify it's a valid number
                        let after_byte = msg.split("at byte ").nth(1).unwrap_or("");
                        let offset_str: String = after_byte.chars().take_while(|c| c.is_ascii_digit()).collect();
                        prop_assert!(
                            !offset_str.is_empty(),
                            "Error message should contain a numeric byte offset after 'at byte ', got: {}",
                            msg
                        );
                        let _offset: usize = offset_str.parse().unwrap();
                    }
                    other => {
                        prop_assert!(false,
                            "Expected CompileError::Parser, got: {:?}", other);
                    }
                }
            }
        }
    }

    // ========================================================================
    // Property 6: Unary Operator Right-Associativity
    // ========================================================================

    // Feature: flux-parser, Property 6: Unary Operator Right-Associativity
    // **Validates: Requirements 15.4**

    /// Generate a chain of unary operators
    fn arb_unary_chain() -> impl Strategy<Value = Vec<&'static str>> {
        proptest::collection::vec(
            prop_oneof![
                Just("not "),
                Just("!"),
                Just("-"),
            ],
            2..5, // 2-4 operators
        )
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_unary_right_associative(
            ops in arb_unary_chain(),
            operand in arb_ident(),
        ) {
            // Build source: "strategy T { x = op1 op2 ... operand }"
            let ops_str: String = ops.iter().copied().collect();
            let source = format!("strategy T {{ x = {}{} }}", ops_str, operand);

            let tokens = lex_with_spans(&source).unwrap();
            let program = parse(tokens).unwrap();

            // Extract the expression
            let expr = match &program.strategy.body[0] {
                StrategyItem::Property(p) => &p.value,
                _ => panic!("Expected Property"),
            };

            // Verify right-to-left nesting:
            // The outermost UnaryOp should correspond to ops[0]
            // The innermost UnaryOp should wrap the operand directly

            // Walk down the chain
            let mut current = expr;
            for (i, op_str) in ops.iter().enumerate() {
                match &current.kind {
                    ExprKind::UnaryOp { op, operand: inner } => {
                        // Verify this operator matches the expected one
                        let expected_op = match *op_str {
                            "not " => UnaryOp::Not,
                            "!" => UnaryOp::Not,
                            "-" => UnaryOp::Neg,
                            _ => unreachable!(),
                        };
                        prop_assert_eq!(*op, expected_op,
                            "Operator at depth {} should be {:?}, got {:?}", i, expected_op, op);
                        current = inner;
                    }
                    _ => {
                        prop_assert!(false,
                            "Expected UnaryOp at depth {}, got {:?}", i, current.kind);
                        return Ok(());
                    }
                }
            }

            // After all operators, we should be at the operand identifier
            match &current.kind {
                ExprKind::Ident(name) => {
                    prop_assert_eq!(name, &operand,
                        "Innermost operand should be '{}', got '{}'", operand, name);
                }
                _ => {
                    prop_assert!(false,
                        "Expected Ident at innermost level, got {:?}", current.kind);
                }
            }
        }
    }
}
