//! Property-based tests for interpreter correctness.
//!
//! These tests validate that the Flux interpreter correctly handles:
//! - Match evaluation (Property 11): selecting the correct arm and binding pattern variables
//! - HashMap insert-get round-trip (Property 12): last-write-wins semantics and missing key behavior
//!
//! Feature: flux-type-system, Property 11: Interpreter Match Evaluation Correctness
//! Feature: flux-type-system, Property 12: Interpreter HashMap Insert-Get Round-Trip

use proptest::prelude::*;

use std::collections::HashMap as StdHashMap;

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


// =============================================================================
// Property 12: Interpreter HashMap Insert-Get Round-Trip
// Feature: flux-type-system, Property 12
// =============================================================================

/// Describes a single HashMap operation (insert or get).
#[derive(Debug, Clone)]
enum HashMapOp {
    Insert { key: String, value: f64 },
    Get { key: String },
}

/// Strategy to generate valid HashMap key strings (short alphanumeric identifiers).
fn arb_key() -> impl Strategy<Value = String> {
    "[a-z]{1,6}".prop_map(|s| s)
}

/// Strategy to generate a sequence of HashMap operations with a mix of inserts and gets.
fn arb_hashmap_ops() -> impl Strategy<Value = Vec<HashMapOp>> {
    // Generate 2-20 operations mixing inserts and gets
    prop::collection::vec(
        prop_oneof![
            // Insert: 70% probability (to build up state)
            7 => (arb_key(), -1000.0f64..1000.0f64).prop_map(|(key, value)| HashMapOp::Insert { key, value }),
            // Get: 30% probability
            3 => arb_key().prop_map(|key| HashMapOp::Get { key }),
        ],
        2..=20,
    )
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

    /// **Validates: Requirements 10.9**
    ///
    /// Property 12: Interpreter HashMap Insert-Get Round-Trip
    ///
    /// For any sequence of HashMap insert operations followed by get operations,
    /// the interpreter SHALL return the most recently inserted value for each key.
    /// Keys not inserted SHALL not be retrievable.
    #[test]
    fn prop_hashmap_insert_get_round_trip(
        ops in arb_hashmap_ops(),
    ) {
        let mut interp = {
            // Build a minimal TypedProgram just to construct an Interpreter
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
                    name: "HashMapTest".to_string(),
                    body: vec![
                        TypedStrategyItem::EventHandler(TypedEventHandler {
                            event_name: "bar".to_string(),
                            body: vec![],
                            span: Span::new(0, 0),
                        }),
                    ],
                    span: Span::new(0, 0),
                },
                span: Span::new(0, 0),
            };
            Interpreter::new(&program)
        };

        // Track expected state: the last-inserted value for each key
        let mut expected: StdHashMap<String, f64> = StdHashMap::new();
        // Current HashMap value in the interpreter
        let mut current_map = Value::HashMap(StdHashMap::new());

        for op in &ops {
            match op {
                HashMapOp::Insert { key, value } => {
                    // Build: current_map.insert(key, value)
                    let mut locals = StdHashMap::new();
                    locals.insert("__map".to_string(), current_map.clone());

                    let receiver = texpr(
                        TypedExprKind::Ident("__map".to_string()),
                        FluxType::Generic("HashMap".to_string(), vec![]),
                    );
                    let insert_expr = texpr(
                        TypedExprKind::MethodCall {
                            receiver: Box::new(receiver),
                            method: "insert".to_string(),
                            args: vec![
                                texpr(TypedExprKind::StringLiteral(key.clone()), FluxType::String),
                                texpr(TypedExprKind::FloatLiteral(*value), FluxType::Float),
                            ],
                        },
                        FluxType::Generic("HashMap".to_string(), vec![]),
                    );

                    let result = interp.eval_expr(&insert_expr, &locals).unwrap();
                    current_map = result;
                    expected.insert(key.clone(), *value);
                }
                HashMapOp::Get { key } => {
                    let mut locals = StdHashMap::new();
                    locals.insert("__map".to_string(), current_map.clone());

                    let receiver = texpr(
                        TypedExprKind::Ident("__map".to_string()),
                        FluxType::Generic("HashMap".to_string(), vec![]),
                    );
                    let get_expr = texpr(
                        TypedExprKind::MethodCall {
                            receiver: Box::new(receiver),
                            method: "get".to_string(),
                            args: vec![
                                texpr(TypedExprKind::StringLiteral(key.clone()), FluxType::String),
                            ],
                        },
                        FluxType::Float,
                    );

                    let result = interp.eval_expr(&get_expr, &locals);

                    if let Some(&expected_val) = expected.get(key) {
                        // Key was previously inserted — get should succeed
                        let val = result.unwrap_or_else(|e| {
                            panic!(
                                "Expected get('{}') to return {}, but got error: {}\nops so far: {:?}",
                                key, expected_val, e, ops
                            )
                        });
                        match val {
                            Value::Float(f) => {
                                prop_assert!(
                                    (f - expected_val).abs() < 1e-10,
                                    "get('{}') returned {} but expected {} (last-write-wins)\nops: {:?}",
                                    key, f, expected_val, ops,
                                );
                            }
                            other => {
                                prop_assert!(
                                    false,
                                    "get('{}') returned {:?} instead of Float({})\nops: {:?}",
                                    key, other, expected_val, ops,
                                );
                            }
                        }
                    } else {
                        // Key was never inserted — get should return Null (per Requirement 7.3/7.4)
                        let val = result.unwrap_or_else(|e| {
                            panic!(
                                "get('{}') should return Null for missing key, but got error: {}\nops: {:?}",
                                key, e, ops
                            )
                        });
                        prop_assert!(
                            matches!(val, Value::Null),
                            "get('{}') should return Null for never-inserted key, but got {:?}\nops: {:?}",
                            key, val, ops,
                        );
                    }
                }
            }
        }
    }
}


// =============================================================================
// Property 13: Enum Value Display Formatting
// Feature: flux-type-system, Property 13
// =============================================================================

/// Strategy for generating valid Flux identifiers for use as enum/variant/field names.
/// Uses a leading letter followed by lowercase alphanumeric characters.
fn arb_flux_ident() -> impl Strategy<Value = String> {
    "[A-Z][a-z0-9]{1,8}".prop_map(|s| s)
}

/// Strategy for generating a field name (lowercase identifier).
fn arb_field_name() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9]{0,5}".prop_map(|s| s)
}

/// Strategy for generating a simple Value (for use as enum field values).
fn arb_simple_value() -> impl Strategy<Value = Value> {
    prop_oneof![
        (-1000.0f64..1000.0f64).prop_map(Value::Float),
        any::<i64>().prop_map(Value::Int),
        any::<bool>().prop_map(Value::Bool),
        "[a-z]{1,10}".prop_map(|s| Value::Str(s)),
    ]
}

/// Description of a generated enum value for the display property test.
#[derive(Debug, Clone)]
struct EnumValueDesc {
    enum_name: String,
    variant_name: String,
    fields: Vec<(String, Value)>,
}

/// Strategy that generates an enum value with 0 fields (unit variant).
fn arb_unit_enum_value() -> impl Strategy<Value = EnumValueDesc> {
    (arb_flux_ident(), arb_flux_ident()).prop_map(|(enum_name, variant_name)| EnumValueDesc {
        enum_name,
        variant_name,
        fields: vec![],
    })
}

/// Strategy that generates an enum value with 1-4 named fields (data variant).
fn arb_data_enum_value() -> impl Strategy<Value = EnumValueDesc> {
    (
        arb_flux_ident(),
        arb_flux_ident(),
        prop::collection::vec((arb_field_name(), arb_simple_value()), 1..=4),
    )
        .prop_map(|(enum_name, variant_name, fields)| EnumValueDesc {
            enum_name,
            variant_name,
            fields,
        })
}

/// Strategy that generates either a unit or data enum value.
fn arb_enum_value() -> impl Strategy<Value = EnumValueDesc> {
    prop_oneof![arb_unit_enum_value(), arb_data_enum_value(),]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

    /// **Validates: Requirements 11.1, 11.2**
    ///
    /// Property 13: Enum Value Display Formatting
    ///
    /// For any enum value, the interpreter's Display implementation SHALL format
    /// unit variants as `EnumName.VariantName` and data variants as
    /// `EnumName.VariantName(field1: value1, field2: value2, ...)` with all fields listed.
    #[test]
    fn prop_enum_value_display_formatting(
        desc in arb_enum_value(),
    ) {
        // Construct the Value::Enum from our description
        let value = Value::Enum {
            enum_name: desc.enum_name.clone(),
            variant_name: desc.variant_name.clone(),
            fields: desc.fields.clone(),
        };

        let display_output = format!("{}", value);

        if desc.fields.is_empty() {
            // Unit variant: should format as "EnumName.VariantName"
            let expected = format!("{}.{}", desc.enum_name, desc.variant_name);
            prop_assert_eq!(
                &display_output,
                &expected,
                "Unit variant display mismatch.\nGot: {}\nExpected: {}\nDesc: {:?}",
                display_output,
                expected,
                desc,
            );
        } else {
            // Data variant: should format as "EnumName.VariantName(field1: value1, field2: value2)"
            let prefix = format!("{}.{}(", desc.enum_name, desc.variant_name);
            prop_assert!(
                display_output.starts_with(&prefix),
                "Data variant should start with '{}', got: '{}'\nDesc: {:?}",
                prefix,
                display_output,
                desc,
            );
            prop_assert!(
                display_output.ends_with(')'),
                "Data variant should end with ')', got: '{}'\nDesc: {:?}",
                display_output,
                desc,
            );

            // Verify all fields are present in the correct format
            let fields_str = &display_output[prefix.len()..display_output.len() - 1];
            let expected_fields: Vec<String> = desc
                .fields
                .iter()
                .map(|(name, val)| format!("{}: {}", name, val))
                .collect();
            let expected_fields_str = expected_fields.join(", ");

            prop_assert_eq!(
                fields_str,
                &expected_fields_str,
                "Data variant fields mismatch.\nGot fields: '{}'\nExpected fields: '{}'\nFull output: '{}'\nDesc: {:?}",
                fields_str,
                expected_fields_str,
                display_output,
                desc,
            );
        }
    }
}


// =============================================================================
// Property 6: HashMap insert/get round-trip
// Feature: interpreter-type-system, Property 6: HashMap insert/get round-trip
// =============================================================================

/// Strategy to generate HashMap key strings (short alphanumeric identifiers for insert/get testing).
fn arb_hashmap_key() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9]{0,7}".prop_map(|s| s)
}

/// Strategy to generate a value to insert (float or int).
fn arb_insert_value() -> impl Strategy<Value = (f64, bool)> {
    prop_oneof![
        // Float values
        (-1000.0f64..1000.0f64).prop_map(|f| (f, true)),
        // Int values (stored as float for simplicity in assertions)
        (-100i64..100i64).prop_map(|i| (i as f64, false)),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

    /// **Validates: Requirements 7.2, 7.3, 7.5**
    ///
    /// Feature: interpreter-type-system, Property 6: HashMap insert/get round-trip
    ///
    /// For any string key K and any value V, inserting K→V into a Value::HashMap
    /// and then calling .get(K) on the resulting HashMap SHALL return a value equal to V.
    /// Additionally, .contains_key(K) SHALL return true for inserted keys and false for
    /// keys never inserted.
    #[test]
    fn prop_hashmap_insert_get_roundtrip(
        entries in prop::collection::vec((arb_hashmap_key(), arb_insert_value()), 1..=15),
        missing_key in "[A-Z][A-Z0-9]{2,5}",  // uppercase keys are never inserted (lowercase only)
    ) {
        let mut interp = {
            // Build a minimal TypedProgram to construct an Interpreter
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
                    name: "HashMapRoundTripTest".to_string(),
                    body: vec![
                        TypedStrategyItem::EventHandler(TypedEventHandler {
                            event_name: "bar".to_string(),
                            body: vec![],
                            span: Span::new(0, 0),
                        }),
                    ],
                    span: Span::new(0, 0),
                },
                span: Span::new(0, 0),
            };
            Interpreter::new(&program)
        };

        // Start with an empty HashMap
        let mut current_map = Value::HashMap(StdHashMap::new());
        // Track expected state for last-write-wins verification
        let mut expected: StdHashMap<String, f64> = StdHashMap::new();

        // Step 1: Insert all key-value pairs
        for (key, (value, is_float)) in &entries {
            let mut locals = StdHashMap::new();
            locals.insert("__map".to_string(), current_map.clone());

            let receiver = texpr(
                TypedExprKind::Ident("__map".to_string()),
                FluxType::Generic("HashMap".to_string(), vec![]),
            );

            let value_expr = if *is_float {
                texpr(TypedExprKind::FloatLiteral(*value), FluxType::Float)
            } else {
                texpr(TypedExprKind::IntLiteral(*value as i64), FluxType::Int)
            };

            let insert_expr = texpr(
                TypedExprKind::MethodCall {
                    receiver: Box::new(receiver),
                    method: "insert".to_string(),
                    args: vec![
                        texpr(TypedExprKind::StringLiteral(key.clone()), FluxType::String),
                        value_expr,
                    ],
                },
                FluxType::Generic("HashMap".to_string(), vec![]),
            );

            let result = interp.eval_expr(&insert_expr, &locals).unwrap();
            current_map = result;
            expected.insert(key.clone(), *value);
        }

        // Step 2: Verify get returns correct values for all inserted keys
        for (key, expected_val) in &expected {
            let mut locals = StdHashMap::new();
            locals.insert("__map".to_string(), current_map.clone());

            let receiver = texpr(
                TypedExprKind::Ident("__map".to_string()),
                FluxType::Generic("HashMap".to_string(), vec![]),
            );
            let get_expr = texpr(
                TypedExprKind::MethodCall {
                    receiver: Box::new(receiver),
                    method: "get".to_string(),
                    args: vec![
                        texpr(TypedExprKind::StringLiteral(key.clone()), FluxType::String),
                    ],
                },
                FluxType::Float,
            );

            let result = interp.eval_expr(&get_expr, &locals).unwrap();
            match result {
                Value::Float(f) => {
                    prop_assert!(
                        (f - expected_val).abs() < 1e-10,
                        "get('{}') returned {} but expected {}",
                        key, f, expected_val,
                    );
                }
                Value::Int(i) => {
                    prop_assert!(
                        (i as f64 - expected_val).abs() < 1e-10,
                        "get('{}') returned Int({}) but expected {}",
                        key, i, expected_val,
                    );
                }
                other => {
                    prop_assert!(
                        false,
                        "get('{}') returned {:?} instead of expected value {}",
                        key, other, expected_val,
                    );
                }
            }
        }

        // Step 3: Verify contains_key returns true for all inserted keys
        for key in expected.keys() {
            let mut locals = StdHashMap::new();
            locals.insert("__map".to_string(), current_map.clone());

            let receiver = texpr(
                TypedExprKind::Ident("__map".to_string()),
                FluxType::Generic("HashMap".to_string(), vec![]),
            );
            let contains_expr = texpr(
                TypedExprKind::MethodCall {
                    receiver: Box::new(receiver),
                    method: "contains_key".to_string(),
                    args: vec![
                        texpr(TypedExprKind::StringLiteral(key.clone()), FluxType::String),
                    ],
                },
                FluxType::Bool,
            );

            let result = interp.eval_expr(&contains_expr, &locals).unwrap();
            match result {
                Value::Bool(true) => {} // expected
                other => prop_assert!(
                    false,
                    "contains_key('{}') should return true for inserted key, got {:?}",
                    key, other,
                ),
            }
        }

        // Step 4: Verify contains_key returns false for a never-inserted key
        {
            let mut locals = StdHashMap::new();
            locals.insert("__map".to_string(), current_map.clone());

            let receiver = texpr(
                TypedExprKind::Ident("__map".to_string()),
                FluxType::Generic("HashMap".to_string(), vec![]),
            );
            let contains_missing_expr = texpr(
                TypedExprKind::MethodCall {
                    receiver: Box::new(receiver),
                    method: "contains_key".to_string(),
                    args: vec![
                        texpr(TypedExprKind::StringLiteral(missing_key.clone()), FluxType::String),
                    ],
                },
                FluxType::Bool,
            );

            let result = interp.eval_expr(&contains_missing_expr, &locals).unwrap();
            match result {
                Value::Bool(false) => {} // expected
                other => prop_assert!(
                    false,
                    "contains_key('{}') should return false for never-inserted key, got {:?}",
                    missing_key, other,
                ),
            }
        }

        // Step 5: Verify get returns Null for a never-inserted key
        {
            let mut locals = StdHashMap::new();
            locals.insert("__map".to_string(), current_map.clone());

            let receiver = texpr(
                TypedExprKind::Ident("__map".to_string()),
                FluxType::Generic("HashMap".to_string(), vec![]),
            );
            let get_missing_expr = texpr(
                TypedExprKind::MethodCall {
                    receiver: Box::new(receiver),
                    method: "get".to_string(),
                    args: vec![
                        texpr(TypedExprKind::StringLiteral(missing_key.clone()), FluxType::String),
                    ],
                },
                FluxType::Float,
            );

            let result = interp.eval_expr(&get_missing_expr, &locals).unwrap();
            match result {
                Value::Null => {} // expected
                other => prop_assert!(
                    false,
                    "get('{}') should return Null for never-inserted key, got {:?}",
                    missing_key, other,
                ),
            }
        }
    }
}


// =============================================================================
// Property 5: Static method dispatch produces correct return value
// Feature: interpreter-type-system, Property 5: Static method dispatch produces correct return value
// =============================================================================

/// Describes a generated static method with parameter names and a computation rule.
#[derive(Debug, Clone)]
struct StaticMethodDesc {
    type_name: String,
    method_name: String,
    param_values: Vec<f64>,
}

/// Strategy for generating a valid type name (uppercase first letter, then lowercase).
fn arb_type_name() -> impl Strategy<Value = String> {
    "[A-Z][a-z]{2,8}".prop_map(|s| s)
}

/// Strategy for generating a valid method name (lowercase).
fn arb_method_name() -> impl Strategy<Value = String> {
    "[a-z]{2,8}".prop_map(|s| s)
}

/// Strategy for generating parameter values (1-4 float values, avoiding extremes for stable arithmetic).
fn arb_param_values() -> impl Strategy<Value = Vec<f64>> {
    prop::collection::vec(-500.0f64..500.0f64, 1..=4)
}

/// Strategy for generating a static method description.
fn arb_static_method_desc() -> impl Strategy<Value = StaticMethodDesc> {
    (arb_type_name(), arb_method_name(), arb_param_values()).prop_map(
        |(type_name, method_name, param_values)| StaticMethodDesc {
            type_name,
            method_name,
            param_values,
        },
    )
}

/// Build a TypedProgram that:
/// 1. Registers a static method on a struct type (no `self` param)
///    The method body computes: sum of all params (param0 + param1 + ... + paramN)
/// 2. Calls that method via `TypeName.method(args...)`
/// 3. Stores the result in a state variable
///
/// The expected result is the sum of all parameter values.
fn build_static_method_program(desc: &StaticMethodDesc) -> TypedProgram {
    use flux_compiler::parser::ast::BinOp;

    let param_names: Vec<String> = (0..desc.param_values.len())
        .map(|i| format!("p{}", i))
        .collect();

    // Build the method body: return p0 + p1 + p2 + ...
    // For a single param, just return p0.
    let return_expr = if param_names.len() == 1 {
        texpr(
            TypedExprKind::Ident(param_names[0].clone()),
            FluxType::Float,
        )
    } else {
        // Build a left-associative addition chain: ((p0 + p1) + p2) + ...
        let mut acc = texpr(
            TypedExprKind::Ident(param_names[0].clone()),
            FluxType::Float,
        );
        for pname in &param_names[1..] {
            acc = texpr(
                TypedExprKind::BinaryOp {
                    left: Box::new(acc),
                    op: BinOp::Add,
                    right: Box::new(texpr(
                        TypedExprKind::Ident(pname.clone()),
                        FluxType::Float,
                    )),
                },
                FluxType::Float,
            );
        }
        acc
    };

    let method_def = TypedFnDef {
        name: desc.method_name.clone(),
        type_params: vec![],
        type_param_bounds: vec![],
        params: param_names.clone(),
        param_types: vec![FluxType::Float; desc.param_values.len()],
        body: vec![TypedStmt::Return(TypedReturnStmt {
            value: Some(return_expr),
            span: Span::new(0, 0),
        })],
        return_type: FluxType::Float,
        span: Span::new(0, 0),
    };

    // Build the method call expression: TypeName.method(val0, val1, ...)
    let call_args: Vec<TypedExpr> = desc.param_values.iter().map(|&v| float_lit(v)).collect();

    let method_call_expr = texpr(
        TypedExprKind::MethodCall {
            receiver: Box::new(texpr(
                TypedExprKind::Ident(desc.type_name.clone()),
                FluxType::Struct(desc.type_name.clone()),
            )),
            method: desc.method_name.clone(),
            args: call_args,
        },
        FluxType::Float,
    );

    // on_bar body: result = TypeName.method(args...)
    let handler_body = vec![assign_stmt("result", method_call_expr)];

    // Build a minimal TypedProgram with impl_blocks that register the static method
    TypedProgram {
        imports: vec![],
        structs: vec![],
        enums: vec![],
        functions: vec![],
        impl_blocks: vec![TypedImplBlock {
            target_type: desc.type_name.clone(),
            trait_name: None,
            methods: vec![method_def],
            span: Span::new(0, 0),
        }],
        traits: vec![],
        data_block: None,
        connector_block: None,
        strategy: TypedStrategy {
            name: "StaticMethodTest".to_string(),
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

proptest! {
    #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

    /// **Validates: Requirements 3.1, 3.2**
    ///
    /// Property 5: Static method dispatch produces correct return value
    ///
    /// For any struct type T with a static method M (no `self` parameter) registered
    /// in impl_methods, calling `T.M(args...)` SHALL invoke the method body with
    /// arguments bound to parameters and return the method's computed value without
    /// attempting to evaluate T as a runtime variable.
    #[test]
    fn prop_static_method_dispatch_correct_return_value(
        desc in arb_static_method_desc(),
    ) {
        let program = build_static_method_program(&desc);
        let mut interp = Interpreter::new(&program);
        let ctx = test_bar();

        interp.on_bar(&ctx);

        let result = match interp.state.get("result") {
            Some(Value::Float(f)) => *f,
            other => panic!(
                "Expected Float in state 'result', got {:?}\ndesc: {:?}",
                other, desc
            ),
        };

        // Expected: sum of all parameter values
        let expected: f64 = desc.param_values.iter().sum();
        let tolerance = 1e-10 * expected.abs().max(1.0);

        prop_assert!(
            (result - expected).abs() < tolerance,
            "Static method dispatch returned wrong value:\n\
             expected={}, got={}\n\
             type_name={}, method_name={}, param_values={:?}",
            expected,
            result,
            desc.type_name,
            desc.method_name,
            desc.param_values,
        );
    }
}


// =============================================================================
// Property 4: Instance method dispatch binds self and resolves fields
// Feature: interpreter-type-system, Property 4
// =============================================================================

/// Describes a generated struct with field names and values for Property 4 testing.
#[derive(Debug, Clone)]
struct InstanceMethodTestCase {
    /// The struct type name (e.g., "TestStruct")
    type_name: String,
    /// Fields: (field_name, field_value) pairs
    fields: Vec<(String, f64)>,
    /// Index of the field to access in the method body via self.field_name
    target_field_idx: usize,
    /// A multiplier applied inside the method: return self.field * multiplier
    multiplier: f64,
    /// An extra argument passed to the method and added to the result
    extra_arg: f64,
}

/// Strategy to generate valid lowercase field names for struct fields.
fn arb_struct_field_name() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_]{0,7}".prop_map(|s| s)
}

/// Strategy to generate a unique set of field names (1-5 fields).
fn arb_unique_field_names() -> impl Strategy<Value = Vec<String>> {
    prop::collection::hash_set(arb_struct_field_name(), 1..=5)
        .prop_map(|set| set.into_iter().collect::<Vec<_>>())
}

/// Strategy to generate an InstanceMethodTestCase.
fn arb_instance_method_test_case() -> impl Strategy<Value = InstanceMethodTestCase> {
    arb_unique_field_names()
        .prop_flat_map(|field_names| {
            let n = field_names.len();
            (
                Just(field_names),
                prop::collection::vec(-1000.0f64..1000.0f64, n..=n),
                0..n,
                // Multiplier: use small integers to avoid floating point imprecision
                (1i32..10).prop_map(|i| i as f64),
                -500.0f64..500.0f64,
            )
        })
        .prop_map(|(field_names, values, target_idx, multiplier, extra_arg)| {
            let fields: Vec<(String, f64)> = field_names
                .into_iter()
                .zip(values.into_iter())
                .collect();
            InstanceMethodTestCase {
                type_name: "TestStruct".to_string(),
                fields,
                target_field_idx: target_idx,
                multiplier,
                extra_arg,
            }
        })
}

/// Build a TypedProgram that:
/// 1. Defines a struct with the given fields
/// 2. Defines an impl method `get_computed(self, extra: Float) -> Float`
///    that returns `self.<target_field> * multiplier + extra`
/// 3. In the on_bar handler: constructs the struct, calls the method, stores result
fn build_instance_method_program(tc: &InstanceMethodTestCase) -> TypedProgram {
    let target_field_name = &tc.fields[tc.target_field_idx].0;

    // Build the struct definition (we don't need TypedStructDef for the interpreter,
    // but the interpreter uses impl_methods registry which is populated from impl_blocks)
    let struct_def = TypedStructDef {
        name: tc.type_name.clone(),
        fields: tc.fields.iter().map(|(name, _)| TypedStructField {
            name: name.clone(),
            resolved_type: FluxType::Float,
            bit_width: None,
            field_decorator_names: vec![],
            span: Span::new(0, 0),
        }).collect(),
        type_params: vec![],
        decorators: vec![],
        span: Span::new(0, 0),
    };

    // Build the impl method: fn get_computed(self, extra) -> Float
    // Body: return self.<target_field> * multiplier + extra
    let method_body = vec![
        TypedStmt::Return(TypedReturnStmt {
            value: Some(texpr(
                TypedExprKind::BinaryOp {
                    left: Box::new(texpr(
                        TypedExprKind::BinaryOp {
                            left: Box::new(texpr(
                                TypedExprKind::MemberAccess {
                                    object: Box::new(texpr(
                                        TypedExprKind::Ident("self".to_string()),
                                        FluxType::Struct(tc.type_name.clone()),
                                    )),
                                    field: target_field_name.clone(),
                                },
                                FluxType::Float,
                            )),
                            op: flux_compiler::parser::ast::BinOp::Mul,
                            right: Box::new(float_lit(tc.multiplier)),
                        },
                        FluxType::Float,
                    )),
                    op: flux_compiler::parser::ast::BinOp::Add,
                    right: Box::new(texpr(
                        TypedExprKind::Ident("extra".to_string()),
                        FluxType::Float,
                    )),
                },
                FluxType::Float,
            )),
            span: Span::new(0, 0),
        }),
    ];

    let method_def = TypedFnDef {
        name: "get_computed".to_string(),
        type_params: vec![],
        type_param_bounds: vec![],
        params: vec!["self".to_string(), "extra".to_string()],
        param_types: vec![FluxType::Struct(tc.type_name.clone()), FluxType::Float],
        body: method_body,
        return_type: FluxType::Float,
        span: Span::new(0, 0),
    };

    let impl_block = TypedImplBlock {
        trait_name: None,
        target_type: tc.type_name.clone(),
        methods: vec![method_def],
        span: Span::new(0, 0),
    };

    // Build struct literal expression: TestStruct { field1 = val1, field2 = val2, ... }
    let struct_literal_fields: Vec<(String, TypedExpr)> = tc.fields
        .iter()
        .map(|(name, val)| (name.clone(), float_lit(*val)))
        .collect();

    let struct_literal = texpr(
        TypedExprKind::StructLiteral {
            struct_name: tc.type_name.clone(),
            fields: struct_literal_fields,
        },
        FluxType::Struct(tc.type_name.clone()),
    );

    // Build method call: obj.get_computed(extra_arg)
    // We assign the struct to a variable first, then call the method on it
    let method_call = texpr(
        TypedExprKind::MethodCall {
            receiver: Box::new(texpr(
                TypedExprKind::Ident("obj".to_string()),
                FluxType::Struct(tc.type_name.clone()),
            )),
            method: "get_computed".to_string(),
            args: vec![float_lit(tc.extra_arg)],
        },
        FluxType::Float,
    );

    // on_bar body:
    //   obj = TestStruct { field1 = v1, field2 = v2, ... }
    //   result = obj.get_computed(extra_arg)
    let handler_body = vec![
        assign_stmt("obj", struct_literal),
        assign_stmt("result", method_call),
    ];

    TypedProgram {
        imports: vec![],
        structs: vec![struct_def],
        enums: vec![],
        functions: vec![],
        impl_blocks: vec![impl_block],
        traits: vec![],
        data_block: None,
        connector_block: None,
        strategy: TypedStrategy {
            name: "InstanceMethodTest".to_string(),
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

proptest! {
    #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

    /// **Validates: Requirements 4.1, 4.2, 4.3, 8.1, 8.2**
    ///
    /// Property 4: Instance method dispatch binds self and resolves fields
    ///
    /// For any struct value with type name T and a method M defined in T's impl_methods
    /// registry, calling `value.M(args...)` SHALL bind `self` to the receiver struct such
    /// that `self.field_name` within the method body returns the receiver's field value,
    /// and all additional arguments SHALL be bound to their declared parameter names.
    #[test]
    fn prop_instance_method_self_binding(
        tc in arb_instance_method_test_case(),
    ) {
        let program = build_instance_method_program(&tc);
        let mut interp = Interpreter::new(&program);
        let ctx = test_bar();

        interp.on_bar(&ctx);

        let result = match interp.state.get("result") {
            Some(Value::Float(f)) => *f,
            other => panic!(
                "Expected Float in state 'result', got {:?}\ntest case: {:?}",
                other, tc
            ),
        };

        // Expected: self.<target_field> * multiplier + extra_arg
        let target_value = tc.fields[tc.target_field_idx].1;
        let expected = target_value * tc.multiplier + tc.extra_arg;
        let tolerance = 1e-10 * expected.abs().max(1.0);

        prop_assert!(
            (result - expected).abs() < tolerance,
            "Instance method self-binding incorrect:\n\
             expected={}, got={}\n\
             target_field='{}', target_value={}, multiplier={}, extra_arg={}\n\
             all fields={:?}",
            expected,
            result,
            tc.fields[tc.target_field_idx].0,
            target_value,
            tc.multiplier,
            tc.extra_arg,
            tc.fields,
        );
    }
}


// =============================================================================
// Property 7: Inherent methods take priority over trait methods
// Feature: interpreter-type-system, Property 7: Inherent methods take priority over trait methods
// =============================================================================

/// Description of a generated struct type with conflicting inherent and trait methods.
#[derive(Debug, Clone)]
struct InherentPriorityDesc {
    /// The struct type name (e.g., "TestStruct0")
    type_name: String,
    /// The conflicting method name (e.g., "compute")
    method_name: String,
    /// The return value from the inherent impl method
    inherent_return: f64,
    /// The return value from the trait impl method
    trait_return: f64,
}

/// Strategy to generate a test case for inherent method priority.
/// Ensures inherent_return != trait_return so we can distinguish which ran.
fn arb_inherent_priority() -> impl Strategy<Value = InherentPriorityDesc> {
    (
        0u32..100u32,                   // type name suffix
        "[a-z]{3,8}",                   // method name
        -1000.0f64..1000.0f64,          // inherent return value
        -1000.0f64..1000.0f64,          // trait return value
    )
        .prop_filter(
            "inherent and trait return values must differ",
            |(_suffix, _method, inherent, trait_ret)| (inherent - trait_ret).abs() > 1e-10,
        )
        .prop_map(|(suffix, method_name, inherent_return, trait_return)| {
            InherentPriorityDesc {
                type_name: format!("TestStruct{}", suffix),
                method_name,
                inherent_return,
                trait_return,
            }
        })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

    /// **Validates: Requirements 8.3**
    ///
    /// Feature: interpreter-type-system, Property 7: Inherent methods take priority over trait methods
    ///
    /// For any struct type T that has both an inherent impl method and a trait impl method
    /// registered with the same method name, invoking that method on a Value::Struct of type T
    /// SHALL execute the inherent impl body (not the trait impl body).
    #[test]
    fn prop_inherent_method_priority_over_trait(
        desc in arb_inherent_priority(),
    ) {
        // Build two impl blocks with the same method name:
        // 1. Inherent impl: returns inherent_return
        // 2. Trait impl: returns trait_return

        let inherent_method = TypedFnDef {
            name: desc.method_name.clone(),
            type_params: vec![],
            type_param_bounds: vec![],
            params: vec!["self".to_string()],
            param_types: vec![FluxType::Struct(desc.type_name.clone())],
            body: vec![TypedStmt::Return(TypedReturnStmt {
                value: Some(float_lit(desc.inherent_return)),
                span: Span::new(0, 0),
            })],
            return_type: FluxType::Float,
            span: Span::new(0, 0),
        };

        let trait_method = TypedFnDef {
            name: desc.method_name.clone(),
            type_params: vec![],
            type_param_bounds: vec![],
            params: vec!["self".to_string()],
            param_types: vec![FluxType::Struct(desc.type_name.clone())],
            body: vec![TypedStmt::Return(TypedReturnStmt {
                value: Some(float_lit(desc.trait_return)),
                span: Span::new(0, 0),
            })],
            return_type: FluxType::Float,
            span: Span::new(0, 0),
        };

        // Inherent impl block (no trait_name)
        let inherent_impl_block = TypedImplBlock {
            trait_name: None,
            target_type: desc.type_name.clone(),
            methods: vec![inherent_method],
            span: Span::new(0, 0),
        };

        // Trait impl block (has trait_name)
        let trait_impl_block = TypedImplBlock {
            trait_name: Some("SomeTrait".to_string()),
            target_type: desc.type_name.clone(),
            methods: vec![trait_method],
            span: Span::new(0, 0),
        };

        // Build a program with both impl blocks.
        // The interpreter registers inherent methods with insert() (always wins)
        // and trait methods with entry().or_insert() (only if absent).
        let program = TypedProgram {
            imports: vec![],
            structs: vec![],
            enums: vec![],
            functions: vec![],
            impl_blocks: vec![inherent_impl_block, trait_impl_block],
            traits: vec![],
            data_block: None,
            connector_block: None,
            strategy: TypedStrategy {
                name: "InherentPriorityTest".to_string(),
                body: vec![
                    TypedStrategyItem::EventHandler(TypedEventHandler {
                        event_name: "bar".to_string(),
                        body: vec![],
                        span: Span::new(0, 0),
                    }),
                ],
                span: Span::new(0, 0),
            },
            span: Span::new(0, 0),
        };

        let mut interp = Interpreter::new(&program);

        // Create a struct instance and invoke the conflicting method
        let struct_val = Value::Struct {
            type_name: desc.type_name.clone(),
            fields: StdHashMap::new(),
        };

        let mut locals = StdHashMap::new();
        locals.insert("instance".to_string(), struct_val);

        let method_call_expr = texpr(
            TypedExprKind::MethodCall {
                receiver: Box::new(texpr(
                    TypedExprKind::Ident("instance".to_string()),
                    FluxType::Struct(desc.type_name.clone()),
                )),
                method: desc.method_name.clone(),
                args: vec![],
            },
            FluxType::Float,
        );

        let result = interp.eval_expr(&method_call_expr, &locals).unwrap();

        match result {
            Value::Float(f) => {
                let tolerance = 1e-10 * desc.inherent_return.abs().max(1.0);
                prop_assert!(
                    (f - desc.inherent_return).abs() < tolerance,
                    "Inherent method should take priority.\n\
                     Expected inherent return: {}\n\
                     Got: {}\n\
                     Trait return would be: {}\n\
                     Type: {}, Method: {}",
                    desc.inherent_return,
                    f,
                    desc.trait_return,
                    desc.type_name,
                    desc.method_name,
                );
            }
            other => {
                prop_assert!(
                    false,
                    "Expected Float result from inherent method, got {:?}\n\
                     Type: {}, Method: {}",
                    other,
                    desc.type_name,
                    desc.method_name,
                );
            }
        }
    }
}


// =============================================================================
// Property 2: Enum variant construction preserves field data
// Feature: interpreter-type-system, Property 2: Enum variant construction preserves field data
// =============================================================================

/// Describes a generated enum variant for the construction property test.
#[derive(Debug, Clone)]
struct EnumConstructionTestCase {
    enum_name: String,
    variant_name: String,
    /// Named fields with their values (0-4 fields for data variants)
    fields: Vec<(String, Value)>,
}

/// Strategy that generates a unit enum variant (0 fields) for construction testing.
fn arb_unit_construction() -> impl Strategy<Value = EnumConstructionTestCase> {
    (arb_flux_ident(), arb_flux_ident()).prop_map(|(enum_name, variant_name)| {
        EnumConstructionTestCase {
            enum_name,
            variant_name,
            fields: vec![],
        }
    })
}

/// Strategy that generates a data enum variant (1-4 named fields) for construction testing.
fn arb_data_construction() -> impl Strategy<Value = EnumConstructionTestCase> {
    (
        arb_flux_ident(),
        arb_flux_ident(),
        prop::collection::vec((arb_field_name(), arb_simple_value()), 1..=4),
    )
        .prop_map(|(enum_name, variant_name, fields)| EnumConstructionTestCase {
            enum_name,
            variant_name,
            fields,
        })
}

/// Strategy that generates either a unit or data enum variant for construction testing.
fn arb_enum_construction_case() -> impl Strategy<Value = EnumConstructionTestCase> {
    prop_oneof![arb_unit_construction(), arb_data_construction()]
}

/// Build the TypedExpr arg literal for a given Value.
fn value_to_typed_expr(val: &Value) -> TypedExpr {
    match val {
        Value::Float(f) => texpr(TypedExprKind::FloatLiteral(*f), FluxType::Float),
        Value::Int(i) => texpr(TypedExprKind::IntLiteral(*i), FluxType::Int),
        Value::Bool(b) => texpr(TypedExprKind::BoolLiteral(*b), FluxType::Bool),
        Value::Str(s) => texpr(TypedExprKind::StringLiteral(s.clone()), FluxType::String),
        _ => texpr(TypedExprKind::FloatLiteral(0.0), FluxType::Float),
    }
}

/// Compare two Values for equality (used in property assertions for enum construction).
fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Float(x), Value::Float(y)) => (x - y).abs() < 1e-10,
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Null, Value::Null) => true,
        _ => false,
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

    /// **Validates: Requirements 5.1, 5.2, 5.3**
    ///
    /// Feature: interpreter-type-system, Property 2: Enum variant construction preserves field data
    ///
    /// For any enum definition with named fields, and any valid argument values,
    /// evaluating an `EnumConstruction` expression SHALL produce an Enum_Value whose
    /// `fields` list contains exactly the provided argument values paired with the
    /// correct field names from the enum definition (or an empty fields list for
    /// unit variants with zero arguments).
    #[test]
    fn prop_enum_variant_construction_preserves_field_data(
        tc in arb_enum_construction_case(),
    ) {
        // Build the enum definition with the variant's named fields
        let variant_def = TypedEnumVariant {
            name: tc.variant_name.clone(),
            fields: tc.fields.iter().map(|(name, val)| {
                let ty = match val {
                    Value::Float(_) => FluxType::Float,
                    Value::Int(_) => FluxType::Int,
                    Value::Bool(_) => FluxType::Bool,
                    Value::Str(_) => FluxType::String,
                    _ => FluxType::Float,
                };
                (name.clone(), ty)
            }).collect(),
            span: Span::new(0, 0),
        };

        let enum_def = TypedEnumDef {
            name: tc.enum_name.clone(),
            type_params: vec![],
            variants: vec![variant_def],
            span: Span::new(0, 0),
        };

        // Build the EnumConstruction TypedExpr with argument expressions
        let args: Vec<TypedExpr> = tc.fields.iter().map(|(_name, val)| {
            value_to_typed_expr(val)
        }).collect();

        let enum_construction_expr = texpr(
            TypedExprKind::EnumConstruction {
                enum_name: tc.enum_name.clone(),
                variant_name: tc.variant_name.clone(),
                args,
            },
            FluxType::Enum(tc.enum_name.clone()),
        );

        // Build a minimal program with the enum def registered
        let program = TypedProgram {
            imports: vec![],
            structs: vec![],
            enums: vec![enum_def],
            functions: vec![],
            impl_blocks: vec![],
            traits: vec![],
            data_block: None,
            connector_block: None,
            strategy: TypedStrategy {
                name: "EnumConstructionTest".to_string(),
                body: vec![
                    TypedStrategyItem::EventHandler(TypedEventHandler {
                        event_name: "bar".to_string(),
                        body: vec![],
                        span: Span::new(0, 0),
                    }),
                ],
                span: Span::new(0, 0),
            },
            span: Span::new(0, 0),
        };

        let mut interp = Interpreter::new(&program);
        let locals = StdHashMap::new();

        // Evaluate the EnumConstruction expression
        let result = interp.eval_expr(&enum_construction_expr, &locals).unwrap();

        // Assert the result is a Value::Enum with correct structure
        match result {
            Value::Enum { enum_name, variant_name, fields } => {
                // Check enum_name matches
                prop_assert_eq!(
                    &enum_name, &tc.enum_name,
                    "Enum name mismatch: expected '{}', got '{}'",
                    tc.enum_name, enum_name,
                );

                // Check variant_name matches
                prop_assert_eq!(
                    &variant_name, &tc.variant_name,
                    "Variant name mismatch: expected '{}', got '{}'",
                    tc.variant_name, variant_name,
                );

                // Check field count matches
                prop_assert_eq!(
                    fields.len(), tc.fields.len(),
                    "Field count mismatch: expected {}, got {}\nExpected fields: {:?}\nGot fields: {:?}",
                    tc.fields.len(), fields.len(), tc.fields, fields,
                );

                // Check each field: name must match and value must match
                for (i, ((expected_name, expected_val), (actual_name, actual_val))) in
                    tc.fields.iter().zip(fields.iter()).enumerate()
                {
                    prop_assert_eq!(
                        actual_name, expected_name,
                        "Field {} name mismatch: expected '{}', got '{}'\nAll fields: {:?}",
                        i, expected_name, actual_name, fields,
                    );

                    prop_assert!(
                        values_equal(expected_val, actual_val),
                        "Field {} ('{}') value mismatch: expected {:?}, got {:?}",
                        i, expected_name, expected_val, actual_val,
                    );
                }
            }
            other => {
                prop_assert!(
                    false,
                    "Expected Value::Enum, got {:?}\nTest case: {:?}",
                    other, tc,
                );
            }
        }
    }
}


// =============================================================================
// Property 3: Match routes to the correct arm and binds fields
// Feature: interpreter-type-system, Property 3: Match routes to the correct arm and binds fields
// =============================================================================

/// Describes a match test case where we verify:
/// 1. The correct arm is selected (routing)
/// 2. ALL bound variables have correct values (not just the first)
#[derive(Debug, Clone)]
struct MatchRoutingTestCase {
    enum_desc: EnumDesc,
    construction: ConstructionDesc,
}

/// Strategy to generate a match routing test case.
/// Generates an enum with 2-4 variants (each with 1-3 fields to ensure bindings are tested),
/// then selects one variant to construct.
fn arb_match_routing_test_case() -> BoxedStrategy<MatchRoutingTestCase> {
    // Generate 2-4 variants, each with 1-3 fields (at least 1 field to verify bindings)
    prop::collection::vec(1usize..=3, 2..=4)
        .prop_flat_map(|field_counts| {
            let num_variants = field_counts.len();
            let variants: Vec<VariantDesc> = field_counts
                .iter()
                .enumerate()
                .map(|(i, &fc)| VariantDesc {
                    name: format!("Variant{}", i),
                    field_count: fc,
                })
                .collect();
            let enum_desc = EnumDesc {
                name: "MatchEnum".to_string(),
                variants,
            };

            // Pick a variant index and generate field values for it
            let fcs = field_counts.clone();
            (Just(enum_desc), 0..num_variants, Just(fcs))
        })
        .prop_flat_map(|(enum_desc, variant_idx, fcs)| {
            let fc = fcs[variant_idx];
            let values = prop::collection::vec(-500.0f64..500.0f64, fc..=fc);
            (Just(enum_desc), Just(variant_idx), values)
        })
        .prop_map(|(enum_desc, variant_index, field_values)| MatchRoutingTestCase {
            enum_desc,
            construction: ConstructionDesc {
                variant_index,
                field_values,
            },
        })
        .boxed()
}

/// Build a TypedProgram for the match routing property test.
///
/// This program:
/// 1. Defines an enum with the generated variants
/// 2. Constructs a value of the selected variant
/// 3. Matches on it, where each arm computes a unique value from ALL bindings:
///    result = variant_index * 1000.0 + sum(b_i * (i+1))
///    This ensures we can verify both correct arm routing AND correct binding of ALL fields.
/// 4. A wildcard arm returns -9999.0 (should never match)
fn build_match_routing_program(tc: &MatchRoutingTestCase) -> TypedProgram {
    use flux_compiler::parser::ast::BinOp;

    let enum_desc = &tc.enum_desc;
    let construction = &tc.construction;

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

    // Build match arms: one per variant
    let mut arms: Vec<TypedMatchArm> = Vec::new();
    for (i, variant) in enum_desc.variants.iter().enumerate() {
        let base_value = (i as f64) * 1000.0;

        // Bindings: b0, b1, b2, ...
        let bindings: Vec<(String, FluxType)> = (0..variant.field_count)
            .map(|j| (format!("b{}", j), FluxType::Float))
            .collect();

        // Arm body computes: base_value + b0*1 + b1*2 + b2*3 + ...
        // This formula uses ALL bindings so we can verify each one is correct
        let mut result_expr = float_lit(base_value);
        for j in 0..variant.field_count {
            let weight = (j + 1) as f64;
            let weighted_binding = texpr(
                TypedExprKind::BinaryOp {
                    left: Box::new(texpr(
                        TypedExprKind::Ident(format!("b{}", j)),
                        FluxType::Float,
                    )),
                    op: BinOp::Mul,
                    right: Box::new(float_lit(weight)),
                },
                FluxType::Float,
            );
            result_expr = texpr(
                TypedExprKind::BinaryOp {
                    left: Box::new(result_expr),
                    op: BinOp::Add,
                    right: Box::new(weighted_binding),
                },
                FluxType::Float,
            );
        }

        let body = vec![expr_stmt(result_expr)];

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

    // Wildcard arm (should never match since all variants are covered)
    arms.push(TypedMatchArm {
        pattern: TypedPattern::Wildcard {
            span: Span::new(0, 0),
        },
        body: vec![expr_stmt(float_lit(-9999.0))],
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

    // on_bar body: result = match enum_val { ... }
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
            name: "MatchRoutingTest".to_string(),
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

/// Compute the expected result for the match routing test.
///
/// Formula: variant_index * 1000.0 + sum(field_values[j] * (j+1))
/// This validates both correct arm routing (via the base) and correct binding
/// of ALL field values (via the weighted sum).
fn expected_match_routing_result(tc: &MatchRoutingTestCase) -> f64 {
    let base = (tc.construction.variant_index as f64) * 1000.0;
    let binding_sum: f64 = tc
        .construction
        .field_values
        .iter()
        .enumerate()
        .map(|(j, &v)| v * ((j + 1) as f64))
        .sum();
    base + binding_sum
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

    /// **Validates: Requirements 6.1, 6.2, 6.5**
    ///
    /// Feature: interpreter-type-system, Property 3: Match routes to the correct arm and binds fields
    ///
    /// For any enum value with variant name V and a match expression containing an arm
    /// whose pattern matches V with N bindings, the interpreter SHALL execute exactly
    /// that arm's body and SHALL make each binding variable available in the arm body
    /// scope with the value equal to the positionally-corresponding field from the enum value.
    #[test]
    fn prop_match_routes_to_correct_arm_and_binds_fields(
        tc in arb_match_routing_test_case(),
    ) {
        let program = build_match_routing_program(&tc);
        let mut interp = Interpreter::new(&program);
        let ctx = test_bar();

        interp.on_bar(&ctx);

        let result = match interp.state.get("result") {
            Some(Value::Float(f)) => *f,
            other => panic!(
                "Expected Float in state 'result', got {:?}\n\
                 enum: {:?}\nconstruction: {:?}",
                other, tc.enum_desc, tc.construction
            ),
        };

        let expected = expected_match_routing_result(&tc);
        let tolerance = 1e-10 * expected.abs().max(1.0);

        prop_assert!(
            (result - expected).abs() < tolerance,
            "Match routing or binding incorrect:\n\
             expected={}, got={}\n\
             variant_index={}, field_values={:?}\n\
             Formula: base({}) + weighted_sum({})\n\
             This verifies:\n\
             - Correct arm selected (base = variant_index * 1000)\n\
             - All bindings correct (each b_i * (i+1) contributes to sum)\n\
             enum_desc={:?}",
            expected,
            result,
            tc.construction.variant_index,
            tc.construction.field_values,
            (tc.construction.variant_index as f64) * 1000.0,
            tc.construction.field_values.iter().enumerate()
                .map(|(j, v)| v * ((j + 1) as f64)).sum::<f64>(),
            tc.enum_desc,
        );
    }
}


// =============================================================================
// Property 1: Struct literal field-access round-trip
// Feature: interpreter-type-system, Property 1: Struct literal field-access round-trip
// =============================================================================

/// Describes a field with its name, expression-building info, and expected Value.
#[derive(Debug, Clone)]
struct StructFieldDesc {
    name: String,
    value: Value,
    expr: TypedExpr,
}

/// Strategy to generate a unique set of lowercase field names (1-8 fields).
fn arb_struct_field_names_unique() -> impl Strategy<Value = Vec<String>> {
    prop::collection::hash_set("[a-z][a-z0-9]{0,6}".prop_map(|s| s), 1..=8)
        .prop_map(|set| set.into_iter().collect::<Vec<_>>())
}

/// Strategy to generate a StructFieldDesc with a random value type (Float, Int, Bool, Str).
fn arb_struct_field_desc(name: String) -> BoxedStrategy<StructFieldDesc> {
    let name1 = name.clone();
    let name2 = name.clone();
    let name3 = name.clone();
    let name4 = name;
    prop_oneof![
        // Float field
        (-1000.0f64..1000.0f64).prop_map(move |f| StructFieldDesc {
            name: name1.clone(),
            value: Value::Float(f),
            expr: texpr(TypedExprKind::FloatLiteral(f), FluxType::Float),
        }),
        // Int field
        (-1000i64..1000i64).prop_map(move |i| StructFieldDesc {
            name: name2.clone(),
            value: Value::Int(i),
            expr: texpr(TypedExprKind::IntLiteral(i), FluxType::Int),
        }),
        // Bool field
        any::<bool>().prop_map(move |b| StructFieldDesc {
            name: name3.clone(),
            value: Value::Bool(b),
            expr: texpr(TypedExprKind::BoolLiteral(b), FluxType::Bool),
        }),
        // Str field
        "[a-z]{1,10}".prop_map(move |s| StructFieldDesc {
            name: name4.clone(),
            value: Value::Str(s.clone()),
            expr: texpr(TypedExprKind::StringLiteral(s), FluxType::String),
        }),
    ]
    .boxed()
}

/// Strategy to generate a complete struct round-trip test case:
/// a random type name, 1-8 unique field names, and random values for each.
fn arb_struct_roundtrip_case() -> impl Strategy<Value = (String, Vec<StructFieldDesc>)> {
    (
        "[A-Z][a-z]{2,8}".prop_map(|s| s),
        arb_struct_field_names_unique(),
    )
        .prop_flat_map(|(type_name, field_names)| {
            let field_strategies: Vec<_> = field_names
                .into_iter()
                .map(|name| arb_struct_field_desc(name).boxed())
                .collect();
            (Just(type_name), field_strategies)
        })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

    /// **Validates: Requirements 1.1, 1.2, 2.1**
    ///
    /// Feature: interpreter-type-system, Property 1: Struct literal field-access round-trip
    ///
    /// For any set of field names and values (strings, integers, floats, booleans),
    /// evaluating a StructLiteral expression to produce a Value::Struct and then
    /// evaluating a MemberAccess on each field name SHALL return the exact value
    /// that was used to construct that field.
    #[test]
    fn prop_struct_literal_field_access_round_trip(
        (type_name, fields) in arb_struct_roundtrip_case(),
    ) {
        // Build a minimal interpreter
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
                name: "StructRoundTripTest".to_string(),
                body: vec![
                    TypedStrategyItem::EventHandler(TypedEventHandler {
                        event_name: "bar".to_string(),
                        body: vec![],
                        span: Span::new(0, 0),
                    }),
                ],
                span: Span::new(0, 0),
            },
            span: Span::new(0, 0),
        };
        let mut interp = Interpreter::new(&program);

        // Step 1: Build and evaluate a StructLiteral expression
        let struct_literal_fields: Vec<(String, TypedExpr)> = fields
            .iter()
            .map(|fd| (fd.name.clone(), fd.expr.clone()))
            .collect();

        let struct_literal_expr = texpr(
            TypedExprKind::StructLiteral {
                struct_name: type_name.clone(),
                fields: struct_literal_fields,
            },
            FluxType::Struct(type_name.clone()),
        );

        let locals = StdHashMap::new();
        let struct_val = interp.eval_expr(&struct_literal_expr, &locals)
            .expect("StructLiteral evaluation should succeed");

        // Verify it produced a Value::Struct
        match &struct_val {
            Value::Struct { type_name: tn, fields: _ } => {
                prop_assert_eq!(
                    tn, &type_name,
                    "Struct type_name mismatch: expected '{}', got '{}'",
                    type_name, tn,
                );
            }
            other => {
                prop_assert!(
                    false,
                    "Expected Value::Struct, got {:?}",
                    other,
                );
            }
        }

        // Step 2: For each field, build a MemberAccess expression and verify
        // the returned value matches the original
        let mut locals_with_obj = StdHashMap::new();
        locals_with_obj.insert("__struct_obj".to_string(), struct_val);

        for field_desc in &fields {
            let member_access_expr = texpr(
                TypedExprKind::MemberAccess {
                    object: Box::new(texpr(
                        TypedExprKind::Ident("__struct_obj".to_string()),
                        FluxType::Struct(type_name.clone()),
                    )),
                    field: field_desc.name.clone(),
                },
                FluxType::Float, // resolved_type doesn't matter for runtime
            );

            let result = interp.eval_expr(&member_access_expr, &locals_with_obj)
                .unwrap_or_else(|e| {
                    panic!(
                        "MemberAccess on field '{}' should succeed, got error: {}\n\
                         type_name: {}, fields: {:?}",
                        field_desc.name, e, type_name, fields
                    )
                });

            prop_assert!(
                values_equal(&result, &field_desc.value),
                "Field '{}' round-trip failed:\n\
                 expected: {:?}\n\
                 got: {:?}\n\
                 type_name: {}\n\
                 all fields: {:?}",
                field_desc.name,
                field_desc.value,
                result,
                type_name,
                fields.iter().map(|f| (&f.name, &f.value)).collect::<Vec<_>>(),
            );
        }
    }
}
