// Feature: flux-structs, Property 1: Struct definition parse-format-parse round trip
// Feature: flux-structs, Property 22: Struct literal formatting line-count rule
//!
//! Unit tests for formatter struct support (task 10.3) and property-based tests
//! for struct definition round-trip (task 10.4) and struct literal line-count rule (task 10.5).
//!
//! **Validates: Requirements 19.1, 19.2, 19.3**

use proptest::prelude::*;

use flux_compiler::lexer::{self, Span};
use flux_compiler::parser;
use flux_compiler::parser::ast::{
    Decorator, DecoratorArg, StructDef, StructField, TypeAnnotation,
};
use flux_cli::formatter::Formatter;

// =============================================================================
// Helpers
// =============================================================================

/// Lex + parse + format a source string.
fn format_source(source: &str) -> String {
    let tokens = lexer::lex_with_spans(source).unwrap();
    let ast = parser::parse(tokens).unwrap();
    Formatter::format(&ast, source)
}

/// Wrap a struct definition in a minimal strategy so it parses as a full Program.
fn wrap_struct(struct_src: &str) -> String {
    format!("{}\nstrategy S {{\n    on bar {{\n        x = 1\n    }}\n}}\n", struct_src)
}

/// Wrap a struct literal expression inside a strategy body.
fn wrap_struct_literal(struct_def: &str, literal_expr: &str) -> String {
    format!(
        "{}\nstrategy S {{\n    on bar {{\n        x = {}\n    }}\n}}\n",
        struct_def, literal_expr
    )
}

// =============================================================================
// Task 10.3: Unit tests for formatter struct support
// =============================================================================

#[test]
fn test_struct_definition_formatting_basic() {
    let source = wrap_struct("struct Point {\n    x: f64,\n    y: f64\n}");
    let formatted = format_source(&source);
    // Should contain struct definition with one field per line, consistent indentation
    assert!(formatted.contains("struct Point {"));
    assert!(formatted.contains("    x: f64,"));
    assert!(formatted.contains("    y: f64"));
    assert!(formatted.contains("}"));
}

#[test]
fn test_struct_definition_formatting_multiple_fields() {
    let source = wrap_struct(
        "struct Quote {\n    bid: f64,\n    ask: f64,\n    bid_size: f64,\n    ask_size: f64,\n    timestamp: f64\n}"
    );
    let formatted = format_source(&source);
    assert!(formatted.contains("struct Quote {"));
    assert!(formatted.contains("    bid: f64,"));
    assert!(formatted.contains("    ask: f64,"));
    assert!(formatted.contains("    bid_size: f64,"));
    assert!(formatted.contains("    ask_size: f64,"));
    assert!(formatted.contains("    timestamp: f64"));
}

#[test]
fn test_struct_definition_formatting_nested_type() {
    let source = wrap_struct(
        "struct Inner {\n    value: f64\n}\n\nstruct Outer {\n    info: Inner,\n    count: int\n}"
    );
    let formatted = format_source(&source);
    assert!(formatted.contains("struct Inner {"));
    assert!(formatted.contains("    value: f64"));
    assert!(formatted.contains("struct Outer {"));
    assert!(formatted.contains("    info: Inner,"));
    assert!(formatted.contains("    count: int"));
}

#[test]
fn test_struct_definition_formatting_fixed_array() {
    let source = wrap_struct("struct Buffer {\n    values: [f64; 20],\n    count: int\n}");
    let formatted = format_source(&source);
    assert!(formatted.contains("    values: [f64; 20],"));
    assert!(formatted.contains("    count: int"));
}

#[test]
fn test_struct_literal_single_line_one_field() {
    let source = wrap_struct_literal(
        "struct Point {\n    x: f64\n}",
        "Point { x = 1.0 }",
    );
    let formatted = format_source(&source);
    assert!(formatted.contains("Point { x = 1.0 }"));
}

#[test]
fn test_struct_literal_single_line_three_fields() {
    let source = wrap_struct_literal(
        "struct Vec3 {\n    x: f64,\n    y: f64,\n    z: f64\n}",
        "Vec3 { x = 1.0, y = 2.0, z = 3.0 }",
    );
    let formatted = format_source(&source);
    // ≤3 fields should be single-line
    assert!(formatted.contains("Vec3 { x = 1.0, y = 2.0, z = 3.0 }"));
}

#[test]
fn test_struct_literal_multi_line_four_fields() {
    let source = wrap_struct_literal(
        "struct Quote {\n    bid: f64,\n    ask: f64,\n    bid_size: f64,\n    ask_size: f64\n}",
        "Quote {\n            bid = 100.0,\n            ask = 101.0,\n            bid_size = 10.0,\n            ask_size = 20.0\n        }",
    );
    let formatted = format_source(&source);
    // >3 fields should be multi-line (one field per line)
    // The struct literal appears in an assignment: x = Quote {\n  bid = ...\n ... }
    // Find the assignment line containing the struct literal
    let lines: Vec<&str> = formatted.lines().collect();
    let assign_line = lines.iter().position(|l| l.contains("x = Quote {")).unwrap();
    // The opening brace should be on the same line as the assignment
    assert!(lines[assign_line].contains("x = Quote {"));
    // Fields should be on separate lines below
    assert!(lines[assign_line + 1].contains("bid = 100.0"));
    assert!(lines[assign_line + 2].contains("ask = 101.0"));
    assert!(lines[assign_line + 3].contains("bid_size = 10.0"));
    assert!(lines[assign_line + 4].contains("ask_size = 20.0"));
}

#[test]
fn test_struct_literal_multi_line_five_fields() {
    let source = wrap_struct_literal(
        "struct Metrics {\n    alpha: f64,\n    beta: f64,\n    gamma: f64,\n    delta: f64,\n    epsilon: f64\n}",
        "Metrics {\n            alpha = 1.0,\n            beta = 2.0,\n            gamma = 3.0,\n            delta = 4.0,\n            epsilon = 5.0\n        }",
    );
    let formatted = format_source(&source);
    let lines: Vec<&str> = formatted.lines().collect();
    let assign_line = lines.iter().position(|l| l.contains("x = Metrics {")).unwrap();
    // >3 fields: multi-line
    assert!(lines[assign_line].contains("Metrics {"));
    assert!(lines[assign_line + 1].contains("alpha = 1.0"));
    assert!(lines[assign_line + 2].contains("beta = 2.0"));
    assert!(lines[assign_line + 3].contains("gamma = 3.0"));
    assert!(lines[assign_line + 4].contains("delta = 4.0"));
    assert!(lines[assign_line + 5].contains("epsilon = 5.0"));
}

#[test]
fn test_decorator_formatting_above_struct() {
    let source = wrap_struct("@aligned(64)\nstruct CacheLine {\n    value: f64\n}");
    let formatted = format_source(&source);
    let lines: Vec<&str> = formatted.lines().collect();
    let decorator_line = lines.iter().position(|l| l.contains("@aligned(64)")).unwrap();
    let struct_line = lines.iter().position(|l| l.contains("struct CacheLine")).unwrap();
    // Decorator must appear on line immediately before struct
    assert_eq!(struct_line, decorator_line + 1);
}

#[test]
fn test_multiple_decorators_formatting() {
    let source = wrap_struct("@packed\n@volatile\nstruct Header {\n    flags: int\n}");
    let formatted = format_source(&source);
    let lines: Vec<&str> = formatted.lines().collect();
    let packed_line = lines.iter().position(|l| l.contains("@packed")).unwrap();
    let volatile_line = lines.iter().position(|l| l.contains("@volatile")).unwrap();
    let struct_line = lines.iter().position(|l| l.contains("struct Header")).unwrap();
    // Decorators in order, immediately above struct
    assert_eq!(volatile_line, packed_line + 1);
    assert_eq!(struct_line, volatile_line + 1);
}

#[test]
fn test_decorator_with_argument_formatting() {
    let source = wrap_struct("@pool(128)\nstruct Order {\n    price: f64,\n    qty: f64\n}");
    let formatted = format_source(&source);
    assert!(formatted.contains("@pool(128)"));
    assert!(formatted.contains("struct Order {"));
}

#[test]
fn test_struct_definition_idempotent() {
    let source = wrap_struct("@aligned(64)\nstruct Tick {\n    price: f64,\n    size: f64,\n    side: int,\n    timestamp: f64\n}");
    let first = format_source(&source);
    let second = format_source(&first);
    assert_eq!(first, second, "Struct formatting should be idempotent");
}

// =============================================================================
// Task 10.4: Property test for struct definition parse-format-parse round trip
// =============================================================================

/// Reserved keywords that cannot be used as identifiers in generated Flux code.
const FLUX_KEYWORDS: &[&str] = &[
    "strategy", "params", "state", "on", "if", "elif", "else", "for", "while",
    "return", "fn", "from", "import", "and", "or", "not", "true", "false",
    "null", "data", "connector", "struct", "bar", "in", "f64", "int", "bool", "str",
];

/// Generate a valid Flux identifier (not a keyword).
fn arb_ident() -> impl Strategy<Value = String> {
    "[a-z][a-z_]{2,8}"
        .prop_filter("must not be a keyword", |s| {
            !FLUX_KEYWORDS.contains(&s.as_str())
        })
}

/// Generate a random type annotation.
fn arb_type_annotation() -> impl Strategy<Value = TypeAnnotation> {
    prop_oneof![
        4 => Just(TypeAnnotation::F64),
        2 => Just(TypeAnnotation::Int),
        2 => Just(TypeAnnotation::Bool),
        1 => Just(TypeAnnotation::Str),
    ]
}

/// Generate a single struct field.
fn arb_struct_field() -> impl Strategy<Value = StructField> {
    (arb_ident(), arb_type_annotation()).prop_map(|(name, field_type)| StructField {
        name,
        field_type,
        field_decorators: vec![],
        span: Span::new(0, 0),
    })
}

/// Generate a decorator (simple name-only or with integer arg).
fn arb_decorator() -> impl Strategy<Value = Decorator> {
    prop_oneof![
        Just(Decorator { name: "packed".to_string(), arg: None, span: Span::new(0, 0) }),
        Just(Decorator { name: "volatile".to_string(), arg: None, span: Span::new(0, 0) }),
        (prop_oneof![Just(8u32), Just(16), Just(32), Just(64)]).prop_map(|n| Decorator {
            name: "aligned".to_string(),
            arg: Some(DecoratorArg::Int(n as i64)),
            span: Span::new(0, 0),
        }),
        (prop_oneof![Just(64u32), Just(128), Just(256)]).prop_map(|n| Decorator {
            name: "pool".to_string(),
            arg: Some(DecoratorArg::Int(n as i64)),
            span: Span::new(0, 0),
        }),
    ]
}

/// Generate a struct definition with unique field names.
fn arb_struct_def() -> impl Strategy<Value = StructDef> {
    (
        arb_ident(),
        prop::collection::vec(arb_struct_field(), 1..=6),
        prop::collection::vec(arb_decorator(), 0..=2),
    )
        .prop_map(|(name, mut fields, decorators)| {
            // Ensure unique field names by deduplicating
            let mut seen = std::collections::HashSet::new();
            fields.retain(|f| seen.insert(f.name.clone()));
            // Ensure at least one field
            if fields.is_empty() {
                fields.push(StructField {
                    name: "value".to_string(),
                    field_type: TypeAnnotation::F64,
                    field_decorators: vec![],
                    span: Span::new(0, 0),
                });
            }
            // Capitalize struct name for convention
            let struct_name = {
                let mut chars = name.chars();
                match chars.next() {
                    Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                    None => "Mystruct".to_string(),
                }
            };
            StructDef {
                name: struct_name,
                type_params: vec![],
                fields,
                decorators,
                span: Span::new(0, 0),
            }
        })
        .prop_filter("struct name must not be keyword", |sd| {
            !FLUX_KEYWORDS.contains(&sd.name.to_lowercase().as_str())
        })
}

/// Build a complete source string from a StructDef (for parse-format-parse testing).
fn struct_def_to_source(sd: &StructDef) -> String {
    let mut s = String::new();
    for dec in &sd.decorators {
        s.push('@');
        s.push_str(&dec.name);
        if let Some(DecoratorArg::Int(n)) = &dec.arg {
            s.push('(');
            s.push_str(&n.to_string());
            s.push(')');
        }
        s.push('\n');
    }
    s.push_str("struct ");
    s.push_str(&sd.name);
    s.push_str(" {\n");
    for (i, field) in sd.fields.iter().enumerate() {
        s.push_str("    ");
        s.push_str(&field.name);
        s.push_str(": ");
        s.push_str(&type_annotation_to_str(&field.field_type));
        if i < sd.fields.len() - 1 {
            s.push(',');
        }
        s.push('\n');
    }
    s.push_str("}\n");
    s
}

fn type_annotation_to_str(ty: &TypeAnnotation) -> String {
    match ty {
        TypeAnnotation::F64 => "f64".to_string(),
        TypeAnnotation::Int => "int".to_string(),
        TypeAnnotation::Bool => "bool".to_string(),
        TypeAnnotation::Str => "str".to_string(),
        TypeAnnotation::Named(n) => n.clone(),
        TypeAnnotation::FixedArray(elem, size) => {
            format!("[{}; {}]", type_annotation_to_str(elem), size)
        }
        TypeAnnotation::BitInt(n) => format!("int({})", n),
        TypeAnnotation::Generic(name, type_args) => {
            let args: Vec<String> = type_args.iter().map(type_annotation_to_str).collect();
            format!("{}[{}]", name, args.join(", "))
        }
    }
}

/// Compare two StructDefs ignoring spans.
fn struct_defs_equivalent(a: &StructDef, b: &StructDef) -> bool {
    if a.name != b.name {
        return false;
    }
    if a.fields.len() != b.fields.len() {
        return false;
    }
    for (fa, fb) in a.fields.iter().zip(b.fields.iter()) {
        if fa.name != fb.name || fa.field_type != fb.field_type {
            return false;
        }
    }
    if a.decorators.len() != b.decorators.len() {
        return false;
    }
    for (da, db) in a.decorators.iter().zip(b.decorators.iter()) {
        if da.name != db.name || da.arg != db.arg {
            return false;
        }
    }
    true
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Feature: flux-structs, Property 1: Struct definition parse-format-parse round trip
    ///
    /// For any valid Flux struct definition source text, parsing it into an AST,
    /// formatting the AST back to source text, and re-parsing the formatted text
    /// SHALL produce an equivalent AST (field names, field types, decorators, and
    /// struct name are preserved).
    ///
    /// **Validates: Requirements 19.3**
    #[test]
    fn prop_struct_def_parse_format_parse_round_trip(struct_def in arb_struct_def()) {
        // Step 1: Generate source from the struct def
        let struct_source = struct_def_to_source(&struct_def);
        let full_source = wrap_struct(&struct_source);

        // Step 2: Parse the generated source
        let tokens1 = lexer::lex_with_spans(&full_source)
            .expect("Generated source should lex");
        let ast1 = parser::parse(tokens1)
            .expect("Generated source should parse");

        // Step 3: Format the parsed AST
        let formatted = Formatter::format(&ast1, &full_source);

        // Step 4: Re-parse the formatted output
        let tokens2 = lexer::lex_with_spans(&formatted)
            .expect("Formatted source should lex");
        let ast2 = parser::parse(tokens2)
            .expect("Formatted source should parse");

        // Step 5: Compare the struct definitions (ignoring spans)
        prop_assert_eq!(
            ast1.structs.len(),
            ast2.structs.len(),
            "Number of struct defs should be preserved"
        );

        for (s1, s2) in ast1.structs.iter().zip(ast2.structs.iter()) {
            prop_assert!(
                struct_defs_equivalent(s1, s2),
                "Struct definitions should be equivalent after round-trip.\n\
                 Original: {:?}\n\
                 After round-trip: {:?}\n\
                 Formatted source:\n{}",
                s1, s2, formatted
            );
        }
    }
}

// =============================================================================
// Task 10.5: Property test for struct literal formatting line-count rule
// =============================================================================

/// Generate a struct literal source with N fields, wrapping it in a valid program.
fn generate_struct_literal_source(field_count: usize) -> String {
    // Build a struct definition with N fields
    let mut struct_def = String::from("struct TestStruct {\n");
    for i in 0..field_count {
        struct_def.push_str(&format!("    field_{}: f64", i));
        if i < field_count - 1 {
            struct_def.push(',');
        }
        struct_def.push('\n');
    }
    struct_def.push_str("}\n");

    // Build the struct literal expression
    let mut literal = String::from("TestStruct { ");
    for i in 0..field_count {
        if i > 0 {
            literal.push_str(", ");
        }
        literal.push_str(&format!("field_{} = {}.0", i, i + 1));
    }
    literal.push_str(" }");

    wrap_struct_literal(&struct_def, &literal)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Feature: flux-structs, Property 22: Struct literal formatting line-count rule
    ///
    /// For any struct literal with N field assignments, the formatter SHALL produce
    /// single-line output when N ≤ 3 and multi-line output (one field per line)
    /// when N > 3.
    ///
    /// **Validates: Requirements 19.2**
    #[test]
    fn prop_struct_literal_formatting_line_count_rule(field_count in 1usize..=8) {
        let source = generate_struct_literal_source(field_count);
        let formatted = format_source(&source);

        // Find the line containing the struct literal assignment "x = TestStruct {"
        let lines: Vec<&str> = formatted.lines().collect();
        let struct_lit_line_idx = lines.iter().position(|l| l.contains("x = TestStruct {"))
            .expect("Formatted output should contain TestStruct literal assignment");

        if field_count <= 3 {
            // Single-line: the entire struct literal should be on one line
            let line = lines[struct_lit_line_idx];
            // All field assignments should be on this same line
            for i in 0..field_count {
                prop_assert!(
                    line.contains(&format!("field_{} = {}.0", i, i + 1)),
                    "For {} fields (≤3), all fields should be on one line.\nLine: {}\nFull formatted:\n{}",
                    field_count, line, formatted
                );
            }
            // The closing brace should be on the same line
            prop_assert!(
                line.contains("}"),
                "Single-line literal should have closing brace on same line.\nLine: {}\nFull formatted:\n{}",
                line, formatted
            );
        } else {
            // Multi-line: each field should be on its own line
            // The opening line has "x = TestStruct {" and a newline (no fields on this line)
            let line = lines[struct_lit_line_idx];
            prop_assert!(
                !line.contains("field_0 ="),
                "For {} fields (>3), fields should NOT be on the opening line.\nLine: {}\nFull formatted:\n{}",
                field_count, line, formatted
            );
            // Each field should appear on a separate subsequent line
            for i in 0..field_count {
                let expected_field = format!("field_{} = {}.0", i, i + 1);
                let field_line = lines[struct_lit_line_idx + 1..].iter()
                    .position(|l| l.contains(&expected_field));
                prop_assert!(
                    field_line.is_some(),
                    "Multi-line literal should have field '{}' on its own line.\nFull formatted:\n{}",
                    expected_field, formatted
                );
            }
            // Count lines between opening and closing brace
            // Find the closing brace line after the struct literal fields
            let closing_brace_idx = lines[struct_lit_line_idx + 1..].iter()
                .position(|l| l.trim().starts_with('}'))
                .map(|i| i + struct_lit_line_idx + 1);
            prop_assert!(
                closing_brace_idx.is_some(),
                "Multi-line literal should have a closing brace line.\nFull formatted:\n{}",
                formatted
            );
        }
    }
}
