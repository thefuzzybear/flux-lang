//! Unit tests for typechecker: struct validation errors.
//!
//! **Validates: Requirements 1.6, 1.7, 2.3, 2.4, 2.5, 3.2, 6.2, 5.5**
//!
//! Tests that the typechecker correctly reports errors for:
//! - Duplicate field names in struct definitions
//! - Undefined type references in struct fields
//! - Zero-size fixed arrays
//! - Missing fields in struct literals
//! - Extra (unknown) fields in struct literals
//! - Type mismatches in struct literal field values
//! - Invalid field access listing available fields
//! - Struct-typed function parameter mismatches

use flux_compiler::error::CompileError;
use flux_compiler::lexer::lex_with_spans;
use flux_compiler::parser::parse;
use flux_compiler::typeck::check;

/// Helper: lex, parse, and typecheck a complete Flux source string.
fn typecheck_source(source: &str) -> Result<flux_compiler::typeck::TypedProgram, CompileError> {
    let tokens = lex_with_spans(source)?;
    let ast = parse(tokens)?;
    check(ast)
}

// ============================================================================
// Requirement 1.6: Duplicate field name in struct definition
// ============================================================================

/// A struct definition with duplicate field names produces a clear error.
#[test]
fn duplicate_field_name_error() {
    let source = r#"
struct Tick {
    price: f64,
    size: f64,
    price: f64
}

strategy Test {
    on bar {
        x = 1.0
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_err(), "Expected duplicate field error");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("duplicate field 'price' in struct 'Tick'"),
        "Error should report duplicate field name and struct name, got: {}",
        msg
    );
}

/// Duplicate field names are detected even when they have different types.
#[test]
fn duplicate_field_different_types_error() {
    let source = r#"
struct Config {
    value: f64,
    name: str,
    value: int
}

strategy Test {
    on bar {
        x = 1.0
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_err(), "Expected duplicate field error");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("duplicate field 'value' in struct 'Config'"),
        "Error should report duplicate field with struct name, got: {}",
        msg
    );
}

// ============================================================================
// Requirement 1.7: Undefined type reference in struct field
// ============================================================================

/// A struct field referencing an undefined struct type produces an error.
#[test]
fn undefined_type_reference_error() {
    let source = r#"
struct Container {
    payload: UnknownType
}

strategy Test {
    on bar {
        x = 1.0
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_err(), "Expected undefined type error");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("unknown type 'UnknownType'"),
        "Error should report the undefined type name, got: {}",
        msg
    );
    assert!(
        msg.contains("struct 'Container'"),
        "Error should report the containing struct name, got: {}",
        msg
    );
    assert!(
        msg.contains("field 'payload'"),
        "Error should report the field name, got: {}",
        msg
    );
}

/// Undefined type in a fixed-size array element type also produces an error.
#[test]
fn undefined_type_in_array_field_error() {
    let source = r#"
struct Book {
    bids: [MissingLevel; 20]
}

strategy Test {
    on bar {
        x = 1.0
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_err(), "Expected undefined type error for array element");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("unknown type 'MissingLevel'"),
        "Error should report the undefined type in array, got: {}",
        msg
    );
}

// ============================================================================
// Requirement 6.2: Zero-size array error
// ============================================================================

/// A fixed-size array with size 0 produces an error.
#[test]
fn zero_size_array_error() {
    let source = r#"
struct Bad {
    values: [f64; 0]
}

strategy Test {
    on bar {
        x = 1.0
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_err(), "Expected zero-size array error");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("array size must be positive, got 0"),
        "Error should report zero-size array issue, got: {}",
        msg
    );
}

// ============================================================================
// Requirement 2.3: Missing fields in struct literal
// ============================================================================

/// A struct literal missing required fields produces an error listing them.
#[test]
fn struct_literal_missing_fields_error() {
    let source = r#"
struct Quote {
    bid: f64,
    ask: f64,
    timestamp: f64
}

strategy Test {
    on bar {
        q = Quote { bid = 100.0 }
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_err(), "Expected missing fields error");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("missing fields"),
        "Error should report missing fields, got: {}",
        msg
    );
    assert!(
        msg.contains("ask"),
        "Error should list 'ask' as missing, got: {}",
        msg
    );
    assert!(
        msg.contains("timestamp"),
        "Error should list 'timestamp' as missing, got: {}",
        msg
    );
}

/// A struct literal with no fields provided lists all fields as missing.
#[test]
fn struct_literal_all_fields_missing_error() {
    let source = r#"
struct Point {
    x: f64,
    y: f64
}

strategy Test {
    on bar {
        p = Point { }
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_err(), "Expected missing fields error");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("missing fields"),
        "Error should report missing fields, got: {}",
        msg
    );
    assert!(
        msg.contains("x"),
        "Error should list 'x' as missing, got: {}",
        msg
    );
    assert!(
        msg.contains("y"),
        "Error should list 'y' as missing, got: {}",
        msg
    );
}

// ============================================================================
// Requirement 2.4: Extra (unknown) field in struct literal
// ============================================================================

/// A struct literal providing a field not defined in the struct produces an error.
#[test]
fn struct_literal_extra_field_error() {
    let source = r#"
struct Tick {
    price: f64,
    size: f64
}

strategy Test {
    on bar {
        t = Tick { price = 100.0, size = 50.0, venue = "NYSE" }
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_err(), "Expected extra field error");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("has no field 'venue'"),
        "Error should report the unknown field name, got: {}",
        msg
    );
}

/// Providing a completely unrelated field name produces the same error format.
#[test]
fn struct_literal_unknown_field_error() {
    let source = r#"
struct Bar {
    open: f64,
    close: f64
}

strategy Test {
    on bar {
        b = Bar { open = 100.0, close = 101.0, nonexistent = 0.0 }
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_err(), "Expected unknown field error");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("has no field 'nonexistent'"),
        "Error should report the unknown field, got: {}",
        msg
    );
}

// ============================================================================
// Requirement 2.5: Type mismatch in struct literal field value
// ============================================================================

/// A struct literal with a field value of the wrong type produces a type mismatch error.
#[test]
fn struct_literal_field_type_mismatch_error() {
    let source = r#"
struct Point {
    x: f64,
    y: f64
}

strategy Test {
    on bar {
        p = Point { x = 1.0, y = "wrong" }
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_err(), "Expected field type mismatch error");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("field 'y' expects"),
        "Error should name the mismatched field, got: {}",
        msg
    );
    assert!(
        msg.contains("Float"),
        "Error should mention expected type, got: {}",
        msg
    );
    assert!(
        msg.contains("String"),
        "Error should mention actual type, got: {}",
        msg
    );
}

/// Passing a bool where f64 is expected produces a type mismatch on the field.
#[test]
fn struct_literal_bool_for_float_field_error() {
    let source = r#"
struct Config {
    threshold: f64,
    enabled: bool
}

strategy Test {
    on bar {
        c = Config { threshold = true, enabled = false }
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_err(), "Expected field type mismatch error");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("field 'threshold' expects"),
        "Error should name the mismatched field, got: {}",
        msg
    );
}

// ============================================================================
// Requirement 3.2: Invalid field access lists available fields
// ============================================================================

/// Accessing a non-existent field on a struct lists available fields.
#[test]
fn invalid_field_access_lists_available_fields() {
    let source = r#"
struct Quote {
    bid: f64,
    ask: f64,
    timestamp: f64
}

strategy Test {
    on bar {
        q = Quote { bid = 100.0, ask = 101.0, timestamp = 0.0 }
        m = q.mid
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_err(), "Expected invalid field access error");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("has no field 'mid'"),
        "Error should report the invalid field name, got: {}",
        msg
    );
    assert!(
        msg.contains("Available:"),
        "Error should list available fields, got: {}",
        msg
    );
    assert!(
        msg.contains("bid"),
        "Available fields should include 'bid', got: {}",
        msg
    );
    assert!(
        msg.contains("ask"),
        "Available fields should include 'ask', got: {}",
        msg
    );
    assert!(
        msg.contains("timestamp"),
        "Available fields should include 'timestamp', got: {}",
        msg
    );
}

/// Field access error on a struct with many fields still lists them all.
#[test]
fn invalid_field_access_on_multifield_struct() {
    let source = r#"
struct Bar {
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64
}

strategy Test {
    on bar {
        b = Bar { open = 1.0, high = 2.0, low = 0.5, close = 1.5, volume = 100.0 }
        x = b.vwap
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_err(), "Expected invalid field access error");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("has no field 'vwap'"),
        "Error should report invalid field, got: {}",
        msg
    );
    assert!(
        msg.contains("open") && msg.contains("high") && msg.contains("low")
            && msg.contains("close") && msg.contains("volume"),
        "Error should list all available fields, got: {}",
        msg
    );
}

// ============================================================================
// Requirement 5.5: Struct-typed function parameter mismatch
// ============================================================================

/// Passing a struct of the wrong type to a function produces a type mismatch error.
#[test]
fn struct_function_parameter_mismatch_error() {
    let source = r#"
struct Quote {
    bid: f64,
    ask: f64
}

struct Tick {
    price: f64,
    size: f64
}

fn calc_spread(q: Quote) {
    return q.ask - q.bid
}

strategy Test {
    on bar {
        t = Tick { price = 100.0, size = 50.0 }
        s = calc_spread(t)
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_err(), "Expected struct type mismatch error");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("expected struct 'Quote'"),
        "Error should name the expected struct type, got: {}",
        msg
    );
    assert!(
        msg.contains("got struct 'Tick'"),
        "Error should name the actual struct type, got: {}",
        msg
    );
}

/// Passing a non-struct value where a struct is expected also reports an error.
#[test]
fn non_struct_for_struct_parameter_error() {
    let source = r#"
struct Quote {
    bid: f64,
    ask: f64
}

fn process(q: Quote) {
    return q.bid
}

strategy Test {
    on bar {
        s = process(42.0)
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_err(), "Expected type mismatch error for non-struct arg");
    let err = result.unwrap_err();
    let msg = err.to_string();
    // Should mention a type mismatch (either generic or struct-specific format)
    assert!(
        msg.contains("Quote") || msg.contains("must be"),
        "Error should mention the expected struct type or type requirement, got: {}",
        msg
    );
}

// ============================================================================
// Positive cases: Valid structs and operations pass type checking
// ============================================================================

/// A valid struct literal with all fields passes type checking.
#[test]
fn valid_struct_literal_passes() {
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
    let result = typecheck_source(source);
    assert!(result.is_ok(), "Expected Ok for valid struct literal, got: {:?}", result.err());
}

/// Valid field access on a struct passes type checking.
#[test]
fn valid_field_access_passes() {
    let source = r#"
struct Quote {
    bid: f64,
    ask: f64
}

strategy Test {
    on bar {
        q = Quote { bid = 100.0, ask = 101.0 }
        spread = q.ask - q.bid
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_ok(), "Expected Ok for valid field access, got: {:?}", result.err());
}

/// Passing the correct struct type to a function passes type checking.
#[test]
fn correct_struct_parameter_passes() {
    let source = r#"
struct Quote {
    bid: f64,
    ask: f64
}

fn calc_spread(q: Quote) {
    return q.ask - q.bid
}

strategy Test {
    on bar {
        q = Quote { bid = 100.0, ask = 101.0 }
        s = calc_spread(q)
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_ok(), "Expected Ok for correct struct parameter, got: {:?}", result.err());
}

/// A struct with a nested struct field type passes when the dependency is defined.
#[test]
fn nested_struct_field_type_passes() {
    let source = r#"
struct Quote {
    bid: f64,
    ask: f64
}

struct MarketSnapshot {
    quote: Quote,
    mid: f64
}

strategy Test {
    on bar {
        q = Quote { bid = 100.0, ask = 101.0 }
        snap = MarketSnapshot { quote = q, mid = 100.5 }
        spread = snap.quote.ask - snap.quote.bid
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_ok(), "Expected Ok for nested struct access, got: {:?}", result.err());
}

/// A struct with a valid fixed-size array field passes type checking.
#[test]
fn valid_fixed_array_field_passes() {
    let source = r#"
struct Window {
    values: [f64; 10],
    count: int
}

strategy Test {
    on bar {
        x = 1.0
    }
}
"#;
    let result = typecheck_source(source);
    assert!(result.is_ok(), "Expected Ok for fixed-size array field, got: {:?}", result.err());
}
