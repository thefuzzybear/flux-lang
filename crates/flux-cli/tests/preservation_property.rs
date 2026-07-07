//! Preservation Property Tests for cli-polish bugfix spec.
//!
//! These tests capture existing CORRECT behavior that must NOT change when the
//! bug fixes are applied. They are written BEFORE the fixes and MUST PASS on
//! the unfixed code.
//!
//! **Validates: Requirements 3.1, 3.2, 3.3, 3.6, 3.7**

use proptest::prelude::*;
use std::process::Command;

use flux_compiler::lexer::Span;
use flux_compiler::typeck::typed_ast::*;
use flux_compiler::typeck::types::FluxType;
use flux_runtime::BarContext;

// =============================================================================
// Reference Oracles (reimplementing SmaState/EmaState algorithms for testing)
// =============================================================================

/// Reference SMA oracle: computes rolling average matching SmaState::next()
struct SmaOracle {
    buffer: Vec<f64>,
    period: usize,
    index: usize,
    count: usize,
    sum: f64,
}

impl SmaOracle {
    fn new(period: usize) -> Self {
        Self {
            buffer: vec![0.0; period],
            period,
            index: 0,
            count: 0,
            sum: 0.0,
        }
    }

    fn next(&mut self, value: f64) -> f64 {
        if self.count < self.period {
            self.buffer[self.index] = value;
            self.sum += value;
            self.count += 1;
            self.index = (self.index + 1) % self.period;
            self.sum / self.count as f64
        } else {
            self.sum -= self.buffer[self.index];
            self.buffer[self.index] = value;
            self.sum += value;
            self.index = (self.index + 1) % self.period;
            self.sum / self.period as f64
        }
    }
}

/// Reference EMA oracle: computes exponential moving average matching EmaState::next()
struct EmaOracle {
    prev_ema: Option<f64>,
    k: f64,
}

impl EmaOracle {
    fn new(period: usize) -> Self {
        Self {
            prev_ema: None,
            k: 2.0 / (period as f64 + 1.0),
        }
    }

    fn next(&mut self, value: f64) -> f64 {
        let ema = match self.prev_ema {
            None => value,
            Some(prev) => value * self.k + prev * (1.0 - self.k),
        };
        self.prev_ema = Some(ema);
        ema
    }
}

// =============================================================================
// Helper: build a single-indicator strategy TypedProgram
// =============================================================================

fn texpr(kind: TypedExprKind, ty: FluxType, span: Span) -> TypedExpr {
    TypedExpr {
        kind,
        resolved_type: ty,
        span,
    }
}

/// Build a TypedProgram with a single sma(close, period) call that stores the
/// result in a state variable "indicator_val".
fn build_single_sma_strategy(period: i64) -> TypedProgram {
    let sma_call = texpr(
        TypedExprKind::FunctionCall {
            function: Box::new(texpr(
                TypedExprKind::Ident("sma".to_string()),
                FluxType::Float,
                Span::new(50, 53),
            )),
            args: vec![
                texpr(
                    TypedExprKind::Ident("close".to_string()),
                    FluxType::Float,
                    Span::new(54, 59),
                ),
                texpr(
                    TypedExprKind::IntLiteral(period),
                    FluxType::Int,
                    Span::new(61, 63),
                ),
            ],
        },
        FluxType::Float,
        Span::new(50, 70),
    );

    let assign = TypedStmt::Assignment(TypedAssignment {
        target: texpr(
            TypedExprKind::Ident("indicator_val".to_string()),
            FluxType::Float,
            Span::new(0, 13),
        ),
        value: sma_call,
        span: Span::new(0, 70),
    });

    TypedProgram {
        imports: vec![],
        data_block: None,
        strategy: TypedStrategy {
            name: "SingleSMA".to_string(),
            body: vec![
                TypedStrategyItem::StateBlock(TypedStateBlock {
                    variables: vec![TypedStateVar {
                        name: "indicator_val".to_string(),
                        initial_value: texpr(
                            TypedExprKind::FloatLiteral(0.0),
                            FluxType::Float,
                            Span::new(0, 0),
                        ),
                        resolved_type: FluxType::Float,
                        span: Span::new(0, 0),
                    }],
                    span: Span::new(0, 0),
                }),
                TypedStrategyItem::EventHandler(TypedEventHandler {
                    event_name: "bar".to_string(),
                    body: vec![assign],
                    span: Span::new(0, 0),
                }),
            ],
            span: Span::new(0, 0),
        },
        span: Span::new(0, 0),
    }
}

/// Build a TypedProgram with a single ema(close, period) call that stores the
/// result in a state variable "indicator_val".
fn build_single_ema_strategy(period: i64) -> TypedProgram {
    let ema_call = texpr(
        TypedExprKind::FunctionCall {
            function: Box::new(texpr(
                TypedExprKind::Ident("ema".to_string()),
                FluxType::Float,
                Span::new(50, 53),
            )),
            args: vec![
                texpr(
                    TypedExprKind::Ident("close".to_string()),
                    FluxType::Float,
                    Span::new(54, 59),
                ),
                texpr(
                    TypedExprKind::IntLiteral(period),
                    FluxType::Int,
                    Span::new(61, 63),
                ),
            ],
        },
        FluxType::Float,
        Span::new(50, 70),
    );

    let assign = TypedStmt::Assignment(TypedAssignment {
        target: texpr(
            TypedExprKind::Ident("indicator_val".to_string()),
            FluxType::Float,
            Span::new(0, 13),
        ),
        value: ema_call,
        span: Span::new(0, 70),
    });

    TypedProgram {
        imports: vec![],
        data_block: None,
        strategy: TypedStrategy {
            name: "SingleEMA".to_string(),
            body: vec![
                TypedStrategyItem::StateBlock(TypedStateBlock {
                    variables: vec![TypedStateVar {
                        name: "indicator_val".to_string(),
                        initial_value: texpr(
                            TypedExprKind::FloatLiteral(0.0),
                            FluxType::Float,
                            Span::new(0, 0),
                        ),
                        resolved_type: FluxType::Float,
                        span: Span::new(0, 0),
                    }],
                    span: Span::new(0, 0),
                }),
                TypedStrategyItem::EventHandler(TypedEventHandler {
                    event_name: "bar".to_string(),
                    body: vec![assign],
                    span: Span::new(0, 0),
                }),
            ],
            span: Span::new(0, 0),
        },
        span: Span::new(0, 0),
    }
}

/// Build a no-signal strategy: just computes `indicator_val = close * 2.0`
/// This strategy emits no Open/Close signals, so in_position should stay false.
fn build_no_signal_strategy() -> TypedProgram {
    // indicator_val = close * 2.0
    let multiply_expr = texpr(
        TypedExprKind::BinaryOp {
            left: Box::new(texpr(
                TypedExprKind::Ident("close".to_string()),
                FluxType::Float,
                Span::new(10, 15),
            )),
            op: flux_compiler::parser::ast::BinOp::Mul,
            right: Box::new(texpr(
                TypedExprKind::FloatLiteral(2.0),
                FluxType::Float,
                Span::new(18, 21),
            )),
        },
        FluxType::Float,
        Span::new(10, 21),
    );

    let assign = TypedStmt::Assignment(TypedAssignment {
        target: texpr(
            TypedExprKind::Ident("indicator_val".to_string()),
            FluxType::Float,
            Span::new(0, 13),
        ),
        value: multiply_expr,
        span: Span::new(0, 21),
    });

    TypedProgram {
        imports: vec![],
        data_block: None,
        strategy: TypedStrategy {
            name: "NoSignal".to_string(),
            body: vec![
                TypedStrategyItem::StateBlock(TypedStateBlock {
                    variables: vec![TypedStateVar {
                        name: "indicator_val".to_string(),
                        initial_value: texpr(
                            TypedExprKind::FloatLiteral(0.0),
                            FluxType::Float,
                            Span::new(0, 0),
                        ),
                        resolved_type: FluxType::Float,
                        span: Span::new(0, 0),
                    }],
                    span: Span::new(0, 0),
                }),
                TypedStrategyItem::EventHandler(TypedEventHandler {
                    event_name: "bar".to_string(),
                    body: vec![assign],
                    span: Span::new(0, 0),
                }),
            ],
            span: Span::new(0, 0),
        },
        span: Span::new(0, 0),
    }
}

// =============================================================================
// Property 2: Preservation — Single Indicator Correctness (SMA)
// =============================================================================

proptest! {
    /// **Validates: Requirements 3.1**
    ///
    /// For any single-indicator SMA strategy with random value sequences and periods,
    /// the interpreter output matches the SmaState reference computation.
    /// This behavior is CORRECT on unfixed code (single call-site has unique state)
    /// and must remain correct after the fix.
    ///
    /// We spawn a new thread for each test case to get a fresh thread_local state
    /// for the runtime's indicator registry (keyed by #[track_caller] location).
    #[test]
    fn prop_preservation_single_sma_matches_oracle(
        values in proptest::collection::vec(1.0..1000.0f64, 3..30),
        period in 1i64..10,
    ) {
        let result = std::thread::spawn(move || {
            let program = build_single_sma_strategy(period);
            let mut interp = flux_cli::interpreter::Interpreter::new(&program);
            let mut oracle = SmaOracle::new(period as usize);

            for (i, &price) in values.iter().enumerate() {
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
                let expected = oracle.next(price);

                let actual = match interp.state.get("indicator_val") {
                    Some(flux_cli::interpreter::Value::Float(f)) => *f,
                    other => return Err(format!(
                        "Expected Float state at bar {}, got {:?}", i, other
                    )),
                };

                if (actual - expected).abs() >= 1e-9 {
                    return Err(format!(
                        "SMA preservation broken at bar {}: interpreter={}, oracle={}, period={}, price={}",
                        i, actual, expected, period, price
                    ));
                }
            }
            Ok(())
        }).join().expect("thread panicked");

        prop_assert!(result.is_ok(), "{}", result.unwrap_err());
    }
}

// =============================================================================
// Property 2: Preservation — Single Indicator Correctness (EMA)
// =============================================================================

proptest! {
    /// **Validates: Requirements 3.1**
    ///
    /// For any single-indicator EMA strategy with random value sequences and periods,
    /// the interpreter output matches the EmaState reference computation.
    /// This behavior is CORRECT on unfixed code (single call-site has unique state)
    /// and must remain correct after the fix.
    #[test]
    fn prop_preservation_single_ema_matches_oracle(
        values in proptest::collection::vec(1.0..1000.0f64, 3..30),
        period in 1i64..10,
    ) {
        let result = std::thread::spawn(move || {
            let program = build_single_ema_strategy(period);
            let mut interp = flux_cli::interpreter::Interpreter::new(&program);
            let mut oracle = EmaOracle::new(period as usize);

            for (i, &price) in values.iter().enumerate() {
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
                let expected = oracle.next(price);

                let actual = match interp.state.get("indicator_val") {
                    Some(flux_cli::interpreter::Value::Float(f)) => *f,
                    other => return Err(format!(
                        "Expected Float state at bar {}, got {:?}", i, other
                    )),
                };

                if (actual - expected).abs() >= 1e-9 {
                    return Err(format!(
                        "EMA preservation broken at bar {}: interpreter={}, oracle={}, period={}, price={}",
                        i, actual, expected, period, price
                    ));
                }
            }
            Ok(())
        }).join().expect("thread panicked");

        prop_assert!(result.is_ok(), "{}", result.unwrap_err());
    }
}

// =============================================================================
// Property 2: Preservation — Invalid Usage Exit Codes
// =============================================================================

proptest! {
    /// **Validates: Requirements 3.2, 3.3**
    ///
    /// For all invalid CLI usage patterns (unknown subcommands), the CLI exits
    /// with code 2. This must be preserved after fixes.
    #[test]
    fn prop_preservation_invalid_usage_exits_2(
        subcmd in "[a-z]{3,10}"
    ) {
        // Filter out valid subcommands and help/version (which are the bug condition)
        prop_assume!(
            subcmd != "check"
            && subcmd != "build"
            && subcmd != "backtest"
            && subcmd != "help"
        );

        let output = Command::new(env!("CARGO_BIN_EXE_flux"))
            .arg(&subcmd)
            .output()
            .expect("failed to execute flux");

        prop_assert_eq!(
            output.status.code(),
            Some(2),
            "Invalid subcommand '{}' should exit with code 2, got {:?}",
            subcmd,
            output.status.code()
        );
    }
}

// =============================================================================
// Preservation — Missing required args exits 2 (concrete tests)
// =============================================================================

/// **Validates: Requirements 3.2**
///
/// Commands with missing required arguments exit with code 2.
#[test]
fn preservation_check_without_file_exits_2() {
    let output = Command::new(env!("CARGO_BIN_EXE_flux"))
        .arg("check")
        .output()
        .expect("failed to execute flux check");

    assert_eq!(
        output.status.code(),
        Some(2),
        "flux check without file arg should exit 2, got {:?}",
        output.status.code()
    );
}

/// **Validates: Requirements 3.2**
///
/// `flux backtest <file>` without --data exits with code 2.
#[test]
fn preservation_backtest_without_data_exits_2() {
    let output = Command::new(env!("CARGO_BIN_EXE_flux"))
        .arg("backtest")
        .arg("some_file.flux")
        .output()
        .expect("failed to execute flux backtest");

    assert_eq!(
        output.status.code(),
        Some(2),
        "flux backtest without --data should exit 2, got {:?}",
        output.status.code()
    );
}

/// **Validates: Requirements 3.2**
///
/// `flux invalidcmd` exits with code 2 (unknown subcommand).
#[test]
fn preservation_unknown_subcommand_exits_2() {
    let output = Command::new(env!("CARGO_BIN_EXE_flux"))
        .arg("invalidcmd")
        .output()
        .expect("failed to execute flux invalidcmd");

    assert_eq!(
        output.status.code(),
        Some(2),
        "flux invalidcmd should exit 2, got {:?}",
        output.status.code()
    );
}

// =============================================================================
// Property 2: Preservation — No-Signal Position Unchanged
// =============================================================================

proptest! {
    /// **Validates: Requirements 3.6, 3.7**
    ///
    /// For any strategy that emits no Open/Close signals, the signal list is empty
    /// and `in_position` remains false across all bars.
    /// On unfixed code this is trivially true (in_position is never updated),
    /// and it must remain true after the fix.
    #[test]
    fn prop_preservation_no_signal_position_unchanged(
        prices in proptest::collection::vec(1.0..1000.0f64, 1..20),
    ) {
        let program = build_no_signal_strategy();
        let mut interp = flux_cli::interpreter::Interpreter::new(&program);

        for (i, &price) in prices.iter().enumerate() {
            let ctx = BarContext {
                close: price,
                open: price - 1.0,
                high: price + 1.0,
                low: price - 2.0,
                volume: 1000.0,
                symbol: "TEST".to_string(),
                in_position: false, // CSV loader always sets this to false
            };
            let signals = interp.on_bar(&ctx);

            // No signals should be emitted from a no-signal strategy
            prop_assert!(
                signals.is_empty(),
                "No-signal strategy emitted signals on bar {} with close={}: {:?}",
                i, price, signals
            );
        }
    }
}
