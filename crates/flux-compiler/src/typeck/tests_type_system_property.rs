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
// Property 17: Name Conflict Detection
// ============================================================================

#[cfg(test)]
mod name_conflict_tests {
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

    /// Pool of valid variant/type names (capitalized, not reserved).
    const NAME_POOL: &[&str] = &[
        "Alpha", "Beta", "Gamma", "Delta", "Epsilon",
        "Zeta", "Eta", "Theta", "Iota", "Kappa",
    ];

    /// Generate a variant count (2-5) for enums where we need at least 2 variants.
    fn arb_variant_count() -> impl Strategy<Value = usize> {
        2usize..=5
    }

    /// Generate a method name from a pool of valid lowercase identifiers.
    const METHOD_POOL: &[&str] = &[
        "alpha", "beta", "gamma", "delta", "epsilon",
        "zeta", "eta", "theta", "iota", "kappa",
    ];

    /// Generate a method count (2-4) for impl blocks with duplicate methods.
    fn arb_method_count() -> impl Strategy<Value = usize> {
        2usize..=4
    }

    // ========================================================================
    // Source Builders
    // ========================================================================

    /// Build a Flux program with an enum that has a duplicate variant name.
    /// The variant at `dup_index` is repeated at the end.
    fn build_duplicate_variant_program(
        enum_name: &str,
        variant_count: usize,
        dup_index: usize,
    ) -> String {
        let mut variants = Vec::new();
        for i in 0..variant_count {
            variants.push(NAME_POOL[i].to_string());
        }
        // Add a duplicate of the variant at dup_index
        let dup_name = &variants[dup_index];

        let mut source = format!("enum {} {{\n", enum_name);
        for v in &variants {
            source.push_str(&format!("    {},\n", v));
        }
        // Duplicate variant
        source.push_str(&format!("    {},\n", dup_name));
        source.push_str("}\n\n");
        source.push_str("strategy Test {\n    on bar {\n        x = 1.0\n    }\n}\n");
        source
    }

    /// Build a Flux program with a struct and an enum that share the same name.
    fn build_enum_struct_conflict_program(conflicting_name: &str) -> String {
        let mut source = String::new();
        // Define a struct with the name
        source.push_str(&format!(
            "struct {} {{\n    value: f64\n}}\n\n",
            conflicting_name
        ));
        // Define an enum with the same name
        source.push_str(&format!(
            "enum {} {{\n    Var0,\n    Var1,\n}}\n\n",
            conflicting_name
        ));
        source.push_str("strategy Test {\n    on bar {\n        x = 1.0\n    }\n}\n");
        source
    }

    /// Build a Flux program with an impl block that has duplicate method names.
    fn build_duplicate_method_program(
        struct_name: &str,
        method_count: usize,
        dup_index: usize,
    ) -> String {
        let mut methods = Vec::new();
        for i in 0..method_count {
            methods.push(METHOD_POOL[i].to_string());
        }
        let dup_name = &methods[dup_index].clone();

        let mut source = String::new();
        // Define the struct
        source.push_str(&format!(
            "struct {} {{\n    value: f64\n}}\n\n",
            struct_name
        ));
        // Define the impl block with methods + a duplicate
        source.push_str(&format!("impl {} {{\n", struct_name));
        for method_name in &methods {
            source.push_str(&format!(
                "    fn {}(self) -> f64 {{\n        return self.value\n    }}\n",
                method_name
            ));
        }
        // Add the duplicate method
        source.push_str(&format!(
            "    fn {}(self) -> f64 {{\n        return self.value\n    }}\n",
            dup_name
        ));
        source.push_str("}\n\n");
        source.push_str("strategy Test {\n    on bar {\n        x = 1.0\n    }\n}\n");
        source
    }

    /// Build a Flux program with two trait definitions that share the same name.
    fn build_duplicate_trait_program(trait_name: &str) -> String {
        let mut source = String::new();
        // First trait definition
        source.push_str(&format!(
            "trait {} {{\n    fn alpha(self) -> f64\n}}\n\n",
            trait_name
        ));
        // Second trait definition with same name
        source.push_str(&format!(
            "trait {} {{\n    fn beta(self) -> f64\n}}\n\n",
            trait_name
        ));
        source.push_str("strategy Test {\n    on bar {\n        x = 1.0\n    }\n}\n");
        source
    }

    // ========================================================================
    // Property 17: Name Conflict Detection
    //
    // **Validates: Requirements 1.5, 1.6, 4.8, 5.4, 5.5**
    // ========================================================================

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// 17a: Duplicate variant names within an enum are rejected.
        ///
        /// For any enum with N (2-5) unique variants, adding a duplicate of an
        /// existing variant SHALL cause the typechecker to report a duplicate
        /// variant error.
        #[test]
        fn prop_duplicate_variant_name_rejected(
            variant_count in arb_variant_count(),
            dup_index_seed in 0usize..100,
            name_idx in 0usize..NAME_POOL.len(),
        ) {
            let dup_index = dup_index_seed % variant_count;
            let enum_name = NAME_POOL[name_idx];
            // Enum name must differ from variant names (use a name beyond variant_count)
            let enum_name = if (0..variant_count).any(|i| NAME_POOL[i] == enum_name) {
                // Use a name that won't collide with variants
                "TestEnum"
            } else {
                enum_name
            };

            let source = build_duplicate_variant_program(enum_name, variant_count, dup_index);
            let result = check_source(&source);

            prop_assert!(
                result.is_err(),
                "Duplicate variant should be rejected.\nSource:\n{}",
                source,
            );

            let err_msg = result.unwrap_err().to_string();
            prop_assert!(
                err_msg.contains("duplicate variant") || err_msg.contains("Duplicate variant"),
                "Error should mention duplicate variant, got: {}",
                err_msg
            );
        }

        /// 17b: Enum name conflicting with an existing struct name is rejected.
        ///
        /// For any type name used by both a struct and an enum, the typechecker
        /// SHALL report a type name conflict error.
        #[test]
        fn prop_enum_struct_name_conflict_rejected(
            name_idx in 0usize..NAME_POOL.len(),
        ) {
            let conflicting_name = NAME_POOL[name_idx];
            let source = build_enum_struct_conflict_program(conflicting_name);
            let result = check_source(&source);

            prop_assert!(
                result.is_err(),
                "Enum/struct name conflict should be rejected.\nSource:\n{}",
                source,
            );

            let err_msg = result.unwrap_err().to_string();
            prop_assert!(
                err_msg.contains("already defined") || err_msg.contains("name conflict") || err_msg.contains("Name conflict"),
                "Error should mention name conflict or already defined, got: {}",
                err_msg
            );
        }

        /// 17c: Duplicate method names in an impl block are rejected.
        ///
        /// For any struct with an impl block containing N (2-4) methods, adding
        /// a duplicate of an existing method SHALL cause the typechecker to report
        /// a duplicate method error.
        #[test]
        fn prop_duplicate_method_name_rejected(
            method_count in arb_method_count(),
            dup_index_seed in 0usize..100,
            name_idx in 0usize..NAME_POOL.len(),
        ) {
            let dup_index = dup_index_seed % method_count;
            let struct_name = NAME_POOL[name_idx];

            let source = build_duplicate_method_program(struct_name, method_count, dup_index);
            let result = check_source(&source);

            prop_assert!(
                result.is_err(),
                "Duplicate method should be rejected.\nSource:\n{}",
                source,
            );

            let err_msg = result.unwrap_err().to_string();
            prop_assert!(
                err_msg.contains("Method") && err_msg.contains("already defined"),
                "Error should mention method already defined, got: {}",
                err_msg
            );
        }

        /// 17d: Duplicate trait names are rejected.
        ///
        /// For any trait name defined twice, the typechecker SHALL report a
        /// name conflict or duplicate trait error.
        #[test]
        fn prop_duplicate_trait_name_rejected(
            name_idx in 0usize..NAME_POOL.len(),
        ) {
            let trait_name = NAME_POOL[name_idx];
            let source = build_duplicate_trait_program(trait_name);
            let result = check_source(&source);

            prop_assert!(
                result.is_err(),
                "Duplicate trait name should be rejected.\nSource:\n{}",
                source,
            );

            let err_msg = result.unwrap_err().to_string();
            prop_assert!(
                err_msg.contains("already defined") || err_msg.contains("Trait name"),
                "Error should mention trait already defined, got: {}",
                err_msg
            );
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


// ============================================================================
// Property 8: Trait Implementation Completeness
// ============================================================================

#[cfg(test)]
mod trait_impl_completeness_tests {
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

    /// A simple type representation for method parameters/return types.
    #[derive(Debug, Clone)]
    enum ParamType {
        Float,
        Bool,
        Int,
    }

    impl ParamType {
        /// Returns the Flux type annotation string.
        fn annotation(&self) -> &str {
            match self {
                ParamType::Float => "f64",
                ParamType::Bool => "bool",
                ParamType::Int => "int",
            }
        }

        /// Returns a valid literal value for this type (used in method bodies).
        fn default_literal(&self) -> &str {
            match self {
                ParamType::Float => "0.0",
                ParamType::Bool => "true",
                ParamType::Int => "0",
            }
        }
    }

    /// A generated trait method signature.
    #[derive(Debug, Clone)]
    struct GenTraitMethod {
        name: String,
        /// Extra parameters beyond self (param_name, param_type)
        params: Vec<(String, ParamType)>,
        return_type: ParamType,
    }

    /// Generate a parameter type.
    fn arb_param_type() -> impl Strategy<Value = ParamType> {
        prop_oneof![
            Just(ParamType::Float),
            Just(ParamType::Bool),
            Just(ParamType::Int),
        ]
    }

    /// Generate a trait method signature with 0-2 extra params (beyond self).
    fn arb_trait_method(idx: usize) -> impl Strategy<Value = GenTraitMethod> {
        let method_name = format!("method{}", idx);
        (
            prop::collection::vec(arb_param_type(), 0..=2),
            arb_param_type(),
        )
            .prop_map(move |(param_types, return_type)| {
                let params: Vec<(String, ParamType)> = param_types
                    .into_iter()
                    .enumerate()
                    .map(|(i, ty)| (format!("p{}", i), ty))
                    .collect();
                GenTraitMethod {
                    name: method_name.clone(),
                    params,
                    return_type,
                }
            })
    }

    /// Generate a trait with 1-4 required methods.
    fn arb_trait_methods() -> impl Strategy<Value = Vec<GenTraitMethod>> {
        (1usize..=4).prop_flat_map(|n| {
            let strats: Vec<_> = (0..n).map(|i| arb_trait_method(i)).collect();
            strats
        })
    }

    // ========================================================================
    // Source building helpers
    // ========================================================================

    /// Build a trait definition source string.
    fn build_trait_def(trait_name: &str, methods: &[GenTraitMethod]) -> String {
        let method_sigs: Vec<String> = methods
            .iter()
            .map(|m| {
                let mut params = vec!["self".to_string()];
                for (pname, ptype) in &m.params {
                    params.push(format!("{}: {}", pname, ptype.annotation()));
                }
                format!(
                    "    fn {}({}) -> {}",
                    m.name,
                    params.join(", "),
                    m.return_type.annotation()
                )
            })
            .collect();
        format!("trait {} {{\n{}\n}}\n", trait_name, method_sigs.join("\n"))
    }

    /// Build a struct definition.
    fn build_struct_def(struct_name: &str) -> String {
        format!("struct {} {{\n    value: f64\n}}\n", struct_name)
    }

    /// Build a trait impl block source string implementing a subset of methods.
    /// `included` is a boolean mask indicating which methods from `all_methods` to include.
    fn build_trait_impl(
        trait_name: &str,
        struct_name: &str,
        all_methods: &[GenTraitMethod],
        included: &[bool],
    ) -> String {
        let impl_methods: Vec<String> = all_methods
            .iter()
            .zip(included.iter())
            .filter(|(_, &inc)| inc)
            .map(|(m, _)| {
                let mut params = vec!["self".to_string()];
                for (pname, ptype) in &m.params {
                    params.push(format!("{}: {}", pname, ptype.annotation()));
                }
                let body = format!("return {}", m.return_type.default_literal());
                format!(
                    "    fn {}({}) -> {} {{\n        {}\n    }}",
                    m.name,
                    params.join(", "),
                    m.return_type.annotation(),
                    body,
                )
            })
            .collect();
        format!(
            "impl {} for {} {{\n{}\n}}\n",
            trait_name,
            struct_name,
            impl_methods.join("\n")
        )
    }

    /// Build a complete Flux program with a struct, trait, impl block, and strategy.
    fn build_program(
        trait_name: &str,
        struct_name: &str,
        methods: &[GenTraitMethod],
        included: &[bool],
    ) -> String {
        let struct_def = build_struct_def(struct_name);
        let trait_def = build_trait_def(trait_name, methods);
        let impl_block = build_trait_impl(trait_name, struct_name, methods, included);
        format!(
            "{}\n{}\n{}\nstrategy Test {{\n    on bar {{\n        x = 1.0\n    }}\n}}\n",
            struct_def, trait_def, impl_block
        )
    }

    // ========================================================================
    // Property 8: Trait Implementation Completeness
    //
    // **Validates: Requirements 6.2, 6.3, 6.4, 6.5**
    //
    // For any trait with N required methods and any impl block claiming to
    // implement that trait, if the impl provides fewer than N methods or any
    // method signature does not match the trait declaration (in parameter types
    // or return type), the typechecker SHALL report an error identifying the
    // missing or mismatched methods. If all methods match, the impl SHALL be
    // accepted.
    // ========================================================================

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// When all required trait methods are provided with matching signatures,
        /// the typechecker accepts the impl block.
        #[test]
        fn prop_complete_trait_impl_accepted(
            methods in arb_trait_methods(),
        ) {
            let n = methods.len();
            let all_included: Vec<bool> = vec![true; n];
            let source = build_program("MyTrait", "MyStruct", &methods, &all_included);

            let result = check_source(&source);
            prop_assert!(
                result.is_ok(),
                "Complete trait impl should be accepted.\nSource:\n{}\nError: {:?}",
                source,
                result.err()
            );
        }

        /// When one or more required trait methods are omitted, the typechecker
        /// rejects the impl and the error mentions the missing method name(s).
        #[test]
        fn prop_incomplete_trait_impl_rejected(
            methods in arb_trait_methods(),
            omit_mask in prop::collection::vec(any::<bool>(), 4..=4),
        ) {
            let n = methods.len();
            // Create an inclusion mask that omits at least one method
            let included: Vec<bool> = omit_mask.iter().take(n).cloned().collect();

            // Ensure at least one method is omitted (not all included)
            let all_included = included.iter().all(|&b| b);
            prop_assume!(!all_included);

            // Determine which methods are missing
            let missing_names: Vec<&str> = methods
                .iter()
                .zip(included.iter())
                .filter(|(_, &inc)| !inc)
                .map(|(m, _)| m.name.as_str())
                .collect();
            prop_assume!(!missing_names.is_empty());

            let source = build_program("MyTrait", "MyStruct", &methods, &included);

            let result = check_source(&source);
            prop_assert!(
                result.is_err(),
                "Incomplete trait impl (missing {:?}) should be rejected.\nSource:\n{}",
                missing_names,
                source,
            );

            // Verify the error mentions the missing method name(s)
            let err_msg = result.unwrap_err().to_string();
            for missing_name in &missing_names {
                prop_assert!(
                    err_msg.contains(missing_name),
                    "Error should mention missing method '{}', got: {}",
                    missing_name,
                    err_msg
                );
            }
        }

        /// When a method's return type does not match the trait declaration,
        /// the typechecker rejects the impl with a signature mismatch error.
        #[test]
        fn prop_wrong_return_type_rejected(
            methods in arb_trait_methods(),
            method_idx in 0usize..4,
        ) {
            let n = methods.len();
            let method_idx = method_idx % n;

            // Build a full impl but with one method having a wrong return type
            let struct_name = "MyStruct";
            let trait_name = "MyTrait";

            let struct_def = build_struct_def(struct_name);
            let trait_def = build_trait_def(trait_name, &methods);

            // Build impl methods, but corrupt one return type
            let impl_methods: Vec<String> = methods
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    let mut params = vec!["self".to_string()];
                    for (pname, ptype) in &m.params {
                        params.push(format!("{}: {}", pname, ptype.annotation()));
                    }

                    // For the target method, use a wrong return type
                    let (ret_annotation, body): (&str, String) = if i == method_idx {
                        // Pick a return type that differs from the declared one
                        match m.return_type {
                            ParamType::Float => ("bool", "return true".to_string()),
                            ParamType::Bool => ("f64", "return 0.0".to_string()),
                            ParamType::Int => ("bool", "return true".to_string()),
                        }
                    } else {
                        (m.return_type.annotation(), format!("return {}", m.return_type.default_literal()))
                    };

                    format!(
                        "    fn {}({}) -> {} {{\n        {}\n    }}",
                        m.name,
                        params.join(", "),
                        ret_annotation,
                        body,
                    )
                })
                .collect();

            let impl_block = format!(
                "impl {} for {} {{\n{}\n}}\n",
                trait_name,
                struct_name,
                impl_methods.join("\n")
            );

            let source = format!(
                "{}\n{}\n{}\nstrategy Test {{\n    on bar {{\n        x = 1.0\n    }}\n}}\n",
                struct_def, trait_def, impl_block
            );

            let result = check_source(&source);
            prop_assert!(
                result.is_err(),
                "Wrong return type for method '{}' should be rejected.\nSource:\n{}",
                methods[method_idx].name,
                source,
            );

            let err_msg = result.unwrap_err().to_string();
            prop_assert!(
                err_msg.contains("signature mismatch") || err_msg.contains("mismatch"),
                "Error should mention signature mismatch, got: {}",
                err_msg
            );
        }
    }
}
