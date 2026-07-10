//! Code generation module: transforms TypedProgram into Rust source code.

mod emitter;
pub(crate) mod fn_context;
mod type_map;

#[cfg(test)]
mod tests_property;

#[cfg(test)]
mod tests_type_system_property;

use crate::error::Result;
use crate::typeck::typed_ast::TypedProgram;

/// Generate Rust source code from a typed Flux AST.
///
/// # Arguments
/// * `program` - A fully type-checked `TypedProgram` AST
///
/// # Returns
/// A `String` containing valid Rust source code on success
///
/// # Errors
/// Returns `CompileError::Codegen` if the AST contains constructs
/// that cannot be represented in Rust (e.g., FluxType::Fn as a field type).
pub fn generate(program: &TypedProgram) -> Result<String> {
    let mut emitter = emitter::CodeEmitter::new(program);
    emitter.emit()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CompileError;
    use crate::lexer::Span;
    use crate::parser::ast::*;
    use crate::typeck::typed_ast::*;
    use crate::typeck::types::{FluxType, FnParams};

    /// Helper: create a typed expression with a given kind and type at span (0,1).
    fn typed_expr(kind: TypedExprKind, resolved_type: FluxType) -> TypedExpr {
        TypedExpr {
            kind,
            resolved_type,
            span: Span::new(0, 1),
        }
    }



    // Test 1: Complete TypedProgram with imports, params, state, event handler
    #[test]
    fn test_generate_complete_program() {
        // Build a TypedProgram with:
        // - import sma from indicators
        // - params: period = 20 (Int)
        // - state: count = 0 (Int)
        // - on_bar handler: count = count + 1; if close > sma(close, period) { OPEN(symbol, 100) }
        let program = TypedProgram {
            imports: vec![Import {
                module_path: "indicators".to_string(),
                names: vec!["sma".to_string()],
                span: Span::new(0, 30),
            }],
            structs: vec![],
            enums: vec![],
            functions: vec![],
            impl_blocks: vec![],
            traits: vec![],
            data_block: None,
            connector_block: None,
            strategy: TypedStrategy {
                name: "MomentumStrategy".to_string(),
                body: vec![
                    TypedStrategyItem::ParamsBlock(TypedParamsBlock {
                        params: vec![TypedParam {
                            name: "period".to_string(),
                            default_value: typed_expr(TypedExprKind::IntLiteral(20), FluxType::Int),
                            resolved_type: FluxType::Int,
                            span: Span::new(50, 62),
                        }],
                        span: Span::new(48, 65),
                    }),
                    TypedStrategyItem::StateBlock(TypedStateBlock {
                        variables: vec![TypedStateVar {
                            name: "count".to_string(),
                            initial_value: typed_expr(TypedExprKind::IntLiteral(0), FluxType::Int),
                            resolved_type: FluxType::Int,
                            span: Span::new(70, 81),
                        }],
                        span: Span::new(68, 85),
                    }),
                    TypedStrategyItem::EventHandler(TypedEventHandler {
                        event_name: "bar".to_string(),
                        body: vec![
                            // count = count + 1
                            TypedStmt::Assignment(TypedAssignment {
                                target: typed_expr(
                                    TypedExprKind::Ident("count".to_string()),
                                    FluxType::Int,
                                ),
                                value: typed_expr(
                                    TypedExprKind::BinaryOp {
                                        left: Box::new(typed_expr(
                                            TypedExprKind::Ident("count".to_string()),
                                            FluxType::Int,
                                        )),
                                        op: BinOp::Add,
                                        right: Box::new(typed_expr(
                                            TypedExprKind::IntLiteral(1),
                                            FluxType::Int,
                                        )),
                                    },
                                    FluxType::Int,
                                ),
                                span: Span::new(90, 110),
                            }),
                            // if close > sma(close, period) { OPEN(symbol, 100) }
                            TypedStmt::If(TypedIfStmt {
                                condition: typed_expr(
                                    TypedExprKind::BinaryOp {
                                        left: Box::new(typed_expr(
                                            TypedExprKind::Ident("close".to_string()),
                                            FluxType::Float,
                                        )),
                                        op: BinOp::Gt,
                                        right: Box::new(typed_expr(
                                            TypedExprKind::FunctionCall {
                                                function: Box::new(typed_expr(
                                                    TypedExprKind::Ident("sma".to_string()),
                                                    FluxType::Fn {
                                                        params: FnParams::VariadicNumeric,
                                                        ret: Box::new(FluxType::Float),
                                                    },
                                                )),
                                                args: vec![
                                                    typed_expr(
                                                        TypedExprKind::Ident("close".to_string()),
                                                        FluxType::Float,
                                                    ),
                                                    typed_expr(
                                                        TypedExprKind::Ident("period".to_string()),
                                                        FluxType::Int,
                                                    ),
                                                ],
                                            },
                                            FluxType::Float,
                                        )),
                                    },
                                    FluxType::Bool,
                                ),
                                body: vec![
                                    // OPEN(symbol, 100) as expression statement
                                    TypedStmt::Expr(TypedExprStmt {
                                        expr: typed_expr(
                                            TypedExprKind::FunctionCall {
                                                function: Box::new(typed_expr(
                                                    TypedExprKind::Ident("OPEN".to_string()),
                                                    FluxType::Fn {
                                                        params: FnParams::Fixed(vec![
                                                            FluxType::String,
                                                            FluxType::Int,
                                                        ]),
                                                        ret: Box::new(FluxType::Signal),
                                                    },
                                                )),
                                                args: vec![
                                                    typed_expr(
                                                        TypedExprKind::Ident("symbol".to_string()),
                                                        FluxType::String,
                                                    ),
                                                    typed_expr(
                                                        TypedExprKind::IntLiteral(100),
                                                        FluxType::Int,
                                                    ),
                                                ],
                                            },
                                            FluxType::Signal,
                                        ),
                                        span: Span::new(130, 150),
                                    }),
                                ],
                                elif_branches: vec![],
                                else_body: None,
                                span: Span::new(115, 155),
                            }),
                        ],
                        span: Span::new(88, 160),
                    }),
                ],
                span: Span::new(32, 165),
            },
            span: Span::new(0, 165),
        };

        let result = generate(&program);
        assert!(result.is_ok(), "generate() failed: {:?}", result.err());
        let output = result.unwrap();

        // Verify all expected sections are present
        assert!(output.contains("use flux_runtime::*;"), "Missing preamble");
        assert!(output.contains("pub struct MomentumStrategy"), "Missing struct");
        assert!(output.contains("pub period: i64"), "Missing period param field");
        assert!(output.contains("count: i64"), "Missing count state field");
        assert!(output.contains("impl Default for MomentumStrategy"), "Missing Default impl");
        assert!(output.contains("period: 20"), "Missing period default value");
        assert!(output.contains("count: 0"), "Missing count initial value");
        assert!(output.contains("impl Strategy for MomentumStrategy"), "Missing Strategy impl");
        assert!(output.contains("fn on_bar(&mut self, ctx: &BarContext) -> Vec<Signal>"), "Missing on_bar method");
        assert!(output.contains("let mut signals: Vec<Signal> = Vec::new();"), "Missing signal declaration");
        assert!(output.contains("self.count = (self.count + 1)"), "Missing count increment");
        assert!(output.contains("ctx.close"), "Missing ctx.close reference");
        assert!(output.contains("sma(ctx.close, self.period)"), "Missing sma call");
        assert!(output.contains("Signal::open(ctx.symbol, 100)"), "Missing OPEN signal");
        assert!(output.contains("signals.push("), "Missing signals.push");
        assert!(output.contains("signals\n"), "Missing signals return");
    }

    // Test 2: Structure ordering — preamble → struct → Default → Strategy
    #[test]
    fn test_generate_structure_ordering() {
        let program = TypedProgram {
            imports: vec![],
            structs: vec![],
            enums: vec![],
            functions: vec![],
            impl_blocks: vec![],
            traits: vec![],
            data_block: None,
            connector_block: None,
            strategy: TypedStrategy {
                name: "OrderTest".to_string(),
                body: vec![
                    TypedStrategyItem::ParamsBlock(TypedParamsBlock {
                        params: vec![TypedParam {
                            name: "n".to_string(),
                            default_value: typed_expr(TypedExprKind::IntLiteral(10), FluxType::Int),
                            resolved_type: FluxType::Int,
                            span: Span::new(10, 15),
                        }],
                        span: Span::new(8, 20),
                    }),
                    TypedStrategyItem::StateBlock(TypedStateBlock {
                        variables: vec![TypedStateVar {
                            name: "x".to_string(),
                            initial_value: typed_expr(TypedExprKind::FloatLiteral(0.0), FluxType::Float),
                            resolved_type: FluxType::Float,
                            span: Span::new(25, 30),
                        }],
                        span: Span::new(22, 35),
                    }),
                    TypedStrategyItem::EventHandler(TypedEventHandler {
                        event_name: "bar".to_string(),
                        body: vec![],
                        span: Span::new(40, 50),
                    }),
                ],
                span: Span::new(0, 55),
            },
            span: Span::new(0, 55),
        };

        let output = generate(&program).unwrap();

        // Find positions of key sections
        let preamble_pos = output.find("use flux_runtime::*;").expect("Missing preamble");
        let struct_pos = output.find("pub struct OrderTest").expect("Missing struct");
        let default_pos = output.find("impl Default for OrderTest").expect("Missing Default");
        let strategy_pos = output.find("impl Strategy for OrderTest").expect("Missing Strategy");

        // Verify ordering
        assert!(
            preamble_pos < struct_pos,
            "Preamble (pos {}) should come before struct (pos {})",
            preamble_pos, struct_pos
        );
        assert!(
            struct_pos < default_pos,
            "Struct (pos {}) should come before Default impl (pos {})",
            struct_pos, default_pos
        );
        assert!(
            default_pos < strategy_pos,
            "Default impl (pos {}) should come before Strategy impl (pos {})",
            default_pos, strategy_pos
        );
    }

    // Test 3: Error case with FluxType::Fn in a field position
    #[test]
    fn test_generate_fn_type_error() {
        let program = TypedProgram {
            imports: vec![],
            structs: vec![],
            enums: vec![],
            functions: vec![],
            impl_blocks: vec![],
            traits: vec![],
            data_block: None,
            connector_block: None,
            strategy: TypedStrategy {
                name: "BadStrategy".to_string(),
                body: vec![
                    TypedStrategyItem::ParamsBlock(TypedParamsBlock {
                        params: vec![TypedParam {
                            name: "callback".to_string(),
                            default_value: typed_expr(TypedExprKind::NullLiteral, FluxType::Null),
                            resolved_type: FluxType::Fn {
                                params: FnParams::Fixed(vec![FluxType::Int]),
                                ret: Box::new(FluxType::Float),
                            },
                            span: Span::new(42, 55),
                        }],
                        span: Span::new(40, 60),
                    }),
                ],
                span: Span::new(0, 65),
            },
            span: Span::new(0, 65),
        };

        let result = generate(&program);
        assert!(result.is_err(), "Expected error for FluxType::Fn in field position");

        match result.unwrap_err() {
            CompileError::Codegen(msg) => {
                assert!(
                    msg.contains("at byte 42:"),
                    "Error message should contain byte offset, got: {}",
                    msg
                );
                assert!(
                    msg.contains("function types cannot be emitted"),
                    "Error message should describe the issue, got: {}",
                    msg
                );
            }
            other => panic!("Expected CompileError::Codegen, got: {:?}", other),
        }
    }

    // Test 4: Empty strategy (no params, no state, no handlers)
    #[test]
    fn test_generate_empty_strategy() {
        let program = TypedProgram {
            imports: vec![],
            structs: vec![],
            enums: vec![],
            functions: vec![],
            impl_blocks: vec![],
            traits: vec![],
            data_block: None,
            connector_block: None,
            strategy: TypedStrategy {
                name: "EmptyStrategy".to_string(),
                body: vec![],
                span: Span::new(0, 20),
            },
            span: Span::new(0, 20),
        };

        let result = generate(&program);
        assert!(result.is_ok(), "generate() failed on empty strategy: {:?}", result.err());
        let output = result.unwrap();

        // Should still produce valid minimal Rust code
        assert!(output.contains("use flux_runtime::*;"), "Missing preamble");
        assert!(output.contains("pub struct EmptyStrategy {"), "Missing struct definition");
        assert!(output.contains("impl Default for EmptyStrategy"), "Missing Default impl");
        assert!(output.contains("impl Strategy for EmptyStrategy"), "Missing Strategy impl");

        // The struct should have no fields (empty braces)
        let struct_start = output.find("pub struct EmptyStrategy {").unwrap();
        let struct_close = output[struct_start..].find('}').unwrap() + struct_start;
        let struct_body = &output[struct_start + "pub struct EmptyStrategy {".len()..struct_close];
        // The body should be just whitespace/newline (no field declarations)
        assert!(
            struct_body.trim().is_empty(),
            "Empty strategy struct should have no fields, got: '{}'",
            struct_body.trim()
        );

        // The Strategy impl should have no methods (empty or just braces)
        assert!(
            !output.contains("fn on_bar"),
            "Empty strategy should not have on_bar method"
        );
    }
}
