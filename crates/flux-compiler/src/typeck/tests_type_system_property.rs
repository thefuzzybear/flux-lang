//! Property-based tests for the type system's exhaustiveness checking.
//!
//! Feature: flux-type-system, Property 5: Exhaustiveness Checking
//!
//! Uses proptest to verify that the exhaustiveness checker correctly accepts
//! fully-covered matches (all variants or wildcard) and rejects partial matches
//! by reporting exactly the missing variants.

#[cfg(test)]
mod tests {
    use crate::lexer::Span;
    use crate::parser::ast::Pattern;
    use crate::typeck::enum_info::{EnumInfo, VariantInfo};
    use crate::typeck::exhaustiveness::{check_exhaustiveness, ExhaustivenessResult};
    use proptest::prelude::*;
    use std::collections::HashSet;

    // ========================================================================
    // Generators
    // ========================================================================

    fn make_span() -> Span {
        Span::new(0, 1)
    }

    /// Generate a unique list of variant names with length between 1 and 5.
    fn arb_variant_names() -> impl Strategy<Value = Vec<String>> {
        let pool = vec![
            "Alpha".to_string(),
            "Beta".to_string(),
            "Gamma".to_string(),
            "Delta".to_string(),
            "Epsilon".to_string(),
        ];
        (1usize..=5).prop_map(move |count| {
            pool.iter().take(count).cloned().collect()
        })
    }

    /// Generate an EnumInfo from a list of variant names (all unit variants).
    fn make_enum_info(variant_names: &[String]) -> EnumInfo {
        let variants: Vec<VariantInfo> = variant_names
            .iter()
            .map(|name| VariantInfo::unit(name.clone(), make_span()))
            .collect();
        EnumInfo::new("TestEnum".to_string(), variants, make_span())
    }

    /// Generate patterns from a subset of variant names, optionally including a wildcard.
    fn make_patterns(
        variant_names: &[String],
        included: &[bool],
        has_wildcard: bool,
    ) -> Vec<Pattern> {
        let mut patterns: Vec<Pattern> = variant_names
            .iter()
            .zip(included.iter())
            .filter(|(_, &inc)| inc)
            .map(|(name, _)| Pattern::Variant {
                enum_name: "TestEnum".to_string(),
                variant_name: name.clone(),
                bindings: vec![],
                span: make_span(),
            })
            .collect();

        if has_wildcard {
            patterns.push(Pattern::Wildcard { span: make_span() });
        }

        patterns
    }

    // ========================================================================
    // Property 5: Exhaustiveness Checking
    // ========================================================================

    // **Validates: Requirements 3.6, 13.3**
    //
    // For any enum with N variants (1-5) and any match expression:
    // - If all variants are covered OR a wildcard is present -> Exhaustive
    // - Otherwise -> NonExhaustive with exactly the uncovered variants
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_exhaustiveness_checking(
            variant_names in arb_variant_names(),
            subset_seed in prop::collection::vec(any::<bool>(), 5..=5),
            has_wildcard in any::<bool>(),
        ) {
            let n = variant_names.len();
            // Use first n booleans from the seed to select the subset
            let included: Vec<bool> = subset_seed.iter().take(n).cloned().collect();

            let enum_info = make_enum_info(&variant_names);
            let patterns = make_patterns(&variant_names, &included, has_wildcard);

            let result = check_exhaustiveness(&enum_info, &patterns);

            // Determine which variants are covered
            let covered: HashSet<&str> = variant_names
                .iter()
                .zip(included.iter())
                .filter(|(_, &inc)| inc)
                .map(|(name, _)| name.as_str())
                .collect();

            let all_covered = variant_names.iter().all(|name| covered.contains(name.as_str()));

            if all_covered || has_wildcard {
                // Should be exhaustive
                prop_assert_eq!(
                    result,
                    ExhaustivenessResult::Exhaustive,
                    "Expected Exhaustive when all variants covered ({}) or wildcard present ({})",
                    all_covered,
                    has_wildcard,
                );
            } else {
                // Should be non-exhaustive with exactly the missing variants
                let expected_missing: HashSet<String> = variant_names
                    .iter()
                    .filter(|name| !covered.contains(name.as_str()))
                    .cloned()
                    .collect();

                match &result {
                    ExhaustivenessResult::NonExhaustive { missing_variants } => {
                        let actual_missing: HashSet<String> =
                            missing_variants.iter().cloned().collect();
                        prop_assert!(
                            actual_missing == expected_missing,
                            "Missing variants mismatch. Expected {:?}, got {:?}",
                            expected_missing,
                            actual_missing,
                        );
                    }
                    ExhaustivenessResult::Exhaustive => {
                        prop_assert!(
                            false,
                            "Expected NonExhaustive with missing {:?}, but got Exhaustive",
                            expected_missing,
                        );
                    }
                }
            }
        }
    }
}


// ============================================================================
// Property 6: Enum Construction Type Validation
// ============================================================================

#[cfg(test)]
mod enum_construction_tests {
    use crate::typeck::typed_ast::TypedProgram;
    use proptest::prelude::*;

    // ========================================================================
    // Helpers
    // ========================================================================

    /// Lex → Parse → Typecheck a Flux source string. Returns the check result.
    fn check_source(source: &str) -> crate::error::Result<TypedProgram> {
        use crate::lexer::lex_with_spans;
        use crate::parser::parse;

        let tokens = lex_with_spans(source).unwrap_or_else(|e| {
            panic!("Lexing failed for source:\n{}\nError: {}", source, e);
        });
        let program = parse(tokens).unwrap_or_else(|e| {
            panic!("Parsing failed for source:\n{}\nError: {}", source, e);
        });
        crate::typeck::check(program)
    }

    // ========================================================================
    // Generators
    // ========================================================================

    /// A simple FluxType representation for generation purposes.
    #[derive(Debug, Clone)]
    enum FieldType {
        Int,
        Float,
        String,
        Bool,
    }

    impl FieldType {
        /// Returns the Flux source type annotation string.
        fn type_annotation(&self) -> &str {
            match self {
                FieldType::Int => "int",
                FieldType::Float => "f64",
                FieldType::String => "str",
                FieldType::Bool => "bool",
            }
        }

        /// Returns a valid literal expression for this type.
        fn valid_literal(&self) -> &str {
            match self {
                FieldType::Int => "42",
                FieldType::Float => "3.14",
                FieldType::String => "\"hello\"",
                FieldType::Bool => "true",
            }
        }

        /// Returns a literal expression that is NOT assignable to this type.
        fn wrong_literal(&self) -> &str {
            match self {
                FieldType::Float => "\"wrong\"",
                FieldType::Int => "\"wrong\"",
                FieldType::String => "42",
                FieldType::Bool => "\"wrong\"",
            }
        }
    }

    /// A generated enum variant with optional fields.
    #[derive(Debug, Clone)]
    struct GenVariant {
        name: String,
        fields: Vec<(String, FieldType)>,
    }

    /// A generated enum definition.
    #[derive(Debug, Clone)]
    struct GenEnum {
        name: String,
        variants: Vec<GenVariant>,
    }

    impl GenEnum {
        /// Produce the Flux source code for this enum definition.
        fn to_source(&self) -> String {
            let mut s = format!("enum {} {{\n", self.name);
            for variant in &self.variants {
                if variant.fields.is_empty() {
                    s.push_str(&format!("    {},\n", variant.name));
                } else {
                    let fields: Vec<String> = variant
                        .fields
                        .iter()
                        .map(|(name, ty)| format!("{}: {}", name, ty.type_annotation()))
                        .collect();
                    s.push_str(&format!("    {}({}),\n", variant.name, fields.join(", ")));
                }
            }
            s.push_str("}\n");
            s
        }
    }

    /// Generate a field type.
    fn arb_field_type() -> impl Strategy<Value = FieldType> {
        prop_oneof![
            Just(FieldType::Int),
            Just(FieldType::Float),
            Just(FieldType::String),
            Just(FieldType::Bool),
        ]
    }

    /// Generate a variant (unit or data with 1-3 fields).
    fn arb_variant(idx: usize) -> impl Strategy<Value = GenVariant> {
        let variant_name = format!("Var{}", idx);
        prop_oneof![
            // Unit variant
            Just(GenVariant {
                name: variant_name.clone(),
                fields: vec![],
            }),
            // Data variant with 1-3 fields
            prop::collection::vec(arb_field_type(), 1..=3).prop_map(move |types| {
                let fields: Vec<(String, FieldType)> = types
                    .into_iter()
                    .enumerate()
                    .map(|(i, ty)| (format!("f{}", i), ty))
                    .collect();
                GenVariant {
                    name: variant_name.clone(),
                    fields,
                }
            }),
        ]
    }

    /// Generate an enum with 1-4 variants.
    fn arb_enum() -> impl Strategy<Value = GenEnum> {
        (1usize..=4).prop_flat_map(|n_variants| {
            let variant_strategies: Vec<_> =
                (0..n_variants).map(|i| arb_variant(i)).collect();
            variant_strategies.prop_map(|variants| GenEnum {
                name: "TestEnum".to_string(),
                variants,
            })
        })
    }

    /// Build a complete Flux program source with an enum definition and a
    /// construction expression inside the strategy's on_bar handler.
    fn build_program(enum_def: &GenEnum, construction_expr: &str) -> String {
        format!(
            "{}\nstrategy Test {{\n    on bar {{\n        x = {}\n    }}\n}}\n",
            enum_def.to_source(),
            construction_expr,
        )
    }

    /// Build a valid enum construction expression for a given variant.
    fn valid_construction(enum_name: &str, variant: &GenVariant) -> String {
        if variant.fields.is_empty() {
            format!("{}.{}", enum_name, variant.name)
        } else {
            let args: Vec<&str> = variant
                .fields
                .iter()
                .map(|(_, ty)| ty.valid_literal())
                .collect();
            format!("{}.{}({})", enum_name, variant.name, args.join(", "))
        }
    }

    // ========================================================================
    // Property 6: Enum Construction Type Validation
    //
    // **Validates: Requirements 2.3, 2.4, 2.5**
    // ========================================================================

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// For any enum definition and a valid construction expression (correct enum
        /// name, variant name, correct arg count and types), the typechecker accepts
        /// the expression with the correct enum type.
        #[test]
        fn prop_valid_enum_construction_accepted(
            enum_def in arb_enum(),
            variant_idx in 0usize..4,
        ) {
            let variant_idx = variant_idx % enum_def.variants.len();
            let variant = &enum_def.variants[variant_idx];
            let construction = valid_construction(&enum_def.name, variant);
            let source = build_program(&enum_def, &construction);

            let result = check_source(&source);
            prop_assert!(
                result.is_ok(),
                "Valid construction {}.{} should be accepted.\nSource:\n{}\nError: {:?}",
                enum_def.name,
                variant.name,
                source,
                result.err()
            );
        }

        /// Validates that constructing a variant with too many arguments
        /// is rejected by the typechecker.
        #[test]
        fn prop_enum_construction_too_many_args_rejected(
            enum_def in arb_enum(),
            variant_idx in 0usize..4,
        ) {
            let variant_idx = variant_idx % enum_def.variants.len();
            let variant = &enum_def.variants[variant_idx];

            // Build args with one extra beyond what's expected
            let mut args: Vec<&str> = variant
                .fields
                .iter()
                .map(|(_, ty)| ty.valid_literal())
                .collect();
            args.push("99");  // extra Int arg

            let construction = format!(
                "{}.{}({})",
                enum_def.name,
                variant.name,
                args.join(", ")
            );
            let source = build_program(&enum_def, &construction);

            let result = check_source(&source);
            prop_assert!(
                result.is_err(),
                "Too many args for {}.{} should be rejected.\nSource:\n{}",
                enum_def.name,
                variant.name,
                source,
            );

            let err_msg = result.unwrap_err().to_string();
            prop_assert!(
                err_msg.contains("expects") && err_msg.contains("argument"),
                "Error should mention argument count mismatch, got: {}",
                err_msg
            );
        }

        /// Validates that constructing a data variant with too few arguments
        /// is rejected by the typechecker.
        #[test]
        fn prop_enum_construction_too_few_args_rejected(
            enum_def in arb_enum(),
            variant_idx in 0usize..4,
        ) {
            let variant_idx = variant_idx % enum_def.variants.len();
            let variant = &enum_def.variants[variant_idx];

            // Only test variants with 2+ fields (so we can drop one)
            prop_assume!(variant.fields.len() >= 2);

            // Provide one fewer arg than expected
            let args: Vec<&str> = variant
                .fields
                .iter()
                .take(variant.fields.len() - 1)
                .map(|(_, ty)| ty.valid_literal())
                .collect();

            let construction = format!(
                "{}.{}({})",
                enum_def.name,
                variant.name,
                args.join(", ")
            );
            let source = build_program(&enum_def, &construction);

            let result = check_source(&source);
            prop_assert!(
                result.is_err(),
                "Too few args for {}.{} should be rejected.\nSource:\n{}",
                enum_def.name,
                variant.name,
                source,
            );

            let err_msg = result.unwrap_err().to_string();
            prop_assert!(
                err_msg.contains("expects") && err_msg.contains("argument"),
                "Error should mention argument count mismatch, got: {}",
                err_msg
            );
        }

        /// Validates that constructing a data variant with wrong argument types
        /// is rejected by the typechecker.
        #[test]
        fn prop_enum_construction_wrong_type_rejected(
            enum_def in arb_enum(),
            variant_idx in 0usize..4,
        ) {
            let variant_idx = variant_idx % enum_def.variants.len();
            let variant = &enum_def.variants[variant_idx];

            // Only test data variants
            prop_assume!(!variant.fields.is_empty());

            // Replace the first field with a wrong-typed literal
            let mut args: Vec<String> = Vec::new();
            for (i, (_, ty)) in variant.fields.iter().enumerate() {
                if i == 0 {
                    args.push(ty.wrong_literal().to_string());
                } else {
                    args.push(ty.valid_literal().to_string());
                }
            }

            let construction = format!(
                "{}.{}({})",
                enum_def.name,
                variant.name,
                args.join(", ")
            );
            let source = build_program(&enum_def, &construction);

            let result = check_source(&source);
            prop_assert!(
                result.is_err(),
                "Wrong type for {}.{} should be rejected.\nSource:\n{}",
                enum_def.name,
                variant.name,
                source,
            );

            let err_msg = result.unwrap_err().to_string();
            prop_assert!(
                err_msg.contains("field") && err_msg.contains("expects"),
                "Error should mention field type mismatch, got: {}",
                err_msg
            );
        }

        /// Validates that constructing with a non-existent variant name
        /// is rejected by the typechecker.
        #[test]
        fn prop_enum_construction_unknown_variant_rejected(enum_def in arb_enum()) {
            let construction = format!("{}.NonExistentVariantXyz", enum_def.name);
            let source = build_program(&enum_def, &construction);

            let result = check_source(&source);
            prop_assert!(
                result.is_err(),
                "Unknown variant should be rejected.\nSource:\n{}",
                source,
            );

            let err_msg = result.unwrap_err().to_string();
            prop_assert!(
                err_msg.contains("has no variant"),
                "Error should mention unknown variant, got: {}",
                err_msg
            );
        }

        /// Validates that constructing with a non-existent enum name
        /// is rejected by the typechecker.
        #[test]
        fn prop_enum_construction_unknown_enum_rejected(enum_def in arb_enum()) {
            // Use a fake enum name that doesn't exist
            let construction = "FakeEnumXyz.Var0";
            let source = build_program(&enum_def, construction);

            let result = check_source(&source);
            prop_assert!(
                result.is_err(),
                "Unknown enum should be rejected.\nSource:\n{}",
                source,
            );

            let err_msg = result.unwrap_err().to_string();
            prop_assert!(
                err_msg.contains("unknown enum type") || err_msg.contains("Unknown") || err_msg.contains("undefined"),
                "Error should mention unknown enum type, got: {}",
                err_msg
            );
        }
    }
}
