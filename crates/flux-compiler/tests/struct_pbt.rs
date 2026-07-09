//! Property-based tests for struct literal field completeness and type correctness.
//!
//! Feature: flux-structs, Property 2: Struct literal field completeness and type correctness
//!
//! **Validates: Requirements 2.2, 2.3, 2.4, 2.5**
//!
//! For any struct definition with fields F and any struct literal expression,
//! the literal type-checks successfully if and only if it provides exactly the
//! fields in F (no missing, no extra, no duplicates) with each value's type
//! matching or being assignable to the declared field type.

use flux_compiler::error::CompileError;
use flux_compiler::lexer::lex_with_spans;
use flux_compiler::parser::parse;
use flux_compiler::typeck::check;
use proptest::prelude::*;

// ============================================================================
// Generators
// ============================================================================

/// The supported scalar field types for struct definitions.
#[derive(Debug, Clone, Copy, PartialEq)]
enum FieldType {
    F64,
    Int,
    Bool,
}

impl FieldType {
    /// Returns the Flux type annotation string.
    fn type_str(&self) -> &'static str {
        match self {
            FieldType::F64 => "f64",
            FieldType::Int => "int",
            FieldType::Bool => "bool",
        }
    }

    /// Returns a valid literal value string for this type.
    fn valid_literal(&self, seed: u32) -> String {
        match self {
            FieldType::F64 => format!("{}.{}", seed % 1000, seed % 100),
            FieldType::Int => format!("{}", seed as i64),
            FieldType::Bool => if seed % 2 == 0 { "true".to_string() } else { "false".to_string() },
        }
    }

    /// Returns a literal value string of a DIFFERENT type (for mismatch testing).
    fn wrong_literal(&self) -> &'static str {
        match self {
            FieldType::F64 => "\"wrong\"",    // String where f64 expected
            FieldType::Int => "\"wrong\"",     // String where int expected
            FieldType::Bool => "\"wrong\"",    // String where bool expected
        }
    }
}

/// Generate a random FieldType.
fn arb_field_type() -> impl Strategy<Value = FieldType> {
    prop_oneof![
        Just(FieldType::F64),
        Just(FieldType::Int),
        Just(FieldType::Bool),
    ]
}

/// A struct field definition: name + type.
#[derive(Debug, Clone)]
struct FieldDef {
    name: String,
    field_type: FieldType,
}

/// Reserved keywords in Flux that cannot be used as field names.
const FLUX_RESERVED: &[&str] = &[
    "strategy", "params", "state", "on", "if", "elif", "else", "for", "while",
    "return", "fn", "from", "import", "and", "or", "not", "true", "false", "null",
    "data", "connector", "struct", "bar", "in", "f64", "int", "bool", "str",
];

/// Generate a valid field name (lowercase letters, 3-8 chars, not a reserved keyword).
fn arb_field_name() -> impl Strategy<Value = String> {
    "[a-z]{3,8}".prop_filter("must not be a reserved keyword", |name| {
        !FLUX_RESERVED.contains(&name.as_str())
    })
}

/// Generate a struct definition with 1-5 fields, all with unique names.
fn arb_struct_def() -> impl Strategy<Value = Vec<FieldDef>> {
    proptest::collection::vec((arb_field_name(), arb_field_type()), 1..=5)
        .prop_filter("fields must have unique names", |fields| {
            let names: std::collections::HashSet<&str> =
                fields.iter().map(|(n, _)| n.as_str()).collect();
            names.len() == fields.len()
        })
        .prop_map(|fields| {
            fields
                .into_iter()
                .map(|(name, field_type)| FieldDef { name, field_type })
                .collect()
        })
}

/// Scenario for struct literal generation.
#[derive(Debug, Clone)]
enum LiteralScenario {
    /// All fields provided with correct types — should pass.
    Complete,
    /// One or more fields missing — should fail with "missing fields".
    MissingFields { indices_to_remove: Vec<usize> },
    /// An extra field that doesn't exist on the struct — should fail with "has no field".
    ExtraField { extra_name: String },
    /// One field has the wrong type — should fail with type mismatch.
    WrongType { field_index: usize },
}

/// Generate a literal scenario for a given struct definition.
fn arb_scenario(field_count: usize) -> impl Strategy<Value = LiteralScenario> {
    prop_oneof![
        // Complete and correct
        3 => Just(LiteralScenario::Complete),
        // Missing 1 or more fields
        3 => proptest::collection::vec(
            0..field_count,
            1..=field_count.max(1),
        )
        .prop_map(|indices| {
            // Deduplicate
            let unique: Vec<usize> = indices
                .into_iter()
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();
            LiteralScenario::MissingFields { indices_to_remove: unique }
        }),
        // Extra field
        2 => "[a-z]{2,8}".prop_map(|extra_name| LiteralScenario::ExtraField {
            extra_name,
        }),
        // Wrong type on one field
        2 => (0..field_count).prop_map(|idx| LiteralScenario::WrongType {
            field_index: idx,
        }),
    ]
}

// ============================================================================
// Source construction helpers
// ============================================================================

/// Build a Flux source string with a struct definition and a struct literal
/// in a strategy body.
fn build_source(fields: &[FieldDef], scenario: &LiteralScenario) -> String {
    let struct_name = "TestStruct";

    // Build struct definition
    let field_defs: Vec<String> = fields
        .iter()
        .map(|f| format!("    {}: {}", f.name, f.field_type.type_str()))
        .collect();
    let struct_def = format!(
        "struct {} {{\n{}\n}}",
        struct_name,
        field_defs.join(",\n")
    );

    // Build struct literal based on scenario
    let literal_fields: Vec<String> = match scenario {
        LiteralScenario::Complete => fields
            .iter()
            .enumerate()
            .map(|(i, f)| format!("{} = {}", f.name, f.field_type.valid_literal(i as u32 + 1)))
            .collect(),

        LiteralScenario::MissingFields { indices_to_remove } => fields
            .iter()
            .enumerate()
            .filter(|(i, _)| !indices_to_remove.contains(i))
            .map(|(i, f)| format!("{} = {}", f.name, f.field_type.valid_literal(i as u32 + 1)))
            .collect(),

        LiteralScenario::ExtraField { extra_name } => {
            let mut lit_fields: Vec<String> = fields
                .iter()
                .enumerate()
                .map(|(i, f)| format!("{} = {}", f.name, f.field_type.valid_literal(i as u32 + 1)))
                .collect();
            lit_fields.push(format!("{} = 99.0", extra_name));
            lit_fields
        }

        LiteralScenario::WrongType { field_index } => fields
            .iter()
            .enumerate()
            .map(|(i, f)| {
                if i == *field_index {
                    format!("{} = {}", f.name, f.field_type.wrong_literal())
                } else {
                    format!("{} = {}", f.name, f.field_type.valid_literal(i as u32 + 1))
                }
            })
            .collect(),
    };

    let literal_str = format!(
        "{} {{ {} }}",
        struct_name,
        literal_fields.join(", ")
    );

    format!(
        "{}\n\nstrategy Test {{\n    on bar {{\n        s = {}\n    }}\n}}\n",
        struct_def, literal_str
    )
}

// ============================================================================
// Helper
// ============================================================================

/// Lex, parse, and typecheck a Flux source string.
fn typecheck_source(source: &str) -> Result<(), CompileError> {
    let tokens = lex_with_spans(source)?;
    let ast = parse(tokens)?;
    check(ast)?;
    Ok(())
}

// ============================================================================
// Property Tests
// ============================================================================

// Feature: flux-structs, Property 2: Struct literal field completeness and type correctness
// **Validates: Requirements 2.2, 2.3, 2.4, 2.5**
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Property: A struct literal providing all fields with correct types passes type-checking.
    #[test]
    fn prop_complete_correct_literal_passes(fields in arb_struct_def()) {
        let scenario = LiteralScenario::Complete;
        let source = build_source(&fields, &scenario);

        let result = typecheck_source(&source);
        prop_assert!(
            result.is_ok(),
            "Complete correct literal should pass type-checking.\nSource:\n{}\nError: {:?}",
            source,
            result.err()
        );
    }

    /// Property: A struct literal missing one or more fields fails with "missing fields" error.
    #[test]
    fn prop_missing_fields_literal_fails(
        fields in arb_struct_def(),
        indices_to_remove in proptest::collection::vec(0usize..5, 1..=3),
    ) {
        // Clamp indices to valid range and deduplicate
        let valid_indices: Vec<usize> = indices_to_remove
            .into_iter()
            .filter(|&i| i < fields.len())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        // Skip if no valid indices to remove (nothing to test)
        prop_assume!(!valid_indices.is_empty());
        // Skip if we'd remove ALL fields — that's still "missing fields" but edge case
        // where the literal is `TestStruct { }` which is also valid to test
        let scenario = LiteralScenario::MissingFields { indices_to_remove: valid_indices.clone() };
        let source = build_source(&fields, &scenario);

        let result = typecheck_source(&source);
        prop_assert!(
            result.is_err(),
            "Literal missing fields should fail type-checking.\nSource:\n{}\nRemoved indices: {:?}",
            source,
            valid_indices
        );

        let err = result.unwrap_err();
        let msg = err.to_string();
        prop_assert!(
            msg.contains("missing fields"),
            "Error should mention 'missing fields', got: {}\nSource:\n{}",
            msg,
            source
        );

        // Verify that at least one removed field name appears in the error message
        let any_name_mentioned = valid_indices.iter().any(|&i| msg.contains(&fields[i].name));
        prop_assert!(
            any_name_mentioned,
            "Error should mention at least one missing field name.\nError: {}\nMissing: {:?}",
            msg,
            valid_indices.iter().map(|&i| &fields[i].name).collect::<Vec<_>>()
        );
    }

    /// Property: A struct literal with an extra field not in the definition fails with
    /// "has no field" error.
    #[test]
    fn prop_extra_field_literal_fails(
        fields in arb_struct_def(),
        extra_name in "[a-z]{3,8}",
    ) {
        // Make sure the extra field name doesn't collide with existing field names or keywords
        let existing_names: std::collections::HashSet<&str> =
            fields.iter().map(|f| f.name.as_str()).collect();
        prop_assume!(!existing_names.contains(extra_name.as_str()));
        prop_assume!(!FLUX_RESERVED.contains(&extra_name.as_str()));

        let scenario = LiteralScenario::ExtraField { extra_name: extra_name.clone() };
        let source = build_source(&fields, &scenario);

        let result = typecheck_source(&source);
        prop_assert!(
            result.is_err(),
            "Literal with extra field should fail type-checking.\nSource:\n{}",
            source
        );

        let err = result.unwrap_err();
        let msg = err.to_string();
        prop_assert!(
            msg.contains("has no field") && msg.contains(&extra_name),
            "Error should mention 'has no field' and the extra field name '{}'.\nGot: {}\nSource:\n{}",
            extra_name,
            msg,
            source
        );
    }

    /// Property: A struct literal with a field value of the wrong type fails with type mismatch.
    #[test]
    fn prop_wrong_type_literal_fails(
        fields in arb_struct_def(),
        field_index_raw in 0usize..5,
    ) {
        let field_index = field_index_raw % fields.len();
        let scenario = LiteralScenario::WrongType { field_index };
        let source = build_source(&fields, &scenario);

        let result = typecheck_source(&source);
        prop_assert!(
            result.is_err(),
            "Literal with wrong-type field should fail type-checking.\nSource:\n{}",
            source
        );

        let err = result.unwrap_err();
        let msg = err.to_string();
        // The error should mention the field name and expected/actual type
        prop_assert!(
            msg.contains(&format!("field '{}'", fields[field_index].name))
                || msg.contains("has no field")
                || msg.contains("type"),
            "Error should mention the problematic field '{}' or type issue.\nGot: {}\nSource:\n{}",
            fields[field_index].name,
            msg,
            source
        );
    }
}


// ============================================================================
// Property 3: Field access type resolution
// ============================================================================
//
// Feature: flux-structs, Property 3: Field access type resolution
//
// **Validates: Requirements 3.1, 3.2**
//
// For any expression of struct type S and any field name, dot-access type-checks
// to the declared field type if the field exists in S, or produces an invalid-field
// error if the field does not exist in S.

/// Build a Flux source that defines a struct, constructs it, and accesses a field.
/// If `access_field` is one of the struct's fields, it should pass.
/// Otherwise, it should fail with "has no field" and "Available:".
fn build_field_access_source(fields: &[FieldDef], access_field: &str) -> String {
    let struct_name = "TestStruct";

    let field_defs: Vec<String> = fields
        .iter()
        .map(|f| format!("    {}: {}", f.name, f.field_type.type_str()))
        .collect();
    let struct_def = format!(
        "struct {} {{\n{}\n}}",
        struct_name,
        field_defs.join(",\n")
    );

    // Construct a complete literal
    let literal_fields: Vec<String> = fields
        .iter()
        .enumerate()
        .map(|(i, f)| format!("{} = {}", f.name, f.field_type.valid_literal(i as u32 + 1)))
        .collect();
    let literal_str = format!(
        "{} {{ {} }}",
        struct_name,
        literal_fields.join(", ")
    );

    format!(
        "{}\n\nstrategy Test {{\n    on bar {{\n        s = {}\n        x = s.{}\n    }}\n}}\n",
        struct_def, literal_str, access_field
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Property 3a: Accessing a valid field on a struct resolves to the declared field type.
    #[test]
    fn prop_valid_field_access_passes(
        fields in arb_struct_def(),
        field_index_raw in 0usize..5,
    ) {
        let field_index = field_index_raw % fields.len();
        let access_field = fields[field_index].name.clone();
        let source = build_field_access_source(&fields, &access_field);

        let result = typecheck_source(&source);
        prop_assert!(
            result.is_ok(),
            "Accessing valid field '{}' should pass type-checking.\nSource:\n{}\nError: {:?}",
            access_field,
            source,
            result.err()
        );
    }

    /// Property 3b: Accessing a non-existent field produces "has no field" and "Available:" error.
    #[test]
    fn prop_invalid_field_access_fails(
        fields in arb_struct_def(),
        bad_field in "[a-z]{3,8}",
    ) {
        // Ensure the bad field name doesn't collide with existing fields or reserved words
        let existing_names: std::collections::HashSet<&str> =
            fields.iter().map(|f| f.name.as_str()).collect();
        prop_assume!(!existing_names.contains(bad_field.as_str()));
        prop_assume!(!FLUX_RESERVED.contains(&bad_field.as_str()));

        let source = build_field_access_source(&fields, &bad_field);

        let result = typecheck_source(&source);
        prop_assert!(
            result.is_err(),
            "Accessing non-existent field '{}' should fail type-checking.\nSource:\n{}",
            bad_field,
            source
        );

        let err = result.unwrap_err();
        let msg = err.to_string();
        prop_assert!(
            msg.contains("has no field") && msg.contains(&bad_field),
            "Error should mention 'has no field' and field name '{}', got: {}\nSource:\n{}",
            bad_field,
            msg,
            source
        );
        prop_assert!(
            msg.contains("Available:"),
            "Error should list available fields with 'Available:', got: {}\nSource:\n{}",
            msg,
            source
        );

        // At least one actual field name should be mentioned in the available list
        let any_field_listed = fields.iter().any(|f| msg.contains(&f.name));
        prop_assert!(
            any_field_listed,
            "Error should list at least one actual field name in Available.\nError: {}\nFields: {:?}",
            msg,
            fields.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
    }
}

// ============================================================================
// Property 4: Duplicate field name detection
// ============================================================================
//
// Feature: flux-structs, Property 4: Duplicate field name detection
//
// **Validates: Requirements 1.6**
//
// For any struct definition containing two or more fields with the same name,
// the type-checker SHALL report a duplicate-field error indicating the repeated name.

/// Build a Flux source with a struct that has a duplicated field name.
fn build_duplicate_field_source(
    unique_fields: &[FieldDef],
    dup_index: usize,
) -> String {
    let struct_name = "DupStruct";

    // Build fields with a duplicate: all unique fields plus a repeat of one
    let mut field_defs: Vec<String> = unique_fields
        .iter()
        .map(|f| format!("    {}: {}", f.name, f.field_type.type_str()))
        .collect();
    // Add the duplicate at the end
    let dup_field = &unique_fields[dup_index];
    field_defs.push(format!("    {}: {}", dup_field.name, dup_field.field_type.type_str()));

    let struct_def = format!(
        "struct {} {{\n{}\n}}",
        struct_name,
        field_defs.join(",\n")
    );

    format!(
        "{}\n\nstrategy Test {{\n    on bar {{\n        x = 1.0\n    }}\n}}\n",
        struct_def
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Property 4: A struct with duplicate field names is rejected with "duplicate field" error.
    #[test]
    fn prop_duplicate_field_name_detected(
        fields in arb_struct_def(),
        dup_index_raw in 0usize..5,
    ) {
        let dup_index = dup_index_raw % fields.len();
        let dup_name = fields[dup_index].name.clone();
        let source = build_duplicate_field_source(&fields, dup_index);

        let result = typecheck_source(&source);
        prop_assert!(
            result.is_err(),
            "Struct with duplicate field '{}' should fail type-checking.\nSource:\n{}",
            dup_name,
            source
        );

        let err = result.unwrap_err();
        let msg = err.to_string();
        prop_assert!(
            msg.contains("duplicate field") && msg.contains(&dup_name),
            "Error should mention 'duplicate field' and the name '{}', got: {}\nSource:\n{}",
            dup_name,
            msg,
            source
        );
    }
}

// ============================================================================
// Property 5: Undefined struct type reference detection
// ============================================================================
//
// Feature: flux-structs, Property 5: Undefined struct type reference detection
//
// **Validates: Requirements 1.7, 20.4**
//
// For any struct definition containing a field whose type references a name not
// defined as a struct in scope, the type-checker SHALL report an undefined-type error.

/// Generate a type name that is NOT a builtin and NOT a defined struct.
fn arb_undefined_type_name() -> impl Strategy<Value = String> {
    "[A-Z][a-z]{3,7}".prop_filter("must not be a builtin type name", |name| {
        !["F64", "Int", "Bool", "Str"].contains(&name.as_str())
    })
}

/// Build a Flux source with a struct referencing an undefined type.
fn build_undefined_type_source(
    valid_fields: &[FieldDef],
    undefined_type: &str,
    bad_field_name: &str,
) -> String {
    let struct_name = "ContainerStruct";

    let mut field_defs: Vec<String> = valid_fields
        .iter()
        .map(|f| format!("    {}: {}", f.name, f.field_type.type_str()))
        .collect();
    // Add a field referencing the undefined type
    field_defs.push(format!("    {}: {}", bad_field_name, undefined_type));

    let struct_def = format!(
        "struct {} {{\n{}\n}}",
        struct_name,
        field_defs.join(",\n")
    );

    format!(
        "{}\n\nstrategy Test {{\n    on bar {{\n        x = 1.0\n    }}\n}}\n",
        struct_def
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Property 5: A struct with a field referencing an undefined type is rejected
    /// with "unknown type" error.
    #[test]
    fn prop_undefined_struct_type_reference_detected(
        fields in proptest::collection::vec((arb_field_name(), arb_field_type()), 0..=3)
            .prop_filter("fields must have unique names", |fields| {
                let names: std::collections::HashSet<&str> =
                    fields.iter().map(|(n, _)| n.as_str()).collect();
                names.len() == fields.len()
            })
            .prop_map(|fields| {
                fields.into_iter()
                    .map(|(name, field_type)| FieldDef { name, field_type })
                    .collect::<Vec<_>>()
            }),
        undefined_type in arb_undefined_type_name(),
        bad_field_name in arb_field_name(),
    ) {
        // Ensure the bad field name doesn't collide with existing fields
        let existing_names: std::collections::HashSet<&str> =
            fields.iter().map(|f| f.name.as_str()).collect();
        prop_assume!(!existing_names.contains(bad_field_name.as_str()));

        let source = build_undefined_type_source(&fields, &undefined_type, &bad_field_name);

        let result = typecheck_source(&source);
        prop_assert!(
            result.is_err(),
            "Struct with undefined type '{}' should fail type-checking.\nSource:\n{}",
            undefined_type,
            source
        );

        let err = result.unwrap_err();
        let msg = err.to_string();
        prop_assert!(
            msg.contains("unknown type") && msg.contains(&undefined_type),
            "Error should mention 'unknown type' and '{}', got: {}\nSource:\n{}",
            undefined_type,
            msg,
            source
        );
    }
}

// ============================================================================
// Property 6: Fixed-size array type validation
// ============================================================================
//
// Feature: flux-structs, Property 6: Fixed-size array type validation
//
// **Validates: Requirements 1.5, 6.2, 6.3**
//
// For any fixed-size array type `[T; N]`, the type-checker SHALL accept it if and
// only if N is a positive integer and T is a valid type. Index access on a valid
// array SHALL resolve to element type T.

/// Build a Flux source with a struct containing a fixed-size array field.
fn build_fixed_array_source(size: usize) -> String {
    format!(
        r#"struct ArrayStruct {{
    values: [f64; {}],
    count: int
}}

strategy Test {{
    on bar {{
        x = 1.0
    }}
}}
"#,
        size
    )
}

/// Build a Flux source that tests index access on a fixed-size array field
/// via a function that takes the struct as a parameter.
/// This avoids needing to construct a struct literal with an array field
/// (which requires runtime/interpreter support not yet available).
fn build_array_index_access_source(size: usize) -> String {
    format!(
        r#"struct ArrayStruct {{
    values: [f64; {}]
}}

fn get_first(a: ArrayStruct) {{
    return a.values[0] + 1.0
}}

strategy Test {{
    on bar {{
        x = 1.0
    }}
}}
"#,
        size
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Property 6a: A struct with a fixed-size array [f64; N] where N > 0 passes type-checking.
    #[test]
    fn prop_valid_fixed_array_passes(size in 1usize..=50) {
        let source = build_fixed_array_source(size);

        let result = typecheck_source(&source);
        prop_assert!(
            result.is_ok(),
            "Fixed array [f64; {}] should pass type-checking.\nSource:\n{}\nError: {:?}",
            size,
            source,
            result.err()
        );
    }

    /// Property 6b: Index access on a fixed-size array resolves to the element type (f64).
    /// We verify this by defining a function that takes a struct with an array field,
    /// indexes into it, and uses the result in arithmetic (which requires f64).
    #[test]
    fn prop_array_index_resolves_to_element_type(size in 1usize..=50) {
        let source = build_array_index_access_source(size);

        let result = typecheck_source(&source);
        prop_assert!(
            result.is_ok(),
            "Index access on [f64; {}] should resolve to f64 and allow arithmetic.\nSource:\n{}\nError: {:?}",
            size,
            source,
            result.err()
        );
    }
}

// ============================================================================
// Property 7: Struct function type safety
// ============================================================================
//
// Feature: flux-structs, Property 7: Struct function type safety
//
// **Validates: Requirements 5.3, 5.5**
//
// For any function with struct-typed parameters and any call site, the call
// type-checks if and only if each argument's type matches the declared struct
// parameter type. Passing a different struct type SHALL produce a type-mismatch error.

/// Generate a struct name (capitalized).
fn arb_struct_name() -> impl Strategy<Value = String> {
    "[A-Z][a-z]{3,7}".prop_filter("must not be a builtin type", |name| {
        !["Test", "Bool", "True", "False", "Null"].contains(&name.as_str())
    })
}

/// Build a Flux source with two struct types, a function taking the first struct
/// as a parameter, and a call site passing either the correct or wrong struct.
fn build_struct_function_source(
    struct_a_name: &str,
    struct_a_fields: &[FieldDef],
    struct_b_name: &str,
    struct_b_fields: &[FieldDef],
    pass_correct: bool,
) -> String {
    // Struct A definition
    let a_field_defs: Vec<String> = struct_a_fields
        .iter()
        .map(|f| format!("    {}: {}", f.name, f.field_type.type_str()))
        .collect();
    let struct_a_def = format!(
        "struct {} {{\n{}\n}}",
        struct_a_name,
        a_field_defs.join(",\n")
    );

    // Struct B definition
    let b_field_defs: Vec<String> = struct_b_fields
        .iter()
        .map(|f| format!("    {}: {}", f.name, f.field_type.type_str()))
        .collect();
    let struct_b_def = format!(
        "struct {} {{\n{}\n}}",
        struct_b_name,
        b_field_defs.join(",\n")
    );

    // Function taking struct A as parameter — access first field to produce a return value
    let fn_body = if !struct_a_fields.is_empty() {
        format!("return p.{}", struct_a_fields[0].name)
    } else {
        "return 0.0".to_string()
    };
    let fn_def = format!(
        "fn process(p: {}) {{\n    {}\n}}",
        struct_a_name, fn_body
    );

    // Construct a literal and call the function
    let (lit_struct_name, lit_fields) = if pass_correct {
        (struct_a_name, struct_a_fields)
    } else {
        (struct_b_name, struct_b_fields)
    };

    let literal_fields: Vec<String> = lit_fields
        .iter()
        .enumerate()
        .map(|(i, f)| format!("{} = {}", f.name, f.field_type.valid_literal(i as u32 + 1)))
        .collect();
    let literal_str = format!(
        "{} {{ {} }}",
        lit_struct_name,
        literal_fields.join(", ")
    );

    format!(
        "{}\n\n{}\n\n{}\n\nstrategy Test {{\n    on bar {{\n        val = {}\n        result = process(val)\n    }}\n}}\n",
        struct_a_def, struct_b_def, fn_def, literal_str
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Property 7a: Calling a function with the correct struct type passes type-checking.
    #[test]
    fn prop_correct_struct_param_passes(
        struct_a_name in arb_struct_name(),
        struct_a_fields in arb_struct_def(),
        struct_b_name in arb_struct_name(),
        struct_b_fields in arb_struct_def(),
    ) {
        // Ensure struct names are different
        prop_assume!(struct_a_name != struct_b_name);
        // Ensure neither is "Test" (used for strategy name)
        prop_assume!(struct_a_name != "Test" && struct_b_name != "Test");

        let source = build_struct_function_source(
            &struct_a_name,
            &struct_a_fields,
            &struct_b_name,
            &struct_b_fields,
            true, // pass correct type
        );

        let result = typecheck_source(&source);
        prop_assert!(
            result.is_ok(),
            "Passing correct struct '{}' should pass type-checking.\nSource:\n{}\nError: {:?}",
            struct_a_name,
            source,
            result.err()
        );
    }

    /// Property 7b: Calling a function with the wrong struct type produces a type-mismatch error.
    #[test]
    fn prop_wrong_struct_param_fails(
        struct_a_name in arb_struct_name(),
        struct_a_fields in arb_struct_def(),
        struct_b_name in arb_struct_name(),
        struct_b_fields in arb_struct_def(),
    ) {
        // Ensure struct names are different
        prop_assume!(struct_a_name != struct_b_name);
        // Ensure neither is "Test" (used for strategy name)
        prop_assume!(struct_a_name != "Test" && struct_b_name != "Test");

        let source = build_struct_function_source(
            &struct_a_name,
            &struct_a_fields,
            &struct_b_name,
            &struct_b_fields,
            false, // pass wrong type
        );

        let result = typecheck_source(&source);
        prop_assert!(
            result.is_err(),
            "Passing wrong struct '{}' where '{}' expected should fail.\nSource:\n{}",
            struct_b_name,
            struct_a_name,
            source
        );

        let err = result.unwrap_err();
        let msg = err.to_string();
        prop_assert!(
            msg.contains("expected struct") && msg.contains(&struct_a_name),
            "Error should mention 'expected struct' and '{}', got: {}\nSource:\n{}",
            struct_a_name,
            msg,
            source
        );
        prop_assert!(
            msg.contains("got struct") && msg.contains(&struct_b_name),
            "Error should mention 'got struct' and '{}', got: {}\nSource:\n{}",
            struct_b_name,
            msg,
            source
        );
    }
}
