//! Type checker for the Flux language.
//!
//! Performs semantic analysis on the parsed AST: resolving identifiers,
//! validating type compatibility, and producing a typed AST.

pub mod types;
pub mod typed_ast;
mod env;
mod checker;
mod builtins;

#[cfg(test)]
mod tests_property;

pub use types::FluxType;
pub use typed_ast::*;

use crate::error::Result;
use crate::parser::Program;

/// Type-check a parsed Program AST.
///
/// This is the main entry point for the type checker. It validates types,
/// resolves identifiers, and produces a `TypedProgram` with type annotations
/// on every expression node.
///
/// # Arguments
///
/// * `program` - A parsed `Program` AST from the parser
///
/// # Returns
///
/// A `TypedProgram` on success, or a `CompileError::Type` on semantic error.
///
/// # Errors
///
/// Returns `CompileError::Type` when the program contains semantic errors.
pub fn check(program: Program) -> Result<TypedProgram> {
    let mut tc = checker::TypeChecker::new();
    tc.check_program(program)
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::error::CompileError;
    use crate::lexer::Span;
    use crate::parser::ast::*;

    /// Helper to create an Expr with a given kind and span
    fn expr(kind: ExprKind, start: usize, end: usize) -> Expr {
        Expr { kind, span: Span::new(start, end) }
    }

    /// Build a complete Program AST representing:
    /// ```text
    /// from indicators import {sma, ema}
    ///
    /// strategy TestStrategy {
    ///     params {
    ///         period = 20
    ///         threshold = 2.0
    ///     }
    ///     state {
    ///         prices = [0.0]
    ///         count = 0
    ///     }
    ///     on_bar {
    ///         prices.append(close)
    ///         count = count + 1
    ///         if close > sma(close, period) {
    ///             OPEN(symbol, 100)
    ///         }
    ///     }
    /// }
    /// ```
    fn build_complete_program() -> Program {
        // Import: from indicators import {sma, ema}
        let import = Import {
            module_path: "indicators".to_string(),
            names: vec!["sma".to_string(), "ema".to_string()],
            span: Span::new(0, 35),
        };

        // Params block
        let params_block = ParamsBlock {
            params: vec![
                Param {
                    name: "period".to_string(),
                    default_value: expr(ExprKind::IntLiteral(20), 70, 72),
                    span: Span::new(60, 72),
                },
                Param {
                    name: "threshold".to_string(),
                    default_value: expr(ExprKind::FloatLiteral(2.0), 90, 93),
                    span: Span::new(80, 93),
                },
            ],
            span: Span::new(50, 100),
        };

        // State block: prices = [""] (List(String)), count = 0 (Int)
        let state_block = StateBlock {
            variables: vec![
                StateVar {
                    name: "prices".to_string(),
                    initial_value: Expr {
                        kind: ExprKind::ListLiteral(vec![
                            expr(ExprKind::StringLiteral("".to_string()), 120, 122),
                        ]),
                        span: Span::new(119, 123),
                    },
                    span: Span::new(110, 123),
                },
                StateVar {
                    name: "count".to_string(),
                    initial_value: expr(ExprKind::IntLiteral(0), 140, 141),
                    span: Span::new(130, 141),
                },
            ],
            span: Span::new(105, 145),
        };

        // Event handler body:
        // Statement 1: prices.append(symbol)
        let stmt1 = Stmt::Expr(ExprStmt {
            expr: Expr {
                kind: ExprKind::MethodCall {
                    receiver: Box::new(expr(ExprKind::Ident("prices".to_string()), 160, 166)),
                    method: "append".to_string(),
                    args: vec![expr(ExprKind::Ident("symbol".to_string()), 174, 180)],
                },
                span: Span::new(160, 181),
            },
            span: Span::new(160, 181),
        });

        // Statement 2: count = count + 1
        let stmt2 = Stmt::Assignment(Assignment {
            target: expr(ExprKind::Ident("count".to_string()), 190, 195),
            value: Expr {
                kind: ExprKind::BinaryOp {
                    left: Box::new(expr(ExprKind::Ident("count".to_string()), 198, 203)),
                    op: BinOp::Add,
                    right: Box::new(expr(ExprKind::IntLiteral(1), 206, 207)),
                },
                span: Span::new(198, 207),
            },
            span: Span::new(190, 207),
        });

        // Statement 3: if close > sma(close, period) { OPEN(symbol, 100) }
        let sma_call = Expr {
            kind: ExprKind::FunctionCall {
                function: Box::new(expr(ExprKind::Ident("sma".to_string()), 225, 228)),
                args: vec![
                    expr(ExprKind::Ident("close".to_string()), 229, 234),
                    expr(ExprKind::Ident("period".to_string()), 236, 242),
                ],
            },
            span: Span::new(225, 243),
        };

        let condition = Expr {
            kind: ExprKind::BinaryOp {
                left: Box::new(expr(ExprKind::Ident("close".to_string()), 218, 223)),
                op: BinOp::Gt,
                right: Box::new(sma_call),
            },
            span: Span::new(218, 243),
        };

        let open_call = Stmt::Expr(ExprStmt {
            expr: Expr {
                kind: ExprKind::FunctionCall {
                    function: Box::new(expr(ExprKind::Ident("OPEN".to_string()), 260, 264)),
                    args: vec![
                        expr(ExprKind::Ident("symbol".to_string()), 265, 271),
                        expr(ExprKind::IntLiteral(100), 273, 276),
                    ],
                },
                span: Span::new(260, 277),
            },
            span: Span::new(260, 277),
        });

        let stmt3 = Stmt::If(IfStmt {
            condition,
            body: vec![open_call],
            elif_branches: vec![],
            else_body: None,
            span: Span::new(215, 285),
        });

        let event_handler = EventHandler {
            event_name: "bar".to_string(),
            body: vec![stmt1, stmt2, stmt3],
            span: Span::new(150, 290),
        };

        // Strategy
        let strategy = Strategy {
            name: "TestStrategy".to_string(),
            body: vec![
                StrategyItem::ParamsBlock(params_block),
                StrategyItem::StateBlock(state_block),
                StrategyItem::EventHandler(event_handler),
            ],
            span: Span::new(37, 295),
        };

        Program {
            imports: vec![import],
            strategy,
            span: Span::new(0, 295),
        }
    }

    #[test]
    fn test_end_to_end_check_returns_ok() {
        let program = build_complete_program();
        let result = check(program);
        assert!(result.is_ok(), "Expected Ok, got: {:?}", result.err());
    }

    #[test]
    fn test_end_to_end_imports_preserved() {
        let program = build_complete_program();
        let typed = check(program).unwrap();

        // Verify imports are preserved
        assert_eq!(typed.imports.len(), 1);
        assert_eq!(typed.imports[0].module_path, "indicators");
        assert_eq!(typed.imports[0].names, vec!["sma", "ema"]);
    }

    #[test]
    fn test_end_to_end_strategy_name() {
        let program = build_complete_program();
        let typed = check(program).unwrap();

        assert_eq!(typed.strategy.name, "TestStrategy");
    }

    #[test]
    fn test_end_to_end_strategy_has_params_state_handler() {
        let program = build_complete_program();
        let typed = check(program).unwrap();

        // Strategy body should have 3 items: params, state, event handler
        assert_eq!(typed.strategy.body.len(), 3);
        assert!(matches!(typed.strategy.body[0], TypedStrategyItem::ParamsBlock(_)));
        assert!(matches!(typed.strategy.body[1], TypedStrategyItem::StateBlock(_)));
        assert!(matches!(typed.strategy.body[2], TypedStrategyItem::EventHandler(_)));
    }

    #[test]
    fn test_end_to_end_params_types() {
        let program = build_complete_program();
        let typed = check(program).unwrap();

        if let TypedStrategyItem::ParamsBlock(ref pb) = typed.strategy.body[0] {
            assert_eq!(pb.params.len(), 2);
            assert_eq!(pb.params[0].name, "period");
            assert_eq!(pb.params[0].resolved_type, FluxType::Int);
            assert_eq!(pb.params[1].name, "threshold");
            assert_eq!(pb.params[1].resolved_type, FluxType::Float);
        } else {
            panic!("Expected ParamsBlock");
        }
    }

    #[test]
    fn test_end_to_end_state_types() {
        let program = build_complete_program();
        let typed = check(program).unwrap();

        if let TypedStrategyItem::StateBlock(ref sb) = typed.strategy.body[1] {
            assert_eq!(sb.variables.len(), 2);
            assert_eq!(sb.variables[0].name, "prices");
            assert_eq!(sb.variables[0].resolved_type, FluxType::List(Box::new(FluxType::String)));
            assert_eq!(sb.variables[1].name, "count");
            assert_eq!(sb.variables[1].resolved_type, FluxType::Int);
        } else {
            panic!("Expected StateBlock");
        }
    }

    #[test]
    fn test_end_to_end_event_handler_body() {
        let program = build_complete_program();
        let typed = check(program).unwrap();

        if let TypedStrategyItem::EventHandler(ref eh) = typed.strategy.body[2] {
            assert_eq!(eh.event_name, "bar");
            assert_eq!(eh.body.len(), 3);

            // Statement 1: prices.append(close) is an expression statement
            assert!(matches!(eh.body[0], TypedStmt::Expr(_)));

            // Statement 2: count = count + 1 is an assignment
            assert!(matches!(eh.body[1], TypedStmt::Assignment(_)));

            // Statement 3: if ... is an if statement
            assert!(matches!(eh.body[2], TypedStmt::If(_)));
        } else {
            panic!("Expected EventHandler");
        }
    }

    #[test]
    fn test_end_to_end_resolved_types_in_handler() {
        let program = build_complete_program();
        let typed = check(program).unwrap();

        if let TypedStrategyItem::EventHandler(ref eh) = typed.strategy.body[2] {
            // Check the assignment: count = count + 1
            if let TypedStmt::Assignment(ref assign) = eh.body[1] {
                // The value (count + 1) should resolve to Int
                assert_eq!(assign.value.resolved_type, FluxType::Int);
            } else {
                panic!("Expected Assignment");
            }

            // Check the if statement condition: close > sma(close, period)
            if let TypedStmt::If(ref if_stmt) = eh.body[2] {
                // Condition should resolve to Bool
                assert_eq!(if_stmt.condition.resolved_type, FluxType::Bool);

                // The OPEN call in the body should resolve to Signal
                assert_eq!(if_stmt.body.len(), 1);
                if let TypedStmt::Expr(ref expr_stmt) = if_stmt.body[0] {
                    assert_eq!(expr_stmt.expr.resolved_type, FluxType::Signal);
                } else {
                    panic!("Expected Expr statement in if body");
                }
            } else {
                panic!("Expected If statement");
            }
        } else {
            panic!("Expected EventHandler");
        }
    }

    #[test]
    fn test_end_to_end_spans_preserved() {
        let program = build_complete_program();
        let input_span = program.span;
        let strategy_span = program.strategy.span;

        let typed = check(program).unwrap();

        // Top-level span preserved
        assert_eq!(typed.span, input_span);
        // Strategy span preserved
        assert_eq!(typed.strategy.span, strategy_span);
    }

    // ===== Task 9.2: Error format and parse→check round-trip tests =====

    /// Build a program with a type error: `if 42 { ... }` (non-Bool condition).
    /// The integer literal at span (218, 220) acts as the condition.
    fn build_program_with_type_error() -> (Program, usize) {
        // A minimal program where the if-condition is an IntLiteral instead of Bool.
        // The condition expr span starts at byte 100.
        let condition_start = 100;

        let condition = Expr {
            kind: ExprKind::IntLiteral(42),
            span: Span::new(condition_start, condition_start + 2),
        };

        let open_call = Stmt::Expr(ExprStmt {
            expr: Expr {
                kind: ExprKind::FunctionCall {
                    function: Box::new(expr(ExprKind::Ident("OPEN".to_string()), 120, 124)),
                    args: vec![
                        expr(ExprKind::Ident("symbol".to_string()), 125, 131),
                        expr(ExprKind::IntLiteral(100), 133, 136),
                    ],
                },
                span: Span::new(120, 137),
            },
            span: Span::new(120, 137),
        });

        let if_stmt = Stmt::If(IfStmt {
            condition,
            body: vec![open_call],
            elif_branches: vec![],
            else_body: None,
            span: Span::new(97, 140),
        });

        let event_handler = EventHandler {
            event_name: "bar".to_string(),
            body: vec![if_stmt],
            span: Span::new(80, 145),
        };

        let strategy = Strategy {
            name: "Bad".to_string(),
            body: vec![StrategyItem::EventHandler(event_handler)],
            span: Span::new(0, 150),
        };

        let program = Program {
            imports: vec![],
            strategy,
            span: Span::new(0, 150),
        };

        (program, condition_start)
    }

    #[test]
    fn test_type_error_format() {
        // Validates: Requirements 1.3, 20.1, 20.4
        let (program, _) = build_program_with_type_error();
        let result = check(program);

        assert!(result.is_err(), "Expected type error, got Ok");
        let err = result.unwrap_err();

        // Must be a CompileError::Type variant
        match &err {
            CompileError::Type(msg) => {
                // Error message must start with "at byte N:"
                assert!(
                    msg.starts_with("at byte "),
                    "Expected error to start with 'at byte ', got: {msg}"
                );
                assert!(
                    msg.contains(':'),
                    "Expected ':' separator in error format, got: {msg}"
                );
            }
            other => panic!("Expected CompileError::Type, got: {other:?}"),
        }
    }

    #[test]
    fn test_type_error_includes_offset() {
        // Validates: Requirements 20.1
        // The byte offset in the error message should match the span.start of
        // the offending expression (the IntLiteral 42 used as if-condition).
        let (program, condition_start) = build_program_with_type_error();
        let result = check(program);

        let err = result.unwrap_err();
        match &err {
            CompileError::Type(msg) => {
                let expected_prefix = format!("at byte {}:", condition_start);
                assert!(
                    msg.starts_with(&expected_prefix),
                    "Expected error to start with '{}', got: {}",
                    expected_prefix,
                    msg
                );
            }
            other => panic!("Expected CompileError::Type, got: {other:?}"),
        }
    }

    #[test]
    fn test_type_error_mismatch_includes_types() {
        // Validates: Requirements 20.2
        // A type mismatch error (non-Bool condition) should mention both
        // the expected type (Bool) and the actual type found (Int).
        let (program, _) = build_program_with_type_error();
        let result = check(program);

        let err = result.unwrap_err();
        match &err {
            CompileError::Type(msg) => {
                assert!(
                    msg.contains("Bool") || msg.contains("bool"),
                    "Expected error to mention 'Bool', got: {msg}"
                );
                assert!(
                    msg.contains("Int") || msg.contains("int"),
                    "Expected error to mention 'Int', got: {msg}"
                );
            }
            other => panic!("Expected CompileError::Type, got: {other:?}"),
        }
    }

    #[test]
    fn test_parse_then_check_valid() {
        // Validates: Requirements 1.1, 1.2, 19.1–19.4
        // Full pipeline round-trip: lex → parse → check for a valid program.
        use crate::lexer::lex_with_spans;
        use crate::parser::parse;

        let source = r#"strategy Simple {
    on_bar {
        if close > open {
            OPEN(symbol, 100)
        }
    }
}"#;

        let tokens = lex_with_spans(source).expect("Lexing failed");
        let program = parse(tokens).expect("Parsing failed");
        let result = check(program);

        assert!(
            result.is_ok(),
            "Expected Ok(TypedProgram), got error: {:?}",
            result.err()
        );

        let typed = result.unwrap();
        assert_eq!(typed.strategy.name, "Simple");
        assert_eq!(typed.strategy.body.len(), 1);
        assert!(matches!(
            typed.strategy.body[0],
            TypedStrategyItem::EventHandler(_)
        ));
    }

    #[test]
    fn test_parse_then_check_type_error() {
        // Validates: Requirements 1.3, 20.1, 20.4
        // Full pipeline round-trip: lex → parse → check for a program with type error.
        use crate::lexer::lex_with_spans;
        use crate::parser::parse;

        let source = r#"strategy Bad {
    on_bar {
        if 42 {
            OPEN(symbol, 100)
        }
    }
}"#;

        let tokens = lex_with_spans(source).expect("Lexing failed");
        let program = parse(tokens).expect("Parsing failed");
        let result = check(program);

        assert!(result.is_err(), "Expected type error, got Ok");

        let err = result.unwrap_err();
        match &err {
            CompileError::Type(msg) => {
                // Should follow the "at byte N:" format
                assert!(
                    msg.starts_with("at byte "),
                    "Expected 'at byte ' prefix, got: {msg}"
                );
                // Should mention Bool (expected) and Int (actual)
                assert!(
                    msg.contains("Bool") || msg.contains("bool"),
                    "Expected error to mention 'Bool', got: {msg}"
                );
            }
            other => panic!("Expected CompileError::Type, got: {other:?}"),
        }
    }

    // ===== Task 6.3: Type checker registration tests for math/stats/portfolio functions =====

    /// Helper: lex → parse → check a Flux source string. Returns the check result.
    fn check_source(source: &str) -> crate::error::Result<TypedProgram> {
        use crate::lexer::lex_with_spans;
        use crate::parser::parse;

        let tokens = lex_with_spans(source).expect("Lexing failed");
        let program = parse(tokens).expect("Parsing failed");
        check(program)
    }

    #[test]
    fn test_tier1_math_functions_pass_type_checking() {
        // Validates: Requirements 3.4, 3.5
        // All Tier 1 math functions should be accepted without imports.
        let source = r#"strategy TestMath {
    on_bar {
        a = abs(close)
        b = sqrt(close)
        c = exp(close)
        d = log(close)
        e = floor(close)
        f = ceil(close)
        g = round(close)
        h = sign(close)
        i = pow(close, 2.0)
        j = min(close, open)
        k = max(close, open)
    }
}"#;

        let result = check_source(source);
        assert!(
            result.is_ok(),
            "Tier 1 math functions should pass type checking: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_tier2_stat_indicators_pass_type_checking() {
        // Validates: Requirements 9.6
        // All Tier 2 statistical functions should be accepted without imports.
        let source = r#"strategy TestStats {
    params {
        period = 20
    }
    on_bar {
        a = stddev(close, period)
        b = variance(close, period)
        c = zscore(close, period)
        d = rsi(close, period)
        e = corr(close, open, period)
        f = covariance(close, open, period)
        g = atr(high, low, close, period)
    }
}"#;

        let result = check_source(source);
        assert!(
            result.is_ok(),
            "Tier 2 stat indicators should pass type checking: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_tier1_functions_return_float_type() {
        // Validates: Requirements 3.1, 3.2, 3.3
        // Tier 1 math functions should resolve to Float return type.
        let source = r#"strategy TestMathTypes {
    on_bar {
        x = abs(close)
        if x > 0.0 {
            OPEN(symbol, 100.0)
        }
    }
}"#;

        let result = check_source(source);
        assert!(
            result.is_ok(),
            "Math function result used in comparison should pass: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_tier2_functions_return_float_type() {
        // Validates: Requirements 9.1, 9.2, 9.3, 9.4, 9.5
        // Tier 2 stat functions should resolve to Float return type.
        let source = r#"strategy TestStatTypes {
    params {
        period = 14
    }
    on_bar {
        r = rsi(close, period)
        if r > 70.0 {
            CLOSE(symbol)
        }
    }
}"#;

        let result = check_source(source);
        assert!(
            result.is_ok(),
            "Stat indicator result used in comparison should pass: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_tier3_fixed_param_functions_wrong_arg_count() {
        // Validates: Requirements 17.1, 17.2
        // Tier 3 functions with Fixed params should produce errors for wrong arg counts.
        // `det` expects 1 argument (MatFloat), calling with 2 should error.
        let source = r#"strategy TestWrongArgs {
    on_bar {
        x = det(close, open)
    }
}"#;

        let result = check_source(source);
        assert!(result.is_err(), "Wrong argument count should produce type error");

        let err = result.unwrap_err();
        match &err {
            CompileError::Type(msg) => {
                assert!(
                    msg.contains("det"),
                    "Error should mention the function name 'det', got: {msg}"
                );
                assert!(
                    msg.contains("expects") || msg.contains("argument"),
                    "Error should mention expected arguments, got: {msg}"
                );
            }
            other => panic!("Expected CompileError::Type, got: {other:?}"),
        }
    }

    #[test]
    fn test_tier3_transpose_wrong_arg_count() {
        // Validates: Requirements 17.1, 17.2
        // `transpose` expects 1 MatFloat argument, calling with 0 should error.
        let source = r#"strategy TestWrongArgs2 {
    on_bar {
        x = transpose()
    }
}"#;

        let result = check_source(source);
        assert!(result.is_err(), "Zero arguments for transpose should produce type error");

        let err = result.unwrap_err();
        match &err {
            CompileError::Type(msg) => {
                assert!(
                    msg.contains("transpose"),
                    "Error should mention 'transpose', got: {msg}"
                );
            }
            other => panic!("Expected CompileError::Type, got: {other:?}"),
        }
    }

    #[test]
    fn test_variadic_numeric_rejects_non_numeric_arg() {
        // Validates: Requirements 3.4, 17.1
        // Variadic numeric functions should reject non-numeric arguments (e.g., String).
        let source = r#"strategy TestBadArg {
    on_bar {
        x = abs("hello")
    }
}"#;

        let result = check_source(source);
        assert!(result.is_err(), "Non-numeric argument to abs should produce type error");

        let err = result.unwrap_err();
        match &err {
            CompileError::Type(msg) => {
                assert!(
                    msg.contains("abs"),
                    "Error should mention 'abs', got: {msg}"
                );
                assert!(
                    msg.contains("numeric"),
                    "Error should mention 'numeric', got: {msg}"
                );
            }
            other => panic!("Expected CompileError::Type, got: {other:?}"),
        }
    }

    #[test]
    fn test_math_functions_no_import_required() {
        // Validates: Requirements 3.5, 9.6
        // Math functions should work without any import statement.
        let source = r#"strategy NoImport {
    on_bar {
        x = sqrt(abs(close))
        y = max(x, 0.0)
    }
}"#;

        let result = check_source(source);
        assert!(
            result.is_ok(),
            "Math functions should work without imports: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_math_functions_composable() {
        // Validates: Requirements 3.1, 3.5
        // Math functions should be composable (nesting calls).
        let source = r#"strategy Composable {
    params {
        period = 20
    }
    on_bar {
        x = abs(sqrt(close))
        y = max(min(close, open), 0.0)
        z = round(stddev(close, period))
    }
}"#;

        let result = check_source(source);
        assert!(
            result.is_ok(),
            "Composed math functions should pass type checking: {:?}",
            result.err()
        );
    }
}
