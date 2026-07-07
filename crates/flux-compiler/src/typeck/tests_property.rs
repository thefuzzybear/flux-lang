//! Property-based tests for the Flux type checker.
//!
//! Uses proptest to verify correctness properties of the type checker across
//! randomly generated well-typed and ill-typed ASTs.

#[cfg(test)]
mod tests {
    use crate::error::CompileError;
    use crate::lexer::Span;
    use crate::parser::ast::{
        Assignment, BinOp, EventHandler, Expr, ExprKind, ExprStmt, ForLoop, IfStmt, Import, Param,
        ParamsBlock, Program, Property, StateBlock, StateVar, Stmt, Strategy as AstStrategy,
        StrategyItem, UnaryOp, WhileLoop,
    };
    use crate::typeck::typed_ast::*;
    use crate::typeck::types::FluxType;
    use proptest::prelude::*;

    // ========================================================================
    // Generators
    // ========================================================================

    /// Generate a valid Span (start < end, reasonable range).
    fn arb_span() -> impl Strategy<Value = Span> {
        (0usize..10000, 1usize..100).prop_map(|(start, len)| Span::new(start, start + len))
    }

    /// Generate a literal expression with a known type.
    fn arb_literal_expr(span: Span) -> impl Strategy<Value = (Expr, FluxType)> {
        prop_oneof![
            // IntLiteral
            (0i64..10000).prop_map(move |v| (
                Expr { kind: ExprKind::IntLiteral(v), span },
                FluxType::Int,
            )),
            // FloatLiteral
            (1u32..999, 1u32..99).prop_map(move |(i, d)| {
                let f: f64 = format!("{}.{}", i, d).parse().unwrap();
                (
                    Expr { kind: ExprKind::FloatLiteral(f), span },
                    FluxType::Float,
                )
            }),
            // StringLiteral
            "[a-z]{1,10}".prop_map(move |s| (
                Expr { kind: ExprKind::StringLiteral(s), span },
                FluxType::String,
            )),
            // BoolLiteral
            any::<bool>().prop_map(move |b| (
                Expr { kind: ExprKind::BoolLiteral(b), span },
                FluxType::Bool,
            )),
        ]
    }

    /// Generate a numeric literal expression (Int or Float).
    fn arb_numeric_expr(span: Span) -> impl Strategy<Value = (Expr, FluxType)> {
        prop_oneof![
            (0i64..10000).prop_map(move |v| (
                Expr { kind: ExprKind::IntLiteral(v), span },
                FluxType::Int,
            )),
            (1u32..999, 1u32..99).prop_map(move |(i, d)| {
                let f: f64 = format!("{}.{}", i, d).parse().unwrap();
                (
                    Expr { kind: ExprKind::FloatLiteral(f), span },
                    FluxType::Float,
                )
            }),
        ]
    }

    /// Generate specifically Int literal expressions.
    fn arb_int_expr(span: Span) -> impl Strategy<Value = Expr> {
        (0i64..10000).prop_map(move |v| Expr {
            kind: ExprKind::IntLiteral(v),
            span,
        })
    }

    /// Generate specifically Float literal expressions.
    fn arb_float_expr(span: Span) -> impl Strategy<Value = Expr> {
        (1u32..999, 1u32..99).prop_map(move |(i, d)| {
            let f: f64 = format!("{}.{}", i, d).parse().unwrap();
            Expr {
                kind: ExprKind::FloatLiteral(f),
                span,
            }
        })
    }

    /// Generate a Bool expression.
    #[allow(dead_code)]
    fn arb_bool_expr(span: Span) -> impl Strategy<Value = Expr> {
        any::<bool>().prop_map(move |b| Expr {
            kind: ExprKind::BoolLiteral(b),
            span,
        })
    }

    /// Generate a non-numeric, non-bool literal (for error-case testing).
    fn arb_non_numeric_expr(span: Span) -> impl Strategy<Value = (Expr, FluxType)> {
        prop_oneof![
            "[a-z]{1,8}".prop_map(move |s| (
                Expr { kind: ExprKind::StringLiteral(s), span },
                FluxType::String,
            )),
            any::<bool>().prop_map(move |b| (
                Expr { kind: ExprKind::BoolLiteral(b), span },
                FluxType::Bool,
            )),
            Just((
                Expr { kind: ExprKind::NullLiteral, span },
                FluxType::Null,
            )),
        ]
    }

    /// Generate an arithmetic binary operator.
    fn arb_arithmetic_op() -> impl Strategy<Value = BinOp> {
        prop_oneof![
            Just(BinOp::Add),
            Just(BinOp::Sub),
            Just(BinOp::Mul),
            Just(BinOp::Div),
            Just(BinOp::Mod),
        ]
    }

    /// Generate a well-typed statement for use inside an event handler.
    /// Each statement uses a unique variable name indexed by `idx` to avoid
    /// type conflicts on reassignment.
    fn arb_well_typed_handler_stmt(span: Span, idx: usize) -> impl Strategy<Value = Stmt> {
        let var_name = format!("tmp_{}", idx);
        let var_name2 = var_name.clone();
        prop_oneof![
            // Simple assignment: `tmp_N = <literal>`
            arb_literal_expr(span).prop_map(move |(expr, _ty)| {
                Stmt::Assignment(Assignment {
                    target: Expr {
                        kind: ExprKind::Ident(var_name.clone()),
                        span,
                    },
                    value: expr,
                    span,
                })
            }),
            // If statement with bool condition and literal assignment body
            (any::<bool>(), arb_literal_expr(span)).prop_map(move |(cond, (body_expr, _ty))| {
                let inner_var = format!("{}_inner", var_name2);
                Stmt::If(IfStmt {
                    condition: Expr {
                        kind: ExprKind::BoolLiteral(cond),
                        span,
                    },
                    body: vec![Stmt::Assignment(Assignment {
                        target: Expr {
                            kind: ExprKind::Ident(inner_var),
                            span,
                        },
                        value: body_expr,
                        span,
                    })],
                    elif_branches: vec![],
                    else_body: None,
                    span,
                })
            }),
        ]
    }

    /// Generate an optional import block (0 or 1 imports with unique names).
    fn arb_imports(span: Span) -> impl Strategy<Value = Vec<Import>> {
        prop_oneof![
            3 => Just(vec![]),
            1 => prop::collection::vec("[a-z]{3,6}", 1..=2)
                .prop_filter("unique names", |names| {
                    let unique: std::collections::HashSet<_> = names.iter().collect();
                    unique.len() == names.len()
                })
                .prop_map(move |names| {
                    vec![Import {
                        module_path: "indicators".to_string(),
                        names,
                        span,
                    }]
                }),
        ]
    }

    /// Generate a valid params block with literal defaults (1-3 params).
    fn arb_params_block(span: Span) -> impl Strategy<Value = ParamsBlock> {
        prop::collection::vec(
            (arb_literal_expr(span), 0u8..100).prop_map(move |((expr, _ty), idx)| Param {
                name: format!("param_{}", idx),
                default_value: expr,
                span,
            }),
            1..=3,
        )
        .prop_map(move |params| ParamsBlock { params, span })
    }

    /// Generate a valid state block with literal initializers (0-2 vars).
    fn arb_state_block(span: Span) -> impl Strategy<Value = Option<StateBlock>> {
        prop_oneof![
            2 => Just(None),
            1 => prop::collection::vec(
                arb_literal_expr(span).prop_map(move |(expr, _ty)| StateVar {
                    name: "state_var".to_string(),
                    initial_value: expr,
                    span,
                }),
                1..=2,
            )
            .prop_map(move |variables| Some(StateBlock { variables, span })),
        ]
    }

    /// Generate a complete well-typed Program.
    ///
    /// The generated program will always pass type checking because:
    /// - Params have literal defaults
    /// - State has literal initializers
    /// - Event handler uses event_name = "bar" (maps to `on_bar`)
    /// - Handler body uses only literal assignments and bool-conditioned if-stmts
    fn arb_well_typed_program() -> impl Strategy<Value = Program> {
        (arb_span(), arb_span(), arb_span(), arb_span())
            .prop_flat_map(|(prog_span, strat_span, handler_span, inner_span)| {
                // Generate 1-3 well-typed handler statements with unique variable names
                let stmts_strategy = (
                    arb_well_typed_handler_stmt(inner_span, 0),
                    prop::option::of(arb_well_typed_handler_stmt(inner_span, 1)),
                    prop::option::of(arb_well_typed_handler_stmt(inner_span, 2)),
                ).prop_map(|(s0, s1, s2)| {
                    let mut stmts = vec![s0];
                    if let Some(s) = s1 { stmts.push(s); }
                    if let Some(s) = s2 { stmts.push(s); }
                    stmts
                });

                (
                    arb_imports(prog_span),
                    arb_params_block(strat_span),
                    arb_state_block(strat_span),
                    stmts_strategy,
                    Just(prog_span),
                    Just(strat_span),
                    Just(handler_span),
                )
            })
            .prop_map(
                |(imports, params_block, state_block, handler_stmts, prog_span, strat_span, handler_span)| {
                    let event_handler = EventHandler {
                        event_name: "bar".to_string(),
                        body: handler_stmts,
                        span: handler_span,
                    };

                    let mut body: Vec<StrategyItem> = vec![StrategyItem::ParamsBlock(params_block)];
                    if let Some(sb) = state_block {
                        body.push(StrategyItem::StateBlock(sb));
                    }
                    body.push(StrategyItem::EventHandler(event_handler));

                    let strategy = AstStrategy {
                        name: "TestStrategy".to_string(),
                        body,
                        span: strat_span,
                    };

                    Program {
                        imports,
                        data_block: None,
                        strategy,
                        span: prog_span,
                    }
                },
            )
    }

    // ========================================================================
    // Helpers
    // ========================================================================

    /// Recursively verify all expressions in a typed statement have resolved types.
    fn verify_all_exprs_typed(stmt: &TypedStmt) {
        match stmt {
            TypedStmt::Expr(es) => {
                assert_ne!(
                    es.expr.resolved_type,
                    FluxType::Null,
                    "Expression should have resolved type, got Null"
                );
            }
            TypedStmt::Assignment(a) => {
                assert_ne!(
                    a.value.resolved_type,
                    FluxType::Null,
                    "Assignment value should have resolved type, got Null"
                );
            }
            TypedStmt::If(ifs) => {
                assert_eq!(
                    ifs.condition.resolved_type,
                    FluxType::Bool,
                    "If condition must be Bool"
                );
                for s in &ifs.body {
                    verify_all_exprs_typed(s);
                }
                for elif in &ifs.elif_branches {
                    assert_eq!(elif.condition.resolved_type, FluxType::Bool);
                    for s in &elif.body {
                        verify_all_exprs_typed(s);
                    }
                }
                if let Some(else_body) = &ifs.else_body {
                    for s in else_body {
                        verify_all_exprs_typed(s);
                    }
                }
            }
            TypedStmt::For(f) => {
                for s in &f.body {
                    verify_all_exprs_typed(s);
                }
            }
            TypedStmt::While(w) => {
                assert_eq!(w.condition.resolved_type, FluxType::Bool);
                for s in &w.body {
                    verify_all_exprs_typed(s);
                }
            }
            TypedStmt::Return(_) => {}
        }
    }

    // ========================================================================
    // Property 1: Structure Preservation with Full Type Annotation
    // ========================================================================

    // **Validates: Requirements 1.2, 19.1, 19.2, 19.4**
    //
    // For any well-typed Program AST, the TypedProgram produced by check()
    // SHALL have the same structural shape and every TypedExpr SHALL have a
    // non-Null resolved_type.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn prop_structure_preservation(program in arb_well_typed_program()) {
            let num_imports = program.imports.len();
            let num_strategy_items = program.strategy.body.len();

            let result = super::super::check(program);
            prop_assert!(result.is_ok(), "Well-typed program should pass: {:?}", result.err());

            let typed = result.unwrap();
            // Structure preserved
            prop_assert_eq!(typed.imports.len(), num_imports);
            prop_assert_eq!(typed.strategy.body.len(), num_strategy_items);

            // All expressions have resolved types
            for item in &typed.strategy.body {
                if let TypedStrategyItem::EventHandler(eh) = item {
                    for stmt in &eh.body {
                        verify_all_exprs_typed(stmt);
                    }
                }
            }
        }
    }

    // ========================================================================
    // Property 2: Span Preservation
    // ========================================================================

    // **Validates: Requirements 19.3**
    //
    // For any well-typed Program AST, the TypedProgram produced by check()
    // SHALL preserve the Span of every top-level AST node.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn prop_span_preservation(program in arb_well_typed_program()) {
            let input_prog_span = program.span;
            let input_strategy_span = program.strategy.span;

            let result = super::super::check(program);
            prop_assert!(result.is_ok(), "Well-typed program should pass: {:?}", result.err());

            let typed = result.unwrap();
            prop_assert_eq!(typed.span, input_prog_span, "Program span must be preserved");
            prop_assert_eq!(typed.strategy.span, input_strategy_span, "Strategy span must be preserved");
        }
    }

    // ========================================================================
    // Property 3: Error Format Consistency
    // ========================================================================

    /// Generate an ill-typed program (if condition is an IntLiteral, not Bool).
    fn arb_ill_typed_program() -> impl Strategy<Value = Program> {
        (arb_span(), arb_span(), arb_span(), 1i64..1000).prop_map(
            |(prog_span, strat_span, handler_span, int_val)| {
                // The condition is an IntLiteral — type checker will reject it
                let bad_condition = Expr {
                    kind: ExprKind::IntLiteral(int_val),
                    span: handler_span,
                };

                let if_stmt = Stmt::If(IfStmt {
                    condition: bad_condition,
                    body: vec![Stmt::Expr(ExprStmt {
                        expr: Expr {
                            kind: ExprKind::IntLiteral(1),
                            span: handler_span,
                        },
                        span: handler_span,
                    })],
                    elif_branches: vec![],
                    else_body: None,
                    span: handler_span,
                });

                let event_handler = EventHandler {
                    event_name: "bar".to_string(),
                    body: vec![if_stmt],
                    span: handler_span,
                };

                let strategy = AstStrategy {
                    name: "Bad".to_string(),
                    body: vec![StrategyItem::EventHandler(event_handler)],
                    span: strat_span,
                };

                Program {
                    imports: vec![],
                        data_block: None,
                    strategy,
                    span: prog_span,
                }
            },
        )
    }

    // **Validates: Requirements 1.3, 20.1, 20.4**
    //
    // For any Program AST that contains a type error, the error returned by
    // check() SHALL be a CompileError::Type(String) where the string starts
    // with "at byte N:" where N is a number.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn prop_error_format_consistency(program in arb_ill_typed_program()) {
            let result = super::super::check(program);
            prop_assert!(result.is_err(), "Ill-typed program should fail");

            let err = result.unwrap_err();
            match &err {
                CompileError::Type(msg) => {
                    prop_assert!(
                        msg.starts_with("at byte "),
                        "Error should start with 'at byte ', got: {}", msg
                    );
                    // Extract the number after "at byte "
                    let after_prefix = &msg["at byte ".len()..];
                    let colon_pos = after_prefix.find(':');
                    prop_assert!(colon_pos.is_some(), "Error should contain ':', got: {}", msg);
                    let num_str = &after_prefix[..colon_pos.unwrap()];
                    prop_assert!(
                        num_str.parse::<usize>().is_ok(),
                        "Byte offset should be a number, got '{}' in: {}", num_str, msg
                    );
                }
                other => {
                    prop_assert!(false, "Expected CompileError::Type, got: {:?}", other);
                }
            }
        }
    }

    // ========================================================================
    // Property 4: Arithmetic Type Propagation
    // ========================================================================

    /// Generate a pair of numeric operands with known types for arithmetic.
    fn arb_numeric_operand_pair(
        span: Span,
    ) -> impl Strategy<Value = (Expr, FluxType, Expr, FluxType)> {
        prop_oneof![
            // Int op Int
            (arb_int_expr(span), arb_int_expr(span)).prop_map(|(l, r)| {
                (l, FluxType::Int, r, FluxType::Int)
            }),
            // Float op Float
            (arb_float_expr(span), arb_float_expr(span)).prop_map(|(l, r)| {
                (l, FluxType::Float, r, FluxType::Float)
            }),
            // Int op Float
            (arb_int_expr(span), arb_float_expr(span)).prop_map(|(l, r)| {
                (l, FluxType::Int, r, FluxType::Float)
            }),
            // Float op Int
            (arb_float_expr(span), arb_int_expr(span)).prop_map(|(l, r)| {
                (l, FluxType::Float, r, FluxType::Int)
            }),
        ]
    }

    // **Validates: Requirements 6.1, 6.2, 6.3**
    //
    // For any binary arithmetic expression with numeric operands:
    // Int op Int → Int, Float op Float → Float, mixed → Float.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        #[test]
        fn prop_arithmetic_type_propagation(
            (left, left_ty, right, right_ty) in arb_span().prop_flat_map(|s| arb_numeric_operand_pair(s)),
            op in arb_arithmetic_op(),
            op_span in arb_span(),
        ) {
            let expected_type = match (&left_ty, &right_ty) {
                (FluxType::Int, FluxType::Int) => FluxType::Int,
                (FluxType::Float, FluxType::Float) => FluxType::Float,
                _ => FluxType::Float, // mixed numeric → Float
            };

            // Build a minimal program wrapping the arithmetic expr in event handler
            let arith_expr = Expr {
                kind: ExprKind::BinaryOp {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                },
                span: op_span,
            };

            let handler_stmt = Stmt::Assignment(Assignment {
                target: Expr {
                    kind: ExprKind::Ident("result".to_string()),
                    span: op_span,
                },
                value: arith_expr,
                span: op_span,
            });

            let program = Program {
                imports: vec![],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![StrategyItem::EventHandler(EventHandler {
                        event_name: "bar".to_string(),
                        body: vec![handler_stmt],
                        span: op_span,
                    })],
                    span: op_span,
                },
                span: op_span,
            };

            let result = super::super::check(program);
            prop_assert!(result.is_ok(), "Numeric arithmetic should pass: {:?}", result.err());

            let typed = result.unwrap();
            if let TypedStrategyItem::EventHandler(eh) = &typed.strategy.body[0] {
                if let TypedStmt::Assignment(assign) = &eh.body[0] {
                    prop_assert_eq!(
                        &assign.value.resolved_type,
                        &expected_type,
                        "Expected {:?}, got {:?}",
                        expected_type,
                        assign.value.resolved_type
                    );
                } else {
                    prop_assert!(false, "Expected assignment statement");
                }
            } else {
                prop_assert!(false, "Expected event handler");
            }
        }
    }

    // ========================================================================
    // Property 5: Arithmetic Requires Numeric Operands
    // ========================================================================

    /// Generate a binary arithmetic expression with at least one non-numeric operand.
    fn arb_non_numeric_arithmetic_program() -> impl Strategy<Value = Program> {
        (arb_span(), arb_span(), arb_arithmetic_op()).prop_flat_map(
            |(prog_span, op_span, op)| {
                // At least one operand is non-numeric
                prop_oneof![
                    // Left non-numeric, right numeric
                    (arb_non_numeric_expr(op_span), arb_numeric_expr(op_span)).prop_map(
                        move |((left, _), (right, _))| (left, right, op, op_span, prog_span)
                    ),
                    // Left numeric, right non-numeric
                    (arb_numeric_expr(op_span), arb_non_numeric_expr(op_span)).prop_map(
                        move |((left, _), (right, _))| (left, right, op, op_span, prog_span)
                    ),
                    // Both non-numeric
                    (arb_non_numeric_expr(op_span), arb_non_numeric_expr(op_span)).prop_map(
                        move |((left, _), (right, _))| (left, right, op, op_span, prog_span)
                    ),
                ]
            },
        )
        .prop_filter(
            "exclude String + String which is valid concatenation",
            |(left, right, op, _, _)| {
                // String + String is valid (concatenation), so filter it out
                !(matches!(op, BinOp::Add)
                    && matches!(left.kind, ExprKind::StringLiteral(_))
                    && matches!(right.kind, ExprKind::StringLiteral(_)))
            },
        )
        .prop_map(|(left, right, op, op_span, prog_span)| {
            let arith_expr = Expr {
                kind: ExprKind::BinaryOp {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                },
                span: op_span,
            };

            let handler_stmt = Stmt::Expr(ExprStmt {
                expr: arith_expr,
                span: op_span,
            });

            Program {
                imports: vec![],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![StrategyItem::EventHandler(EventHandler {
                        event_name: "bar".to_string(),
                        body: vec![handler_stmt],
                        span: op_span,
                    })],
                    span: prog_span,
                },
                span: prog_span,
            }
        })
    }

    // **Validates: Requirements 6.5, 6.7**
    //
    // For any binary arithmetic expression with at least one non-numeric operand
    // (excluding String + String), the type checker SHALL return a type error.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        #[test]
        fn prop_arithmetic_requires_numeric(program in arb_non_numeric_arithmetic_program()) {
            let result = super::super::check(program);
            prop_assert!(result.is_err(), "Non-numeric arithmetic should fail, got Ok");

            let err = result.unwrap_err();
            match &err {
                CompileError::Type(msg) => {
                    prop_assert!(
                        msg.contains("numeric operands") || msg.contains("requires numeric"),
                        "Error should mention numeric requirement, got: {}", msg
                    );
                }
                other => {
                    prop_assert!(false, "Expected CompileError::Type, got: {:?}", other);
                }
            }
        }
    }

    // ========================================================================
    // Property 6: Comparison Type Rules
    // ========================================================================

    /// Generate an ordering comparison operator.
    fn arb_ordering_op() -> impl Strategy<Value = BinOp> {
        prop_oneof![
            Just(BinOp::Lt),
            Just(BinOp::Le),
            Just(BinOp::Gt),
            Just(BinOp::Ge),
        ]
    }

    /// Generate an equality comparison operator.
    fn arb_equality_op() -> impl Strategy<Value = BinOp> {
        prop_oneof![Just(BinOp::Eq), Just(BinOp::Ne)]
    }

    /// Generate a non-numeric expression (excludes Bool since we need things
    /// that are clearly not numeric for ordering tests).
    fn arb_non_numeric_for_comparison(span: Span) -> impl Strategy<Value = Expr> {
        prop_oneof![
            "[a-z]{1,8}".prop_map(move |s| Expr {
                kind: ExprKind::StringLiteral(s),
                span,
            }),
            any::<bool>().prop_map(move |b| Expr {
                kind: ExprKind::BoolLiteral(b),
                span,
            }),
            Just(Expr {
                kind: ExprKind::NullLiteral,
                span,
            }),
        ]
    }

    /// Helper to wrap a single statement in a minimal program with an event handler.
    fn wrap_in_handler(stmt: Stmt, span: Span) -> Program {
        Program {
            imports: vec![],
            data_block: None,
            strategy: AstStrategy {
                name: "T".to_string(),
                body: vec![StrategyItem::EventHandler(EventHandler {
                    event_name: "bar".to_string(),
                    body: vec![stmt],
                    span,
                })],
                span,
            },
            span,
        }
    }

    // **Validates: Requirements 7.1, 7.2, 7.3, 7.4, 7.5**
    //
    // For any ordering comparison with numeric operands → Bool.
    // For any ordering comparison with non-numeric operand → error.
    // For any equality comparison with same type or both numeric → Bool.
    // For any equality comparison with incompatible types → error.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        #[test]
        fn prop_comparison_numeric_ordering_returns_bool(
            (left, _, right, _) in arb_span().prop_flat_map(|s| arb_numeric_operand_pair(s)),
            op in arb_ordering_op(),
            span in arb_span(),
        ) {
            let cmp_expr = Expr {
                kind: ExprKind::BinaryOp {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                },
                span,
            };

            let stmt = Stmt::Assignment(Assignment {
                target: Expr {
                    kind: ExprKind::Ident("result".to_string()),
                    span,
                },
                value: cmp_expr,
                span,
            });

            let program = wrap_in_handler(stmt, span);
            let result = super::super::check(program);
            prop_assert!(result.is_ok(), "Numeric ordering should pass: {:?}", result.err());

            let typed = result.unwrap();
            if let TypedStrategyItem::EventHandler(eh) = &typed.strategy.body[0] {
                if let TypedStmt::Assignment(assign) = &eh.body[0] {
                    prop_assert_eq!(
                        &assign.value.resolved_type,
                        &FluxType::Bool,
                        "Ordering comparison of numerics should be Bool, got {:?}",
                        assign.value.resolved_type
                    );
                } else {
                    prop_assert!(false, "Expected assignment statement");
                }
            } else {
                prop_assert!(false, "Expected event handler");
            }
        }

        #[test]
        fn prop_comparison_non_numeric_ordering_errors(
            left in arb_span().prop_flat_map(|s| arb_non_numeric_for_comparison(s)),
            right in arb_span().prop_flat_map(|s| arb_int_expr(s)),
            op in arb_ordering_op(),
            span in arb_span(),
        ) {
            let cmp_expr = Expr {
                kind: ExprKind::BinaryOp {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                },
                span,
            };

            let stmt = Stmt::Expr(ExprStmt { expr: cmp_expr, span });
            let program = wrap_in_handler(stmt, span);
            let result = super::super::check(program);
            prop_assert!(result.is_err(), "Non-numeric ordering should fail");

            let err = result.unwrap_err();
            match &err {
                CompileError::Type(msg) => {
                    prop_assert!(
                        msg.contains("numeric operands"),
                        "Error should mention numeric operands, got: {}", msg
                    );
                }
                other => {
                    prop_assert!(false, "Expected CompileError::Type, got: {:?}", other);
                }
            }
        }

        #[test]
        fn prop_equality_same_type_returns_bool(
            (expr, _ty) in arb_span().prop_flat_map(|s| arb_literal_expr(s)),
            op in arb_equality_op(),
            span in arb_span(),
        ) {
            // Clone the expr to use as both left and right (same type)
            let left = expr.clone();
            let right = expr;

            let eq_expr = Expr {
                kind: ExprKind::BinaryOp {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                },
                span,
            };

            let stmt = Stmt::Assignment(Assignment {
                target: Expr {
                    kind: ExprKind::Ident("result".to_string()),
                    span,
                },
                value: eq_expr,
                span,
            });

            let program = wrap_in_handler(stmt, span);
            let result = super::super::check(program);
            prop_assert!(result.is_ok(), "Equality of same type should pass: {:?}", result.err());

            let typed = result.unwrap();
            if let TypedStrategyItem::EventHandler(eh) = &typed.strategy.body[0] {
                if let TypedStmt::Assignment(assign) = &eh.body[0] {
                    prop_assert_eq!(
                        &assign.value.resolved_type,
                        &FluxType::Bool,
                        "Equality comparison should be Bool, got {:?}",
                        assign.value.resolved_type
                    );
                } else {
                    prop_assert!(false, "Expected assignment statement");
                }
            } else {
                prop_assert!(false, "Expected event handler");
            }
        }

        #[test]
        fn prop_equality_incompatible_types_errors(
            span in arb_span(),
            op in arb_equality_op(),
        ) {
            // String vs Int is always incompatible for equality
            let left = Expr {
                kind: ExprKind::StringLiteral("hello".to_string()),
                span,
            };
            let right = Expr {
                kind: ExprKind::IntLiteral(42),
                span,
            };

            let eq_expr = Expr {
                kind: ExprKind::BinaryOp {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                },
                span,
            };

            let stmt = Stmt::Expr(ExprStmt { expr: eq_expr, span });
            let program = wrap_in_handler(stmt, span);
            let result = super::super::check(program);
            prop_assert!(result.is_err(), "Incompatible equality should fail");

            let err = result.unwrap_err();
            match &err {
                CompileError::Type(msg) => {
                    prop_assert!(
                        msg.contains("matching types"),
                        "Error should mention matching types, got: {}", msg
                    );
                }
                other => {
                    prop_assert!(false, "Expected CompileError::Type, got: {:?}", other);
                }
            }
        }
    }

    // ========================================================================
    // Property 7: Logical Operators Require Bool
    // ========================================================================

    /// Generate a logical binary operator.
    fn arb_logical_op() -> impl Strategy<Value = BinOp> {
        prop_oneof![Just(BinOp::And), Just(BinOp::Or)]
    }

    /// Generate a non-Bool expression for logical operator error testing.
    fn arb_non_bool_expr(span: Span) -> impl Strategy<Value = Expr> {
        prop_oneof![
            (0i64..10000).prop_map(move |v| Expr {
                kind: ExprKind::IntLiteral(v),
                span,
            }),
            (1u32..999, 1u32..99).prop_map(move |(i, d)| {
                let f: f64 = format!("{}.{}", i, d).parse().unwrap();
                Expr {
                    kind: ExprKind::FloatLiteral(f),
                    span,
                }
            }),
            "[a-z]{1,8}".prop_map(move |s| Expr {
                kind: ExprKind::StringLiteral(s),
                span,
            }),
        ]
    }

    // **Validates: Requirements 8.1, 8.2, 8.3, 8.4, 8.5**
    //
    // For any binary logical expression with both operands Bool → result Bool.
    // For any logical expression with non-Bool operand → error.
    // For unary Not with Bool → Bool. For unary Not with non-Bool → error.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        #[test]
        fn prop_logical_bool_operands_return_bool(
            left_val in any::<bool>(),
            right_val in any::<bool>(),
            op in arb_logical_op(),
            span in arb_span(),
        ) {
            let logic_expr = Expr {
                kind: ExprKind::BinaryOp {
                    left: Box::new(Expr { kind: ExprKind::BoolLiteral(left_val), span }),
                    op,
                    right: Box::new(Expr { kind: ExprKind::BoolLiteral(right_val), span }),
                },
                span,
            };

            let stmt = Stmt::Assignment(Assignment {
                target: Expr {
                    kind: ExprKind::Ident("result".to_string()),
                    span,
                },
                value: logic_expr,
                span,
            });

            let program = wrap_in_handler(stmt, span);
            let result = super::super::check(program);
            prop_assert!(result.is_ok(), "Bool logical should pass: {:?}", result.err());

            let typed = result.unwrap();
            if let TypedStrategyItem::EventHandler(eh) = &typed.strategy.body[0] {
                if let TypedStmt::Assignment(assign) = &eh.body[0] {
                    prop_assert_eq!(
                        &assign.value.resolved_type,
                        &FluxType::Bool,
                        "Logical op on Bool should be Bool, got {:?}",
                        assign.value.resolved_type
                    );
                } else {
                    prop_assert!(false, "Expected assignment statement");
                }
            } else {
                prop_assert!(false, "Expected event handler");
            }
        }

        #[test]
        fn prop_logical_non_bool_operand_errors(
            non_bool in arb_span().prop_flat_map(|s| arb_non_bool_expr(s)),
            op in arb_logical_op(),
            span in arb_span(),
        ) {
            // Left operand is non-Bool, right is Bool
            let logic_expr = Expr {
                kind: ExprKind::BinaryOp {
                    left: Box::new(non_bool),
                    op,
                    right: Box::new(Expr { kind: ExprKind::BoolLiteral(true), span }),
                },
                span,
            };

            let stmt = Stmt::Expr(ExprStmt { expr: logic_expr, span });
            let program = wrap_in_handler(stmt, span);
            let result = super::super::check(program);
            prop_assert!(result.is_err(), "Non-Bool logical should fail");

            let err = result.unwrap_err();
            match &err {
                CompileError::Type(msg) => {
                    prop_assert!(
                        msg.contains("boolean operands"),
                        "Error should mention boolean operands, got: {}", msg
                    );
                }
                other => {
                    prop_assert!(false, "Expected CompileError::Type, got: {:?}", other);
                }
            }
        }

        #[test]
        fn prop_unary_not_bool_returns_bool(
            val in any::<bool>(),
            span in arb_span(),
        ) {
            let not_expr = Expr {
                kind: ExprKind::UnaryOp {
                    op: UnaryOp::Not,
                    operand: Box::new(Expr { kind: ExprKind::BoolLiteral(val), span }),
                },
                span,
            };

            let stmt = Stmt::Assignment(Assignment {
                target: Expr {
                    kind: ExprKind::Ident("result".to_string()),
                    span,
                },
                value: not_expr,
                span,
            });

            let program = wrap_in_handler(stmt, span);
            let result = super::super::check(program);
            prop_assert!(result.is_ok(), "Not Bool should pass: {:?}", result.err());

            let typed = result.unwrap();
            if let TypedStrategyItem::EventHandler(eh) = &typed.strategy.body[0] {
                if let TypedStmt::Assignment(assign) = &eh.body[0] {
                    prop_assert_eq!(
                        &assign.value.resolved_type,
                        &FluxType::Bool,
                        "Unary Not on Bool should be Bool, got {:?}",
                        assign.value.resolved_type
                    );
                } else {
                    prop_assert!(false, "Expected assignment statement");
                }
            } else {
                prop_assert!(false, "Expected event handler");
            }
        }

        #[test]
        fn prop_unary_not_non_bool_errors(
            non_bool in arb_span().prop_flat_map(|s| arb_non_bool_expr(s)),
            span in arb_span(),
        ) {
            let not_expr = Expr {
                kind: ExprKind::UnaryOp {
                    op: UnaryOp::Not,
                    operand: Box::new(non_bool),
                },
                span,
            };

            let stmt = Stmt::Expr(ExprStmt { expr: not_expr, span });
            let program = wrap_in_handler(stmt, span);
            let result = super::super::check(program);
            prop_assert!(result.is_err(), "Not non-Bool should fail");

            let err = result.unwrap_err();
            match &err {
                CompileError::Type(msg) => {
                    prop_assert!(
                        msg.contains("boolean operand"),
                        "Error should mention boolean operand, got: {}", msg
                    );
                }
                other => {
                    prop_assert!(false, "Expected CompileError::Type, got: {:?}", other);
                }
            }
        }
    }

    // ========================================================================
    // Property 8: Conditions Must Be Bool
    // ========================================================================

    // **Validates: Requirements 9.1, 9.2, 9.3, 9.4, 10.1, 10.2**
    //
    // For any if/while condition that is Bool → accepted.
    // For any if/while condition that is non-Bool → error.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn prop_if_condition_bool_accepted(
            cond_val in any::<bool>(),
            span in arb_span(),
        ) {
            let if_stmt = Stmt::If(IfStmt {
                condition: Expr { kind: ExprKind::BoolLiteral(cond_val), span },
                body: vec![Stmt::Assignment(Assignment {
                    target: Expr { kind: ExprKind::Ident("x".to_string()), span },
                    value: Expr { kind: ExprKind::IntLiteral(1), span },
                    span,
                })],
                elif_branches: vec![],
                else_body: None,
                span,
            });

            let program = wrap_in_handler(if_stmt, span);
            let result = super::super::check(program);
            prop_assert!(result.is_ok(), "If with Bool condition should pass: {:?}", result.err());
        }

        #[test]
        fn prop_if_condition_non_bool_errors(
            non_bool in arb_span().prop_flat_map(|s| arb_non_bool_expr(s)),
            span in arb_span(),
        ) {
            let if_stmt = Stmt::If(IfStmt {
                condition: non_bool,
                body: vec![Stmt::Assignment(Assignment {
                    target: Expr { kind: ExprKind::Ident("x".to_string()), span },
                    value: Expr { kind: ExprKind::IntLiteral(1), span },
                    span,
                })],
                elif_branches: vec![],
                else_body: None,
                span,
            });

            let program = wrap_in_handler(if_stmt, span);
            let result = super::super::check(program);
            prop_assert!(result.is_err(), "If with non-Bool condition should fail");

            let err = result.unwrap_err();
            match &err {
                CompileError::Type(msg) => {
                    prop_assert!(
                        msg.contains("condition must be Bool"),
                        "Error should mention condition must be Bool, got: {}", msg
                    );
                }
                other => {
                    prop_assert!(false, "Expected CompileError::Type, got: {:?}", other);
                }
            }
        }

        #[test]
        fn prop_while_condition_bool_accepted(
            cond_val in any::<bool>(),
            span in arb_span(),
        ) {
            let while_stmt = Stmt::While(WhileLoop {
                condition: Expr { kind: ExprKind::BoolLiteral(cond_val), span },
                body: vec![Stmt::Assignment(Assignment {
                    target: Expr { kind: ExprKind::Ident("x".to_string()), span },
                    value: Expr { kind: ExprKind::IntLiteral(1), span },
                    span,
                })],
                span,
            });

            let program = wrap_in_handler(while_stmt, span);
            let result = super::super::check(program);
            prop_assert!(result.is_ok(), "While with Bool condition should pass: {:?}", result.err());
        }

        #[test]
        fn prop_while_condition_non_bool_errors(
            non_bool in arb_span().prop_flat_map(|s| arb_non_bool_expr(s)),
            span in arb_span(),
        ) {
            let while_stmt = Stmt::While(WhileLoop {
                condition: non_bool,
                body: vec![Stmt::Assignment(Assignment {
                    target: Expr { kind: ExprKind::Ident("x".to_string()), span },
                    value: Expr { kind: ExprKind::IntLiteral(1), span },
                    span,
                })],
                span,
            });

            let program = wrap_in_handler(while_stmt, span);
            let result = super::super::check(program);
            prop_assert!(result.is_err(), "While with non-Bool condition should fail");

            let err = result.unwrap_err();
            match &err {
                CompileError::Type(msg) => {
                    prop_assert!(
                        msg.contains("condition must be Bool"),
                        "Error should mention condition must be Bool, got: {}", msg
                    );
                }
                other => {
                    prop_assert!(false, "Expected CompileError::Type, got: {:?}", other);
                }
            }
        }
    }

    // ========================================================================
    // Property 9: Scope Resolution Order (Shadowing)
    // ========================================================================

    // **Validates: Requirements 5.1**
    //
    // For any program where the same identifier is defined at strategy scope
    // (param) and then re-assigned in handler scope, the type checker resolves
    // to the innermost binding's type.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn prop_scope_resolution_inner_shadows_outer(
            param_val in 0i64..10000,
            span in arb_span(),
        ) {
            // Create a program with param "x" = Int, then in handler assign
            // a new variable "y" = x (should resolve x as Int from param scope).
            // Then assign "x" = <same type Int> (reassignment ok since same type).
            // Then assign "z" = x (should still resolve to Int).
            let params_block = ParamsBlock {
                params: vec![Param {
                    name: "x".to_string(),
                    default_value: Expr { kind: ExprKind::IntLiteral(param_val), span },
                    span,
                }],
                span,
            };

            let handler_body = vec![
                // y = x (x resolves to Int from params)
                Stmt::Assignment(Assignment {
                    target: Expr { kind: ExprKind::Ident("y".to_string()), span },
                    value: Expr { kind: ExprKind::Ident("x".to_string()), span },
                    span,
                }),
                // x = 99 (reassignment with compatible Int)
                Stmt::Assignment(Assignment {
                    target: Expr { kind: ExprKind::Ident("x".to_string()), span },
                    value: Expr { kind: ExprKind::IntLiteral(99), span },
                    span,
                }),
                // z = x (should still be Int)
                Stmt::Assignment(Assignment {
                    target: Expr { kind: ExprKind::Ident("z".to_string()), span },
                    value: Expr { kind: ExprKind::Ident("x".to_string()), span },
                    span,
                }),
            ];

            let program = Program {
                imports: vec![],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![
                        StrategyItem::ParamsBlock(params_block),
                        StrategyItem::EventHandler(EventHandler {
                            event_name: "bar".to_string(),
                            body: handler_body,
                            span,
                        }),
                    ],
                    span,
                },
                span,
            };

            let result = super::super::check(program);
            prop_assert!(result.is_ok(), "Shadowing program should pass: {:?}", result.err());

            let typed = result.unwrap();
            // Check that the last assignment (z = x) resolves x as Int
            if let TypedStrategyItem::EventHandler(eh) = &typed.strategy.body[1] {
                // y = x assignment: value (x) should be Int
                if let TypedStmt::Assignment(assign_y) = &eh.body[0] {
                    prop_assert_eq!(
                        &assign_y.value.resolved_type,
                        &FluxType::Int,
                        "y = x should resolve x as Int from params, got {:?}",
                        assign_y.value.resolved_type
                    );
                }
                // z = x assignment: value (x) should be Int
                if let TypedStmt::Assignment(assign_z) = &eh.body[2] {
                    prop_assert_eq!(
                        &assign_z.value.resolved_type,
                        &FluxType::Int,
                        "z = x should resolve x as Int, got {:?}",
                        assign_z.value.resolved_type
                    );
                }
            } else {
                prop_assert!(false, "Expected event handler");
            }
        }
    }

    // ========================================================================
    // Property 10: Undefined Identifier Produces Error with Name
    // ========================================================================

    // **Validates: Requirements 5.5, 20.3**
    //
    // For any identifier not in scope, the type checker returns an error
    // whose message contains the identifier name.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        #[test]
        fn prop_undefined_identifier_error_contains_name(
            name in "[a-z][a-z0-9_]{2,10}",
            span in arb_span(),
        ) {
            // Filter out names that would collide with built-in market data
            // or signal functions (which are injected in handler scope).
            let reserved = [
                "close", "open", "high", "low", "volume", "symbol",
                "in_position", "OPEN", "CLOSE",
            ];
            prop_assume!(!reserved.contains(&name.as_str()));

            let ident_expr = Expr {
                kind: ExprKind::Ident(name.clone()),
                span,
            };

            let stmt = Stmt::Expr(ExprStmt { expr: ident_expr, span });
            let program = wrap_in_handler(stmt, span);
            let result = super::super::check(program);
            prop_assert!(result.is_err(), "Undefined identifier should fail");

            let err = result.unwrap_err();
            match &err {
                CompileError::Type(msg) => {
                    prop_assert!(
                        msg.contains(&name),
                        "Error should contain the identifier name '{}', got: {}", name, msg
                    );
                    prop_assert!(
                        msg.contains("undefined identifier"),
                        "Error should mention 'undefined identifier', got: {}", msg
                    );
                }
                other => {
                    prop_assert!(false, "Expected CompileError::Type, got: {:?}", other);
                }
            }
        }
    }

    // ========================================================================
    // Property 11: Literal Type Inference for Params and State
    // ========================================================================

    /// Generate a literal expression tagged with the expected FluxType for params.
    fn arb_param_literal(span: Span) -> impl Strategy<Value = (Expr, FluxType)> {
        prop_oneof![
            (0i64..10000).prop_map(move |v| (
                Expr { kind: ExprKind::IntLiteral(v), span },
                FluxType::Int,
            )),
            (1u32..999, 1u32..99).prop_map(move |(i, d)| {
                let f: f64 = format!("{}.{}", i, d).parse().unwrap();
                (
                    Expr { kind: ExprKind::FloatLiteral(f), span },
                    FluxType::Float,
                )
            }),
            "[a-z]{1,8}".prop_map(move |s| (
                Expr { kind: ExprKind::StringLiteral(s), span },
                FluxType::String,
            )),
            any::<bool>().prop_map(move |b| (
                Expr { kind: ExprKind::BoolLiteral(b), span },
                FluxType::Bool,
            )),
        ]
    }

    // **Validates: Requirements 5.2, 5.3, 12.1, 12.2, 12.3, 12.4, 13.1**
    //
    // For any param with a literal default: IntLiteral→Int, FloatLiteral→Float,
    // StringLiteral→String, BoolLiteral→Bool. Same for state variable
    // initializers that are literals.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn prop_literal_type_inference_params(
            literals in arb_span().prop_flat_map(|s| {
                prop::collection::vec(arb_param_literal(s), 1..=4)
            }),
            span in arb_span(),
        ) {
            // Build a program with params from the generated literals
            let params: Vec<Param> = literals.iter().enumerate().map(|(i, (expr, _ty))| {
                Param {
                    name: format!("p_{}", i),
                    default_value: expr.clone(),
                    span,
                }
            }).collect();

            let program = Program {
                imports: vec![],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![
                        StrategyItem::ParamsBlock(ParamsBlock { params, span }),
                        StrategyItem::EventHandler(EventHandler {
                            event_name: "bar".to_string(),
                            body: vec![Stmt::Expr(ExprStmt {
                                expr: Expr { kind: ExprKind::IntLiteral(1), span },
                                span,
                            })],
                            span,
                        }),
                    ],
                    span,
                },
                span,
            };

            let result = super::super::check(program);
            prop_assert!(result.is_ok(), "Params with literal defaults should pass: {:?}", result.err());

            let typed = result.unwrap();
            if let TypedStrategyItem::ParamsBlock(pb) = &typed.strategy.body[0] {
                for (i, (_, expected_ty)) in literals.iter().enumerate() {
                    prop_assert_eq!(
                        &pb.params[i].resolved_type,
                        expected_ty,
                        "Param {} expected {:?}, got {:?}",
                        i, expected_ty, pb.params[i].resolved_type
                    );
                }
            } else {
                prop_assert!(false, "Expected ParamsBlock");
            }
        }

        #[test]
        fn prop_literal_type_inference_state(
            literals in arb_span().prop_flat_map(|s| {
                prop::collection::vec(arb_param_literal(s), 1..=3)
            }),
            span in arb_span(),
        ) {
            // Build a program with state vars from the generated literals
            let state_vars: Vec<StateVar> = literals.iter().enumerate().map(|(i, (expr, _ty))| {
                StateVar {
                    name: format!("s_{}", i),
                    initial_value: expr.clone(),
                    span,
                }
            }).collect();

            let program = Program {
                imports: vec![],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![
                        StrategyItem::StateBlock(StateBlock { variables: state_vars, span }),
                        StrategyItem::EventHandler(EventHandler {
                            event_name: "bar".to_string(),
                            body: vec![Stmt::Expr(ExprStmt {
                                expr: Expr { kind: ExprKind::IntLiteral(1), span },
                                span,
                            })],
                            span,
                        }),
                    ],
                    span,
                },
                span,
            };

            let result = super::super::check(program);
            prop_assert!(result.is_ok(), "State with literal inits should pass: {:?}", result.err());

            let typed = result.unwrap();
            if let TypedStrategyItem::StateBlock(sb) = &typed.strategy.body[0] {
                for (i, (_, expected_ty)) in literals.iter().enumerate() {
                    prop_assert_eq!(
                        &sb.variables[i].resolved_type,
                        expected_ty,
                        "State var {} expected {:?}, got {:?}",
                        i, expected_ty, sb.variables[i].resolved_type
                    );
                }
            } else {
                prop_assert!(false, "Expected StateBlock");
            }
        }
    }

    // ========================================================================
    // Property 12: List Literal Type Inference
    // ========================================================================

    // **Validates: Requirements 2.3, 2.5, 2.6**
    //
    // Homogeneous numeric list → VecFloat. Mixed Int/Float → VecFloat.
    // Incompatible (String+Int) → error.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn prop_list_literal_homogeneous_int(
            values in prop::collection::vec(0i64..10000, 1..=5),
            span in arb_span(),
        ) {
            let elements: Vec<Expr> = values.iter().map(|v| Expr {
                kind: ExprKind::IntLiteral(*v),
                span,
            }).collect();

            let state_vars = vec![StateVar {
                name: "items".to_string(),
                initial_value: Expr { kind: ExprKind::ListLiteral(elements), span },
                span,
            }];

            let program = Program {
                imports: vec![],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![
                        StrategyItem::StateBlock(StateBlock { variables: state_vars, span }),
                        StrategyItem::EventHandler(EventHandler {
                            event_name: "bar".to_string(),
                            body: vec![Stmt::Expr(ExprStmt {
                                expr: Expr { kind: ExprKind::IntLiteral(1), span },
                                span,
                            })],
                            span,
                        }),
                    ],
                    span,
                },
                span,
            };

            let result = super::super::check(program);
            prop_assert!(result.is_ok(), "Homogeneous Int list should pass: {:?}", result.err());

            let typed = result.unwrap();
            if let TypedStrategyItem::StateBlock(sb) = &typed.strategy.body[0] {
                prop_assert_eq!(
                    &sb.variables[0].resolved_type,
                    &FluxType::VecFloat,
                    "Expected VecFloat, got {:?}",
                    sb.variables[0].resolved_type
                );
            } else {
                prop_assert!(false, "Expected StateBlock");
            }
        }

        #[test]
        fn prop_list_literal_mixed_numeric(
            int_vals in prop::collection::vec(0i64..10000, 1..=3),
            float_idx in 0usize..4,
            span in arb_span(),
        ) {
            // Build a list with at least one Int and one Float
            let mut elements: Vec<Expr> = int_vals.iter().map(|v| Expr {
                kind: ExprKind::IntLiteral(*v),
                span,
            }).collect();
            // Insert a Float at the given index (clamped to valid range)
            let insert_at = float_idx.min(elements.len());
            elements.insert(insert_at, Expr {
                kind: ExprKind::FloatLiteral(3.14),
                span,
            });

            let state_vars = vec![StateVar {
                name: "nums".to_string(),
                initial_value: Expr { kind: ExprKind::ListLiteral(elements), span },
                span,
            }];

            let program = Program {
                imports: vec![],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![
                        StrategyItem::StateBlock(StateBlock { variables: state_vars, span }),
                        StrategyItem::EventHandler(EventHandler {
                            event_name: "bar".to_string(),
                            body: vec![Stmt::Expr(ExprStmt {
                                expr: Expr { kind: ExprKind::IntLiteral(1), span },
                                span,
                            })],
                            span,
                        }),
                    ],
                    span,
                },
                span,
            };

            let result = super::super::check(program);
            prop_assert!(result.is_ok(), "Mixed numeric list should pass: {:?}", result.err());

            let typed = result.unwrap();
            if let TypedStrategyItem::StateBlock(sb) = &typed.strategy.body[0] {
                prop_assert_eq!(
                    &sb.variables[0].resolved_type,
                    &FluxType::VecFloat,
                    "Expected VecFloat for mixed numeric, got {:?}",
                    sb.variables[0].resolved_type
                );
            } else {
                prop_assert!(false, "Expected StateBlock");
            }
        }

        #[test]
        fn prop_list_literal_incompatible_errors(
            int_val in 0i64..10000,
            str_val in "[a-z]{1,8}",
            span in arb_span(),
        ) {
            // Mix String and Int in a list → should error
            let elements = vec![
                Expr { kind: ExprKind::StringLiteral(str_val), span },
                Expr { kind: ExprKind::IntLiteral(int_val), span },
            ];

            let state_vars = vec![StateVar {
                name: "bad".to_string(),
                initial_value: Expr { kind: ExprKind::ListLiteral(elements), span },
                span,
            }];

            let program = Program {
                imports: vec![],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![
                        StrategyItem::StateBlock(StateBlock { variables: state_vars, span }),
                        StrategyItem::EventHandler(EventHandler {
                            event_name: "bar".to_string(),
                            body: vec![Stmt::Expr(ExprStmt {
                                expr: Expr { kind: ExprKind::IntLiteral(1), span },
                                span,
                            })],
                            span,
                        }),
                    ],
                    span,
                },
                span,
            };

            let result = super::super::check(program);
            prop_assert!(result.is_err(), "Incompatible list should fail");

            let err = result.unwrap_err();
            match &err {
                CompileError::Type(msg) => {
                    prop_assert!(
                        msg.contains("incompatible types"),
                        "Error should mention incompatible types, got: {}", msg
                    );
                }
                other => {
                    prop_assert!(false, "Expected CompileError::Type, got: {:?}", other);
                }
            }
        }
    }

    // ========================================================================
    // Property 13: Reassignment Type Consistency
    // ========================================================================

    // **Validates: Requirements 11.2, 11.3**
    //
    // Assignment to existing variable with compatible type (same or Int→Float)
    // → accepted. Assignment with incompatible type → error.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn prop_reassignment_compatible_accepted(
            init_val in 0i64..10000,
            reassign_val in 0i64..10000,
            span in arb_span(),
        ) {
            // Assign Int, then reassign Int → ok
            let handler_body = vec![
                Stmt::Assignment(Assignment {
                    target: Expr { kind: ExprKind::Ident("x".to_string()), span },
                    value: Expr { kind: ExprKind::IntLiteral(init_val), span },
                    span,
                }),
                Stmt::Assignment(Assignment {
                    target: Expr { kind: ExprKind::Ident("x".to_string()), span },
                    value: Expr { kind: ExprKind::IntLiteral(reassign_val), span },
                    span,
                }),
            ];

            let program = Program {
                imports: vec![],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![StrategyItem::EventHandler(EventHandler {
                        event_name: "bar".to_string(),
                        body: handler_body,
                        span,
                    })],
                    span,
                },
                span,
            };

            let result = super::super::check(program);
            prop_assert!(result.is_ok(), "Same-type reassignment should pass: {:?}", result.err());
        }

        #[test]
        fn prop_reassignment_int_to_float_accepted(
            init_val in 0i64..10000,
            span in arb_span(),
        ) {
            // Assign Float, then reassign Int → ok (Int assignable to Float)
            let handler_body = vec![
                Stmt::Assignment(Assignment {
                    target: Expr { kind: ExprKind::Ident("x".to_string()), span },
                    value: Expr { kind: ExprKind::FloatLiteral(1.5), span },
                    span,
                }),
                Stmt::Assignment(Assignment {
                    target: Expr { kind: ExprKind::Ident("x".to_string()), span },
                    value: Expr { kind: ExprKind::IntLiteral(init_val), span },
                    span,
                }),
            ];

            let program = Program {
                imports: vec![],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![StrategyItem::EventHandler(EventHandler {
                        event_name: "bar".to_string(),
                        body: handler_body,
                        span,
                    })],
                    span,
                },
                span,
            };

            let result = super::super::check(program);
            prop_assert!(result.is_ok(), "Int→Float reassignment should pass: {:?}", result.err());
        }

        #[test]
        fn prop_reassignment_incompatible_errors(
            int_val in 0i64..10000,
            str_val in "[a-z]{1,8}",
            span in arb_span(),
        ) {
            // Assign Int, then reassign String → error
            let handler_body = vec![
                Stmt::Assignment(Assignment {
                    target: Expr { kind: ExprKind::Ident("x".to_string()), span },
                    value: Expr { kind: ExprKind::IntLiteral(int_val), span },
                    span,
                }),
                Stmt::Assignment(Assignment {
                    target: Expr { kind: ExprKind::Ident("x".to_string()), span },
                    value: Expr { kind: ExprKind::StringLiteral(str_val), span },
                    span,
                }),
            ];

            let program = Program {
                imports: vec![],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![StrategyItem::EventHandler(EventHandler {
                        event_name: "bar".to_string(),
                        body: handler_body,
                        span,
                    })],
                    span,
                },
                span,
            };

            let result = super::super::check(program);
            prop_assert!(result.is_err(), "Incompatible reassignment should fail");

            let err = result.unwrap_err();
            match &err {
                CompileError::Type(msg) => {
                    prop_assert!(
                        msg.contains("cannot assign"),
                        "Error should mention cannot assign, got: {}", msg
                    );
                }
                other => {
                    prop_assert!(false, "Expected CompileError::Type, got: {:?}", other);
                }
            }
        }
    }

    // ========================================================================
    // Property 14: List Method Type Resolution
    // ========================================================================

    // **Validates: Requirements 15.1, 15.2, 15.3, 15.4**
    //
    // `List(T).append(T)` → Void. `List(T).len()` → Int. `List(T).pop()` → T.
    // Unknown method on List → error. Method on non-List → error.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn prop_list_method_append_returns_void(
            append_val in "[a-z]{1,5}",
            span in arb_span(),
        ) {
            // state: items = ["a"] (use String list to keep List(T) type)
            // handler: items.append(N) → Void
            let state_vars = vec![StateVar {
                name: "items".to_string(),
                initial_value: Expr {
                    kind: ExprKind::ListLiteral(vec![
                        Expr { kind: ExprKind::StringLiteral("a".to_string()), span },
                    ]),
                    span,
                },
                span,
            }];

            let method_call_expr = Expr {
                kind: ExprKind::MethodCall {
                    receiver: Box::new(Expr { kind: ExprKind::Ident("items".to_string()), span }),
                    method: "append".to_string(),
                    args: vec![Expr { kind: ExprKind::StringLiteral(append_val), span }],
                },
                span,
            };

            let program = Program {
                imports: vec![],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![
                        StrategyItem::StateBlock(StateBlock { variables: state_vars, span }),
                        StrategyItem::EventHandler(EventHandler {
                            event_name: "bar".to_string(),
                            body: vec![Stmt::Expr(ExprStmt { expr: method_call_expr, span })],
                            span,
                        }),
                    ],
                    span,
                },
                span,
            };

            let result = super::super::check(program);
            prop_assert!(result.is_ok(), "append should pass: {:?}", result.err());

            let typed = result.unwrap();
            if let TypedStrategyItem::EventHandler(eh) = &typed.strategy.body[1] {
                if let TypedStmt::Expr(es) = &eh.body[0] {
                    prop_assert_eq!(
                        &es.expr.resolved_type,
                        &FluxType::Void,
                        "append should return Void, got {:?}",
                        es.expr.resolved_type
                    );
                } else {
                    prop_assert!(false, "Expected expr statement");
                }
            } else {
                prop_assert!(false, "Expected event handler");
            }
        }

        #[test]
        fn prop_list_method_len_returns_int(
            span in arb_span(),
        ) {
            let state_vars = vec![StateVar {
                name: "items".to_string(),
                initial_value: Expr {
                    kind: ExprKind::ListLiteral(vec![
                        Expr { kind: ExprKind::StringLiteral("a".to_string()), span },
                    ]),
                    span,
                },
                span,
            }];

            let method_call_expr = Expr {
                kind: ExprKind::MethodCall {
                    receiver: Box::new(Expr { kind: ExprKind::Ident("items".to_string()), span }),
                    method: "len".to_string(),
                    args: vec![],
                },
                span,
            };

            let stmt = Stmt::Assignment(Assignment {
                target: Expr { kind: ExprKind::Ident("n".to_string()), span },
                value: method_call_expr,
                span,
            });

            let program = Program {
                imports: vec![],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![
                        StrategyItem::StateBlock(StateBlock { variables: state_vars, span }),
                        StrategyItem::EventHandler(EventHandler {
                            event_name: "bar".to_string(),
                            body: vec![stmt],
                            span,
                        }),
                    ],
                    span,
                },
                span,
            };

            let result = super::super::check(program);
            prop_assert!(result.is_ok(), "len should pass: {:?}", result.err());

            let typed = result.unwrap();
            if let TypedStrategyItem::EventHandler(eh) = &typed.strategy.body[1] {
                if let TypedStmt::Assignment(assign) = &eh.body[0] {
                    prop_assert_eq!(
                        &assign.value.resolved_type,
                        &FluxType::Int,
                        "len should return Int, got {:?}",
                        assign.value.resolved_type
                    );
                } else {
                    prop_assert!(false, "Expected assignment");
                }
            } else {
                prop_assert!(false, "Expected event handler");
            }
        }

        #[test]
        fn prop_list_method_pop_returns_element_type(
            span in arb_span(),
        ) {
            let state_vars = vec![StateVar {
                name: "items".to_string(),
                initial_value: Expr {
                    kind: ExprKind::ListLiteral(vec![
                        Expr { kind: ExprKind::StringLiteral("a".to_string()), span },
                    ]),
                    span,
                },
                span,
            }];

            let method_call_expr = Expr {
                kind: ExprKind::MethodCall {
                    receiver: Box::new(Expr { kind: ExprKind::Ident("items".to_string()), span }),
                    method: "pop".to_string(),
                    args: vec![],
                },
                span,
            };

            let stmt = Stmt::Assignment(Assignment {
                target: Expr { kind: ExprKind::Ident("last".to_string()), span },
                value: method_call_expr,
                span,
            });

            let program = Program {
                imports: vec![],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![
                        StrategyItem::StateBlock(StateBlock { variables: state_vars, span }),
                        StrategyItem::EventHandler(EventHandler {
                            event_name: "bar".to_string(),
                            body: vec![stmt],
                            span,
                        }),
                    ],
                    span,
                },
                span,
            };

            let result = super::super::check(program);
            prop_assert!(result.is_ok(), "pop should pass: {:?}", result.err());

            let typed = result.unwrap();
            if let TypedStrategyItem::EventHandler(eh) = &typed.strategy.body[1] {
                if let TypedStmt::Assignment(assign) = &eh.body[0] {
                    prop_assert_eq!(
                        &assign.value.resolved_type,
                        &FluxType::String,
                        "pop on List(String) should return String, got {:?}",
                        assign.value.resolved_type
                    );
                } else {
                    prop_assert!(false, "Expected assignment");
                }
            } else {
                prop_assert!(false, "Expected event handler");
            }
        }

        #[test]
        fn prop_list_unknown_method_errors(
            method_name in "[a-z]{3,8}",
            span in arb_span(),
        ) {
            // Filter out known methods
            prop_assume!(method_name != "append" && method_name != "len" && method_name != "pop");

            let state_vars = vec![StateVar {
                name: "items".to_string(),
                initial_value: Expr {
                    kind: ExprKind::ListLiteral(vec![
                        Expr { kind: ExprKind::StringLiteral("a".to_string()), span },
                    ]),
                    span,
                },
                span,
            }];

            let method_call_expr = Expr {
                kind: ExprKind::MethodCall {
                    receiver: Box::new(Expr { kind: ExprKind::Ident("items".to_string()), span }),
                    method: method_name.clone(),
                    args: vec![],
                },
                span,
            };

            let program = Program {
                imports: vec![],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![
                        StrategyItem::StateBlock(StateBlock { variables: state_vars, span }),
                        StrategyItem::EventHandler(EventHandler {
                            event_name: "bar".to_string(),
                            body: vec![Stmt::Expr(ExprStmt { expr: method_call_expr, span })],
                            span,
                        }),
                    ],
                    span,
                },
                span,
            };

            let result = super::super::check(program);
            prop_assert!(result.is_err(), "Unknown method should fail");

            let err = result.unwrap_err();
            match &err {
                CompileError::Type(msg) => {
                    prop_assert!(
                        msg.contains("does not have method"),
                        "Error should mention method not found, got: {}", msg
                    );
                    prop_assert!(
                        msg.contains(&method_name),
                        "Error should contain method name '{}', got: {}", method_name, msg
                    );
                }
                other => {
                    prop_assert!(false, "Expected CompileError::Type, got: {:?}", other);
                }
            }
        }

        #[test]
        fn prop_method_on_non_list_errors(
            span in arb_span(),
        ) {
            // Call .append() on an Int variable → error
            let handler_body = vec![
                Stmt::Assignment(Assignment {
                    target: Expr { kind: ExprKind::Ident("x".to_string()), span },
                    value: Expr { kind: ExprKind::IntLiteral(42), span },
                    span,
                }),
                Stmt::Expr(ExprStmt {
                    expr: Expr {
                        kind: ExprKind::MethodCall {
                            receiver: Box::new(Expr { kind: ExprKind::Ident("x".to_string()), span }),
                            method: "append".to_string(),
                            args: vec![Expr { kind: ExprKind::IntLiteral(1), span }],
                        },
                        span,
                    },
                    span,
                }),
            ];

            let program = Program {
                imports: vec![],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![StrategyItem::EventHandler(EventHandler {
                        event_name: "bar".to_string(),
                        body: handler_body,
                        span,
                    })],
                    span,
                },
                span,
            };

            let result = super::super::check(program);
            prop_assert!(result.is_err(), "Method on non-List should fail");

            let err = result.unwrap_err();
            match &err {
                CompileError::Type(msg) => {
                    prop_assert!(
                        msg.contains("does not have method"),
                        "Error should mention method not supported, got: {}", msg
                    );
                }
                other => {
                    prop_assert!(false, "Expected CompileError::Type, got: {:?}", other);
                }
            }
        }
    }

    // ========================================================================
    // Property 15: Index Access Type Rules
    // ========================================================================

    // **Validates: Requirements 16.1, 16.2, 16.3**
    //
    // `List(T)[Int]` → T. Non-List indexing → error. Non-Int index → error.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn prop_index_access_list_int_returns_element_type(
            idx_val in 0i64..100,
            span in arb_span(),
        ) {
            // state: items = ["a"] (use String list to keep List(T) type)
            // handler: result = items[idx]
            let state_vars = vec![StateVar {
                name: "items".to_string(),
                initial_value: Expr {
                    kind: ExprKind::ListLiteral(vec![
                        Expr { kind: ExprKind::StringLiteral("a".to_string()), span },
                    ]),
                    span,
                },
                span,
            }];

            let index_expr = Expr {
                kind: ExprKind::IndexAccess {
                    object: Box::new(Expr { kind: ExprKind::Ident("items".to_string()), span }),
                    index: Box::new(Expr { kind: ExprKind::IntLiteral(idx_val), span }),
                },
                span,
            };

            let stmt = Stmt::Assignment(Assignment {
                target: Expr { kind: ExprKind::Ident("result".to_string()), span },
                value: index_expr,
                span,
            });

            let program = Program {
                imports: vec![],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![
                        StrategyItem::StateBlock(StateBlock { variables: state_vars, span }),
                        StrategyItem::EventHandler(EventHandler {
                            event_name: "bar".to_string(),
                            body: vec![stmt],
                            span,
                        }),
                    ],
                    span,
                },
                span,
            };

            let result = super::super::check(program);
            prop_assert!(result.is_ok(), "List[Int] should pass: {:?}", result.err());

            let typed = result.unwrap();
            if let TypedStrategyItem::EventHandler(eh) = &typed.strategy.body[1] {
                if let TypedStmt::Assignment(assign) = &eh.body[0] {
                    prop_assert_eq!(
                        &assign.value.resolved_type,
                        &FluxType::String,
                        "List(String)[Int] should yield String, got {:?}",
                        assign.value.resolved_type
                    );
                } else {
                    prop_assert!(false, "Expected assignment");
                }
            } else {
                prop_assert!(false, "Expected event handler");
            }
        }

        #[test]
        fn prop_index_access_non_list_errors(
            idx_val in 0i64..100,
            span in arb_span(),
        ) {
            // Indexing an Int variable → error
            let handler_body = vec![
                Stmt::Assignment(Assignment {
                    target: Expr { kind: ExprKind::Ident("x".to_string()), span },
                    value: Expr { kind: ExprKind::IntLiteral(42), span },
                    span,
                }),
                Stmt::Assignment(Assignment {
                    target: Expr { kind: ExprKind::Ident("y".to_string()), span },
                    value: Expr {
                        kind: ExprKind::IndexAccess {
                            object: Box::new(Expr { kind: ExprKind::Ident("x".to_string()), span }),
                            index: Box::new(Expr { kind: ExprKind::IntLiteral(idx_val), span }),
                        },
                        span,
                    },
                    span,
                }),
            ];

            let program = Program {
                imports: vec![],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![StrategyItem::EventHandler(EventHandler {
                        event_name: "bar".to_string(),
                        body: handler_body,
                        span,
                    })],
                    span,
                },
                span,
            };

            let result = super::super::check(program);
            prop_assert!(result.is_err(), "Non-List indexing should fail");

            let err = result.unwrap_err();
            match &err {
                CompileError::Type(msg) => {
                    prop_assert!(
                        msg.contains("does not support indexing"),
                        "Error should mention indexing not supported, got: {}", msg
                    );
                }
                other => {
                    prop_assert!(false, "Expected CompileError::Type, got: {:?}", other);
                }
            }
        }

        #[test]
        fn prop_index_access_non_int_index_errors(
            span in arb_span(),
        ) {
            // Indexing a List with String index → error
            let state_vars = vec![StateVar {
                name: "items".to_string(),
                initial_value: Expr {
                    kind: ExprKind::ListLiteral(vec![
                        Expr { kind: ExprKind::StringLiteral("a".to_string()), span },
                    ]),
                    span,
                },
                span,
            }];

            let index_expr = Expr {
                kind: ExprKind::IndexAccess {
                    object: Box::new(Expr { kind: ExprKind::Ident("items".to_string()), span }),
                    index: Box::new(Expr { kind: ExprKind::StringLiteral("bad".to_string()), span }),
                },
                span,
            };

            let stmt = Stmt::Assignment(Assignment {
                target: Expr { kind: ExprKind::Ident("result".to_string()), span },
                value: index_expr,
                span,
            });

            let program = Program {
                imports: vec![],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![
                        StrategyItem::StateBlock(StateBlock { variables: state_vars, span }),
                        StrategyItem::EventHandler(EventHandler {
                            event_name: "bar".to_string(),
                            body: vec![stmt],
                            span,
                        }),
                    ],
                    span,
                },
                span,
            };

            let result = super::super::check(program);
            prop_assert!(result.is_err(), "Non-Int index should fail");

            let err = result.unwrap_err();
            match &err {
                CompileError::Type(msg) => {
                    prop_assert!(
                        msg.contains("index must be Int"),
                        "Error should mention index must be Int, got: {}", msg
                    );
                }
                other => {
                    prop_assert!(false, "Expected CompileError::Type, got: {:?}", other);
                }
            }
        }
    }

    // ========================================================================
    // Property 16: Imported Functions Accept Numeric Args and Return Float
    // ========================================================================

    // **Validates: Requirements 14.1, 14.3, 14.4, 18.1, 18.2**
    //
    // For any imported function called with numeric args → return type is Float.
    // For any call to a non-callable identifier → error.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn prop_imported_function_returns_float(
            fn_name in "[a-z]{3,6}",
            num_args in 1usize..=3,
            span in arb_span(),
        ) {
            let import = Import {
                module_path: "ind".to_string(),
                names: vec![fn_name.clone()],
                span,
            };

            let args: Vec<Expr> = (0..num_args).map(|i| {
                Expr { kind: ExprKind::IntLiteral(i as i64 + 1), span }
            }).collect();

            let call_expr = Expr {
                kind: ExprKind::FunctionCall {
                    function: Box::new(Expr { kind: ExprKind::Ident(fn_name.clone()), span }),
                    args,
                },
                span,
            };

            let stmt = Stmt::Assignment(Assignment {
                target: Expr { kind: ExprKind::Ident("result".to_string()), span },
                value: call_expr,
                span,
            });

            let program = Program {
                imports: vec![import],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![StrategyItem::EventHandler(EventHandler {
                        event_name: "bar".to_string(),
                        body: vec![stmt],
                        span,
                    })],
                    span,
                },
                span,
            };

            let result = super::super::check(program);
            prop_assert!(result.is_ok(), "Imported fn with numeric args should pass: {:?}", result.err());

            let typed = result.unwrap();
            if let TypedStrategyItem::EventHandler(eh) = &typed.strategy.body[0] {
                if let TypedStmt::Assignment(assign) = &eh.body[0] {
                    prop_assert_eq!(
                        &assign.value.resolved_type,
                        &FluxType::Float,
                        "Imported function should return Float, got {:?}",
                        assign.value.resolved_type
                    );
                } else {
                    prop_assert!(false, "Expected assignment statement");
                }
            } else {
                prop_assert!(false, "Expected event handler");
            }
        }

        #[test]
        fn prop_call_non_callable_identifier_errors(
            span in arb_span(),
        ) {
            // Assign an Int to a variable, then try to call it as a function → error
            let handler_body = vec![
                Stmt::Assignment(Assignment {
                    target: Expr { kind: ExprKind::Ident("x".to_string()), span },
                    value: Expr { kind: ExprKind::IntLiteral(42), span },
                    span,
                }),
                Stmt::Expr(ExprStmt {
                    expr: Expr {
                        kind: ExprKind::FunctionCall {
                            function: Box::new(Expr { kind: ExprKind::Ident("x".to_string()), span }),
                            args: vec![Expr { kind: ExprKind::IntLiteral(1), span }],
                        },
                        span,
                    },
                    span,
                }),
            ];

            let program = Program {
                imports: vec![],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![StrategyItem::EventHandler(EventHandler {
                        event_name: "bar".to_string(),
                        body: handler_body,
                        span,
                    })],
                    span,
                },
                span,
            };

            let result = super::super::check(program);
            prop_assert!(result.is_err(), "Calling non-callable should fail");

            let err = result.unwrap_err();
            match &err {
                CompileError::Type(msg) => {
                    prop_assert!(
                        msg.contains("is not a function"),
                        "Error should mention not a function, got: {}", msg
                    );
                }
                other => {
                    prop_assert!(false, "Expected CompileError::Type, got: {:?}", other);
                }
            }
        }
    }

    // ========================================================================
    // Property 17: Signal Function Argument Validation
    // ========================================================================

    // **Validates: Requirements 4.4, 4.5, 4.6**
    //
    // OPEN with wrong arg count (not 2) → error.
    // CLOSE with wrong arg count (not 1 or 2) → error.
    // OPEN/CLOSE with wrong arg types → error.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn prop_open_wrong_arg_count_errors(
            bad_count in prop_oneof![Just(0usize), Just(1usize), Just(3usize), Just(4usize)],
            span in arb_span(),
        ) {
            // OPEN expects exactly 2 args (String, Float)
            let args: Vec<Expr> = (0..bad_count).map(|i| {
                if i == 0 {
                    Expr { kind: ExprKind::StringLiteral("AAPL".to_string()), span }
                } else {
                    Expr { kind: ExprKind::FloatLiteral(1.0), span }
                }
            }).collect();

            let call_expr = Expr {
                kind: ExprKind::FunctionCall {
                    function: Box::new(Expr { kind: ExprKind::Ident("OPEN".to_string()), span }),
                    args,
                },
                span,
            };

            let program = Program {
                imports: vec![],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![StrategyItem::EventHandler(EventHandler {
                        event_name: "bar".to_string(),
                        body: vec![Stmt::Expr(ExprStmt { expr: call_expr, span })],
                        span,
                    })],
                    span,
                },
                span,
            };

            let result = super::super::check(program);
            prop_assert!(result.is_err(), "OPEN with {} args should fail", bad_count);
        }

        #[test]
        fn prop_close_wrong_arg_count_errors(
            bad_count in prop_oneof![Just(0usize), Just(3usize), Just(4usize)],
            span in arb_span(),
        ) {
            // CLOSE expects 1 or 2 args
            let args: Vec<Expr> = (0..bad_count).map(|i| {
                if i == 0 {
                    Expr { kind: ExprKind::StringLiteral("AAPL".to_string()), span }
                } else {
                    Expr { kind: ExprKind::FloatLiteral(1.0), span }
                }
            }).collect();

            let call_expr = Expr {
                kind: ExprKind::FunctionCall {
                    function: Box::new(Expr { kind: ExprKind::Ident("CLOSE".to_string()), span }),
                    args,
                },
                span,
            };

            let program = Program {
                imports: vec![],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![StrategyItem::EventHandler(EventHandler {
                        event_name: "bar".to_string(),
                        body: vec![Stmt::Expr(ExprStmt { expr: call_expr, span })],
                        span,
                    })],
                    span,
                },
                span,
            };

            let result = super::super::check(program);
            prop_assert!(result.is_err(), "CLOSE with {} args should fail", bad_count);
        }

        #[test]
        fn prop_open_wrong_arg_types_errors(
            span in arb_span(),
        ) {
            // OPEN(Int, Int) instead of (String, Float) → error
            let call_expr = Expr {
                kind: ExprKind::FunctionCall {
                    function: Box::new(Expr { kind: ExprKind::Ident("OPEN".to_string()), span }),
                    args: vec![
                        Expr { kind: ExprKind::IntLiteral(42), span },
                        Expr { kind: ExprKind::FloatLiteral(1.0), span },
                    ],
                },
                span,
            };

            let program = Program {
                imports: vec![],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![StrategyItem::EventHandler(EventHandler {
                        event_name: "bar".to_string(),
                        body: vec![Stmt::Expr(ExprStmt { expr: call_expr, span })],
                        span,
                    })],
                    span,
                },
                span,
            };

            let result = super::super::check(program);
            prop_assert!(result.is_err(), "OPEN with wrong arg types should fail");

            let err = result.unwrap_err();
            match &err {
                CompileError::Type(msg) => {
                    prop_assert!(
                        msg.contains("OPEN") && (msg.contains("must be") || msg.contains("argument")),
                        "Error should mention OPEN argument type mismatch, got: {}", msg
                    );
                }
                other => {
                    prop_assert!(false, "Expected CompileError::Type, got: {:?}", other);
                }
            }
        }

        #[test]
        fn prop_close_wrong_arg_types_errors(
            span in arb_span(),
        ) {
            // CLOSE(Int) instead of (String) → error
            let call_expr = Expr {
                kind: ExprKind::FunctionCall {
                    function: Box::new(Expr { kind: ExprKind::Ident("CLOSE".to_string()), span }),
                    args: vec![
                        Expr { kind: ExprKind::IntLiteral(42), span },
                    ],
                },
                span,
            };

            let program = Program {
                imports: vec![],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![StrategyItem::EventHandler(EventHandler {
                        event_name: "bar".to_string(),
                        body: vec![Stmt::Expr(ExprStmt { expr: call_expr, span })],
                        span,
                    })],
                    span,
                },
                span,
            };

            let result = super::super::check(program);
            prop_assert!(result.is_err(), "CLOSE with wrong arg types should fail");

            let err = result.unwrap_err();
            match &err {
                CompileError::Type(msg) => {
                    prop_assert!(
                        msg.contains("CLOSE") && (msg.contains("incompatible") || msg.contains("argument")),
                        "Error should mention CLOSE argument type mismatch, got: {}", msg
                    );
                }
                other => {
                    prop_assert!(false, "Expected CompileError::Type, got: {:?}", other);
                }
            }
        }
    }

    // ========================================================================
    // Property 18: Market Data Scope Restriction
    // ========================================================================

    // **Validates: Requirements 3.4**
    //
    // Market data identifiers (`close`, `open`, `high`, `low`, `volume`,
    // `symbol`, `in_position`) used outside event handler → error.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn prop_market_data_outside_handler_errors(
            idx in 0usize..7,
            span in arb_span(),
        ) {
            let market_names = ["close", "open", "high", "low", "volume", "symbol", "in_position"];
            let name = market_names[idx].to_string();

            // Reference a market data identifier in a strategy Property (outside handler)
            let property = Property {
                name: "my_prop".to_string(),
                value: Expr { kind: ExprKind::Ident(name.clone()), span },
                span,
            };

            let program = Program {
                imports: vec![],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![
                        StrategyItem::Property(property),
                        StrategyItem::EventHandler(EventHandler {
                            event_name: "bar".to_string(),
                            body: vec![Stmt::Expr(ExprStmt {
                                expr: Expr { kind: ExprKind::IntLiteral(1), span },
                                span,
                            })],
                            span,
                        }),
                    ],
                    span,
                },
                span,
            };

            let result = super::super::check(program);
            prop_assert!(result.is_err(), "Market data '{}' outside handler should fail", name);

            let err = result.unwrap_err();
            match &err {
                CompileError::Type(msg) => {
                    prop_assert!(
                        msg.contains("only available inside event handlers") || msg.contains("undefined identifier"),
                        "Error should restrict market data outside handler, got: {}", msg
                    );
                }
                other => {
                    prop_assert!(false, "Expected CompileError::Type, got: {:?}", other);
                }
            }
        }
    }

    // ========================================================================
    // Property 19: For-Loop Variable Binding
    // ========================================================================

    // **Validates: Requirements 5.6, 10.3, 10.5**
    //
    // For-loop over List(T) binds loop variable to type T.
    // Loop variable is not accessible after the loop.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn prop_for_loop_binds_variable_to_element_type(
            span in arb_span(),
        ) {
            // state: items = ["a"] (use String list to keep List(T) type)
            // handler: for item in items { x = item }
            // Verify x resolves to String (element type of List(String))
            let state_vars = vec![StateVar {
                name: "items".to_string(),
                initial_value: Expr {
                    kind: ExprKind::ListLiteral(vec![
                        Expr { kind: ExprKind::StringLiteral("a".to_string()), span },
                    ]),
                    span,
                },
                span,
            }];

            let for_loop = Stmt::For(ForLoop {
                variable: "item".to_string(),
                iterable: Expr { kind: ExprKind::Ident("items".to_string()), span },
                body: vec![
                    Stmt::Assignment(Assignment {
                        target: Expr { kind: ExprKind::Ident("x".to_string()), span },
                        value: Expr { kind: ExprKind::Ident("item".to_string()), span },
                        span,
                    }),
                ],
                span,
            });

            let program = Program {
                imports: vec![],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![
                        StrategyItem::StateBlock(StateBlock { variables: state_vars, span }),
                        StrategyItem::EventHandler(EventHandler {
                            event_name: "bar".to_string(),
                            body: vec![for_loop],
                            span,
                        }),
                    ],
                    span,
                },
                span,
            };

            let result = super::super::check(program);
            prop_assert!(result.is_ok(), "For-loop with valid List should pass: {:?}", result.err());

            let typed = result.unwrap();
            if let TypedStrategyItem::EventHandler(eh) = &typed.strategy.body[1] {
                if let TypedStmt::For(f) = &eh.body[0] {
                    // Loop variable type should be String
                    prop_assert_eq!(
                        &f.variable_type,
                        &FluxType::String,
                        "Loop variable should be String, got {:?}",
                        f.variable_type
                    );
                    // Body assignment x = item: value should be String
                    if let TypedStmt::Assignment(assign) = &f.body[0] {
                        prop_assert_eq!(
                            &assign.value.resolved_type,
                            &FluxType::String,
                            "x = item should resolve item as String, got {:?}",
                            assign.value.resolved_type
                        );
                    } else {
                        prop_assert!(false, "Expected assignment in for body");
                    }
                } else {
                    prop_assert!(false, "Expected for loop");
                }
            } else {
                prop_assert!(false, "Expected event handler");
            }
        }

        #[test]
        fn prop_for_loop_variable_not_accessible_after(
            span in arb_span(),
        ) {
            // state: items = ["a"]
            // handler: for item in items { z = item }; y = item  ← error
            let state_vars = vec![StateVar {
                name: "items".to_string(),
                initial_value: Expr {
                    kind: ExprKind::ListLiteral(vec![
                        Expr { kind: ExprKind::StringLiteral("a".to_string()), span },
                    ]),
                    span,
                },
                span,
            }];

            let for_loop = Stmt::For(ForLoop {
                variable: "item".to_string(),
                iterable: Expr { kind: ExprKind::Ident("items".to_string()), span },
                body: vec![
                    Stmt::Assignment(Assignment {
                        target: Expr { kind: ExprKind::Ident("z".to_string()), span },
                        value: Expr { kind: ExprKind::Ident("item".to_string()), span },
                        span,
                    }),
                ],
                span,
            });

            // After the loop, try to access "item"
            let after_loop = Stmt::Assignment(Assignment {
                target: Expr { kind: ExprKind::Ident("y".to_string()), span },
                value: Expr { kind: ExprKind::Ident("item".to_string()), span },
                span,
            });

            let program = Program {
                imports: vec![],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![
                        StrategyItem::StateBlock(StateBlock { variables: state_vars, span }),
                        StrategyItem::EventHandler(EventHandler {
                            event_name: "bar".to_string(),
                            body: vec![for_loop, after_loop],
                            span,
                        }),
                    ],
                    span,
                },
                span,
            };

            let result = super::super::check(program);
            prop_assert!(result.is_err(), "Accessing loop var after loop should fail");

            let err = result.unwrap_err();
            match &err {
                CompileError::Type(msg) => {
                    prop_assert!(
                        msg.contains("undefined identifier") && msg.contains("item"),
                        "Error should mention undefined identifier 'item', got: {}", msg
                    );
                }
                other => {
                    prop_assert!(false, "Expected CompileError::Type, got: {:?}", other);
                }
            }
        }
    }

    // ========================================================================
    // Property 20: Nested Scope Isolation
    // ========================================================================

    // **Validates: Requirements 9.5, 10.5**
    //
    // Variable declared inside if/loop body is NOT accessible after the block.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn prop_if_body_variable_not_accessible_after(
            span in arb_span(),
        ) {
            // handler: if true { inner = 1 }; z = inner  ← error
            let if_stmt = Stmt::If(IfStmt {
                condition: Expr { kind: ExprKind::BoolLiteral(true), span },
                body: vec![
                    Stmt::Assignment(Assignment {
                        target: Expr { kind: ExprKind::Ident("inner".to_string()), span },
                        value: Expr { kind: ExprKind::IntLiteral(1), span },
                        span,
                    }),
                ],
                elif_branches: vec![],
                else_body: None,
                span,
            });

            let after_if = Stmt::Assignment(Assignment {
                target: Expr { kind: ExprKind::Ident("z".to_string()), span },
                value: Expr { kind: ExprKind::Ident("inner".to_string()), span },
                span,
            });

            let program = Program {
                imports: vec![],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![StrategyItem::EventHandler(EventHandler {
                        event_name: "bar".to_string(),
                        body: vec![if_stmt, after_if],
                        span,
                    })],
                    span,
                },
                span,
            };

            let result = super::super::check(program);
            prop_assert!(result.is_err(), "Accessing if-body var after block should fail");

            let err = result.unwrap_err();
            match &err {
                CompileError::Type(msg) => {
                    prop_assert!(
                        msg.contains("undefined identifier") && msg.contains("inner"),
                        "Error should mention undefined identifier 'inner', got: {}", msg
                    );
                }
                other => {
                    prop_assert!(false, "Expected CompileError::Type, got: {:?}", other);
                }
            }
        }

        #[test]
        fn prop_while_body_variable_not_accessible_after(
            span in arb_span(),
        ) {
            // handler: while true { wvar = 1 }; q = wvar  ← error
            let while_stmt = Stmt::While(WhileLoop {
                condition: Expr { kind: ExprKind::BoolLiteral(true), span },
                body: vec![
                    Stmt::Assignment(Assignment {
                        target: Expr { kind: ExprKind::Ident("wvar".to_string()), span },
                        value: Expr { kind: ExprKind::IntLiteral(1), span },
                        span,
                    }),
                ],
                span,
            });

            let after_while = Stmt::Assignment(Assignment {
                target: Expr { kind: ExprKind::Ident("q".to_string()), span },
                value: Expr { kind: ExprKind::Ident("wvar".to_string()), span },
                span,
            });

            let program = Program {
                imports: vec![],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![StrategyItem::EventHandler(EventHandler {
                        event_name: "bar".to_string(),
                        body: vec![while_stmt, after_while],
                        span,
                    })],
                    span,
                },
                span,
            };

            let result = super::super::check(program);
            prop_assert!(result.is_err(), "Accessing while-body var after block should fail");

            let err = result.unwrap_err();
            match &err {
                CompileError::Type(msg) => {
                    prop_assert!(
                        msg.contains("undefined identifier") && msg.contains("wvar"),
                        "Error should mention undefined identifier 'wvar', got: {}", msg
                    );
                }
                other => {
                    prop_assert!(false, "Expected CompileError::Type, got: {:?}", other);
                }
            }
        }
    }

    // ========================================================================
    // Feature: portfolio-construction, Property 1: All-Numeric List Literal Infers VecFloat
    // ========================================================================

    // **Validates: Requirements 1.1, 1.2**
    //
    // For any list literal expression containing only elements that resolve to
    // numeric types (Int or Float), the type checker SHALL infer the type as
    // VecFloat.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_portfolio_vecfloat_inference(
            numeric_exprs in arb_span().prop_flat_map(|s| {
                prop::collection::vec(arb_numeric_expr(s), 1..=20)
            }),
            span in arb_span(),
        ) {
            // Extract expressions from the (Expr, FluxType) pairs
            let elements: Vec<Expr> = numeric_exprs.iter().map(|(expr, _)| expr.clone()).collect();
            let list_len = elements.len();

            // Wrap the list literal in a state variable assignment
            let state_vars = vec![StateVar {
                name: "weights".to_string(),
                initial_value: Expr {
                    kind: ExprKind::ListLiteral(elements),
                    span,
                },
                span,
            }];

            let program = Program {
                imports: vec![],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![
                        StrategyItem::StateBlock(StateBlock { variables: state_vars, span }),
                        StrategyItem::EventHandler(EventHandler {
                            event_name: "bar".to_string(),
                            body: vec![Stmt::Expr(ExprStmt {
                                expr: Expr { kind: ExprKind::IntLiteral(1), span },
                                span,
                            })],
                            span,
                        }),
                    ],
                    span,
                },
                span,
            };

            let result = super::super::check(program);
            prop_assert!(
                result.is_ok(),
                "All-numeric list of length {} should pass type checking: {:?}",
                list_len,
                result.err()
            );

            let typed = result.unwrap();
            if let TypedStrategyItem::StateBlock(sb) = &typed.strategy.body[0] {
                prop_assert_eq!(
                    &sb.variables[0].resolved_type,
                    &FluxType::VecFloat,
                    "All-numeric list literal (length {}) should infer VecFloat, got {:?}",
                    list_len,
                    sb.variables[0].resolved_type
                );
            } else {
                prop_assert!(false, "Expected StateBlock as first strategy item");
            }
        }
    }

    // ========================================================================
    // Feature: portfolio-construction, Property 3: Non-Numeric List Element Produces Type Error
    // ========================================================================

    // **Validates: Requirements 1.6**
    //
    // For any list literal containing at least one element whose resolved type
    // is not Int or Float, the type checker SHALL report a type error indicating
    // the expected numeric type and the actual offending element type.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_portfolio_non_numeric_list_element_type_error(
            numeric_count in 1usize..=5,
            non_numeric_insert_pos in 0usize..6,
            non_numeric in arb_span().prop_flat_map(|s| arb_non_numeric_expr(s)),
            span in arb_span(),
        ) {
            // Build a list with numeric elements first, then insert a non-numeric
            let mut elements: Vec<Expr> = (0..numeric_count).map(|i| Expr {
                kind: ExprKind::FloatLiteral(1.0 + i as f64),
                span,
            }).collect();

            // Insert the non-numeric element at a valid position
            let insert_at = non_numeric_insert_pos.min(elements.len());
            let (non_numeric_expr, _non_numeric_ty) = non_numeric;
            elements.insert(insert_at, non_numeric_expr);

            // Place the list literal in the handler body as an assignment.
            // This ensures it goes through check_list_literal() which produces
            // the "list literal expected numeric element" error.
            let handler_stmt = Stmt::Assignment(Assignment {
                target: Expr {
                    kind: ExprKind::Ident("mixed".to_string()),
                    span,
                },
                value: Expr { kind: ExprKind::ListLiteral(elements), span },
                span,
            });

            let program = Program {
                imports: vec![],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![
                        StrategyItem::EventHandler(EventHandler {
                            event_name: "bar".to_string(),
                            body: vec![handler_stmt],
                            span,
                        }),
                    ],
                    span,
                },
                span,
            };

            let result = super::super::check(program);
            prop_assert!(
                result.is_err(),
                "List with non-numeric element among numerics should produce a type error"
            );

            let err = result.unwrap_err();
            match &err {
                CompileError::Type(msg) => {
                    prop_assert!(
                        msg.contains("list literal expected numeric element"),
                        "Error should contain 'list literal expected numeric element', got: {}", msg
                    );
                }
                other => {
                    prop_assert!(false, "Expected CompileError::Type, got: {:?}", other);
                }
            }
        }
    }

    // ========================================================================
    // Feature: portfolio-construction, Property 5: Non-Int VecFloat Index Produces Type Error
    // ========================================================================

    // **Validates: Requirements 2.5**
    //
    // For any VecFloat value and any index expression whose type is not Int
    // (Float, String, Bool), the type checker SHALL report a type error
    // indicating the index must be Int.

    /// Generate a non-Int index expression (Float, String, or Bool literal).
    fn arb_non_int_index_expr(span: Span) -> impl Strategy<Value = Expr> {
        prop_oneof![
            // Float index
            (1u32..999, 1u32..99).prop_map(move |(i, d)| {
                let f: f64 = format!("{}.{}", i, d).parse().unwrap();
                Expr {
                    kind: ExprKind::FloatLiteral(f),
                    span,
                }
            }),
            // String index
            "[a-z]{1,8}".prop_map(move |s| Expr {
                kind: ExprKind::StringLiteral(s),
                span,
            }),
            // Bool index
            any::<bool>().prop_map(move |b| Expr {
                kind: ExprKind::BoolLiteral(b),
                span,
            }),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_portfolio_vecfloat_non_int_index(
            num_elements in 1usize..=5,
            float_vals in prop::collection::vec(1u32..999u32, 1..=5),
            non_int_index in arb_span().prop_flat_map(|s| arb_non_int_index_expr(s)),
            span in arb_span(),
        ) {
            // Build a VecFloat literal with numeric elements
            let elements: Vec<Expr> = float_vals.iter().take(num_elements).map(|v| Expr {
                kind: ExprKind::FloatLiteral(*v as f64),
                span,
            }).collect();

            // Create a state variable initialized to a VecFloat
            let state_vars = vec![StateVar {
                name: "weights".to_string(),
                initial_value: Expr { kind: ExprKind::ListLiteral(elements), span },
                span,
            }];

            // Index the VecFloat with a non-Int expression
            let index_expr = Expr {
                kind: ExprKind::IndexAccess {
                    object: Box::new(Expr { kind: ExprKind::Ident("weights".to_string()), span }),
                    index: Box::new(non_int_index),
                },
                span,
            };

            let stmt = Stmt::Assignment(Assignment {
                target: Expr { kind: ExprKind::Ident("val".to_string()), span },
                value: index_expr,
                span,
            });

            let program = Program {
                imports: vec![],
                        data_block: None,
                strategy: AstStrategy {
                    name: "T".to_string(),
                    body: vec![
                        StrategyItem::StateBlock(StateBlock { variables: state_vars, span }),
                        StrategyItem::EventHandler(EventHandler {
                            event_name: "bar".to_string(),
                            body: vec![stmt],
                            span,
                        }),
                    ],
                    span,
                },
                span,
            };

            let result = super::super::check(program);
            prop_assert!(result.is_err(), "Non-Int VecFloat index should produce a type error");

            let err = result.unwrap_err();
            match &err {
                CompileError::Type(msg) => {
                    prop_assert!(
                        msg.contains("VecFloat index must be Int"),
                        "Error should mention 'VecFloat index must be Int', got: {}", msg
                    );
                }
                other => {
                    prop_assert!(false, "Expected CompileError::Type, got: {:?}", other);
                }
            }
        }
    }
}
