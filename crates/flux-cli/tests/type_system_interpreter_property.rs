//! Property-based tests for interpreter match evaluation correctness.
//!
//! These tests validate that the Flux interpreter correctly selects the matching
//! arm in a match expression and binds pattern variables to the correct field values.
//!
//! Feature: flux-type-system, Property 11: Interpreter Match Evaluation Correctness

use proptest::prelude::*;

use flux_compiler::lexer::Span;
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

/// Build a float literal expression.
fn float_lit(v: f64) -> TypedExpr {
    texpr(TypedExprKind::FloatLiteral(v), FluxType::Float)
}

/// Build an identifier expression.
fn ident_expr(name: &str) -> TypedExpr {
    texpr(TypedExprKind::Ident(name.to_string()), FluxType::Float)
}

/// Build an assignment statement.
fn assign_stmt(target: &str, value_expr: TypedExpr) -> TypedStmt {
    TypedStmt::Assignment(TypedAssignment {
        target: ident_expr(target),
        value: value_expr,
        span: Span::new(0, 0),
    })
}

/// Build an expression statement.
fn expr_stmt(expr: TypedExpr) -> TypedStmt {
    TypedStmt::Expr(TypedExprStmt {
        expr,
        span: Span::new(0, 0),
    })
}

// =============================================================================
// Enum variant description for generation
// =============================================================================

/// Describes a generated enum variant with its field count.
#[derive(Debug, Clone)]
struct VariantDesc {
    name: String,
    field_count: usize,
}

/// Describes a generated enum with its variants.
#[derive(Debug, Clone)]
struct EnumDesc {
    name: String,
    variants: Vec<VariantDesc>,
}

/// Describes which variant to construct and with what field values.
#[derive(Debug, Clone)]
struct ConstructionDesc {
    variant_index: usize,
    field_values: Vec<f64>,
}

// =============================================================================
// Strategies for proptest
// =============================================================================

/// Generate an enum description with 1-4 variants, each with 0-3 fields.
fn arb_enum_desc() -> impl Strategy<Value = EnumDesc> {
    // Generate 1-4 variants
    prop::collection::vec(
        // Each variant has 0-3 fields
        0usize..4,
        1..=4,
    )
    .prop_map(|field_counts| {
        let variants: Vec<VariantDesc> = field_counts
            .into_iter()
            .enumerate()
            .map(|(i, field_count)| VariantDesc {
                name: format!("Var{}", i),
                field_count,
            })
            .collect();
        EnumDesc {
            name: "TestEnum".to_string(),
            variants,
        }
    })
}

/// Generate a construction description for a given enum.
fn arb_construction(enum_desc: &EnumDesc) -> BoxedStrategy<ConstructionDesc> {
    let num_variants = enum_desc.variants.len();
    let field_counts: Vec<usize> = enum_desc.variants.iter().map(|v| v.field_count).collect();

    (0..num_variants)
        .prop_flat_map(move |variant_idx| {
            let fc = field_counts[variant_idx];
            let values_strategy = prop::collection::vec(
                -1000.0f64..1000.0f64,
                fc..=fc,
            );
            (Just(variant_idx), values_strategy)
        })
        .prop_map(|(variant_index, field_values)| ConstructionDesc {
            variant_index,
            field_values,
        })
        .boxed()
}

// =============================================================================
// Program builder
// =============================================================================

/// Build a TypedProgram that:
/// 1. Defines an enum with the given variants
/// 2. Constructs a value of a specific variant
/// 3. Matches on it with all arms + wildcard
/// 4. Returns the result into a state variable
///
/// Each arm returns a unique float that identifies which arm was selected:
/// - Variant arms with no fields return `(variant_index as f64) * 100.0`
/// - Variant arms with fields return `(variant_index as f64) * 100.0 + field0`
///   (using the first field value as an offset to verify binding correctness)
/// - Wildcard arm returns 9999.0 (should never match when all variants covered)
fn build_match_program(
    enum_desc: &EnumDesc,
    construction: &ConstructionDesc,
) -> TypedProgram {
    // Build the TypedEnumDef
    let typed_variants: Vec<TypedEnumVariant> = enum_desc
        .variants
        .iter()
        .map(|v| TypedEnumVariant {
            name: v.name.clone(),
            fields: (0..v.field_count)
                .map(|i| (format!("f{}", i), FluxType::Float))
                .collect(),
            span: Span::new(0, 0),
        })
        .collect();

    let enum_def = TypedEnumDef {
        name: enum_desc.name.clone(),
        type_params: vec![],
        variants: typed_variants,
        span: Span::new(0, 0),
    };

    // Build the enum construction expression
    let selected_variant = &enum_desc.variants[construction.variant_index];
    let construction_args: Vec<TypedExpr> = construction
        .field_values
        .iter()
        .map(|&v| float_lit(v))
        .collect();

    let enum_construction_expr = texpr(
        TypedExprKind::EnumConstruction {
            enum_name: enum_desc.name.clone(),
            variant_name: selected_variant.name.clone(),
            args: construction_args,
        },
        FluxType::Enum(enum_desc.name.clone()),
    );

    // Build match arms: one arm per variant + a wildcard
    let mut arms: Vec<TypedMatchArm> = Vec::new();
    for (i, variant) in enum_desc.variants.iter().enumerate() {
        let base_value = (i as f64) * 100.0;

        let (body, bindings): (Vec<TypedStmt>, Vec<(String, FluxType)>) = if variant.field_count == 0
        {
            // Unit variant: arm body returns base_value
            let body = vec![expr_stmt(float_lit(base_value))];
            let bindings = vec![];
            (body, bindings)
        } else {
            // Data variant: arm body returns base_value + first_field_binding
            let bindings: Vec<(String, FluxType)> = (0..variant.field_count)
                .map(|j| (format!("b{}", j), FluxType::Float))
                .collect();
            // Return base_value + b0 to verify binding correctness
            let body = vec![expr_stmt(texpr(
                TypedExprKind::BinaryOp {
                    left: Box::new(float_lit(base_value)),
                    op: flux_compiler::parser::ast::BinOp::Add,
                    right: Box::new(texpr(
                        TypedExprKind::Ident("b0".to_string()),
                        FluxType::Float,
                    )),
                },
                FluxType::Float,
            ))];
            (body, bindings)
        };

        arms.push(TypedMatchArm {
            pattern: TypedPattern::Variant {
                enum_name: enum_desc.name.clone(),
                variant_name: variant.name.clone(),
                bindings,
                span: Span::new(0, 0),
            },
            body,
            span: Span::new(0, 0),
        });
    }

    // Add a wildcard arm (should never be reached since all variants are covered)
    arms.push(TypedMatchArm {
        pattern: TypedPattern::Wildcard {
            span: Span::new(0, 0),
        },
        body: vec![expr_stmt(float_lit(9999.0))],
        span: Span::new(0, 0),
    });

    // Build the match expression
    let match_expr = texpr(
        TypedExprKind::Match(TypedMatchExpr {
            scrutinee: Box::new(enum_construction_expr),
            arms,
            result_type: FluxType::Float,
            span: Span::new(0, 0),
        }),
        FluxType::Float,
    );

    // Strategy on_bar body:
    //   result = match val { ... }
    let handler_body = vec![assign_stmt("result", match_expr)];

    TypedProgram {
        imports: vec![],
        structs: vec![],
        enums: vec![enum_def],
        functions: vec![],
        impl_blocks: vec![],
            traits: vec![],
        data_block: None,
        connector_block: None,
        strategy: TypedStrategy {
            name: "MatchTest".to_string(),
            body: vec![
                TypedStrategyItem::StateBlock(TypedStateBlock {
                    variables: vec![TypedStateVar {
                        name: "result".to_string(),
                        initial_value: float_lit(-1.0),
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

/// Compute the expected result value for a given construction.
///
/// If the selected variant is a unit variant (0 fields), returns `variant_index * 100.0`.
/// If it has fields, returns `variant_index * 100.0 + first_field_value`.
fn expected_result(enum_desc: &EnumDesc, construction: &ConstructionDesc) -> f64 {
    let variant = &enum_desc.variants[construction.variant_index];
    let base = (construction.variant_index as f64) * 100.0;
    if variant.field_count == 0 {
        base
    } else {
        base + construction.field_values[0]
    }
}

// =============================================================================
// Property 11: Interpreter Match Evaluation Correctness
// Feature: flux-type-system, Property 11
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

    /// **Validates: Requirements 3.9**
    ///
    /// Property 11: Interpreter Match Evaluation Correctness
    ///
    /// For any enum value and any exhaustive match expression over that enum's type,
    /// the interpreter SHALL select and evaluate exactly the first arm whose pattern
    /// matches the value's variant, with pattern-bound variables correctly bound to
    /// the corresponding field values.
    #[test]
    fn prop_match_evaluation_correctness(
        enum_desc in arb_enum_desc(),
    ) {
        // Generate a construction for the enum
        let construction_strategy = arb_construction(&enum_desc);
        // Use a fixed test runner to pick a construction from the strategy
        let mut runner = proptest::test_runner::TestRunner::default();
        let construction = construction_strategy
            .new_tree(&mut runner)
            .unwrap()
            .current();

        let program = build_match_program(&enum_desc, &construction);
        let mut interp = Interpreter::new(&program);
        let ctx = test_bar();

        interp.on_bar(&ctx);

        let result = match interp.state.get("result") {
            Some(Value::Float(f)) => *f,
            other => panic!(
                "Expected Float in state 'result', got {:?}\nenum: {:?}\nconstruction: {:?}",
                other, enum_desc, construction
            ),
        };

        let expected = expected_result(&enum_desc, &construction);
        let tolerance = 1e-10 * expected.abs().max(1.0);

        prop_assert!(
            (result - expected).abs() < tolerance,
            "Match selected wrong arm or binding incorrect:\n\
             expected={}, got={}\n\
             variant_index={}, field_values={:?}\n\
             enum_desc={:?}",
            expected,
            result,
            construction.variant_index,
            construction.field_values,
            enum_desc,
        );
    }
}
