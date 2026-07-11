//! Property-based tests for type system parsing round-trip.
//!
//! Feature: flux-type-system
//!
//! This file contains property tests validating that parsing produces ASTs
//! that can be pretty-printed back to source and parsed again to equivalent ASTs.

use flux_compiler::lexer::lex_with_spans;
use flux_compiler::parser::{parse, pretty_print_program, MatchExpr, Pattern, Program, Expr, ExprKind, Stmt, TypeAnnotation, TypeParam};
use proptest::prelude::*;

// ============================================================================
// Property 1: Enum Definition Parsing Round-Trip
// ============================================================================

/// Feature: flux-type-system, Property 1: Enum Definition Parsing Round-Trip
///
/// **Validates: Requirements 1.2, 1.3, 12.1**
///
/// For any valid enum definition source text (with any combination of unit
/// and data variants, arbitrary valid identifiers, and supported type annotations),
/// parsing to AST and pretty-printing back to source and parsing again SHALL
/// produce an equivalent AST.

// ============================================================================
// Generators for Enum Definition
// ============================================================================

/// Reserved keywords in Flux that cannot be used as identifiers.
const FLUX_RESERVED: &[&str] = &[
    "strategy", "params", "state", "on", "if", "elif", "else", "for", "while",
    "return", "fn", "from", "import", "and", "or", "not", "true", "false", "null",
    "data", "connector", "struct", "bar", "in", "f64", "int", "bool", "str",
    "enum", "match", "self", "impl", "trait",
];

/// Generate a valid enum name (capitalized, not a reserved keyword).
fn arb_enum_name() -> impl Strategy<Value = String> {
    "[A-Z][a-z]{2,7}".prop_filter("must not be a reserved keyword", |name| {
        !FLUX_RESERVED.contains(&name.as_str())
    })
}

/// Generate a valid variant name (capitalized, not a reserved keyword).
fn arb_variant_name() -> impl Strategy<Value = String> {
    "[A-Z][a-z]{2,7}".prop_filter("must not be a reserved keyword", |name| {
        !FLUX_RESERVED.contains(&name.as_str())
    })
}

/// Generate a valid field name (lowercase, not a reserved keyword).
fn arb_field_name() -> impl Strategy<Value = String> {
    "[a-z]{2,8}".prop_filter("must not be a reserved keyword", |name| {
        !FLUX_RESERVED.contains(&name.as_str())
    })
}

/// Supported field types for enum variants.
#[derive(Debug, Clone, PartialEq)]
enum FieldType {
    F64,
    Int,
    Bool,
    Str,
}

impl FieldType {
    fn type_annotation(&self) -> TypeAnnotation {
        match self {
            FieldType::F64 => TypeAnnotation::F64,
            FieldType::Int => TypeAnnotation::Int,
            FieldType::Bool => TypeAnnotation::Bool,
            FieldType::Str => TypeAnnotation::Str,
        }
    }
    
    fn type_str(&self) -> &'static str {
        match self {
            FieldType::F64 => "f64",
            FieldType::Int => "int",
            FieldType::Bool => "bool",
            FieldType::Str => "str",
        }
    }
}

/// Generate a random field type.
fn arb_field_type() -> impl Strategy<Value = FieldType> {
    prop_oneof![
        Just(FieldType::F64),
        Just(FieldType::Int),
        Just(FieldType::Bool),
        Just(FieldType::Str),
    ]
}

/// Generate an enum field definition.
fn arb_enum_field() -> impl Strategy<Value = (String, FieldType)> {
    (arb_field_name(), arb_field_type())
}

/// Generate an enum variant with 0-3 fields.
fn arb_enum_variant() -> impl Strategy<Value = (String, Vec<(String, FieldType)>)> {
    (arb_variant_name(), proptest::collection::vec(arb_enum_field(), 0..=3))
}

/// Generate an enum definition with 1-5 variants.
fn arb_enum_def() -> impl Strategy<Value = (String, Vec<(String, Vec<(String, FieldType)>)>)> {
    (arb_enum_name(), proptest::collection::vec(arb_enum_variant(), 1..=5))
        .prop_filter("variant names must be unique", |(_, variants)| {
            let names: std::collections::HashSet<&str> =
                variants.iter().map(|(n, _)| n.as_str()).collect();
            names.len() == variants.len()
        })
}

// ============================================================================
// Source construction helpers for Enum Definition
// ============================================================================

/// Build a Flux source string with an enum definition.
fn build_enum_source(enum_name: &str, variants: &[(String, Vec<(String, FieldType)>)]) -> String {
    let variant_strs: Vec<String> = variants
        .iter()
        .map(|(variant_name, fields)| {
            if fields.is_empty() {
                variant_name.clone()
            } else {
                let field_strs: Vec<String> = fields
                    .iter()
                    .map(|(fname, ftype)| format!("{}: {}", fname, ftype.type_str()))
                    .collect();
                format!("{}({})", variant_name, field_strs.join(", "))
            }
        })
        .collect();

    format!(
        "enum {} {{\n    {}\n}}\n\nstrategy Test {{\n    on bar {{\n        x = 1.0\n    }}\n}}\n",
        enum_name,
        variant_strs.join(",\n    ")
    )
}

// ============================================================================
// Property Tests for Enum Definition
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Property 1: Enum definition parse → pretty-print → parse round-trip.
    #[test]
    fn prop_enum_def_round_trip(
        (enum_name, variants) in arb_enum_def()
    ) {
        let source = build_enum_source(&enum_name, &variants);
        
        // Parse the source
        let tokens1 = lex_with_spans(&source).expect("first lex should succeed");
        let ast1 = parse(tokens1).expect("first parse should succeed");
        
        // Pretty-print the AST back to source
        let pretty = pretty_print_program(&ast1);
        
        // Parse the pretty-printed source
        let tokens2 = lex_with_spans(&pretty).expect("second lex should succeed");
        let ast2 = parse(tokens2).expect("second parse should succeed");
        
        // Compare the ASTs structurally (ignoring spans)
        assert_enums_equal(&ast1, &ast2);
    }
}

/// Compare two programs for structural equality of their enum definitions.
fn assert_enums_equal(prog1: &Program, prog2: &Program) {
    assert_eq!(
        prog1.enums.len(),
        prog2.enums.len(),
        "Enum count mismatch: {} vs {}",
        prog1.enums.len(),
        prog2.enums.len()
    );

    for (e1, e2) in prog1.enums.iter().zip(prog2.enums.iter()) {
        assert_eq!(e1.name, e2.name, "Enum name mismatch");
        assert_eq!(
            e1.variants.len(),
            e2.variants.len(),
            "Variant count mismatch for enum {}",
            e1.name
        );

        for (v1, v2) in e1.variants.iter().zip(e2.variants.iter()) {
            assert_eq!(v1.name, v2.name, "Variant name mismatch in enum {}", e1.name);
            assert_eq!(
                v1.fields.len(),
                v2.fields.len(),
                "Field count mismatch for variant {}.{}",
                e1.name,
                v1.name
            );

            for (f1, f2) in v1.fields.iter().zip(v2.fields.iter()) {
                assert_eq!(
                    f1.name, f2.name,
                    "Field name mismatch in enum {}.{}",
                    e1.name, v1.name
                );
                // Compare type annotations structurally
                assert_type_annotations_equal(&f1.field_type, &f2.field_type, &format!("{}.{}", e1.name, v1.name));
            }
        }
    }
}

/// Compare two type annotations for structural equality.
fn assert_type_annotations_equal(t1: &TypeAnnotation, t2: &TypeAnnotation, context: &str) {
    match (t1, t2) {
        (TypeAnnotation::F64, TypeAnnotation::F64) => {}
        (TypeAnnotation::Int, TypeAnnotation::Int) => {}
        (TypeAnnotation::Bool, TypeAnnotation::Bool) => {}
        (TypeAnnotation::Str, TypeAnnotation::Str) => {}
        (TypeAnnotation::Named(n1), TypeAnnotation::Named(n2)) => {
            assert_eq!(n1, n2, "Named type mismatch in {}", context);
        }
        (TypeAnnotation::Generic(n1, args1), TypeAnnotation::Generic(n2, args2)) => {
            assert_eq!(n1, n2, "Generic type name mismatch in {}", context);
            assert_eq!(args1.len(), args2.len(), "Generic arg count mismatch in {}", context);
            for (a1, a2) in args1.iter().zip(args2.iter()) {
                assert_type_annotations_equal(a1, a2, context);
            }
        }
        _ => panic!("Type annotation mismatch in {}: {:?} vs {:?}", context, t1, t2),
    }
}

// ============================================================================
// Property 2: Match Expression Parsing Round-Trip
// ============================================================================

/// Feature: flux-type-system, Property 2: Match Expression Parsing Round-Trip
///
/// **Validates: Requirements 3.2, 3.3, 3.4, 12.2**
///
/// For any valid match expression source text (with any number of arms,
/// variant patterns with bindings, and wildcard patterns), parsing to AST
/// and pretty-printing back to source and parsing again SHALL produce an
/// equivalent AST.

// ============================================================================
// Generators for Match Expression
// ============================================================================

/// Generate a binding variable name (lowercase).
fn arb_binding_name() -> impl Strategy<Value = String> {
    "[a-z]{1,3}".prop_filter("must not be a reserved keyword", |name| {
        !FLUX_RESERVED.contains(&name.as_str())
    })
}

/// A pattern for a match arm.
#[derive(Debug, Clone)]
enum TestPattern {
    /// Variant pattern: EnumName.VariantName(binding1, binding2)
    Variant {
        enum_name: String,
        variant_name: String,
        bindings: Vec<String>,
    },
    /// Wildcard pattern: _
    Wildcard,
}

/// Generate a pattern for a match arm.
fn arb_pattern(enum_name: String, variant_name: String, max_bindings: usize) -> impl Strategy<Value = TestPattern> {
    // 70% variant patterns, 30% wildcard
    prop_oneof![
        7 => (proptest::collection::vec(arb_binding_name(), 0..=max_bindings))
            .prop_map(move |bindings| TestPattern::Variant {
                enum_name: enum_name.clone(),
                variant_name: variant_name.clone(),
                bindings,
            }),
        3 => Just(TestPattern::Wildcard),
    ]
}

/// Generate a match arm pattern given enum info.
fn arb_match_arm_pattern(
    enum_name: String,
    variants: Vec<(String, usize)>, // (variant_name, field_count)
) -> impl Strategy<Value = TestPattern> {
    // Pick a random variant
    let variants_clone = variants.clone();
    let enum_name_clone = enum_name.clone();
    
    proptest::sample::select(variants)
        .prop_flat_map(move |(variant_name, field_count)| {
            let enum_name_inner = enum_name.clone();
            arb_pattern(enum_name_inner, variant_name, field_count)
        })
        .prop_map(move |p| p)
}

/// A match arm with pattern and body statements.
#[derive(Debug, Clone)]
struct TestMatchArm {
    pattern: TestPattern,
    body: Vec<String>, // Simple expressions as strings
}

/// Generate a match arm.
fn arb_match_arm(
    enum_name: String,
    variants: Vec<(String, usize)>,
) -> impl Strategy<Value = TestMatchArm> {
    let body_exprs = vec!["x = 1.0".to_string(), "y = 2.0".to_string()];
    
    arb_match_arm_pattern(enum_name, variants)
        .prop_map(move |pattern| TestMatchArm {
            pattern,
            body: body_exprs.clone(),
        })
}

/// Generate a match expression test case.
fn arb_match_test() -> impl Strategy<Value = (String, Vec<(String, usize)>, Vec<TestMatchArm>)> {
    // First generate an enum
    arb_enum_def().prop_flat_map(|(enum_name, variants)| {
        // Convert to (variant_name, field_count) pairs
        let variant_info: Vec<(String, usize)> = variants
            .iter()
            .map(|(vname, fields)| (vname.clone(), fields.len()))
            .collect();
        
        // Generate 1-5 match arms
        let enum_name_clone = enum_name.clone();
        let variant_info_clone = variant_info.clone();
        
        proptest::collection::vec(
            arb_match_arm(enum_name_clone, variant_info_clone),
            1..=5
        ).prop_map(move |arms| (enum_name.clone(), variant_info.clone(), arms))
    })
}

// ============================================================================
// Source construction helpers for Match Expression
// ============================================================================

/// Build a Flux source string with an enum definition and a match expression.
fn build_match_source(
    enum_name: &str,
    variants: &[(String, Vec<(String, FieldType)>)],
    arms: &[TestMatchArm],
) -> String {
    // Build enum definition
    let variant_strs: Vec<String> = variants
        .iter()
        .map(|(variant_name, fields)| {
            if fields.is_empty() {
                variant_name.clone()
            } else {
                let field_strs: Vec<String> = fields
                    .iter()
                    .map(|(fname, ftype)| format!("{}: {}", fname, ftype.type_str()))
                    .collect();
                format!("{}({})", variant_name, field_strs.join(", "))
            }
        })
        .collect();

    let enum_def = format!(
        "enum {} {{\n    {}\n}}",
        enum_name,
        variant_strs.join(",\n    ")
    );

    // Build match expression
    let arm_strs: Vec<String> = arms
        .iter()
        .map(|arm| {
            let pattern_str = match &arm.pattern {
                TestPattern::Variant { enum_name, variant_name, bindings } => {
                    if bindings.is_empty() {
                        format!("{}.{}", enum_name, variant_name)
                    } else {
                        format!("{}.{}({})", enum_name, variant_name, bindings.join(", "))
                    }
                }
                TestPattern::Wildcard => "_".to_string(),
            };
            
            let body_str = arm.body.join("\n        ");
            format!("    {} => {{\n        {}\n    }}", pattern_str, body_str)
        })
        .collect();

    let match_expr = format!(
        "match value {{\n{}\n}}",
        arm_strs.join("\n")
    );

    format!(
        "{}\n\nstrategy Test {{\n    on bar {{\n        {}\n    }}\n}}\n",
        enum_def, match_expr
    )
}

// ============================================================================
// Property Tests for Match Expression
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Property 2: Match expression parse → pretty-print → parse round-trip.
    #[test]
    fn prop_match_expr_round_trip(
        (enum_name, variants_with_fields) in arb_enum_def()
    ) {
        // Generate match arms based on the enum variants
        let variant_info: Vec<(String, usize)> = variants_with_fields
            .iter()
            .map(|(vname, fields)| (vname.clone(), fields.len()))
            .collect();
        
        // Generate 1-5 match arms with wildcard at the end
        let mut arms: Vec<TestMatchArm> = vec![];
        
        // Add a few variant patterns
        for (vname, field_count) in variant_info.iter().take(3) {
            arms.push(TestMatchArm {
                pattern: TestPattern::Variant {
                    enum_name: enum_name.clone(),
                    variant_name: vname.clone(),
                    bindings: (0..*field_count).map(|i| format!("b{}", i)).collect(),
                },
                body: vec!["x = 1.0".to_string()],
            });
        }
        
        // Add wildcard at the end for exhaustiveness
        arms.push(TestMatchArm {
            pattern: TestPattern::Wildcard,
            body: vec!["x = 0.0".to_string()],
        });
        
        let source = build_match_source(&enum_name, &variants_with_fields, &arms);
        
        // Parse the source
        let tokens1 = lex_with_spans(&source).expect("first lex should succeed");
        let ast1 = parse(tokens1).expect("first parse should succeed");
        
        // Pretty-print the AST back to source
        let pretty = pretty_print_program(&ast1);
        
        // Parse the pretty-printed source
        let tokens2 = lex_with_spans(&pretty).expect("second lex should succeed");
        let ast2 = parse(tokens2).expect("second parse should succeed");
        
        // Compare the ASTs structurally (ignoring spans)
        assert_match_exprs_equal(&ast1, &ast2);
    }
}

/// Compare two programs for structural equality of their match expressions.
fn assert_match_exprs_equal(prog1: &Program, prog2: &Program) {
    // First compare enums
    assert_enums_equal(prog1, prog2);
    
    // Then compare match expressions in the strategy bodies
    // We need to extract match expressions from the strategy bodies
    let match_exprs1 = extract_match_exprs(prog1);
    let match_exprs2 = extract_match_exprs(prog2);
    
    assert_eq!(
        match_exprs1.len(),
        match_exprs2.len(),
        "Match expression count mismatch"
    );
    
    for (m1, m2) in match_exprs1.iter().zip(match_exprs2.iter()) {
        compare_match_exprs(m1, m2);
    }
}

/// Extract match expressions from a program's strategy body.
fn extract_match_exprs(prog: &Program) -> Vec<MatchExpr> {
    let mut matches = vec![];
    
    for item in &prog.strategy.body {
        if let flux_compiler::parser::StrategyItem::EventHandler(handler) = item {
            for stmt in &handler.body {
                extract_match_from_stmt(stmt, &mut matches);
            }
        }
    }
    
    matches
}

/// Recursively extract match expressions from statements.
fn extract_match_from_stmt(stmt: &Stmt, matches: &mut Vec<MatchExpr>) {
    match stmt {
        Stmt::Expr(expr_stmt) => {
            extract_match_from_expr(&expr_stmt.expr, matches);
        }
        Stmt::Assignment(assign) => {
            extract_match_from_expr(&assign.value, matches);
        }
        Stmt::If(if_stmt) => {
            for s in &if_stmt.body {
                extract_match_from_stmt(s, matches);
            }
            for s in if_stmt.else_body.iter().flatten() {
                extract_match_from_stmt(s, matches);
            }
        }
        Stmt::For(for_loop) => {
            for s in &for_loop.body {
                extract_match_from_stmt(s, matches);
            }
        }
        Stmt::While(while_loop) => {
            for s in &while_loop.body {
                extract_match_from_stmt(s, matches);
            }
        }
        Stmt::Return(ret) => {
            if let Some(expr) = &ret.value {
                extract_match_from_expr(expr, matches);
            }
        }
    }
}

/// Recursively extract match expressions from expressions.
fn extract_match_from_expr(expr: &Expr, matches: &mut Vec<MatchExpr>) {
    match &expr.kind {
        ExprKind::Match(m) => {
            matches.push(m.clone());
        }
        ExprKind::BinaryOp { left, right, .. } => {
            extract_match_from_expr(left, matches);
            extract_match_from_expr(right, matches);
        }
        ExprKind::UnaryOp { operand, .. } => {
            extract_match_from_expr(operand, matches);
        }
        ExprKind::FunctionCall { function, args } => {
            extract_match_from_expr(function, matches);
            for arg in args {
                extract_match_from_expr(arg, matches);
            }
        }
        ExprKind::MethodCall { receiver, args, .. } => {
            extract_match_from_expr(receiver, matches);
            for arg in args {
                extract_match_from_expr(arg, matches);
            }
        }
        ExprKind::MemberAccess { object, .. } => {
            extract_match_from_expr(object, matches);
        }
        ExprKind::IndexAccess { object, index } => {
            extract_match_from_expr(object, matches);
            extract_match_from_expr(index, matches);
        }
        ExprKind::ListLiteral(elems) => {
            for e in elems {
                extract_match_from_expr(e, matches);
            }
        }
        ExprKind::StructLiteral { fields, .. } => {
            for (_, v) in fields {
                extract_match_from_expr(v, matches);
            }
        }
        ExprKind::EnumConstruction { args, .. } => {
            for arg in args {
                extract_match_from_expr(arg, matches);
            }
        }
        _ => {}
    }
}

/// Compare two match expressions for structural equality.
fn compare_match_exprs(m1: &MatchExpr, m2: &MatchExpr) {
    // Compare scrutinee expressions
    compare_exprs(&m1.scrutinee, &m2.scrutinee);
    
    // Compare arm count
    assert_eq!(
        m1.arms.len(),
        m2.arms.len(),
        "Match arm count mismatch"
    );
    
    // Compare each arm
    for (arm1, arm2) in m1.arms.iter().zip(m2.arms.iter()) {
        compare_patterns(&arm1.pattern, &arm2.pattern);
        compare_stmts(&arm1.body, &arm2.body);
    }
}

/// Compare two expressions for structural equality.
fn compare_exprs(e1: &Expr, e2: &Expr) {
    match (&e1.kind, &e2.kind) {
        (ExprKind::IntLiteral(n1), ExprKind::IntLiteral(n2)) => {
            assert_eq!(n1, n2, "Int literal mismatch");
        }
        (ExprKind::FloatLiteral(f1), ExprKind::FloatLiteral(f2)) => {
            assert!((f1 - f2).abs() < 1e-10, "Float literal mismatch: {} vs {}", f1, f2);
        }
        (ExprKind::StringLiteral(s1), ExprKind::StringLiteral(s2)) => {
            assert_eq!(s1, s2, "String literal mismatch");
        }
        (ExprKind::BoolLiteral(b1), ExprKind::BoolLiteral(b2)) => {
            assert_eq!(b1, b2, "Bool literal mismatch");
        }
        (ExprKind::Ident(n1), ExprKind::Ident(n2)) => {
            assert_eq!(n1, n2, "Identifier mismatch");
        }
        (ExprKind::MemberAccess { object: o1, field: f1 }, ExprKind::MemberAccess { object: o2, field: f2 }) => {
            compare_exprs(o1, o2);
            assert_eq!(f1, f2, "Field name mismatch");
        }
        (ExprKind::EnumConstruction { enum_name: en1, variant_name: vn1, args: a1 },
         ExprKind::EnumConstruction { enum_name: en2, variant_name: vn2, args: a2 }) => {
            assert_eq!(en1, en2, "Enum name mismatch in construction");
            assert_eq!(vn1, vn2, "Variant name mismatch in construction");
            assert_eq!(a1.len(), a2.len(), "Enum construction arg count mismatch");
            for (arg1, arg2) in a1.iter().zip(a2.iter()) {
                compare_exprs(arg1, arg2);
            }
        }
        (ExprKind::Match(m1), ExprKind::Match(m2)) => {
            compare_match_exprs(m1, m2);
        }
        _ => {
            // For other expression types, just check they're the same variant
            assert_eq!(
                std::mem::discriminant(&e1.kind),
                std::mem::discriminant(&e2.kind),
                "Expression kind mismatch: {:?} vs {:?}",
                e1.kind,
                e2.kind
            );
        }
    }
}

/// Compare two patterns for structural equality.
fn compare_patterns(p1: &Pattern, p2: &Pattern) {
    match (p1, p2) {
        (Pattern::Variant { enum_name: en1, variant_name: vn1, bindings: b1, .. },
         Pattern::Variant { enum_name: en2, variant_name: vn2, bindings: b2, .. }) => {
            assert_eq!(en1, en2, "Pattern enum name mismatch");
            assert_eq!(vn1, vn2, "Pattern variant name mismatch");
            assert_eq!(b1, b2, "Pattern bindings mismatch");
        }
        (Pattern::Wildcard { .. }, Pattern::Wildcard { .. }) => {}
        _ => panic!("Pattern kind mismatch: {:?} vs {:?}", p1, p2),
    }
}

/// Compare two statement vectors for structural equality.
fn compare_stmts(s1: &[Stmt], s2: &[Stmt]) {
    assert_eq!(s1.len(), s2.len(), "Statement count mismatch");
    
    for (stmt1, stmt2) in s1.iter().zip(s2.iter()) {
        match (stmt1, stmt2) {
            (Stmt::Assignment(a1), Stmt::Assignment(a2)) => {
                compare_exprs(&a1.value, &a2.value);
            }
            (Stmt::Expr(e1), Stmt::Expr(e2)) => {
                compare_exprs(&e1.expr, &e2.expr);
            }
            (Stmt::Return(r1), Stmt::Return(r2)) => {
                assert_eq!(r1.value.is_some(), r2.value.is_some(), "Return value presence mismatch");
                if let (Some(v1), Some(v2)) = (&r1.value, &r2.value) {
                    compare_exprs(v1, v2);
                }
            }
            _ => {
                assert_eq!(
                    std::mem::discriminant(stmt1),
                    std::mem::discriminant(stmt2),
                    "Statement kind mismatch"
                );
            }
        }
    }
}


// ============================================================================
// Property 3: Impl Block Parsing Round-Trip (impl portion)
// ============================================================================

/// Feature: flux-type-system, Property 3: Impl Block and Trait Definition Parsing Round-Trip
///
/// **Validates: Requirements 4.2, 4.3, 4.4, 12.3**
///
/// For any valid impl block source text (with any number of methods,
/// self/static classification, and varying parameters), parsing to AST
/// and pretty-printing back to source and parsing again SHALL produce an
/// equivalent AST.

// ============================================================================
// Generators for Impl Block
// ============================================================================

/// Generate a valid struct/type name for impl block target (capitalized).
fn arb_struct_name() -> impl Strategy<Value = String> {
    "[A-Z][a-z]{2,7}".prop_filter("must not be a reserved keyword", |name| {
        !FLUX_RESERVED.contains(&name.as_str())
    })
}

/// Generate a valid method name (lowercase).
fn arb_method_name() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9]{1,6}".prop_filter("must not be a reserved keyword", |name| {
        !FLUX_RESERVED.contains(&name.as_str())
    })
}

/// Generate a valid parameter name (lowercase).
fn arb_param_name() -> impl Strategy<Value = String> {
    "[a-z]{2,5}".prop_filter("must not be a reserved keyword", |name| {
        !FLUX_RESERVED.contains(&name.as_str())
    })
}

/// Whether a method has self as its first parameter.
#[derive(Debug, Clone)]
enum MethodKind {
    Instance, // has `self` parameter
    Static,   // no `self` parameter
}

/// A generated method definition.
#[derive(Debug, Clone)]
struct TestMethod {
    name: String,
    kind: MethodKind,
    params: Vec<(String, FieldType)>, // params other than self
    return_type: Option<FieldType>,
}

/// Generate a method definition.
fn arb_method() -> impl Strategy<Value = TestMethod> {
    (
        arb_method_name(),
        prop_oneof![Just(MethodKind::Instance), Just(MethodKind::Static)],
        proptest::collection::vec((arb_param_name(), arb_field_type()), 0..=2),
        proptest::option::of(arb_field_type()),
    )
        .prop_filter("param names must be unique", |(_, _, params, _)| {
            let names: std::collections::HashSet<&str> =
                params.iter().map(|(n, _)| n.as_str()).collect();
            names.len() == params.len()
        })
        .prop_map(|(name, kind, params, return_type)| TestMethod {
            name,
            kind,
            params,
            return_type,
        })
}

/// Generate an impl block test case: struct name + 1-3 methods with unique names.
fn arb_impl_block_test() -> impl Strategy<Value = (String, Vec<TestMethod>)> {
    (
        arb_struct_name(),
        proptest::collection::vec(arb_method(), 1..=3),
    )
        .prop_filter("method names must be unique", |(_, methods)| {
            let names: std::collections::HashSet<&str> =
                methods.iter().map(|m| m.name.as_str()).collect();
            names.len() == methods.len()
        })
}

// ============================================================================
// Source construction helpers for Impl Block
// ============================================================================

/// Build a Flux source string with a struct definition and an impl block.
fn build_impl_source(struct_name: &str, methods: &[TestMethod]) -> String {
    // Build struct definition (simple single field)
    let struct_def = format!(
        "struct {} {{\n    field: f64\n}}\n",
        struct_name
    );

    // Build impl block
    let method_strs: Vec<String> = methods
        .iter()
        .map(|method| {
            // Build parameter list
            let mut params = Vec::new();
            if matches!(method.kind, MethodKind::Instance) {
                params.push("self".to_string());
            }
            for (pname, ptype) in &method.params {
                params.push(format!("{}: {}", pname, ptype.type_str()));
            }
            let params_str = params.join(", ");

            // Build return type
            let ret_str = match &method.return_type {
                Some(rt) => format!(" -> {}", rt.type_str()),
                None => String::new(),
            };

            // Build method body (simple return statement)
            let body = if method.params.is_empty() {
                "        return 1.0".to_string()
            } else {
                format!("        return {}", method.params[0].0)
            };

            format!(
                "    fn {}({}){} {{\n{}\n    }}",
                method.name, params_str, ret_str, body
            )
        })
        .collect();

    let impl_block = format!(
        "impl {} {{\n{}\n}}\n",
        struct_name,
        method_strs.join("\n")
    );

    format!(
        "{}\n{}\nstrategy Test {{\n    on bar {{\n        x = 1.0\n    }}\n}}\n",
        struct_def, impl_block
    )
}

// ============================================================================
// Property Tests for Impl Block
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Property 3: Impl block parse → pretty-print → parse round-trip.
    #[test]
    fn prop_impl_block_round_trip(
        (struct_name, methods) in arb_impl_block_test()
    ) {
        let source = build_impl_source(&struct_name, &methods);

        // Parse the source
        let tokens1 = lex_with_spans(&source).expect("first lex should succeed");
        let ast1 = parse(tokens1).expect("first parse should succeed");

        // Pretty-print the AST back to source
        let pretty = pretty_print_program(&ast1);

        // Parse the pretty-printed source
        let tokens2 = lex_with_spans(&pretty).expect("second lex should succeed");
        let ast2 = parse(tokens2).expect("second parse should succeed");

        // Compare the ASTs structurally (ignoring spans)
        assert_impl_blocks_equal(&ast1, &ast2);
    }
}

/// Compare two programs for structural equality of their impl blocks.
fn assert_impl_blocks_equal(prog1: &Program, prog2: &Program) {
    assert_eq!(
        prog1.impl_blocks.len(),
        prog2.impl_blocks.len(),
        "Impl block count mismatch: {} vs {}",
        prog1.impl_blocks.len(),
        prog2.impl_blocks.len()
    );

    for (ib1, ib2) in prog1.impl_blocks.iter().zip(prog2.impl_blocks.iter()) {
        // Same target type
        assert_eq!(
            ib1.target_type, ib2.target_type,
            "Impl block target type mismatch"
        );

        // Same trait name (both None for inherent impls)
        assert_eq!(
            ib1.trait_name, ib2.trait_name,
            "Impl block trait name mismatch"
        );

        // Same method count
        assert_eq!(
            ib1.methods.len(),
            ib2.methods.len(),
            "Method count mismatch for impl {}",
            ib1.target_type
        );

        // Compare each method
        for (m1, m2) in ib1.methods.iter().zip(ib2.methods.iter()) {
            assert_eq!(
                m1.name, m2.name,
                "Method name mismatch in impl {}",
                ib1.target_type
            );

            // Same parameter count
            assert_eq!(
                m1.params.len(),
                m2.params.len(),
                "Param count mismatch for method {}.{}",
                ib1.target_type,
                m1.name
            );

            // Compare parameter names and types
            for (p1, p2) in m1.params.iter().zip(m2.params.iter()) {
                assert_eq!(
                    p1.name, p2.name,
                    "Param name mismatch in method {}.{}",
                    ib1.target_type,
                    m1.name
                );

                // Compare param type annotations (both should be present or absent)
                match (&p1.param_type, &p2.param_type) {
                    (Some(t1), Some(t2)) => {
                        assert_type_annotations_equal(
                            t1,
                            t2,
                            &format!("{}.{}", ib1.target_type, m1.name),
                        );
                    }
                    (None, None) => {} // both have no type annotation (e.g., `self`)
                    _ => panic!(
                        "Param type presence mismatch for param '{}' in method {}.{}",
                        p1.name, ib1.target_type, m1.name
                    ),
                }
            }

            // Compare return types
            match (&m1.return_type, &m2.return_type) {
                (Some(t1), Some(t2)) => {
                    assert_type_annotations_equal(
                        t1,
                        t2,
                        &format!("{}.{} return type", ib1.target_type, m1.name),
                    );
                }
                (None, None) => {}
                _ => panic!(
                    "Return type presence mismatch for method {}.{}",
                    ib1.target_type, m1.name
                ),
            }
        }
    }
}


// ============================================================================
// Property 4: Generic Parameter Parsing Round-Trip
// ============================================================================

/// Feature: flux-type-system, Property 4: Generic Parameter Parsing Round-Trip
///
/// **Validates: Requirements 7.1, 8.1, 9.1, 12.3, 12.4**
///
/// For any valid generic struct definition, generic function definition, or
/// trait-bounded type parameter source text, parsing to AST and pretty-printing
/// back to source and parsing again SHALL produce an equivalent AST.

// ============================================================================
// Generators for Generic Parameters
// ============================================================================

/// Generate a valid type parameter name (single uppercase letter).
fn arb_type_param_name() -> impl Strategy<Value = String> {
    "[A-Z]".prop_map(|s| s.to_string())
}

/// Generate a valid trait bound name (capitalized, not a reserved keyword).
fn arb_trait_bound_name() -> impl Strategy<Value = String> {
    "[A-Z][a-z]{3,8}".prop_filter("must not be a reserved keyword", |name| {
        !FLUX_RESERVED.contains(&name.as_str())
    })
}

/// A type parameter with an optional trait bound.
#[derive(Debug, Clone)]
struct TestTypeParam {
    name: String,
    bound: Option<String>,
}

/// Generate a type parameter (with or without a bound).
fn arb_type_param() -> impl Strategy<Value = TestTypeParam> {
    (arb_type_param_name(), proptest::option::of(arb_trait_bound_name()))
        .prop_map(|(name, bound)| TestTypeParam { name, bound })
}

/// Generate 1-3 type parameters with unique names.
fn arb_type_params() -> impl Strategy<Value = Vec<TestTypeParam>> {
    proptest::collection::vec(arb_type_param(), 1..=3)
        .prop_filter("type param names must be unique", |params| {
            let names: std::collections::HashSet<&str> =
                params.iter().map(|p| p.name.as_str()).collect();
            names.len() == params.len()
        })
}

/// A generic struct test case.
#[derive(Debug, Clone)]
struct GenericStructTest {
    name: String,
    type_params: Vec<TestTypeParam>,
    fields: Vec<(String, GenericFieldType)>,
}

/// Field types that can reference type parameters.
#[derive(Debug, Clone)]
enum GenericFieldType {
    Concrete(FieldType),
    TypeParam(String), // references a type parameter name like T
}

impl GenericFieldType {
    fn to_source(&self) -> String {
        match self {
            GenericFieldType::Concrete(ft) => ft.type_str().to_string(),
            GenericFieldType::TypeParam(name) => name.clone(),
        }
    }
}

/// Generate a field type that may reference one of the type parameters.
fn arb_generic_field_type(type_param_names: Vec<String>) -> impl Strategy<Value = GenericFieldType> {
    let concrete = arb_field_type().prop_map(GenericFieldType::Concrete);
    if type_param_names.is_empty() {
        concrete.boxed()
    } else {
        prop_oneof![
            3 => concrete,
            2 => proptest::sample::select(type_param_names).prop_map(GenericFieldType::TypeParam),
        ].boxed()
    }
}

/// Generate a generic struct test case.
fn arb_generic_struct_test() -> impl Strategy<Value = GenericStructTest> {
    (arb_struct_name(), arb_type_params()).prop_flat_map(|(name, type_params)| {
        let param_names: Vec<String> = type_params.iter().map(|p| p.name.clone()).collect();
        let fields_strategy = proptest::collection::vec(
            (arb_field_name(), arb_generic_field_type(param_names)),
            1..=3,
        ).prop_filter("field names must be unique", |fields| {
            let names: std::collections::HashSet<&str> =
                fields.iter().map(|(n, _)| n.as_str()).collect();
            names.len() == fields.len()
        });

        (Just(name), Just(type_params), fields_strategy)
    }).prop_map(|(name, type_params, fields)| GenericStructTest {
        name,
        type_params,
        fields,
    })
}

/// A generic function test case.
#[derive(Debug, Clone)]
struct GenericFnTest {
    name: String,
    type_params: Vec<TestTypeParam>,
    params: Vec<(String, GenericFieldType)>,
    return_type: Option<GenericFieldType>,
}

/// Generate a generic function test case.
fn arb_generic_fn_test() -> impl Strategy<Value = GenericFnTest> {
    (arb_method_name(), arb_type_params()).prop_flat_map(|(name, type_params)| {
        let param_names: Vec<String> = type_params.iter().map(|p| p.name.clone()).collect();
        let param_names2 = param_names.clone();
        let params_strategy = proptest::collection::vec(
            (arb_param_name(), arb_generic_field_type(param_names)),
            1..=3,
        ).prop_filter("param names must be unique and not 'self'", |params| {
            let names: std::collections::HashSet<&str> =
                params.iter().map(|(n, _)| n.as_str()).collect();
            names.len() == params.len() && !names.contains("self")
        });

        let ret_strategy = proptest::option::of(arb_generic_field_type(param_names2));

        (Just(name), Just(type_params), params_strategy, ret_strategy)
    }).prop_map(|(name, type_params, params, return_type)| GenericFnTest {
        name,
        type_params,
        params,
        return_type,
    })
}

// ============================================================================
// Source construction helpers for Generic Parameters
// ============================================================================

/// Format type params to source: `[T, U: MyTrait]`
fn format_test_type_params(type_params: &[TestTypeParam]) -> String {
    if type_params.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = type_params
        .iter()
        .map(|tp| {
            if let Some(ref bound) = tp.bound {
                format!("{}: {}", tp.name, bound)
            } else {
                tp.name.clone()
            }
        })
        .collect();
    format!("[{}]", parts.join(", "))
}

/// Build a Flux source with a generic struct definition.
fn build_generic_struct_source(test: &GenericStructTest) -> String {
    let type_params_str = format_test_type_params(&test.type_params);
    let field_strs: Vec<String> = test.fields
        .iter()
        .map(|(name, ftype)| format!("    {}: {}", name, ftype.to_source()))
        .collect();

    format!(
        "struct {}{} {{\n{}\n}}\n\nstrategy Test {{\n    on bar {{\n        x = 1.0\n    }}\n}}\n",
        test.name,
        type_params_str,
        field_strs.join(",\n")
    )
}

/// Build a Flux source with a generic function definition.
fn build_generic_fn_source(test: &GenericFnTest) -> String {
    let type_params_str = format_test_type_params(&test.type_params);
    let param_strs: Vec<String> = test.params
        .iter()
        .map(|(name, ftype)| format!("{}: {}", name, ftype.to_source()))
        .collect();
    let ret_str = match &test.return_type {
        Some(rt) => format!(" -> {}", rt.to_source()),
        None => String::new(),
    };

    format!(
        "fn {}{} ({}){} {{\n    return 1.0\n}}\n\nstrategy Test {{\n    on bar {{\n        x = 1.0\n    }}\n}}\n",
        test.name,
        type_params_str,
        param_strs.join(", "),
        ret_str
    )
}

/// Build a Flux source with a trait-bounded generic function.
fn build_bounded_generic_fn_source(test: &GenericFnTest) -> String {
    // Force all type params to have bounds for this test variant
    let bounded_params: Vec<TestTypeParam> = test.type_params.iter().map(|tp| {
        TestTypeParam {
            name: tp.name.clone(),
            bound: Some(tp.bound.clone().unwrap_or_else(|| "MyTrait".to_string())),
        }
    }).collect();

    let type_params_str = format_test_type_params(&bounded_params);
    let param_strs: Vec<String> = test.params
        .iter()
        .map(|(name, ftype)| format!("{}: {}", name, ftype.to_source()))
        .collect();
    let ret_str = match &test.return_type {
        Some(rt) => format!(" -> {}", rt.to_source()),
        None => String::new(),
    };

    format!(
        "fn {}{} ({}){} {{\n    return 1.0\n}}\n\nstrategy Test {{\n    on bar {{\n        x = 1.0\n    }}\n}}\n",
        test.name,
        type_params_str,
        param_strs.join(", "),
        ret_str
    )
}

// ============================================================================
// Comparison helpers for Generic Parameters
// ============================================================================

/// Compare two TypeParam vectors for structural equality.
fn assert_type_params_equal(params1: &[TypeParam], params2: &[TypeParam], context: &str) {
    assert_eq!(
        params1.len(),
        params2.len(),
        "Type param count mismatch in {}: {} vs {}",
        context,
        params1.len(),
        params2.len()
    );

    for (p1, p2) in params1.iter().zip(params2.iter()) {
        assert_eq!(
            p1.name, p2.name,
            "Type param name mismatch in {}: '{}' vs '{}'",
            context, p1.name, p2.name
        );
        assert_eq!(
            p1.bound, p2.bound,
            "Type param bound mismatch in {} for param '{}': {:?} vs {:?}",
            context, p1.name, p1.bound, p2.bound
        );
    }
}

/// Compare two programs for structural equality of their generic struct definitions.
fn assert_generic_structs_equal(prog1: &Program, prog2: &Program) {
    assert_eq!(
        prog1.structs.len(),
        prog2.structs.len(),
        "Struct count mismatch: {} vs {}",
        prog1.structs.len(),
        prog2.structs.len()
    );

    for (s1, s2) in prog1.structs.iter().zip(prog2.structs.iter()) {
        assert_eq!(s1.name, s2.name, "Struct name mismatch");

        // Compare type parameters
        assert_type_params_equal(
            &s1.type_params,
            &s2.type_params,
            &format!("struct {}", s1.name),
        );

        // Compare fields
        assert_eq!(
            s1.fields.len(),
            s2.fields.len(),
            "Field count mismatch for struct {}",
            s1.name
        );
        for (f1, f2) in s1.fields.iter().zip(s2.fields.iter()) {
            assert_eq!(
                f1.name, f2.name,
                "Field name mismatch in struct {}",
                s1.name
            );
            assert_type_annotations_equal(
                &f1.field_type,
                &f2.field_type,
                &format!("struct {}.{}", s1.name, f1.name),
            );
        }
    }
}

/// Compare two programs for structural equality of their generic function definitions.
fn assert_generic_fns_equal(prog1: &Program, prog2: &Program) {
    assert_eq!(
        prog1.functions.len(),
        prog2.functions.len(),
        "Function count mismatch: {} vs {}",
        prog1.functions.len(),
        prog2.functions.len()
    );

    for (f1, f2) in prog1.functions.iter().zip(prog2.functions.iter()) {
        assert_eq!(f1.name, f2.name, "Function name mismatch");

        // Compare type parameters
        assert_type_params_equal(
            &f1.type_params,
            &f2.type_params,
            &format!("fn {}", f1.name),
        );

        // Compare params
        assert_eq!(
            f1.params.len(),
            f2.params.len(),
            "Param count mismatch for fn {}",
            f1.name
        );
        for (p1, p2) in f1.params.iter().zip(f2.params.iter()) {
            assert_eq!(
                p1.name, p2.name,
                "Param name mismatch in fn {}",
                f1.name
            );
            match (&p1.param_type, &p2.param_type) {
                (Some(t1), Some(t2)) => {
                    assert_type_annotations_equal(t1, t2, &format!("fn {}.{}", f1.name, p1.name));
                }
                (None, None) => {}
                _ => panic!(
                    "Param type presence mismatch for '{}' in fn {}",
                    p1.name, f1.name
                ),
            }
        }

        // Compare return types
        match (&f1.return_type, &f2.return_type) {
            (Some(t1), Some(t2)) => {
                assert_type_annotations_equal(t1, t2, &format!("fn {} return type", f1.name));
            }
            (None, None) => {}
            _ => panic!("Return type presence mismatch for fn {}", f1.name),
        }
    }
}

// ============================================================================
// Property Tests for Generic Parameters
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Property 4a: Generic struct definition parse → pretty-print → parse round-trip.
    #[test]
    fn prop_generic_struct_round_trip(
        test in arb_generic_struct_test()
    ) {
        let source = build_generic_struct_source(&test);

        // Parse the source
        let tokens1 = lex_with_spans(&source).expect("first lex should succeed");
        let ast1 = parse(tokens1).expect("first parse should succeed");

        // Pretty-print the AST back to source
        let pretty = pretty_print_program(&ast1);

        // Parse the pretty-printed source
        let tokens2 = lex_with_spans(&pretty).expect("second lex should succeed");
        let ast2 = parse(tokens2).expect("second parse should succeed");

        // Compare the ASTs structurally (ignoring spans)
        assert_generic_structs_equal(&ast1, &ast2);
    }

    /// Property 4b: Generic function definition parse → pretty-print → parse round-trip.
    #[test]
    fn prop_generic_fn_round_trip(
        test in arb_generic_fn_test()
    ) {
        let source = build_generic_fn_source(&test);

        // Parse the source
        let tokens1 = lex_with_spans(&source).expect("first lex should succeed");
        let ast1 = parse(tokens1).expect("first parse should succeed");

        // Pretty-print the AST back to source
        let pretty = pretty_print_program(&ast1);

        // Parse the pretty-printed source
        let tokens2 = lex_with_spans(&pretty).expect("second lex should succeed");
        let ast2 = parse(tokens2).expect("second parse should succeed");

        // Compare the ASTs structurally (ignoring spans)
        assert_generic_fns_equal(&ast1, &ast2);
    }

    /// Property 4c: Trait-bounded generic function parse → pretty-print → parse round-trip.
    #[test]
    fn prop_bounded_generic_fn_round_trip(
        test in arb_generic_fn_test()
    ) {
        let source = build_bounded_generic_fn_source(&test);

        // Parse the source
        let tokens1 = lex_with_spans(&source).expect("first lex should succeed");
        let ast1 = parse(tokens1).expect("first parse should succeed");

        // Pretty-print the AST back to source
        let pretty = pretty_print_program(&ast1);

        // Parse the pretty-printed source
        let tokens2 = lex_with_spans(&pretty).expect("second lex should succeed");
        let ast2 = parse(tokens2).expect("second parse should succeed");

        // Compare the ASTs structurally (ignoring spans)
        assert_generic_fns_equal(&ast1, &ast2);
    }
}


// ============================================================================
// Property 18: Parser Error Span Reporting
// ============================================================================

/// Feature: flux-type-system, Property 18: Parser Error Span Reporting
///
/// **Validates: Requirements 13.1, 13.2**
///
/// For any malformed enum definition, match expression, impl block, or trait
/// definition, the parser SHALL report an error that includes a source span
/// pointing to or near the location of the syntax error.

// ============================================================================
// Generators for Malformed Source
// ============================================================================

/// Generate malformed enum definitions that should cause parse errors.
fn arb_malformed_enum() -> impl Strategy<Value = String> {
    let enum_name = arb_enum_name();
    let variant_name = arb_variant_name();

    (enum_name, variant_name).prop_flat_map(|(ename, vname)| {
        prop_oneof![
            // Missing opening brace after enum name
            Just(format!(
                "enum {} \n    {}\n}}\n\nstrategy Test {{\n    on bar {{\n        x = 1.0\n    }}\n}}",
                ename, vname
            )),
            // Missing closing brace
            Just(format!(
                "enum {} {{\n    {}\n\nstrategy Test {{\n    on bar {{\n        x = 1.0\n    }}\n}}",
                ename, vname
            )),
            // Missing colon in field type annotation
            Just(format!(
                "enum {} {{\n    {}(price f64)\n}}\n\nstrategy Test {{\n    on bar {{\n        x = 1.0\n    }}\n}}",
                ename, vname
            )),
            // Missing closing paren for variant fields
            Just(format!(
                "enum {} {{\n    {}(price: f64\n}}\n\nstrategy Test {{\n    on bar {{\n        x = 1.0\n    }}\n}}",
                ename, vname
            )),
            // Number where variant name expected
            Just(format!(
                "enum {} {{\n    123\n}}\n\nstrategy Test {{\n    on bar {{\n        x = 1.0\n    }}\n}}",
                ename
            )),
        ]
    })
}

/// Generate malformed match expressions that should cause parse errors.
fn arb_malformed_match() -> impl Strategy<Value = String> {
    let enum_name = arb_enum_name();
    let variant_name = arb_variant_name();

    (enum_name, variant_name).prop_flat_map(|(ename, vname)| {
        prop_oneof![
            // Missing opening brace after scrutinee
            Just(format!(
                "enum {} {{\n    {}\n}}\n\nstrategy Test {{\n    on bar {{\n        match value\n            {} => {{ x = 1.0 }}\n        }}\n    }}\n}}",
                ename, vname, vname
            )),
            // Missing arrow (=>) in match arm
            Just(format!(
                "enum {} {{\n    {}\n}}\n\nstrategy Test {{\n    on bar {{\n        match value {{\n            {}.{} {{ x = 1.0 }}\n        }}\n    }}\n}}",
                ename, vname, ename, vname
            )),
            // Missing body braces in match arm
            Just(format!(
                "enum {} {{\n    {}\n}}\n\nstrategy Test {{\n    on bar {{\n        match value {{\n            {}.{} => x = 1.0\n        }}\n    }}\n}}",
                ename, vname, ename, vname
            )),
            // Missing closing brace for match
            Just(format!(
                "enum {} {{\n    {}\n}}\n\nstrategy Test {{\n    on bar {{\n        match value {{\n            {}.{} => {{ x = 1.0 }}\n    }}\n}}",
                ename, vname, ename, vname
            )),
        ]
    })
}

/// Generate malformed impl blocks that should cause parse errors.
fn arb_malformed_impl() -> impl Strategy<Value = String> {
    let struct_name = arb_struct_name();
    let method_name = arb_method_name();

    (struct_name, method_name).prop_flat_map(|(sname, mname)| {
        prop_oneof![
            // Missing struct name after impl
            Just(format!(
                "struct {} {{\n    val: f64\n}}\n\nimpl {{\n    fn {}(self) -> f64 {{\n        return self.val\n    }}\n}}\n\nstrategy Test {{\n    on bar {{\n        x = 1.0\n    }}\n}}",
                sname, mname
            )),
            // Missing opening brace for impl block
            Just(format!(
                "struct {} {{\n    val: f64\n}}\n\nimpl {}\n    fn {}(self) -> f64 {{\n        return self.val\n    }}\n}}\n\nstrategy Test {{\n    on bar {{\n        x = 1.0\n    }}\n}}",
                sname, sname, mname
            )),
            // Non-fn item inside impl block
            Just(format!(
                "struct {} {{\n    val: f64\n}}\n\nimpl {} {{\n    x = 1.0\n}}\n\nstrategy Test {{\n    on bar {{\n        x = 1.0\n    }}\n}}",
                sname, sname
            )),
            // Missing closing brace for impl block
            Just(format!(
                "struct {} {{\n    val: f64\n}}\n\nimpl {} {{\n    fn {}(self) -> f64 {{\n        return self.val\n    }}\n\nstrategy Test {{\n    on bar {{\n        x = 1.0\n    }}\n}}",
                sname, sname, mname
            )),
        ]
    })
}

/// Generate malformed trait definitions that should cause parse errors.
fn arb_malformed_trait() -> impl Strategy<Value = String> {
    let trait_name = arb_struct_name(); // same naming pattern (capitalized)
    let method_name = arb_method_name();

    (trait_name, method_name).prop_flat_map(|(tname, mname)| {
        prop_oneof![
            // Missing opening brace after trait name
            Just(format!(
                "trait {}\n    fn {}(self) -> f64\n}}\n\nstrategy Test {{\n    on bar {{\n        x = 1.0\n    }}\n}}",
                tname, mname
            )),
            // Missing closing brace
            Just(format!(
                "trait {} {{\n    fn {}(self) -> f64\n\nstrategy Test {{\n    on bar {{\n        x = 1.0\n    }}\n}}",
                tname, mname
            )),
            // Non-fn item inside trait body
            Just(format!(
                "trait {} {{\n    x = 1.0\n}}\n\nstrategy Test {{\n    on bar {{\n        x = 1.0\n    }}\n}}",
                tname
            )),
            // Number where trait name expected
            Just(format!(
                "trait 123 {{\n    fn {}(self) -> f64\n}}\n\nstrategy Test {{\n    on bar {{\n        x = 1.0\n    }}\n}}",
                mname
            )),
        ]
    })
}

/// Combined generator: pick one of the malformed constructs.
fn arb_malformed_type_system_source() -> impl Strategy<Value = String> {
    prop_oneof![
        arb_malformed_enum(),
        arb_malformed_match(),
        arb_malformed_impl(),
        arb_malformed_trait(),
    ]
}

// ============================================================================
// Property Tests for Parser Error Span Reporting
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Property 18: Malformed type system constructs produce parser errors with span info.
    #[test]
    fn prop_parser_error_span_reporting(source in arb_malformed_type_system_source()) {
        let tokens = lex_with_spans(&source);

        // Skip if lexing itself fails — we want to test parser error spans
        if let Ok(tokens) = tokens {
            let result = parse(tokens);

            // Should be an error (malformed source)
            prop_assert!(
                result.is_err(),
                "Expected parse error for malformed source:\n{}", source
            );

            let err = result.unwrap_err();

            // Should be CompileError::Parser variant with "at byte" span info
            match &err {
                flux_compiler::CompileError::Parser(msg) => {
                    prop_assert!(
                        msg.contains("at byte "),
                        "Parser error should contain 'at byte ' span info, got: {}",
                        msg
                    );

                    // Verify the byte offset is a valid number
                    let after_byte = msg.split("at byte ").nth(1).unwrap_or("");
                    let offset_str: String = after_byte
                        .chars()
                        .take_while(|c| c.is_ascii_digit())
                        .collect();
                    prop_assert!(
                        !offset_str.is_empty(),
                        "Error should contain numeric byte offset after 'at byte ', got: {}",
                        msg
                    );

                    // Parse the offset to confirm it's a valid usize
                    let offset: usize = offset_str.parse().unwrap();

                    // The byte offset should be within a reasonable range of the source length
                    // (it points to a position in the token stream, which may be at or near source end)
                    prop_assert!(
                        offset <= source.len() + 1,
                        "Byte offset {} exceeds source length {} for error: {}",
                        offset,
                        source.len(),
                        msg
                    );
                }
                other => {
                    prop_assert!(
                        false,
                        "Expected CompileError::Parser, got: {:?}",
                        other
                    );
                }
            }
        }
    }
}
