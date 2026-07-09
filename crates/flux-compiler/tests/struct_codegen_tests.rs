//! Tests for struct codegen emission.
//!
//! Task 6.5: Unit tests for codegen struct emission
//! Task 6.6: Property test for codegen struct dependency order (Property 8)
//! Task 6.7: Property test for fixed-size array codegen correctness (Property 23)

use flux_compiler::compile;
use proptest::prelude::*;

// ============================================================================
// Task 6.5: Unit tests for codegen struct emission
// ============================================================================
//
// **Validates: Requirements 4.1, 4.4, 17.1, 17.3, 17.4, 2.6, 3.3**

/// Test: emitted struct includes `#[derive(Clone, Copy)]`
#[test]
fn struct_emits_derive_clone_copy() {
    let source = r#"
struct Point {
    x: f64,
    y: f64
}

strategy Test {
    on bar {
        p = Point { x = 1.0, y = 2.0 }
    }
}
"#;
    let output = compile(source).expect("compilation should succeed");

    assert!(
        output.contains("#[derive(Clone, Copy)]"),
        "Emitted struct should include #[derive(Clone, Copy)]. Got:\n{}",
        output
    );
    assert!(
        output.contains("pub struct Point"),
        "Emitted struct should include 'pub struct Point'. Got:\n{}",
        output
    );
    assert!(
        output.contains("pub x: f64"),
        "Emitted struct should include field 'pub x: f64'. Got:\n{}",
        output
    );
    assert!(
        output.contains("pub y: f64"),
        "Emitted struct should include field 'pub y: f64'. Got:\n{}",
        output
    );
}

/// Test: nested struct field type emits correctly
#[test]
fn nested_struct_field_type_emits_correctly() {
    let source = r#"
struct Inner {
    value: f64
}

struct Outer {
    nested: Inner,
    count: int
}

strategy Test {
    on bar {
        x = 1.0
    }
}
"#;
    let output = compile(source).expect("compilation should succeed");

    // The Outer struct should reference Inner as a field type
    assert!(
        output.contains("pub nested: Inner"),
        "Outer struct should have field 'pub nested: Inner'. Got:\n{}",
        output
    );
    assert!(
        output.contains("pub count: i64"),
        "Outer struct should have field 'pub count: i64'. Got:\n{}",
        output
    );
    // Both structs should have the derive attribute
    let derive_count = output.matches("#[derive(Clone, Copy)]").count();
    assert!(
        derive_count >= 2,
        "Both structs should have #[derive(Clone, Copy)], found {} occurrences. Got:\n{}",
        derive_count,
        output
    );
}

/// Test: fixed array field type emits correctly
#[test]
fn fixed_array_field_type_emits_correctly() {
    let source = r#"
struct DataBuffer {
    values: [f64; 20],
    flags: [bool; 5],
    counts: [int; 10]
}

strategy Test {
    on bar {
        x = 1.0
    }
}
"#;
    let output = compile(source).expect("compilation should succeed");

    assert!(
        output.contains("pub values: [f64; 20]"),
        "Should emit 'pub values: [f64; 20]'. Got:\n{}",
        output
    );
    assert!(
        output.contains("pub flags: [bool; 5]"),
        "Should emit 'pub flags: [bool; 5]'. Got:\n{}",
        output
    );
    assert!(
        output.contains("pub counts: [i64; 10]"),
        "Should emit 'pub counts: [i64; 10]'. Got:\n{}",
        output
    );
}

/// Test: struct literal codegen output
#[test]
fn struct_literal_codegen_output() {
    let source = r#"
struct Point {
    x: f64,
    y: f64
}

strategy Test {
    on bar {
        p = Point { x = 1.5, y = 2.5 }
    }
}
"#;
    let output = compile(source).expect("compilation should succeed");

    // The struct literal should produce Rust struct literal syntax
    assert!(
        output.contains("Point { x: ") && output.contains(", y: "),
        "Should emit Rust struct literal 'Point {{ x: ..., y: ... }}'. Got:\n{}",
        output
    );
}

/// Test: field access codegen output
#[test]
fn field_access_codegen_output() {
    let source = r#"
struct Point {
    x: f64,
    y: f64
}

fn get_x(p: Point) {
    return p.x
}

strategy Test {
    on bar {
        pt = Point { x = 3.0, y = 4.0 }
        val = get_x(pt)
    }
}
"#;
    let output = compile(source).expect("compilation should succeed");

    // Field access should be emitted as `p.x`
    assert!(
        output.contains("p.x"),
        "Should emit field access as 'p.x'. Got:\n{}",
        output
    );
}

// ============================================================================
// Task 6.6: Property test for codegen struct dependency order
// ============================================================================
//
// Feature: flux-structs, Property 8: Codegen emits structs in dependency order
//
// **Validates: Requirements 17.5**
//
// For any set of struct definitions where struct A contains a field of type B,
// the code generator SHALL emit struct B before struct A in the output.
// The emitted order SHALL be a valid topological sort of the dependency graph.

/// Reserved keywords in Flux that cannot be used as names.
const FLUX_RESERVED: &[&str] = &[
    "strategy", "params", "state", "on", "if", "elif", "else", "for", "while",
    "return", "fn", "from", "import", "and", "or", "not", "true", "false", "null",
    "data", "connector", "struct", "bar", "in", "f64", "int", "bool", "str",
];

/// Generate a valid struct name (capitalized, 4-8 chars, not reserved).
fn arb_struct_name() -> impl Strategy<Value = String> {
    "[A-Z][a-z]{3,7}".prop_filter("must not be reserved or Test", |name| {
        !FLUX_RESERVED.contains(&name.to_lowercase().as_str())
            && name != "Test"
            && name != "Bool"
            && name != "True"
            && name != "False"
            && name != "Null"
    })
}

/// Represents a linear dependency chain of structs:
/// chain[0] has no dependencies, chain[1] depends on chain[0], etc.
#[derive(Debug, Clone)]
struct DependencyChain {
    /// Struct names in dependency order (no deps first).
    names: Vec<String>,
}

/// Generate a linear dependency chain of 2-4 structs with unique names.
fn arb_dependency_chain() -> impl Strategy<Value = DependencyChain> {
    proptest::collection::vec(arb_struct_name(), 2..=4)
        .prop_filter("names must be unique", |names| {
            let unique: std::collections::HashSet<&str> =
                names.iter().map(|s| s.as_str()).collect();
            unique.len() == names.len()
        })
        .prop_map(|names| DependencyChain { names })
}

/// Build Flux source for a dependency chain, deliberately declaring in REVERSE order
/// so the codegen has to reorder them.
fn build_dependency_chain_source(chain: &DependencyChain) -> String {
    let mut structs = Vec::new();

    // chain.names[0] is the leaf (no struct deps), chain.names[1] depends on chain.names[0], etc.
    for (i, name) in chain.names.iter().enumerate() {
        if i == 0 {
            // Leaf struct — only scalar fields
            structs.push(format!(
                "struct {} {{\n    value: f64\n}}",
                name
            ));
        } else {
            // Depends on the previous struct in the chain
            let dep_name = &chain.names[i - 1];
            structs.push(format!(
                "struct {} {{\n    inner: {},\n    count: int\n}}",
                name, dep_name
            ));
        }
    }

    // Reverse the declaration order to test that codegen reorders them
    structs.reverse();

    format!(
        "{}\n\nstrategy Test {{\n    on bar {{\n        x = 1.0\n    }}\n}}\n",
        structs.join("\n\n")
    )
}

/// Find the position of "pub struct <Name>" in the output for ordering verification.
fn find_struct_position(output: &str, name: &str) -> Option<usize> {
    output.find(&format!("pub struct {}", name))
}

// Feature: flux-structs, Property 8: Codegen emits structs in dependency order
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Property 8: For any dependency chain A → B → C, the emitted output has
    /// A appearing before B appearing before C.
    #[test]
    fn prop_codegen_emits_structs_in_dependency_order(chain in arb_dependency_chain()) {
        let source = build_dependency_chain_source(&chain);

        let output = compile(&source).expect(&format!(
            "Compilation should succeed for dependency chain.\nSource:\n{}",
            source
        ));

        // Verify each struct in the chain appears in the output
        for name in &chain.names {
            prop_assert!(
                output.contains(&format!("pub struct {}", name)),
                "Output should contain 'pub struct {}'. Got:\n{}",
                name,
                output
            );
        }

        // Verify ordering: chain.names[i] must appear BEFORE chain.names[i+1]
        // because chain.names[i+1] depends on chain.names[i]
        for i in 0..chain.names.len() - 1 {
            let dep_pos = find_struct_position(&output, &chain.names[i])
                .expect(&format!("Should find struct {}", chain.names[i]));
            let container_pos = find_struct_position(&output, &chain.names[i + 1])
                .expect(&format!("Should find struct {}", chain.names[i + 1]));

            prop_assert!(
                dep_pos < container_pos,
                "Struct '{}' (dependency) should appear before '{}' (container) in output.\n\
                 {} position: {}, {} position: {}\nSource:\n{}\nOutput:\n{}",
                chain.names[i],
                chain.names[i + 1],
                chain.names[i],
                dep_pos,
                chain.names[i + 1],
                container_pos,
                source,
                output
            );
        }
    }
}

// ============================================================================
// Task 6.7: Property test for fixed-size array codegen correctness
// ============================================================================
//
// Feature: flux-structs, Property 23: Fixed-size array codegen correctness
//
// **Validates: Requirements 4.4, 6.4, 17.4**
//
// For any struct containing a field of type `[T; N]`, the code generator SHALL
// emit `[rust_T; N]` as the Rust field type, where rust_T is the Rust mapping
// of Flux type T.

/// The scalar element types for fixed arrays and their Flux/Rust representations.
#[derive(Debug, Clone, Copy)]
enum ArrayElemType {
    F64,
    Int,
    Bool,
}

impl ArrayElemType {
    /// Flux source type name.
    fn flux_str(&self) -> &'static str {
        match self {
            ArrayElemType::F64 => "f64",
            ArrayElemType::Int => "int",
            ArrayElemType::Bool => "bool",
        }
    }

    /// Expected Rust type string in codegen output.
    fn rust_str(&self) -> &'static str {
        match self {
            ArrayElemType::F64 => "f64",
            ArrayElemType::Int => "i64",
            ArrayElemType::Bool => "bool",
        }
    }
}

/// Generate a random array element type.
fn arb_array_elem_type() -> impl Strategy<Value = ArrayElemType> {
    prop_oneof![
        Just(ArrayElemType::F64),
        Just(ArrayElemType::Int),
        Just(ArrayElemType::Bool),
    ]
}

/// A struct field with a fixed-size array type.
#[derive(Debug, Clone)]
struct ArrayFieldSpec {
    field_name: String,
    elem_type: ArrayElemType,
    size: usize,
}

/// Generate valid field names for array fields.
fn arb_array_field_name() -> impl Strategy<Value = String> {
    "[a-z]{3,8}".prop_filter("must not be reserved", |name| {
        !FLUX_RESERVED.contains(&name.as_str())
    })
}

/// Generate an ArrayFieldSpec with varying element types and sizes.
fn arb_array_field() -> impl Strategy<Value = ArrayFieldSpec> {
    (arb_array_field_name(), arb_array_elem_type(), 1usize..=100).prop_map(
        |(field_name, elem_type, size)| ArrayFieldSpec {
            field_name,
            elem_type,
            size,
        },
    )
}

/// Generate a struct with 1-3 fixed-array fields (all with unique names).
fn arb_array_struct() -> impl Strategy<Value = Vec<ArrayFieldSpec>> {
    proptest::collection::vec(arb_array_field(), 1..=3).prop_filter(
        "field names must be unique",
        |fields| {
            let names: std::collections::HashSet<&str> =
                fields.iter().map(|f| f.field_name.as_str()).collect();
            names.len() == fields.len()
        },
    )
}

/// Build Flux source for a struct with fixed-array fields.
fn build_array_struct_source(fields: &[ArrayFieldSpec]) -> String {
    let field_defs: Vec<String> = fields
        .iter()
        .map(|f| format!("    {}: [{}; {}]", f.field_name, f.elem_type.flux_str(), f.size))
        .collect();

    format!(
        "struct ArrayTest {{\n{}\n}}\n\nstrategy Test {{\n    on bar {{\n        x = 1.0\n    }}\n}}\n",
        field_defs.join(",\n")
    )
}

// Feature: flux-structs, Property 23: Fixed-size array codegen correctness
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Property 23: For any struct with a fixed-array field [T; N], the codegen
    /// emits [rust_T; N] in the Rust output.
    #[test]
    fn prop_fixed_array_codegen_correctness(fields in arb_array_struct()) {
        let source = build_array_struct_source(&fields);

        let output = compile(&source).expect(&format!(
            "Compilation should succeed for array struct.\nSource:\n{}",
            source
        ));

        // Verify each field emits the correct Rust array type
        for field in &fields {
            let expected_rust_type = format!(
                "[{}; {}]",
                field.elem_type.rust_str(),
                field.size
            );
            let expected_field_decl = format!(
                "pub {}: {}",
                field.field_name, expected_rust_type
            );

            prop_assert!(
                output.contains(&expected_field_decl),
                "Output should contain '{}' for field '{}' with Flux type [{}, {}].\nSource:\n{}\nOutput:\n{}",
                expected_field_decl,
                field.field_name,
                field.elem_type.flux_str(),
                field.size,
                source,
                output
            );
        }
    }
}
