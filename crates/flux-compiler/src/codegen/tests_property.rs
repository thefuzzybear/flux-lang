//! Property-based tests for the Flux code generator.
//!
//! Uses proptest to verify correctness properties of the code generator across
//! randomly generated typed AST inputs.

#[cfg(test)]
mod tests {
    use crate::codegen::generate;
    use crate::error::CompileError;
    use crate::lexer::Span;
    use crate::parser::ast::{BinOp, Import};
    use crate::typeck::typed_ast::*;
    use crate::typeck::types::{FluxType, FnParams};
    use proptest::prelude::*;

    // ========================================================================
    // Constants
    // ========================================================================

    /// Market data identifiers that resolve to `ctx.name`.
    const MARKET_DATA: &[&str] = &[
        "close", "open", "high", "low", "volume", "symbol", "in_position",
    ];

    // ========================================================================
    // Generators
    // ========================================================================

    /// Generate a valid FluxType (excluding Fn) suitable for struct field types.
    /// Includes leaf types and recursive List types up to depth 2.
    fn arb_field_type() -> impl Strategy<Value = FluxType> {
        let leaf = prop_oneof![
            Just(FluxType::Int),
            Just(FluxType::Float),
            Just(FluxType::String),
            Just(FluxType::Bool),
            Just(FluxType::Signal),
            Just(FluxType::Null),
            Just(FluxType::Void),
        ];
        leaf.prop_recursive(2, 8, 4, |inner| {
            inner.prop_map(|t| FluxType::List(Box::new(t)))
        })
    }

    /// Generate a TypedExpr consistent with a given FluxType.
    /// Produces literal expressions whose kind matches the resolved_type.
    fn arb_typed_expr(
        ty: FluxType,
    ) -> impl Strategy<Value = crate::typeck::typed_ast::TypedExpr> {
        use crate::lexer::Span;
        use crate::typeck::typed_ast::{TypedExpr, TypedExprKind};

        let span = Span::new(0, 1);
        match ty.clone() {
            FluxType::Int => (0i64..10000)
                .prop_map(move |v| TypedExpr {
                    kind: TypedExprKind::IntLiteral(v),
                    resolved_type: FluxType::Int,
                    span,
                })
                .boxed(),
            FluxType::Float => (0u32..999, 1u32..99)
                .prop_map(move |(i, d)| {
                    let f: f64 = format!("{}.{}", i, d).parse().unwrap();
                    TypedExpr {
                        kind: TypedExprKind::FloatLiteral(f),
                        resolved_type: FluxType::Float,
                        span,
                    }
                })
                .boxed(),
            FluxType::String => "[a-z]{1,8}"
                .prop_map(move |s| TypedExpr {
                    kind: TypedExprKind::StringLiteral(s),
                    resolved_type: FluxType::String,
                    span,
                })
                .boxed(),
            FluxType::Bool => any::<bool>()
                .prop_map(move |b| TypedExpr {
                    kind: TypedExprKind::BoolLiteral(b),
                    resolved_type: FluxType::Bool,
                    span,
                })
                .boxed(),
            FluxType::Null | FluxType::Void => Just(TypedExpr {
                kind: TypedExprKind::NullLiteral,
                resolved_type: ty,
                span,
            })
            .boxed(),
            FluxType::Signal => Just(TypedExpr {
                kind: TypedExprKind::NullLiteral,
                resolved_type: FluxType::Signal,
                span,
            })
            .boxed(),
            FluxType::List(_) => Just(TypedExpr {
                kind: TypedExprKind::ListLiteral(vec![]),
                resolved_type: ty,
                span,
            })
            .boxed(),
            FluxType::Fn { .. } => unreachable!("Generator excludes Fn"),
        }
    }

    /// Generate an arithmetic binary operator (Add, Sub, Mul, Div, Mod).
    /// These are the operators that can produce numeric coercion scenarios.
    fn arb_arithmetic_op() -> impl Strategy<Value = BinOp> {
        prop_oneof![
            Just(BinOp::Add),
            Just(BinOp::Sub),
            Just(BinOp::Mul),
            Just(BinOp::Div),
            Just(BinOp::Mod),
        ]
    }

    // ========================================================================
    // Property 1: Type Mapping Correctness
    // ========================================================================

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        // Feature: flux-codegen, Property 1: Type Mapping Correctness
        /// **Validates: Requirements 2.1, 2.2, 2.3, 2.4, 2.5, 2.6, 2.7, 2.8**
        #[test]
        fn prop_type_mapping_correctness(ty in arb_field_type()) {
            let result = super::super::type_map::map_type(&ty, 0);
            prop_assert!(result.is_ok(), "map_type should succeed for non-Fn type {:?}", ty);
            let rust_type = result.unwrap();
            prop_assert!(!rust_type.is_empty(), "Mapped type string should not be empty");

            // Verify specific mappings
            match &ty {
                FluxType::Int => prop_assert_eq!(&rust_type, "i64"),
                FluxType::Float => prop_assert_eq!(&rust_type, "f64"),
                FluxType::String => prop_assert_eq!(&rust_type, "String"),
                FluxType::Bool => prop_assert_eq!(&rust_type, "bool"),
                FluxType::Signal => prop_assert_eq!(&rust_type, "Signal"),
                FluxType::Null => prop_assert_eq!(&rust_type, "()"),
                FluxType::Void => prop_assert_eq!(&rust_type, "()"),
                FluxType::List(_) => {
                    prop_assert!(rust_type.starts_with("Vec<"), "List type should start with Vec<, got: {}", rust_type);
                    prop_assert!(rust_type.ends_with('>'), "List type should end with >, got: {}", rust_type);
                }
                FluxType::Fn { .. } => unreachable!("Generator excludes Fn"),
            }
        }
    }

    // ========================================================================
    // Property 2: Identifier Context Resolution
    // ========================================================================

    /// Generate a valid identifier name that avoids market data names and Rust keywords.
    fn arb_safe_ident() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9_]{2,6}".prop_filter("must not be market data or keyword", |n| {
            !MARKET_DATA.contains(&n.as_str())
                && !matches!(
                    n.as_str(),
                    "as" | "break" | "const" | "continue" | "crate" | "else" | "enum"
                        | "extern" | "false" | "fn" | "for" | "if" | "impl" | "in"
                        | "let" | "loop" | "match" | "mod" | "move" | "mut" | "pub"
                        | "ref" | "return" | "self" | "static" | "struct" | "super"
                        | "trait" | "true" | "type" | "unsafe" | "use" | "where"
                        | "while" | "async" | "await" | "dyn" | "result"
                )
        })
    }

    /// Helper: create a TypedExpr with given kind and type at span (0,1).
    fn typed_expr(kind: TypedExprKind, resolved_type: FluxType) -> TypedExpr {
        TypedExpr {
            kind,
            resolved_type,
            span: Span::new(0, 1),
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        // Feature: flux-codegen, Property 2: Identifier Context Resolution
        /// **Validates: Requirements 7.1, 7.2, 7.3, 7.4**
        #[test]
        fn prop_identifier_context_resolution(
            param_name in arb_safe_ident(),
            state_name in arb_safe_ident(),
        ) {
            prop_assume!(param_name != state_name);

            let program = TypedProgram {
                imports: vec![Import {
                    module_path: "indicators".to_string(),
                    names: vec!["sma".to_string()],
                    span: Span::new(0, 20),
                }],
                strategy: TypedStrategy {
                    name: "TestStrategy".to_string(),
                    body: vec![
                        TypedStrategyItem::ParamsBlock(TypedParamsBlock {
                            params: vec![TypedParam {
                                name: param_name.clone(),
                                default_value: typed_expr(
                                    TypedExprKind::IntLiteral(10),
                                    FluxType::Int,
                                ),
                                resolved_type: FluxType::Int,
                                span: Span::new(30, 40),
                            }],
                            span: Span::new(28, 45),
                        }),
                        TypedStrategyItem::StateBlock(TypedStateBlock {
                            variables: vec![TypedStateVar {
                                name: state_name.clone(),
                                initial_value: typed_expr(
                                    TypedExprKind::IntLiteral(0),
                                    FluxType::Int,
                                ),
                                resolved_type: FluxType::Int,
                                span: Span::new(50, 60),
                            }],
                            span: Span::new(48, 65),
                        }),
                        TypedStrategyItem::EventHandler(TypedEventHandler {
                            event_name: "bar".to_string(),
                            body: vec![
                                // Reference param
                                TypedStmt::Expr(TypedExprStmt {
                                    expr: typed_expr(
                                        TypedExprKind::Ident(param_name.clone()),
                                        FluxType::Int,
                                    ),
                                    span: Span::new(70, 80),
                                }),
                                // Reference state
                                TypedStmt::Expr(TypedExprStmt {
                                    expr: typed_expr(
                                        TypedExprKind::Ident(state_name.clone()),
                                        FluxType::Int,
                                    ),
                                    span: Span::new(85, 95),
                                }),
                                // Reference market data (close)
                                TypedStmt::Expr(TypedExprStmt {
                                    expr: typed_expr(
                                        TypedExprKind::Ident("close".to_string()),
                                        FluxType::Float,
                                    ),
                                    span: Span::new(100, 110),
                                }),
                                // Call imported function (bare name)
                                TypedStmt::Expr(TypedExprStmt {
                                    expr: typed_expr(
                                        TypedExprKind::FunctionCall {
                                            function: Box::new(typed_expr(
                                                TypedExprKind::Ident("sma".to_string()),
                                                FluxType::Fn {
                                                    params: crate::typeck::types::FnParams::VariadicNumeric,
                                                    ret: Box::new(FluxType::Float),
                                                },
                                            )),
                                            args: vec![typed_expr(
                                                TypedExprKind::Ident("close".to_string()),
                                                FluxType::Float,
                                            )],
                                        },
                                        FluxType::Float,
                                    ),
                                    span: Span::new(115, 130),
                                }),
                            ],
                            span: Span::new(68, 135),
                        }),
                    ],
                    span: Span::new(22, 140),
                },
                span: Span::new(0, 140),
            };

            let result = generate(&program);
            prop_assert!(result.is_ok(), "generate() failed: {:?}", result.err());
            let output = result.unwrap();

            // Verify: market data "close" → "ctx.close"
            prop_assert!(
                output.contains("ctx.close"),
                "Market data 'close' should be 'ctx.close', got:\n{}", output
            );

            // Verify: param → "self.{param_name}"
            let self_param = format!("self.{}", param_name);
            prop_assert!(
                output.contains(&self_param),
                "Param '{}' should be '{}', got:\n{}", param_name, self_param, output
            );

            // Verify: state → "self.{state_name}"
            let self_state = format!("self.{}", state_name);
            prop_assert!(
                output.contains(&self_state),
                "State '{}' should be '{}', got:\n{}", state_name, self_state, output
            );

            // Verify: imported function "sma" appears as bare name in call
            prop_assert!(
                output.contains("sma(ctx.close)"),
                "Imported function 'sma' should be bare in call, got:\n{}", output
            );
        }
    }

    // ========================================================================
    // Property 4: Numeric Coercion Correctness
    // ========================================================================

    /// Build a minimal TypedProgram with an on_bar handler containing a single
    /// assignment: `result = left op right`, where the binary expression uses
    /// the given operands and operator.
    fn build_binary_expr_program(
        left: crate::typeck::typed_ast::TypedExpr,
        op: BinOp,
        right: crate::typeck::typed_ast::TypedExpr,
        result_type: FluxType,
    ) -> crate::typeck::typed_ast::TypedProgram {
        use crate::lexer::Span;
        use crate::typeck::typed_ast::*;

        let binary_expr = TypedExpr {
            kind: TypedExprKind::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            },
            resolved_type: result_type.clone(),
            span: Span::new(10, 20),
        };

        let assignment = TypedStmt::Assignment(TypedAssignment {
            target: TypedExpr {
                kind: TypedExprKind::Ident("result".to_string()),
                resolved_type: result_type,
                span: Span::new(5, 11),
            },
            value: binary_expr,
            span: Span::new(5, 20),
        });

        TypedProgram {
            imports: vec![],
            strategy: TypedStrategy {
                name: "CoercionTest".to_string(),
                body: vec![TypedStrategyItem::EventHandler(TypedEventHandler {
                    event_name: "bar".to_string(),
                    body: vec![assignment],
                    span: Span::new(0, 30),
                })],
                span: Span::new(0, 30),
            },
            span: Span::new(0, 30),
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        // Feature: flux-codegen, Property 4: Numeric Coercion Correctness
        /// **Validates: Requirements 8.14, 16.1, 16.2, 16.3**
        #[test]
        fn prop_numeric_coercion_correctness(
            int_val in 0i64..1000,
            float_val in 0.1f64..100.0,
            op in arb_arithmetic_op(),
            mixed in any::<bool>(),
        ) {
            use crate::lexer::Span;
            use crate::typeck::typed_ast::{TypedExpr, TypedExprKind};

            let span = Span::new(0, 1);

            if mixed {
                // Sub-property 1: Mixed-type case (Int + Float)
                // When one operand is Int and the other is Float, the output
                // MUST contain "as f64" to cast the Int operand.
                let left = TypedExpr {
                    kind: TypedExprKind::IntLiteral(int_val),
                    resolved_type: FluxType::Int,
                    span,
                };
                let right = TypedExpr {
                    kind: TypedExprKind::FloatLiteral(float_val),
                    resolved_type: FluxType::Float,
                    span,
                };

                let program = build_binary_expr_program(left, op, right, FluxType::Float);
                let result = super::super::generate(&program);
                prop_assert!(result.is_ok(), "generate() should succeed, got: {:?}", result.err());
                let output = result.unwrap();
                prop_assert!(
                    output.contains("as f64"),
                    "Mixed-type binary op (Int {:?} Float) should contain 'as f64', got:\n{}",
                    op,
                    output
                );
            } else {
                // Sub-property 2: Same-type case (Int + Int or Float + Float)
                // When both operands have the same numeric type, the output
                // MUST NOT contain "as f64".

                // Alternate between Int+Int and Float+Float based on int_val parity
                if int_val % 2 == 0 {
                    // Int + Int case
                    let left = TypedExpr {
                        kind: TypedExprKind::IntLiteral(int_val),
                        resolved_type: FluxType::Int,
                        span,
                    };
                    let right = TypedExpr {
                        kind: TypedExprKind::IntLiteral(int_val + 1),
                        resolved_type: FluxType::Int,
                        span,
                    };

                    let program = build_binary_expr_program(left, op, right, FluxType::Int);
                    let result = super::super::generate(&program);
                    prop_assert!(result.is_ok(), "generate() should succeed, got: {:?}", result.err());
                    let output = result.unwrap();
                    prop_assert!(
                        !output.contains("as f64"),
                        "Same-type binary op (Int {:?} Int) should NOT contain 'as f64', got:\n{}",
                        op,
                        output
                    );
                } else {
                    // Float + Float case
                    let left = TypedExpr {
                        kind: TypedExprKind::FloatLiteral(float_val),
                        resolved_type: FluxType::Float,
                        span,
                    };
                    let right = TypedExpr {
                        kind: TypedExprKind::FloatLiteral(float_val + 1.0),
                        resolved_type: FluxType::Float,
                        span,
                    };

                    let program = build_binary_expr_program(left, op, right, FluxType::Float);
                    let result = super::super::generate(&program);
                    prop_assert!(result.is_ok(), "generate() should succeed, got: {:?}", result.err());
                    let output = result.unwrap();
                    prop_assert!(
                        !output.contains("as f64"),
                        "Same-type binary op (Float {:?} Float) should NOT contain 'as f64', got:\n{}",
                        op,
                        output
                    );
                }
            }
        }
    }

    // ========================================================================
    // Property 3: Binary Operator Symbol Mapping
    // ========================================================================

    /// Generator for all BinOp variants.
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

    /// Map BinOp to expected Rust operator string.
    fn expected_operator(op: BinOp) -> &'static str {
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
            BinOp::And => "&&",
            BinOp::Or => "||",
        }
    }

    /// Build a minimal TypedProgram with an on_bar handler containing a binary
    /// expression statement, using appropriate operand types for the operator.
    fn build_binop_program(
        op: BinOp,
        left: crate::typeck::typed_ast::TypedExpr,
        right: crate::typeck::typed_ast::TypedExpr,
    ) -> crate::typeck::typed_ast::TypedProgram {
        use crate::lexer::Span;
        use crate::typeck::typed_ast::*;

        let result_type = match op {
            BinOp::And | BinOp::Or | BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le
            | BinOp::Gt | BinOp::Ge => FluxType::Bool,
            _ => left.resolved_type.clone(),
        };

        let binop_expr = TypedExpr {
            kind: TypedExprKind::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            },
            resolved_type: result_type,
            span: Span::new(100, 120),
        };

        TypedProgram {
            imports: vec![],
            strategy: TypedStrategy {
                name: "BinOpTest".to_string(),
                body: vec![TypedStrategyItem::EventHandler(TypedEventHandler {
                    event_name: "bar".to_string(),
                    body: vec![TypedStmt::Expr(TypedExprStmt {
                        expr: binop_expr,
                        span: Span::new(100, 121),
                    })],
                    span: Span::new(90, 130),
                })],
                span: Span::new(0, 140),
            },
            span: Span::new(0, 140),
        }
    }

    // ========================================================================
    // Property 5: Signal Collection Integrity
    // ========================================================================

    /// Build a TypedProgram with an on_bar handler containing `num_signals`
    /// OPEN calls (each produces Signal type) as expression statements.
    fn build_signal_handler_program(num_signals: usize) -> TypedProgram {
        let mut body: Vec<TypedStmt> = Vec::new();
        for i in 0..num_signals {
            let open_call = TypedExpr {
                kind: TypedExprKind::FunctionCall {
                    function: Box::new(TypedExpr {
                        kind: TypedExprKind::Ident("OPEN".to_string()),
                        resolved_type: FluxType::Fn {
                            params: FnParams::Fixed(vec![FluxType::String, FluxType::Int]),
                            ret: Box::new(FluxType::Signal),
                        },
                        span: Span::new(100 + i * 20, 104 + i * 20),
                    }),
                    args: vec![
                        TypedExpr {
                            kind: TypedExprKind::Ident("symbol".to_string()),
                            resolved_type: FluxType::String,
                            span: Span::new(105 + i * 20, 111 + i * 20),
                        },
                        TypedExpr {
                            kind: TypedExprKind::IntLiteral(100),
                            resolved_type: FluxType::Int,
                            span: Span::new(113 + i * 20, 116 + i * 20),
                        },
                    ],
                },
                resolved_type: FluxType::Signal,
                span: Span::new(100 + i * 20, 117 + i * 20),
            };
            body.push(TypedStmt::Expr(TypedExprStmt {
                expr: open_call,
                span: Span::new(100 + i * 20, 118 + i * 20),
            }));
        }

        TypedProgram {
            imports: vec![],
            strategy: TypedStrategy {
                name: "SignalTest".to_string(),
                body: vec![TypedStrategyItem::EventHandler(TypedEventHandler {
                    event_name: "bar".to_string(),
                    body,
                    span: Span::new(50, 200),
                })],
                span: Span::new(0, 210),
            },
            span: Span::new(0, 210),
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        // Feature: flux-codegen, Property 5: Signal Collection Integrity
        /// **Validates: Requirements 14.1, 15.1, 15.2, 15.3, 15.4**
        #[test]
        fn prop_signal_collection_integrity(num_signals in 0usize..5) {
            let program = build_signal_handler_program(num_signals);
            let result = generate(&program);
            prop_assert!(result.is_ok(), "generate() failed: {:?}", result.err());
            let output = result.unwrap();

            // Verify signal declaration is present
            prop_assert!(
                output.contains("let mut signals: Vec<Signal> = Vec::new();"),
                "Output must contain signal declaration, got:\n{}", output
            );

            // Verify signals is the final return expression (line before closing `}` of method)
            prop_assert!(
                output.contains("        signals\n"),
                "Output must contain 'signals' as final return expression, got:\n{}", output
            );

            // Verify signals.push( appears exactly num_signals times
            let push_count = output.matches("signals.push(").count();
            prop_assert_eq!(
                push_count, num_signals,
                "Expected {} signals.push( occurrences, found {} in:\n{}",
                num_signals, push_count, output
            );
        }
    }

    // ========================================================================
    // Property 6: Generated Code Structural Invariant
    // ========================================================================

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        // Feature: flux-codegen, Property 6: Generated Code Structural Invariant
        /// **Validates: Requirements 3.1, 3.3, 4.1, 4.6, 5.1, 17.3**
        #[test]
        fn prop_generated_code_structure(name in "[A-Z][a-zA-Z]{2,10}") {
            // Build a minimal TypedProgram with strategy named `name`
            let program = TypedProgram {
                imports: vec![],
                strategy: TypedStrategy {
                    name: name.clone(),
                    body: vec![
                        TypedStrategyItem::ParamsBlock(TypedParamsBlock {
                            params: vec![TypedParam {
                                name: "n".to_string(),
                                default_value: typed_expr(TypedExprKind::IntLiteral(5), FluxType::Int),
                                resolved_type: FluxType::Int,
                                span: Span::new(10, 15),
                            }],
                            span: Span::new(8, 20),
                        }),
                        TypedStrategyItem::EventHandler(TypedEventHandler {
                            event_name: "bar".to_string(),
                            body: vec![],
                            span: Span::new(25, 40),
                        }),
                    ],
                    span: Span::new(0, 45),
                },
                span: Span::new(0, 45),
            };

            let result = generate(&program);
            prop_assert!(result.is_ok(), "generate() failed: {:?}", result.err());
            let output = result.unwrap();

            // Verify all four sections exist
            let preamble = "use flux_runtime::*;";
            let struct_decl = format!("pub struct {} {{", name);
            let default_impl = format!("impl Default for {} {{", name);
            let strategy_impl = format!("impl Strategy for {} {{", name);

            prop_assert!(output.contains(preamble), "Missing preamble in:\n{}", output);
            prop_assert!(output.contains(&struct_decl), "Missing struct in:\n{}", output);
            prop_assert!(output.contains(&default_impl), "Missing Default impl in:\n{}", output);
            prop_assert!(output.contains(&strategy_impl), "Missing Strategy impl in:\n{}", output);

            // Verify ordering
            let pos_preamble = output.find(preamble).unwrap();
            let pos_struct = output.find(&struct_decl).unwrap();
            let pos_default = output.find(&default_impl).unwrap();
            let pos_strategy = output.find(&strategy_impl).unwrap();

            prop_assert!(pos_preamble < pos_struct,
                "Preamble must come before struct: {} < {}", pos_preamble, pos_struct);
            prop_assert!(pos_struct < pos_default,
                "Struct must come before Default: {} < {}", pos_struct, pos_default);
            prop_assert!(pos_default < pos_strategy,
                "Default must come before Strategy: {} < {}", pos_default, pos_strategy);

            // Verify blank lines separate sections
            // After preamble there's a blank line before struct
            let between_preamble_struct = &output[pos_preamble + preamble.len()..pos_struct];
            prop_assert!(between_preamble_struct.contains("\n\n"),
                "Expected blank line between preamble and struct, got: {:?}", between_preamble_struct);

            // Between struct and Default there's a blank line
            // Find the closing `}\n` of the struct by searching forward from pos_struct
            let struct_body = &output[pos_struct..pos_default];
            prop_assert!(struct_body.contains("}\n\n"),
                "Expected blank line between struct and Default impl, got: {:?}", struct_body);

            // Between Default and Strategy there's a blank line
            let default_body = &output[pos_default..pos_strategy];
            prop_assert!(default_body.contains("}\n\n"),
                "Expected blank line between Default and Strategy impl, got: {:?}", default_body);
        }
    }

    // ========================================================================
    // Property 7: String Concatenation Uses format!
    // ========================================================================

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        // Feature: flux-codegen, Property 7: String Concatenation Uses format!
        /// **Validates: Requirements 20.1, 20.2**
        #[test]
        fn prop_string_concat_uses_format(
            left_str in "[a-z]{1,5}",
            right_str in "[a-z]{1,5}",
        ) {
            // Build a program with Add on two String-typed expressions
            let left = TypedExpr {
                kind: TypedExprKind::StringLiteral(left_str.clone()),
                resolved_type: FluxType::String,
                span: Span::new(10, 20),
            };
            let right = TypedExpr {
                kind: TypedExprKind::StringLiteral(right_str.clone()),
                resolved_type: FluxType::String,
                span: Span::new(25, 35),
            };

            let binary_expr = TypedExpr {
                kind: TypedExprKind::BinaryOp {
                    left: Box::new(left),
                    op: BinOp::Add,
                    right: Box::new(right),
                },
                resolved_type: FluxType::String,
                span: Span::new(10, 35),
            };

            let program = TypedProgram {
                imports: vec![],
                strategy: TypedStrategy {
                    name: "ConcatTest".to_string(),
                    body: vec![TypedStrategyItem::EventHandler(TypedEventHandler {
                        event_name: "bar".to_string(),
                        body: vec![TypedStmt::Expr(TypedExprStmt {
                            expr: binary_expr,
                            span: Span::new(10, 36),
                        })],
                        span: Span::new(5, 40),
                    })],
                    span: Span::new(0, 45),
                },
                span: Span::new(0, 45),
            };

            let result = generate(&program);
            prop_assert!(result.is_ok(), "generate() failed: {:?}", result.err());
            let output = result.unwrap();

            // Verify output contains format! macro for string concat
            prop_assert!(
                output.contains("format!(\"{}{}\","),
                "String concatenation should use format! macro, got:\n{}", output
            );

            // Verify output does NOT contain the `+` operator for this expression
            // Since format! is used, there should be no standalone `+` between two String::from calls
            prop_assert!(
                !output.contains(&format!("String::from(\"{}\") + String::from(\"{}\")", left_str, right_str)),
                "String concat should NOT use + operator, got:\n{}", output
            );
        }
    }

    // ========================================================================
    // Property 8: Error Reporting Includes Byte Offset
    // ========================================================================

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        // Feature: flux-codegen, Property 8: Error Reporting Includes Byte Offset
        /// **Validates: Requirements 1.3, 18.1, 18.3**
        #[test]
        fn prop_error_includes_byte_offset(offset in 0usize..10000) {
            // Build a program with a param whose type is FluxType::Fn
            let program = TypedProgram {
                imports: vec![],
                strategy: TypedStrategy {
                    name: "ErrTest".to_string(),
                    body: vec![TypedStrategyItem::ParamsBlock(TypedParamsBlock {
                        params: vec![TypedParam {
                            name: "callback".to_string(),
                            default_value: typed_expr(TypedExprKind::NullLiteral, FluxType::Null),
                            resolved_type: FluxType::Fn {
                                params: FnParams::Fixed(vec![FluxType::Int]),
                                ret: Box::new(FluxType::Float),
                            },
                            span: Span::new(offset, offset + 10),
                        }],
                        span: Span::new(0, offset + 20),
                    })],
                    span: Span::new(0, offset + 30),
                },
                span: Span::new(0, offset + 30),
            };

            let result = generate(&program);
            prop_assert!(result.is_err(), "Expected error for FluxType::Fn in field position");

            match result.unwrap_err() {
                CompileError::Codegen(msg) => {
                    let expected_prefix = format!("at byte {}:", offset);
                    prop_assert!(
                        msg.contains(&expected_prefix),
                        "Error should contain '{}', got: {}", expected_prefix, msg
                    );
                }
                other => {
                    prop_assert!(false, "Expected CompileError::Codegen, got: {:?}", other);
                }
            }
        }
    }

    // ========================================================================
    // Property 3: Binary Operator Symbol Mapping
    // ========================================================================

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        // Feature: flux-codegen, Property 3: Binary Operator Symbol Mapping
        /// **Validates: Requirements 8.1, 8.2, 8.3, 8.4, 8.5, 8.6, 8.7, 8.8, 8.9, 8.10, 8.11, 8.12, 8.13**
        #[test]
        fn prop_binary_operator_symbol(op in arb_binop()) {
            use crate::lexer::Span;
            use crate::typeck::typed_ast::{TypedExpr, TypedExprKind};

            let span_left = Span::new(101, 105);
            let span_right = Span::new(108, 113);

            // Determine appropriate operand types:
            // - And/Or use Bool operands
            // - All other ops use Int operands (same-type to avoid coercion)
            let (left, right) = if op == BinOp::And || op == BinOp::Or {
                (
                    TypedExpr {
                        kind: TypedExprKind::BoolLiteral(true),
                        resolved_type: FluxType::Bool,
                        span: span_left,
                    },
                    TypedExpr {
                        kind: TypedExprKind::BoolLiteral(false),
                        resolved_type: FluxType::Bool,
                        span: span_right,
                    },
                )
            } else {
                (
                    TypedExpr {
                        kind: TypedExprKind::IntLiteral(42),
                        resolved_type: FluxType::Int,
                        span: span_left,
                    },
                    TypedExpr {
                        kind: TypedExprKind::IntLiteral(7),
                        resolved_type: FluxType::Int,
                        span: span_right,
                    },
                )
            };

            let program = build_binop_program(op, left, right);
            let result = super::super::generate(&program);
            prop_assert!(result.is_ok(), "generate() failed for op {:?}: {:?}", op, result.err());
            let output = result.unwrap();

            let expected_op = expected_operator(op);

            // Verify the output contains the correct operator symbol wrapped in parens:
            // (left OP right)
            let expected_pattern = if op == BinOp::And || op == BinOp::Or {
                // For logical ops: (true && false) or (true || false)
                format!("(true {} false)", expected_op)
            } else {
                // For arithmetic/comparison ops: (42 OP 7)
                format!("(42 {} 7)", expected_op)
            };

            prop_assert!(
                output.contains(&expected_pattern),
                "Expected pattern '{}' not found in output for op {:?}.\nOutput:\n{}",
                expected_pattern, op, output
            );
        }
    }
}
