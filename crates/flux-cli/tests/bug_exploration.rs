//! Bug Condition Exploration Tests for cli-polish bugfix spec.
//!
//! These tests encode the EXPECTED correct behavior. They are designed to FAIL
//! on unfixed code, confirming the bugs exist. After the fixes are applied,
//! these same tests should PASS to confirm the bugs are resolved.
//!
//! **Validates: Requirements 1.1, 1.2, 1.3, 1.4, 1.5, 1.6**

use std::process::Command;

// =============================================================================
// Bug 1: Indicator State Collision
// =============================================================================

/// Bug 1 (Indicator Collision): When the interpreter evaluates two distinct
/// sma() calls with different periods, both should produce CORRECT values
/// matching independent computation. On unfixed code, both share the same
/// state buffer due to #[track_caller] resolving to the same Rust source line,
/// corrupting the results.
///
/// Strategy: sma(close, 3) at span (100, 120) and sma(close, 10) at span (200, 225)
/// Feed 15 bars of data. sma(close, 3) on bar 15 should equal the mean of the
/// last 3 close prices = mean([130, 140, 150]) = 140.0.
/// On unfixed code, the value is wrong because both sma calls share one state buffer.
///
/// **Validates: Requirements 1.1**
#[test]
fn bug1_indicator_state_collision_sma_different_periods() {
    use flux_compiler::lexer::Span;
    use flux_compiler::typeck::typed_ast::*;
    use flux_compiler::typeck::types::FluxType;
    use flux_runtime::BarContext;

    // Helper to build a TypedExpr
    fn texpr(kind: TypedExprKind, ty: FluxType, span: Span) -> TypedExpr {
        TypedExpr { kind, resolved_type: ty, span }
    }

    // Build the "sma(close, 3)" call at span (100, 120)
    let sma_fast_call = texpr(
        TypedExprKind::FunctionCall {
            function: Box::new(texpr(
                TypedExprKind::Ident("sma".to_string()),
                FluxType::Float,
                Span::new(100, 103),
            )),
            args: vec![
                texpr(TypedExprKind::Ident("close".to_string()), FluxType::Float, Span::new(104, 109)),
                texpr(TypedExprKind::IntLiteral(3), FluxType::Int, Span::new(111, 112)),
            ],
        },
        FluxType::Float,
        Span::new(100, 120),
    );

    // Build the "sma(close, 10)" call at span (200, 225)
    let sma_slow_call = texpr(
        TypedExprKind::FunctionCall {
            function: Box::new(texpr(
                TypedExprKind::Ident("sma".to_string()),
                FluxType::Float,
                Span::new(200, 203),
            )),
            args: vec![
                texpr(TypedExprKind::Ident("close".to_string()), FluxType::Float, Span::new(204, 209)),
                texpr(TypedExprKind::IntLiteral(10), FluxType::Int, Span::new(211, 213)),
            ],
        },
        FluxType::Float,
        Span::new(200, 225),
    );

    // Assign sma results to state variables: fast_val = sma(close, 3), slow_val = sma(close, 10)
    let assign_fast = TypedStmt::Assignment(TypedAssignment {
        target: texpr(TypedExprKind::Ident("fast_val".to_string()), FluxType::Float, Span::new(0, 8)),
        value: sma_fast_call,
        span: Span::new(0, 120),
    });

    let assign_slow = TypedStmt::Assignment(TypedAssignment {
        target: texpr(TypedExprKind::Ident("slow_val".to_string()), FluxType::Float, Span::new(0, 8)),
        value: sma_slow_call,
        span: Span::new(0, 225),
    });

    let program = TypedProgram {
        imports: vec![],
            structs: vec![],
            enums: vec![],
        functions: vec![],
        impl_blocks: vec![],
        data_block: None,
        connector_block: None,
        strategy: TypedStrategy {
            name: "DualSMA".to_string(),
            body: vec![
                TypedStrategyItem::StateBlock(TypedStateBlock {
                    variables: vec![
                        TypedStateVar {
                            name: "fast_val".to_string(),
                            initial_value: texpr(TypedExprKind::FloatLiteral(0.0), FluxType::Float, Span::new(0, 0)),
                            resolved_type: FluxType::Float,
                            span: Span::new(0, 0),
                        },
                        TypedStateVar {
                            name: "slow_val".to_string(),
                            initial_value: texpr(TypedExprKind::FloatLiteral(0.0), FluxType::Float, Span::new(0, 0)),
                            resolved_type: FluxType::Float,
                            span: Span::new(0, 0),
                        },
                    ],
                    span: Span::new(0, 0),
                }),
                TypedStrategyItem::EventHandler(TypedEventHandler {
                    event_name: "bar".to_string(),
                    body: vec![assign_fast, assign_slow],
                    span: Span::new(0, 0),
                }),
            ],
            span: Span::new(0, 0),
        },
        span: Span::new(0, 0),
    };

    let mut interp = flux_cli::interpreter::Interpreter::new(&program);

    // Feed 15 bars with increasing close prices [10, 20, 30, ..., 150]
    let close_prices: Vec<f64> = (1..=15).map(|i| i as f64 * 10.0).collect();

    for &price in &close_prices {
        let ctx = BarContext {
            close: price,
            open: price - 1.0,
            high: price + 1.0,
            low: price - 2.0,
            volume: 1000.0,
            symbol: "TEST".to_string(),
            in_position: false,
        };
        interp.on_bar(&ctx);
    }

    // Compute expected values independently:
    // sma(close, 3) on bar 15 = mean of last 3 prices = mean([130, 140, 150]) = 140.0
    let expected_fast: f64 = (130.0 + 140.0 + 150.0) / 3.0; // = 140.0

    let fast_val = match interp.state.get("fast_val") {
        Some(flux_cli::interpreter::Value::Float(f)) => *f,
        other => panic!("Expected fast_val to be Float, got {:?}", other),
    };

    // The fast SMA must match independent computation.
    // On unfixed code, it returns a wrong value because the sma(close, 10) call
    // interleaves pushes into the same period-3 buffer, corrupting the result.
    assert!(
        (fast_val - expected_fast).abs() < 1e-10,
        "Bug 1 Counterexample: sma(close, 3) on bar 15 = {}, expected {}. \
         On unfixed code, both sma() calls share a single state buffer (keyed by same \
         Rust call-site via #[track_caller]), so interleaved pushes corrupt the rolling average. \
         The second sma(close, 10) call pushes into the period-3 buffer, producing garbage.",
        fast_val, expected_fast
    );
}

// =============================================================================
// Bug 2 & 3: Help/Version Exit Codes
// =============================================================================

/// Bug 2 (Help Exit Code): `flux --help` should exit with code 0.
/// On unfixed code, it exits with code 2 because the Err branch of try_parse()
/// unconditionally calls process::exit(USAGE_ERROR).
///
/// **Validates: Requirements 1.2**
#[test]
fn bug2_help_flag_exits_with_code_0() {
    let output = Command::new(env!("CARGO_BIN_EXE_flux"))
        .arg("--help")
        .output()
        .expect("failed to execute flux --help");

    assert_eq!(
        output.status.code(),
        Some(0),
        "Bug 2 Counterexample: `flux --help` exited with code {:?}, expected 0. \
         On unfixed code, exits 2 because try_parse Err branch always calls process::exit(USAGE_ERROR)",
        output.status.code()
    );
}

/// Bug 3 (Version Exit Code): `flux --version` should exit with code 0.
/// On unfixed code, it exits with code 2 because the Err branch of try_parse()
/// unconditionally calls process::exit(USAGE_ERROR).
///
/// **Validates: Requirements 1.3**
#[test]
fn bug3_version_flag_exits_with_code_0() {
    let output = Command::new(env!("CARGO_BIN_EXE_flux"))
        .arg("--version")
        .output()
        .expect("failed to execute flux --version");

    assert_eq!(
        output.status.code(),
        Some(0),
        "Bug 3 Counterexample: `flux --version` exited with code {:?}, expected 0. \
         On unfixed code, exits 2 because try_parse Err branch always calls process::exit(USAGE_ERROR)",
        output.status.code()
    );
}

// =============================================================================
// Bug 4: Compile Pipeline Stub
// =============================================================================

/// Bug 4 (Compile Pipeline): `flux_compiler::compile()` with valid source should
/// return `Ok(...)`. On unfixed code, it returns `Err(CompileError::NotImplemented)`
/// because the function body is a TODO stub.
///
/// **Validates: Requirements 1.4**
#[test]
fn bug4_compile_pipeline_returns_ok_for_valid_source() {
    let source = "strategy S { on bar {} }";
    let result = flux_compiler::compile(source);

    assert!(
        result.is_ok(),
        "Bug 4 Counterexample: compile(\"{}\") returned {:?}, expected Ok(...). \
         On unfixed code, returns Err(NotImplemented) because the compile() body is a stub.",
        source,
        result.err()
    );
}

// =============================================================================
// Bug 5 & 6: Position Tracking
// =============================================================================

/// Bug 5 & 6 (Position Tracking): A strategy with `if not in_position { OPEN(symbol, 100) }`
/// should only emit 1 Open signal across 3 bars. After the first Open on bar 1,
/// `in_position` should become true, preventing Opens on bars 2 and 3.
///
/// On unfixed code, `in_position` is never updated (always reads from BarContext which
/// is always false from the CSV loader), so Open is emitted on EVERY bar.
///
/// **Validates: Requirements 1.5, 1.6**
#[test]
fn bug5_6_position_tracking_prevents_duplicate_opens() {
    use flux_compiler::lexer::Span;
    use flux_compiler::typeck::typed_ast::*;
    use flux_compiler::typeck::types::FluxType;
    use flux_runtime::BarContext;

    // Helper to build a TypedExpr
    fn texpr(kind: TypedExprKind, ty: FluxType, span: Span) -> TypedExpr {
        TypedExpr { kind, resolved_type: ty, span }
    }

    // Build: if not in_position { OPEN(symbol, 100.0) }
    let open_call = texpr(
        TypedExprKind::FunctionCall {
            function: Box::new(texpr(
                TypedExprKind::Ident("OPEN".to_string()),
                FluxType::Signal,
                Span::new(50, 54),
            )),
            args: vec![
                texpr(TypedExprKind::Ident("symbol".to_string()), FluxType::String, Span::new(55, 61)),
                texpr(TypedExprKind::FloatLiteral(100.0), FluxType::Float, Span::new(63, 68)),
            ],
        },
        FluxType::Signal,
        Span::new(50, 69),
    );

    let not_in_position = texpr(
        TypedExprKind::UnaryOp {
            op: flux_compiler::parser::ast::UnaryOp::Not,
            operand: Box::new(texpr(
                TypedExprKind::Ident("in_position".to_string()),
                FluxType::Bool,
                Span::new(10, 21),
            )),
        },
        FluxType::Bool,
        Span::new(6, 21),
    );

    let if_stmt = TypedStmt::If(TypedIfStmt {
        condition: not_in_position,
        body: vec![TypedStmt::Expr(TypedExprStmt {
            expr: open_call,
            span: Span::new(50, 69),
        })],
        elif_branches: vec![],
        else_body: None,
        span: Span::new(0, 70),
    });

    let program = TypedProgram {
        imports: vec![],
            structs: vec![],
            enums: vec![],
        functions: vec![],
        impl_blocks: vec![],
        data_block: None,
        connector_block: None,
        strategy: TypedStrategy {
            name: "PositionGuard".to_string(),
            body: vec![
                TypedStrategyItem::EventHandler(TypedEventHandler {
                    event_name: "bar".to_string(),
                    body: vec![if_stmt],
                    span: Span::new(0, 0),
                }),
            ],
            span: Span::new(0, 0),
        },
        span: Span::new(0, 0),
    };

    let mut interp = flux_cli::interpreter::Interpreter::new(&program);

    // Feed 3 bars
    let mut all_signals = Vec::new();
    for i in 0..3 {
        let ctx = BarContext {
            close: 100.0 + i as f64,
            open: 99.0 + i as f64,
            high: 101.0 + i as f64,
            low: 98.0 + i as f64,
            volume: 1000.0,
            symbol: "TEST".to_string(),
            in_position: false, // CSV loader always sets this to false
        };
        let signals = interp.on_bar(&ctx);
        all_signals.extend(signals);
    }

    // Count Open signals
    let open_count = all_signals.iter().filter(|s| matches!(s, flux_runtime::Signal::Open { .. })).count();

    assert_eq!(
        open_count, 1,
        "Bug 5&6 Counterexample: Expected 1 Open signal across 3 bars (position guard), \
         but got {}. On unfixed code, in_position is never updated after emitting Open, \
         so the guard `if not in_position` always passes and Open is emitted on every bar.",
        open_count
    );
}
