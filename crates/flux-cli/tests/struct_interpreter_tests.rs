//! Unit tests and property tests for interpreter struct support.
//!
//! Task 8.5: Unit tests verifying struct literal evaluation and fixed-array bounds.
//! Task 8.6: Property test for interpreter struct value semantics (independence after copy).
//!
//! **Validates: Requirements 18.1, 6.5, 18.3, 18.4**

use proptest::prelude::*;

use flux_cli::interpreter::{Interpreter, Value};
use flux_compiler::lexer;
use flux_compiler::lexer::Span;
use flux_compiler::parser;
use flux_compiler::typeck;
use flux_compiler::typeck::typed_ast::*;
use flux_compiler::typeck::types::FluxType;
use flux_runtime::BarContext;

// =============================================================================
// Helpers
// =============================================================================

/// Helper: compile Flux source through lex → parse → typecheck, returning an Interpreter.
fn compile_to_interpreter(source: &str) -> Interpreter {
    let tokens = lexer::lex_with_spans(source).expect("lexer failed");
    let ast = parser::parse(tokens).expect("parser failed");
    let typed_program = typeck::check(ast).expect("typechecker failed");
    Interpreter::new(&typed_program)
}

/// Helper: create a BarContext with specified values.
fn bar(symbol: &str, close: f64, open: f64) -> BarContext {
    BarContext {
        symbol: symbol.to_string(),
        close,
        open,
        high: close + 1.0,
        low: open - 1.0,
        volume: 1000.0,
        in_position: false,
    }
}

/// Shortcut to build a TypedExpr with a given kind, type, and dummy span.
fn texpr(kind: TypedExprKind, ty: FluxType) -> TypedExpr {
    TypedExpr {
        kind,
        resolved_type: ty,
        span: Span::new(0, 0),
    }
}

/// Build a float literal expression.
fn float_lit(v: f64) -> TypedExpr {
    texpr(TypedExprKind::FloatLiteral(v), FluxType::Float)
}

/// Build an int literal expression.
fn int_lit(v: i64) -> TypedExpr {
    texpr(TypedExprKind::IntLiteral(v), FluxType::Int)
}

/// Build an identifier expression.
fn ident_expr(name: &str, ty: FluxType) -> TypedExpr {
    texpr(TypedExprKind::Ident(name.to_string()), ty)
}

/// Build an assignment statement: `target = value_expr`.
fn assign_stmt(target: &str, target_type: FluxType, value_expr: TypedExpr) -> TypedStmt {
    TypedStmt::Assignment(TypedAssignment {
        target: ident_expr(target, target_type),
        value: value_expr,
        span: Span::new(0, 0),
    })
}

// =============================================================================
// Task 8.5: Unit tests for interpreter struct support
// =============================================================================

/// Test evaluating a struct literal produces correct field values.
/// Validates: Requirement 18.1
#[test]
fn test_struct_literal_produces_correct_field_values() {
    let source = r#"
struct Point {
    x: f64,
    y: f64
}

strategy Test {
    state {
        result_x = 0.0
        result_y = 0.0
    }
    on bar {
        p = Point { x = 3.5, y = 7.2 }
        result_x = p.x
        result_y = p.y
    }
}
"#;

    let mut interp = compile_to_interpreter(source);
    let ctx = bar("AAPL", 100.0, 99.0);
    interp.on_bar(&ctx);

    match interp.state.get("result_x") {
        Some(Value::Float(f)) => assert!(
            (*f - 3.5).abs() < f64::EPSILON,
            "Expected result_x=3.5, got {}",
            f
        ),
        other => panic!("Expected Float(3.5) for result_x, got {:?}", other),
    }

    match interp.state.get("result_y") {
        Some(Value::Float(f)) => assert!(
            (*f - 7.2).abs() < f64::EPSILON,
            "Expected result_y=7.2, got {}",
            f
        ),
        other => panic!("Expected Float(7.2) for result_y, got {:?}", other),
    }
}

/// Test evaluating a struct with multiple field types (f64, int, bool).
/// Validates: Requirement 18.1
#[test]
fn test_struct_literal_multiple_types() {
    let source = r#"
struct Config {
    threshold: f64,
    period: int,
    enabled: bool
}

strategy Test {
    state {
        t = 0.0
        p = 0
        e = false
    }
    on bar {
        cfg = Config { threshold = 2.5, period = 20, enabled = true }
        t = cfg.threshold
        p = cfg.period
        e = cfg.enabled
    }
}
"#;

    let mut interp = compile_to_interpreter(source);
    let ctx = bar("AAPL", 100.0, 99.0);
    interp.on_bar(&ctx);

    match interp.state.get("t") {
        Some(Value::Float(f)) => assert!(
            (*f - 2.5).abs() < f64::EPSILON,
            "Expected threshold=2.5, got {}",
            f
        ),
        other => panic!("Expected Float(2.5) for threshold, got {:?}", other),
    }

    match interp.state.get("p") {
        Some(Value::Int(i)) => assert_eq!(*i, 20, "Expected period=20, got {}", i),
        other => panic!("Expected Int(20) for period, got {:?}", other),
    }

    match interp.state.get("e") {
        Some(Value::Bool(b)) => assert!(*b, "Expected enabled=true, got {}", b),
        other => panic!("Expected Bool(true) for enabled, got {:?}", other),
    }
}

/// Test out-of-bounds fixed array access produces an error.
///
/// Constructs a TypedProgram directly with a struct containing a List (representing
/// a fixed-size array at the interpreter level), then performs an out-of-bounds index.
/// Validates: Requirement 6.5
#[test]
fn test_out_of_bounds_fixed_array_access_produces_error() {
    // Build a typed program that:
    //   1. Constructs a struct with a "values" field holding a list [10.0, 20.0, 30.0]
    //   2. Accesses values[5] (out of bounds for size 3)
    //   3. Assigns to state var "result"
    // The interpreter should error, leaving state unchanged.

    let struct_type = FluxType::Struct("Data".to_string());
    let array_type = FluxType::FixedArray(Box::new(FluxType::Float), 3);

    // Build struct literal: Data { values = [10.0, 20.0, 30.0] }
    let struct_literal = texpr(
        TypedExprKind::StructLiteral {
            struct_name: "Data".to_string(),
            fields: vec![(
                "values".to_string(),
                texpr(
                    TypedExprKind::ListLiteral(vec![float_lit(10.0), float_lit(20.0), float_lit(30.0)]),
                    array_type.clone(),
                ),
            )],
        },
        struct_type.clone(),
    );

    // d = Data { values = [10.0, 20.0, 30.0] }
    let assign_d = assign_stmt("d", struct_type.clone(), struct_literal);

    // d.values[5] — out of bounds access
    let field_access = texpr(
        TypedExprKind::MemberAccess {
            object: Box::new(ident_expr("d", struct_type.clone())),
            field: "values".to_string(),
        },
        array_type.clone(),
    );

    let index_access = texpr(
        TypedExprKind::IndexAccess {
            object: Box::new(field_access),
            index: Box::new(int_lit(5)),
        },
        FluxType::Float,
    );

    // result = d.values[5]
    let assign_result = assign_stmt("result", FluxType::Float, index_access);

    let handler_body = vec![assign_d, assign_result];

    let program = TypedProgram {
        imports: vec![],
        structs: vec![TypedStructDef {
            name: "Data".to_string(),
            fields: vec![TypedStructField {
                name: "values".to_string(),
                resolved_type: array_type,
                bit_width: None,
                field_decorator_names: vec![],
                span: Span::new(0, 0),
            }],
            decorators: vec![],
            span: Span::new(0, 0),
        }],
        functions: vec![],
        data_block: None,
        connector_block: None,
        strategy: TypedStrategy {
            name: "Test".to_string(),
            body: vec![
                TypedStrategyItem::StateBlock(TypedStateBlock {
                    variables: vec![TypedStateVar {
                        name: "result".to_string(),
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
    };

    let mut interp = Interpreter::new(&program);
    let ctx = bar("AAPL", 100.0, 99.0);
    let signals = interp.on_bar(&ctx);

    // The out-of-bounds access should cause an error, resulting in no signals
    // and the state remaining unchanged (at initial value).
    assert!(signals.is_empty(), "Expected no signals on out-of-bounds access");

    match interp.state.get("result") {
        Some(Value::Float(f)) => assert!(
            (*f - 0.0).abs() < f64::EPSILON,
            "Expected result=0.0 (unchanged due to error), got {}",
            f
        ),
        other => panic!("Expected Float(0.0) in state, got {:?}", other),
    }
}

/// Test valid fixed array access returns the correct element via AST construction.
/// Validates: Requirement 6.5
#[test]
fn test_valid_fixed_array_access_returns_correct_element() {
    let struct_type = FluxType::Struct("Data".to_string());
    let array_type = FluxType::FixedArray(Box::new(FluxType::Float), 3);

    // Build struct literal: Data { values = [10.0, 20.0, 30.0] }
    let struct_literal = texpr(
        TypedExprKind::StructLiteral {
            struct_name: "Data".to_string(),
            fields: vec![(
                "values".to_string(),
                texpr(
                    TypedExprKind::ListLiteral(vec![float_lit(10.0), float_lit(20.0), float_lit(30.0)]),
                    array_type.clone(),
                ),
            )],
        },
        struct_type.clone(),
    );

    // d = Data { values = [10.0, 20.0, 30.0] }
    let assign_d = assign_stmt("d", struct_type.clone(), struct_literal);

    // d.values[1] — valid access
    let field_access = texpr(
        TypedExprKind::MemberAccess {
            object: Box::new(ident_expr("d", struct_type.clone())),
            field: "values".to_string(),
        },
        array_type.clone(),
    );

    let index_access = texpr(
        TypedExprKind::IndexAccess {
            object: Box::new(field_access),
            index: Box::new(int_lit(1)),
        },
        FluxType::Float,
    );

    // result = d.values[1]
    let assign_result = assign_stmt("result", FluxType::Float, index_access);

    let handler_body = vec![assign_d, assign_result];

    let program = TypedProgram {
        imports: vec![],
        structs: vec![TypedStructDef {
            name: "Data".to_string(),
            fields: vec![TypedStructField {
                name: "values".to_string(),
                resolved_type: array_type,
                bit_width: None,
                field_decorator_names: vec![],
                span: Span::new(0, 0),
            }],
            decorators: vec![],
            span: Span::new(0, 0),
        }],
        functions: vec![],
        data_block: None,
        connector_block: None,
        strategy: TypedStrategy {
            name: "Test".to_string(),
            body: vec![
                TypedStrategyItem::StateBlock(TypedStateBlock {
                    variables: vec![TypedStateVar {
                        name: "result".to_string(),
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
    };

    let mut interp = Interpreter::new(&program);
    let ctx = bar("AAPL", 100.0, 99.0);
    interp.on_bar(&ctx);

    // d.values[1] should produce 20.0
    match interp.state.get("result") {
        Some(Value::Float(f)) => assert!(
            (*f - 20.0).abs() < f64::EPSILON,
            "Expected result=20.0, got {}",
            f
        ),
        other => panic!("Expected Float(20.0) in state, got {:?}", other),
    }
}

// =============================================================================
// Task 8.6: Property test for interpreter struct value semantics
// Feature: flux-structs, Property 9: Interpreter struct value semantics
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

    /// **Validates: Requirements 18.3, 18.4**
    ///
    /// Property 9: Interpreter struct value semantics (independence after copy)
    ///
    /// For any struct value, when assigned to a new variable, subsequent reassignment
    /// of the original variable SHALL NOT affect the copied value.
    ///
    /// The test generates struct field values, creates a struct, copies it to another
    /// variable, then reassigns the original to a struct with different values, and
    /// verifies the copy retains the original values.
    #[test]
    fn prop_struct_value_semantics_independence_after_copy(
        x1 in -1000.0f64..1000.0,
        y1 in -1000.0f64..1000.0,
        x2 in -1000.0f64..1000.0,
        y2 in -1000.0f64..1000.0,
    ) {
        // Ensure original and reassigned values differ so we can detect mutation
        prop_assume!((x1 - x2).abs() > 0.001 || (y1 - y2).abs() > 0.001);

        let source = format!(r#"
struct Point {{
    x: f64,
    y: f64
}}

strategy Test {{
    state {{
        copy_x = 0.0
        copy_y = 0.0
    }}
    on bar {{
        original = Point {{ x = {x1}, y = {y1} }}
        copy = original
        original = Point {{ x = {x2}, y = {y2} }}
        copy_x = copy.x
        copy_y = copy.y
    }}
}}
"#);

        let mut interp = compile_to_interpreter(&source);
        let ctx = bar("AAPL", 100.0, 99.0);
        interp.on_bar(&ctx);

        let copy_x = match interp.state.get("copy_x") {
            Some(Value::Float(f)) => *f,
            other => panic!("Expected Float in state 'copy_x', got {:?}", other),
        };

        let copy_y = match interp.state.get("copy_y") {
            Some(Value::Float(f)) => *f,
            other => panic!("Expected Float in state 'copy_y', got {:?}", other),
        };

        // The copy should retain the ORIGINAL values (x1, y1), not the reassigned values (x2, y2)
        prop_assert!(
            (copy_x - x1).abs() < 1e-10,
            "Value semantics violated for x: copy_x={}, expected x1={} (x2={})",
            copy_x, x1, x2
        );
        prop_assert!(
            (copy_y - y1).abs() < 1e-10,
            "Value semantics violated for y: copy_y={}, expected y1={} (y2={})",
            copy_y, y1, y2
        );
    }
}
