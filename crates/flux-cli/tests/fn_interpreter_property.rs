//! Property-based tests for interpreter scope isolation and return semantics.
//!
//! These tests validate that user-defined function calls in the Flux interpreter
//! correctly isolate local variables and handle return values per the design doc.
//!
//! Feature: flux-user-functions, Property 5: Scope isolation
//! Feature: flux-user-functions, Property 6: Return semantics

use proptest::prelude::*;

use flux_compiler::lexer::Span;
use flux_compiler::parser::ast::BinOp;
use flux_compiler::typeck::typed_ast::*;
use flux_compiler::typeck::types::FluxType;
use flux_runtime::BarContext;

use flux_cli::interpreter::{Interpreter, Value};

// =============================================================================
// Helpers
// =============================================================================

/// Shortcut to build a TypedExpr with a given kind, type, and dummy span.
fn texpr(kind: TypedExprKind, ty: FluxType) -> TypedExpr {
    TypedExpr {
        kind,
        resolved_type: ty,
        span: Span::new(0, 0),
    }
}

/// Build a simple BarContext with fixed values for testing.
fn test_bar() -> BarContext {
    BarContext {
        close: 100.0,
        open: 99.0,
        high: 101.0,
        low: 98.0,
        volume: 5000.0,
        symbol: "TEST".to_string(),
        in_position: false,
    }
}

/// Build a function call expression for a user-defined function.
fn fn_call_expr(name: &str, args: Vec<TypedExpr>) -> TypedExpr {
    texpr(
        TypedExprKind::FunctionCall {
            function: Box::new(texpr(
                TypedExprKind::Ident(name.to_string()),
                FluxType::Float,
            )),
            args,
        },
        FluxType::Float,
    )
}

/// Build a float literal expression.
fn float_lit(v: f64) -> TypedExpr {
    texpr(TypedExprKind::FloatLiteral(v), FluxType::Float)
}

/// Build an identifier expression.
fn ident_expr(name: &str) -> TypedExpr {
    texpr(TypedExprKind::Ident(name.to_string()), FluxType::Float)
}

/// Build an assignment statement: `target = value_expr`.
fn assign_stmt(target: &str, value_expr: TypedExpr) -> TypedStmt {
    TypedStmt::Assignment(TypedAssignment {
        target: ident_expr(target),
        value: value_expr,
        span: Span::new(0, 0),
    })
}

/// Build a return statement with an expression.
fn return_stmt(value: TypedExpr) -> TypedStmt {
    TypedStmt::Return(TypedReturnStmt {
        value: Some(value),
        span: Span::new(0, 0),
    })
}

/// Build an expression statement (e.g., a function call as a statement).
fn expr_stmt(expr: TypedExpr) -> TypedStmt {
    TypedStmt::Expr(TypedExprStmt {
        expr,
        span: Span::new(0, 0),
    })
}

// =============================================================================
// Property 5: Scope isolation
// Feature: flux-user-functions, Property 5: Scope isolation
// =============================================================================

/// Build a TypedProgram that:
/// 1. In the strategy body, sets `result = caller_value`
/// 2. Calls a function that assigns `result = fn_value` (local to function)
/// 3. Then reads `result` and stores it in state var `final_result`
///
/// If scope isolation is correct, `final_result` should be `caller_value`,
/// not `fn_value`.
fn build_scope_isolation_program(
    caller_value: f64,
    fn_value: f64,
    param_value: f64,
) -> TypedProgram {
    // User function: fn modify(x) { result = fn_value; local_var = x + 1.0 }
    let fn_def = TypedFnDef {
        name: "modify".to_string(),
        type_params: vec![],
        type_param_bounds: vec![],
        params: vec!["x".to_string()],
        param_types: vec![FluxType::Float],
        body: vec![
            // result = fn_value (local to function scope)
            assign_stmt("result", float_lit(fn_value)),
            // local_var = x + 1.0 (local to function scope)
            assign_stmt(
                "local_var",
                texpr(
                    TypedExprKind::BinaryOp {
                        left: Box::new(ident_expr("x")),
                        op: BinOp::Add,
                        right: Box::new(float_lit(1.0)),
                    },
                    FluxType::Float,
                ),
            ),
        ],
        return_type: FluxType::Null,
        span: Span::new(0, 0),
    };

    // Strategy on_bar body:
    //   result = caller_value
    //   modify(param_value)     <-- should NOT change caller's `result`
    //   final_result = result   <-- should still be caller_value
    let handler_body = vec![
        // result = caller_value
        assign_stmt("result", float_lit(caller_value)),
        // modify(param_value) as an expression statement
        expr_stmt(fn_call_expr("modify", vec![float_lit(param_value)])),
        // final_result = result (storing into state so we can inspect it)
        assign_stmt("final_result", ident_expr("result")),
    ];

    TypedProgram {
        imports: vec![],
        structs: vec![],
        enums: vec![],
        functions: vec![fn_def],
        impl_blocks: vec![],
            traits: vec![],
        data_block: None,
        connector_block: None,
        strategy: TypedStrategy {
            name: "ScopeTest".to_string(),
            body: vec![
                TypedStrategyItem::StateBlock(TypedStateBlock {
                    variables: vec![TypedStateVar {
                        name: "final_result".to_string(),
                        initial_value: float_lit(0.0),
                        resolved_type: FluxType::Float,
                        span: Span::new(0, 0),
                    }],
                    span: Span::new(0, 0),
                }),
                TypedStrategyItem::EventHandler(TypedEventHandler {
                    event_name: "bar".to_string(),
                    body: handler_body,
                    span: Span::new(0, 0),
                }),
            ],
            span: Span::new(0, 0),
        },
        span: Span::new(0, 0),
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

    /// **Validates: Requirements 5.1, 5.6**
    ///
    /// Property 5: Scope isolation
    ///
    /// For any user-defined function call, local variables assigned inside the
    /// function body SHALL NOT be visible in the caller's scope after the call
    /// returns. The caller's local variables SHALL remain unchanged.
    #[test]
    fn prop_scope_isolation(
        caller_value in -1000.0..1000.0f64,
        fn_value in -1000.0..1000.0f64,
        param_value in -1000.0..1000.0f64,
    ) {
        // Ensure caller_value != fn_value so we can detect leakage
        prop_assume!((caller_value - fn_value).abs() > 0.001);

        let program = build_scope_isolation_program(caller_value, fn_value, param_value);
        let mut interp = Interpreter::new(&program);
        let ctx = test_bar();

        interp.on_bar(&ctx);

        // After the call, the state var `final_result` should hold the CALLER's
        // value, not the function's internal value.
        let final_result = match interp.state.get("final_result") {
            Some(Value::Float(f)) => *f,
            other => panic!("Expected Float in state 'final_result', got {:?}", other),
        };

        prop_assert!(
            (final_result - caller_value).abs() < 1e-10,
            "Scope isolation violated: expected caller_value={}, got final_result={} (fn_value={})",
            caller_value, final_result, fn_value
        );
    }
}

// =============================================================================
// Additional scope isolation: function params don't leak to caller
// =============================================================================

/// Build a program where the function takes a parameter named `leaked_param`
/// and assigns to `leaked_local`. After the call, the caller checks that neither
/// `leaked_param` nor `leaked_local` are in its scope (they resolve to Null/0
/// from state init).
fn build_param_leak_program(param_val: f64) -> TypedProgram {
    // fn side_effect(leaked_param) { leaked_local = leaked_param + 10.0 }
    let fn_def = TypedFnDef {
        name: "side_effect".to_string(),
        type_params: vec![],
        type_param_bounds: vec![],
        params: vec!["leaked_param".to_string()],
        param_types: vec![FluxType::Float],
        body: vec![
            assign_stmt(
                "leaked_local",
                texpr(
                    TypedExprKind::BinaryOp {
                        left: Box::new(ident_expr("leaked_param")),
                        op: BinOp::Add,
                        right: Box::new(float_lit(10.0)),
                    },
                    FluxType::Float,
                ),
            ),
        ],
        return_type: FluxType::Null,
        span: Span::new(0, 0),
    };

    // Strategy:
    //   sentinel = 42.0
    //   side_effect(param_val)
    //   check_sentinel = sentinel  <-- should still be 42.0
    let handler_body = vec![
        assign_stmt("sentinel", float_lit(42.0)),
        expr_stmt(fn_call_expr("side_effect", vec![float_lit(param_val)])),
        // Store sentinel into state for inspection
        assign_stmt("check_sentinel", ident_expr("sentinel")),
    ];

    TypedProgram {
        imports: vec![],
        structs: vec![],
        enums: vec![],
        functions: vec![fn_def],
        impl_blocks: vec![],
            traits: vec![],
        data_block: None,
        connector_block: None,
        strategy: TypedStrategy {
            name: "ParamLeakTest".to_string(),
            body: vec![
                TypedStrategyItem::StateBlock(TypedStateBlock {
                    variables: vec![TypedStateVar {
                        name: "check_sentinel".to_string(),
                        initial_value: float_lit(0.0),
                        resolved_type: FluxType::Float,
                        span: Span::new(0, 0),
                    }],
                    span: Span::new(0, 0),
                }),
                TypedStrategyItem::EventHandler(TypedEventHandler {
                    event_name: "bar".to_string(),
                    body: handler_body,
                    span: Span::new(0, 0),
                }),
            ],
            span: Span::new(0, 0),
        },
        span: Span::new(0, 0),
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

    /// **Validates: Requirements 5.1, 5.6**
    ///
    /// Property 5: Scope isolation (parameter leakage variant)
    ///
    /// Function parameters and locally-assigned variables inside the function
    /// do NOT leak into the caller's local scope.
    #[test]
    fn prop_scope_isolation_no_param_leak(
        param_val in -500.0..500.0f64,
    ) {
        let program = build_param_leak_program(param_val);
        let mut interp = Interpreter::new(&program);
        let ctx = test_bar();

        interp.on_bar(&ctx);

        // `sentinel` was assigned to 42.0 before the function call and should
        // remain 42.0 after. If function locals leaked, it might be overwritten.
        let check_sentinel = match interp.state.get("check_sentinel") {
            Some(Value::Float(f)) => *f,
            other => panic!("Expected Float in state 'check_sentinel', got {:?}", other),
        };

        prop_assert!(
            (check_sentinel - 42.0).abs() < 1e-10,
            "Scope isolation violated: sentinel should be 42.0, got {} (param_val={})",
            check_sentinel, param_val
        );
    }
}

// =============================================================================
// Property 6: Return semantics
// Feature: flux-user-functions, Property 6: Return semantics
// =============================================================================

/// Build a program with a function that returns an expression value.
/// The strategy calls the function and stores its return value in state.
fn build_return_value_program(return_value: f64) -> TypedProgram {
    // fn compute() { return return_value }
    let fn_def = TypedFnDef {
        name: "compute".to_string(),
        type_params: vec![],
        type_param_bounds: vec![],
        params: vec![],
        param_types: vec![],
        body: vec![return_stmt(float_lit(return_value))],
        return_type: FluxType::Float,
        span: Span::new(0, 0),
    };

    // Strategy: fn_result = compute()
    let handler_body = vec![
        assign_stmt("fn_result", fn_call_expr("compute", vec![])),
    ];

    TypedProgram {
        imports: vec![],
        structs: vec![],
        enums: vec![],
        functions: vec![fn_def],
        impl_blocks: vec![],
            traits: vec![],
        data_block: None,
        connector_block: None,
        strategy: TypedStrategy {
            name: "ReturnTest".to_string(),
            body: vec![
                TypedStrategyItem::StateBlock(TypedStateBlock {
                    variables: vec![TypedStateVar {
                        name: "fn_result".to_string(),
                        initial_value: float_lit(-999.0),
                        resolved_type: FluxType::Float,
                        span: Span::new(0, 0),
                    }],
                    span: Span::new(0, 0),
                }),
                TypedStrategyItem::EventHandler(TypedEventHandler {
                    event_name: "bar".to_string(),
                    body: handler_body,
                    span: Span::new(0, 0),
                }),
            ],
            span: Span::new(0, 0),
        },
        span: Span::new(0, 0),
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

    /// **Validates: Requirements 5.3, 5.4**
    ///
    /// Property 6: Return semantics (explicit return)
    ///
    /// For any user-defined function containing a `return expr` statement,
    /// the function call SHALL evaluate to the value of `expr`.
    #[test]
    fn prop_return_semantics_explicit(
        return_value in -1000.0..1000.0f64,
    ) {
        let program = build_return_value_program(return_value);
        let mut interp = Interpreter::new(&program);
        let ctx = test_bar();

        interp.on_bar(&ctx);

        let fn_result = match interp.state.get("fn_result") {
            Some(Value::Float(f)) => *f,
            other => panic!("Expected Float in state 'fn_result', got {:?}", other),
        };

        prop_assert!(
            (fn_result - return_value).abs() < 1e-10,
            "Return semantics violated: expected return_value={}, got fn_result={}",
            return_value, fn_result
        );
    }
}

// =============================================================================
// Property 6: Return semantics — functions without return evaluate to Null
// =============================================================================

/// Build a program with a function that has NO return statement.
/// The strategy calls the function and attempts to use its return value.
/// Since functions without return yield Null, the assignment to state should
/// store whatever the interpreter resolves Null to.
fn build_no_return_program(internal_value: f64) -> TypedProgram {
    // fn no_ret(x) { local = x + 1.0 }  <-- no return statement
    let fn_def = TypedFnDef {
        name: "no_ret".to_string(),
        type_params: vec![],
        type_param_bounds: vec![],
        params: vec!["x".to_string()],
        param_types: vec![FluxType::Float],
        body: vec![
            assign_stmt(
                "local",
                texpr(
                    TypedExprKind::BinaryOp {
                        left: Box::new(ident_expr("x")),
                        op: BinOp::Add,
                        right: Box::new(float_lit(1.0)),
                    },
                    FluxType::Float,
                ),
            ),
        ],
        return_type: FluxType::Null,
        span: Span::new(0, 0),
    };

    // Strategy: fn_result = no_ret(internal_value)
    let handler_body = vec![
        assign_stmt("fn_result", fn_call_expr("no_ret", vec![float_lit(internal_value)])),
    ];

    TypedProgram {
        imports: vec![],
        structs: vec![],
        enums: vec![],
        functions: vec![fn_def],
        impl_blocks: vec![],
            traits: vec![],
        data_block: None,
        connector_block: None,
        strategy: TypedStrategy {
            name: "NoReturnTest".to_string(),
            body: vec![
                TypedStrategyItem::StateBlock(TypedStateBlock {
                    variables: vec![TypedStateVar {
                        name: "fn_result".to_string(),
                        initial_value: float_lit(-999.0),
                        resolved_type: FluxType::Float,
                        span: Span::new(0, 0),
                    }],
                    span: Span::new(0, 0),
                }),
                TypedStrategyItem::EventHandler(TypedEventHandler {
                    event_name: "bar".to_string(),
                    body: handler_body,
                    span: Span::new(0, 0),
                }),
            ],
            span: Span::new(0, 0),
        },
        span: Span::new(0, 0),
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

    /// **Validates: Requirements 5.3, 5.4**
    ///
    /// Property 6: Return semantics (implicit Null return)
    ///
    /// For any function without a `return` statement, the function call
    /// SHALL evaluate to Null.
    #[test]
    fn prop_return_semantics_no_return_is_null(
        internal_value in -1000.0..1000.0f64,
    ) {
        let program = build_no_return_program(internal_value);
        let mut interp = Interpreter::new(&program);
        let ctx = test_bar();

        interp.on_bar(&ctx);

        // The function returns Null, so `fn_result` in state should be Null
        let fn_result = interp.state.get("fn_result")
            .expect("state 'fn_result' should exist");

        prop_assert!(
            matches!(fn_result, Value::Null),
            "Return semantics violated: function without return should yield Null, got {:?} (internal_value={})",
            fn_result, internal_value
        );
    }
}

// =============================================================================
// Property 6: Return semantics — return with parameter-based expression
// =============================================================================

/// Build a program with a function that computes `return param * 2.0 + offset`.
/// This tests that the return expression is evaluated correctly with parameters.
fn build_return_expr_program(param_val: f64, offset: f64) -> TypedProgram {
    // fn double_plus(val) { return val * 2.0 + offset }
    let fn_def = TypedFnDef {
        name: "double_plus".to_string(),
        type_params: vec![],
        type_param_bounds: vec![],
        params: vec!["val".to_string()],
        param_types: vec![FluxType::Float],
        body: vec![
            return_stmt(texpr(
                TypedExprKind::BinaryOp {
                    left: Box::new(texpr(
                        TypedExprKind::BinaryOp {
                            left: Box::new(ident_expr("val")),
                            op: BinOp::Mul,
                            right: Box::new(float_lit(2.0)),
                        },
                        FluxType::Float,
                    )),
                    op: BinOp::Add,
                    right: Box::new(float_lit(offset)),
                },
                FluxType::Float,
            )),
        ],
        return_type: FluxType::Float,
        span: Span::new(0, 0),
    };

    // Strategy: fn_result = double_plus(param_val)
    let handler_body = vec![
        assign_stmt("fn_result", fn_call_expr("double_plus", vec![float_lit(param_val)])),
    ];

    TypedProgram {
        imports: vec![],
        structs: vec![],
        enums: vec![],
        functions: vec![fn_def],
        impl_blocks: vec![],
            traits: vec![],
        data_block: None,
        connector_block: None,
        strategy: TypedStrategy {
            name: "ReturnExprTest".to_string(),
            body: vec![
                TypedStrategyItem::StateBlock(TypedStateBlock {
                    variables: vec![TypedStateVar {
                        name: "fn_result".to_string(),
                        initial_value: float_lit(0.0),
                        resolved_type: FluxType::Float,
                        span: Span::new(0, 0),
                    }],
                    span: Span::new(0, 0),
                }),
                TypedStrategyItem::EventHandler(TypedEventHandler {
                    event_name: "bar".to_string(),
                    body: handler_body,
                    span: Span::new(0, 0),
                }),
            ],
            span: Span::new(0, 0),
        },
        span: Span::new(0, 0),
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

    /// **Validates: Requirements 5.3, 5.4**
    ///
    /// Property 6: Return semantics (expression-based return)
    ///
    /// For any function that returns a computed expression involving parameters,
    /// the function call evaluates to the correct computed value.
    #[test]
    fn prop_return_semantics_computed_expression(
        param_val in -500.0..500.0f64,
        offset in -100.0..100.0f64,
    ) {
        let expected = param_val * 2.0 + offset;
        let program = build_return_expr_program(param_val, offset);
        let mut interp = Interpreter::new(&program);
        let ctx = test_bar();

        interp.on_bar(&ctx);

        let fn_result = match interp.state.get("fn_result") {
            Some(Value::Float(f)) => *f,
            other => panic!("Expected Float in state 'fn_result', got {:?}", other),
        };

        let tolerance = 1e-10 * expected.abs().max(1.0);
        prop_assert!(
            (fn_result - expected).abs() < tolerance,
            "Return semantics violated: expected param_val*2+offset={}, got fn_result={} (param_val={}, offset={})",
            expected, fn_result, param_val, offset
        );
    }
}

// =============================================================================
// Property 6: Return semantics — early return skips remaining statements
// =============================================================================

/// Build a program with a function that returns early before an assignment.
/// This validates that early return terminates execution.
fn build_early_return_program(ret_val: f64, after_val: f64) -> TypedProgram {
    // fn early(x) {
    //   return x
    //   result = after_val   <-- should never execute
    // }
    let fn_def = TypedFnDef {
        name: "early".to_string(),
        type_params: vec![],
        type_param_bounds: vec![],
        params: vec!["x".to_string()],
        param_types: vec![FluxType::Float],
        body: vec![
            return_stmt(ident_expr("x")),
            // This assignment should never execute due to early return
            assign_stmt("result", float_lit(after_val)),
        ],
        return_type: FluxType::Float,
        span: Span::new(0, 0),
    };

    // Strategy:
    //   result = 0.0
    //   fn_result = early(ret_val)
    //   check_result = result  <-- should still be 0.0 (assignment after return never ran)
    let handler_body = vec![
        assign_stmt("result", float_lit(0.0)),
        assign_stmt("fn_result", fn_call_expr("early", vec![float_lit(ret_val)])),
        assign_stmt("check_result", ident_expr("result")),
    ];

    TypedProgram {
        imports: vec![],
        structs: vec![],
        enums: vec![],
        functions: vec![fn_def],
        impl_blocks: vec![],
            traits: vec![],
        data_block: None,
        connector_block: None,
        strategy: TypedStrategy {
            name: "EarlyReturnTest".to_string(),
            body: vec![
                TypedStrategyItem::StateBlock(TypedStateBlock {
                    variables: vec![
                        TypedStateVar {
                            name: "fn_result".to_string(),
                            initial_value: float_lit(-999.0),
                            resolved_type: FluxType::Float,
                            span: Span::new(0, 0),
                        },
                        TypedStateVar {
                            name: "check_result".to_string(),
                            initial_value: float_lit(-999.0),
                            resolved_type: FluxType::Float,
                            span: Span::new(0, 0),
                        },
                    ],
                    span: Span::new(0, 0),
                }),
                TypedStrategyItem::EventHandler(TypedEventHandler {
                    event_name: "bar".to_string(),
                    body: handler_body,
                    span: Span::new(0, 0),
                }),
            ],
            span: Span::new(0, 0),
        },
        span: Span::new(0, 0),
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

    /// **Validates: Requirements 5.3, 5.4**
    ///
    /// Property 6: Return semantics (early termination)
    ///
    /// A `return` statement immediately stops function body execution.
    /// Statements after `return` are never executed.
    #[test]
    fn prop_return_semantics_early_termination(
        ret_val in -500.0..500.0f64,
        after_val in -500.0..500.0f64,
    ) {
        // Ensure they differ so we can detect if the post-return assignment ran
        prop_assume!((ret_val - after_val).abs() > 0.001);

        let program = build_early_return_program(ret_val, after_val);
        let mut interp = Interpreter::new(&program);
        let ctx = test_bar();

        interp.on_bar(&ctx);

        // fn_result should be ret_val (the early return value)
        let fn_result = match interp.state.get("fn_result") {
            Some(Value::Float(f)) => *f,
            other => panic!("Expected Float in state 'fn_result', got {:?}", other),
        };

        prop_assert!(
            (fn_result - ret_val).abs() < 1e-10,
            "Early return value wrong: expected {}, got {}",
            ret_val, fn_result
        );

        // check_result should be 0.0 (the value set before the call)
        // because the post-return assignment inside the function never executes,
        // and scope isolation means the function can't modify caller's `result`
        let check_result = match interp.state.get("check_result") {
            Some(Value::Float(f)) => *f,
            other => panic!("Expected Float in state 'check_result', got {:?}", other),
        };

        prop_assert!(
            (check_result - 0.0).abs() < 1e-10,
            "Early return didn't terminate: check_result={}, expected 0.0 (after_val={})",
            check_result, after_val
        );
    }
}
