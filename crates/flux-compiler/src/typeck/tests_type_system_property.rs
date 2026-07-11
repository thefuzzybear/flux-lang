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

// ============================================================================
// Property 10: Trait Bound Satisfaction
// ============================================================================

#[cfg(test)]
mod trait_bound_satisfaction_tests {
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

    /// Pool of trait names to choose from.
    const TRAIT_NAMES: &[&str] = &[
        "DataFeed", "Indicator", "Fillable", "Sortable", "Renderable",
    ];

    /// Pool of struct names.
    const STRUCT_NAMES: &[&str] = &[
        "LiveFeed", "HistFeed", "Widget", "Sensor", "Record",
    ];

    /// Pool of method names for traits.
    const METHOD_NAMES: &[&str] = &[
        "compute", "process", "evaluate", "transform",
    ];

    /// Pool of generic function names.
    const FN_NAMES: &[&str] = &[
        "run_bounded", "apply_feed", "handle_item", "execute_task",
    ];

    /// Return types for trait methods.
    #[derive(Debug, Clone)]
    enum ReturnType {
        Float,
        Bool,
        Int,
    }

    impl ReturnType {
        fn annotation(&self) -> &str {
            match self {
                ReturnType::Float => "f64",
                ReturnType::Bool => "bool",
                ReturnType::Int => "int",
            }
        }

        fn default_literal(&self) -> &str {
            match self {
                ReturnType::Float => "0.0",
                ReturnType::Bool => "true",
                ReturnType::Int => "0",
            }
        }
    }

    fn arb_return_type() -> impl Strategy<Value = ReturnType> {
        prop_oneof![
            Just(ReturnType::Float),
            Just(ReturnType::Bool),
            Just(ReturnType::Int),
        ]
    }

    /// Number of methods in a trait (1-3).
    fn arb_method_count() -> impl Strategy<Value = usize> {
        1usize..=3
    }

    /// Generate a list of method definitions for a trait.
    #[derive(Debug, Clone)]
    struct TraitMethod {
        name: String,
        return_type: ReturnType,
    }

    fn arb_trait_methods() -> impl Strategy<Value = Vec<TraitMethod>> {
        arb_method_count().prop_flat_map(|count| {
            proptest::collection::vec(arb_return_type(), count..=count).prop_map(
                move |return_types| {
                    return_types
                        .into_iter()
                        .enumerate()
                        .map(|(i, rt)| TraitMethod {
                            name: METHOD_NAMES[i % METHOD_NAMES.len()].to_string(),
                            return_type: rt,
                        })
                        .collect()
                },
            )
        })
    }

    // ========================================================================
    // Program builders
    // ========================================================================

    /// Builds a trait definition source.
    fn build_trait_def(trait_name: &str, methods: &[TraitMethod]) -> String {
        let method_sigs: Vec<String> = methods
            .iter()
            .map(|m| format!("    fn {}(self) -> {}", m.name, m.return_type.annotation()))
            .collect();
        format!("trait {} {{\n{}\n}}\n", trait_name, method_sigs.join("\n"))
    }

    /// Builds a struct definition.
    fn build_struct_def(struct_name: &str) -> String {
        format!("struct {} {{\n    value: f64\n}}\n", struct_name)
    }

    /// Builds a trait impl block for a struct.
    fn build_trait_impl(
        trait_name: &str,
        struct_name: &str,
        methods: &[TraitMethod],
    ) -> String {
        let method_impls: Vec<String> = methods
            .iter()
            .map(|m| {
                format!(
                    "    fn {}(self) -> {} {{\n        return {}\n    }}",
                    m.name,
                    m.return_type.annotation(),
                    m.return_type.default_literal()
                )
            })
            .collect();
        format!(
            "impl {} for {} {{\n{}\n}}\n",
            trait_name,
            struct_name,
            method_impls.join("\n")
        )
    }

    /// Builds a generic function with a trait bound.
    fn build_bounded_generic_fn(
        fn_name: &str,
        type_param: &str,
        trait_name: &str,
    ) -> String {
        format!(
            "fn {}[{}: {}](item: {}) -> f64 {{\n    return 1.0\n}}\n",
            fn_name, type_param, trait_name, type_param
        )
    }

    /// Builds the strategy that calls the bounded generic function.
    fn build_strategy_calling(fn_name: &str, struct_name: &str) -> String {
        format!(
            "strategy Test {{\n    on bar {{\n        s = {} {{ value = 1.0 }}\n        result = {}(s)\n    }}\n}}\n",
            struct_name, fn_name
        )
    }

    // ========================================================================
    // Property 10: Trait Bound Satisfaction
    //
    // **Validates: Requirements 9.2, 9.3**
    //
    // For any generic function with a trait-bounded type parameter `T: SomeTrait`,
    // if the concrete type used at the call site implements `SomeTrait`, the
    // typechecker SHALL accept the call. If the concrete type does NOT implement
    // the trait, the typechecker SHALL report a trait bound violation error
    // naming the type and required trait.
    // ========================================================================

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// When a generic function with bound `T: Trait` is called with a type
        /// that implements the trait, the typechecker accepts.
        #[test]
        fn prop_trait_bound_satisfied_accepted(
            trait_idx in 0usize..5,
            struct_idx in 0usize..5,
            fn_idx in 0usize..4,
            methods in arb_trait_methods(),
        ) {
            let trait_name = TRAIT_NAMES[trait_idx];
            let struct_name = STRUCT_NAMES[struct_idx];
            let fn_name = FN_NAMES[fn_idx];

            let source = format!(
                "{}\n{}\n{}\n{}\n{}",
                build_trait_def(trait_name, &methods),
                build_struct_def(struct_name),
                build_trait_impl(trait_name, struct_name, &methods),
                build_bounded_generic_fn(fn_name, "T", trait_name),
                build_strategy_calling(fn_name, struct_name),
            );

            let result = check_source(&source);
            prop_assert!(
                result.is_ok(),
                "Calling bounded generic fn with implementing type should be accepted.\nSource:\n{}\nError: {:?}",
                source,
                result.err()
            );
        }

        /// When a generic function with bound `T: Trait` is called with a type
        /// that does NOT implement the trait, the typechecker rejects with an error
        /// that names the type and the required trait.
        #[test]
        fn prop_trait_bound_violated_rejected(
            trait_idx in 0usize..5,
            struct_idx in 0usize..5,
            fn_idx in 0usize..4,
            methods in arb_trait_methods(),
        ) {
            let trait_name = TRAIT_NAMES[trait_idx];
            let struct_name = STRUCT_NAMES[struct_idx];
            let fn_name = FN_NAMES[fn_idx];

            // Build the program WITHOUT the trait impl — struct does not implement the trait
            let source = format!(
                "{}\n{}\n{}\n{}",
                build_trait_def(trait_name, &methods),
                build_struct_def(struct_name),
                build_bounded_generic_fn(fn_name, "T", trait_name),
                build_strategy_calling(fn_name, struct_name),
            );

            let result = check_source(&source);
            prop_assert!(
                result.is_err(),
                "Calling bounded generic fn with non-implementing type should be rejected.\nSource:\n{}",
                source,
            );

            // Verify the error mentions the struct name and trait name
            let err_msg = result.unwrap_err().to_string();
            prop_assert!(
                err_msg.contains(struct_name),
                "Error should mention the type '{}', got: {}",
                struct_name,
                err_msg
            );
            prop_assert!(
                err_msg.contains(trait_name),
                "Error should mention the trait '{}', got: {}",
                trait_name,
                err_msg
            );
            prop_assert!(
                err_msg.contains("does not implement trait"),
                "Error should contain 'does not implement trait', got: {}",
                err_msg
            );
        }
    }
}


// ============================================================================
// Property 9: Generic Type Argument Substitution
// ============================================================================

#[cfg(test)]
mod generic_type_arg_substitution_tests {
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

    /// Pool of type parameter names.
    const TYPE_PARAM_NAMES: &[&str] = &["T", "U", "V", "W"];

    /// Concrete types that can be substituted for type parameters.
    #[derive(Debug, Clone)]
    enum ConcreteType {
        Float,
        Int,
        Bool,
        Str,
    }

    impl ConcreteType {
        /// Returns the Flux source annotation for this type.
        fn annotation(&self) -> &str {
            match self {
                ConcreteType::Float => "f64",
                ConcreteType::Int => "int",
                ConcreteType::Bool => "bool",
                ConcreteType::Str => "str",
            }
        }

        /// Returns a valid literal expression for this type.
        fn literal(&self) -> &str {
            match self {
                ConcreteType::Float => "3.14",
                ConcreteType::Int => "42",
                ConcreteType::Bool => "true",
                ConcreteType::Str => "\"hello\"",
            }
        }
    }

    /// Generate a concrete type.
    fn arb_concrete_type() -> impl Strategy<Value = ConcreteType> {
        prop_oneof![
            Just(ConcreteType::Float),
            Just(ConcreteType::Int),
            Just(ConcreteType::Bool),
            Just(ConcreteType::Str),
        ]
    }

    /// Generate a number of type parameters (1-4).
    fn arb_type_param_count() -> impl Strategy<Value = usize> {
        1usize..=4
    }

    /// Generate a list of concrete types for type arguments.
    fn arb_concrete_types(count: usize) -> impl Strategy<Value = Vec<ConcreteType>> {
        prop::collection::vec(arb_concrete_type(), count..=count)
    }

    // ========================================================================
    // Source Builders
    // ========================================================================

    /// Build a generic struct definition with K type parameters.
    /// Each type param gets a field that uses it.
    fn build_generic_struct(struct_name: &str, k: usize) -> String {
        let type_params: Vec<&str> = TYPE_PARAM_NAMES.iter().take(k).copied().collect();
        let type_param_list = type_params.join(", ");

        let mut fields = Vec::new();
        for (i, tp) in type_params.iter().enumerate() {
            fields.push(format!("    field{}: {}", i, tp));
        }

        format!(
            "struct {}[{}] {{\n{}\n}}\n",
            struct_name,
            type_param_list,
            fields.join(",\n")
        )
    }

    /// Build a struct that uses the generic struct as a field with concrete type args.
    fn build_user_struct_with_generic_field(
        generic_struct_name: &str,
        concrete_types: &[ConcreteType],
    ) -> String {
        let type_args: Vec<&str> = concrete_types.iter().map(|t| t.annotation()).collect();
        let type_arg_list = type_args.join(", ");
        format!(
            "struct Holder {{\n    inner: {}[{}]\n}}\n",
            generic_struct_name, type_arg_list
        )
    }

    /// Build a struct that uses the generic struct with WRONG number of type args.
    fn build_user_struct_with_wrong_arg_count(
        generic_struct_name: &str,
        _expected_k: usize,
        actual_count: usize,
    ) -> String {
        // Generate `actual_count` concrete type annotations
        let type_args: Vec<&str> = (0..actual_count)
            .map(|i| match i % 4 {
                0 => "f64",
                1 => "int",
                2 => "bool",
                _ => "str",
            })
            .collect();
        let type_arg_list = type_args.join(", ");
        format!(
            "struct Holder {{\n    inner: {}[{}]\n}}\n",
            generic_struct_name, type_arg_list
        )
    }

    /// Build a generic function with K type parameters.
    /// The function takes one parameter of each type param and returns f64.
    fn build_generic_function(fn_name: &str, k: usize) -> String {
        let type_params: Vec<&str> = TYPE_PARAM_NAMES.iter().take(k).copied().collect();
        let type_param_list = type_params.join(", ");

        let params: Vec<String> = type_params
            .iter()
            .enumerate()
            .map(|(i, tp)| format!("arg{}: {}", i, tp))
            .collect();
        let param_list = params.join(", ");

        format!(
            "fn {}[{}]({}) -> f64 {{\n    return 1.0\n}}\n",
            fn_name, type_param_list, param_list
        )
    }

    /// Build a function call with the given concrete argument literals.
    fn build_generic_fn_call(fn_name: &str, concrete_types: &[ConcreteType]) -> String {
        let args: Vec<&str> = concrete_types.iter().map(|t| t.literal()).collect();
        format!("{}({})", fn_name, args.join(", "))
    }

    /// Build a minimal strategy block.
    fn build_strategy(body_expr: &str) -> String {
        format!(
            "strategy Test {{\n    on bar {{\n        result = {}\n    }}\n}}\n",
            body_expr
        )
    }

    // ========================================================================
    // Property 9: Generic Type Argument Substitution
    //
    // **Validates: Requirements 7.2, 7.3, 7.4, 8.2, 8.3**
    //
    // For any generic struct or function with K type parameters:
    // - When instantiated with exactly K concrete type arguments, the typechecker
    //   SHALL substitute all occurrences and accept.
    // - If the count of arguments does not equal K, an error SHALL be reported.
    // ========================================================================

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// 9a: A generic struct with K type params, when used with exactly K
        /// concrete type arguments, is accepted by the typechecker.
        #[test]
        fn prop_generic_struct_correct_arg_count_accepted(
            concrete_types in arb_type_param_count().prop_flat_map(|k| arb_concrete_types(k)),
        ) {
            let k = concrete_types.len();

            let generic_struct = build_generic_struct("Container", k);
            let user_struct = build_user_struct_with_generic_field("Container", &concrete_types);
            let strategy = build_strategy("1.0");

            let source = format!("{}\n{}\n{}", generic_struct, user_struct, strategy);

            let result = check_source(&source);
            prop_assert!(
                result.is_ok(),
                "Generic struct with {} type params instantiated with {} type args should be accepted.\nSource:\n{}\nError: {:?}",
                k,
                concrete_types.len(),
                source,
                result.err()
            );
        }

        /// 9b: A generic struct with K type params, when used with a different
        /// number of type arguments (fewer or more), is rejected by the typechecker.
        #[test]
        fn prop_generic_struct_wrong_arg_count_rejected(
            k in arb_type_param_count(),
            delta in 1usize..=3,
            add_or_sub in any::<bool>(),
        ) {
            // Compute a wrong arg count: either k + delta or k - delta (clamped to >= 1)
            let actual_count = if add_or_sub {
                k + delta
            } else {
                if k > delta { k - delta } else { 0 }
            };

            // actual_count must differ from k and must be >= 1 for valid syntax
            prop_assume!(actual_count != k && actual_count >= 1);

            let generic_struct = build_generic_struct("Container", k);
            let user_struct = build_user_struct_with_wrong_arg_count("Container", k, actual_count);
            let strategy = build_strategy("1.0");

            let source = format!("{}\n{}\n{}", generic_struct, user_struct, strategy);

            let result = check_source(&source);
            prop_assert!(
                result.is_err(),
                "Generic struct with {} type params instantiated with {} type args should be rejected.\nSource:\n{}",
                k,
                actual_count,
                source,
            );

            let err_msg = result.unwrap_err().to_string();
            prop_assert!(
                err_msg.contains("expected") && err_msg.contains("type arguments"),
                "Error should mention expected type arguments count, got: {}",
                err_msg
            );
        }

        /// 9c: A generic function with K type params, when called with exactly K
        /// arguments of concrete types, is accepted by the typechecker (type inference succeeds).
        #[test]
        fn prop_generic_fn_correct_arg_count_accepted(
            concrete_types in arb_type_param_count().prop_flat_map(|k| arb_concrete_types(k)),
        ) {
            let k = concrete_types.len();

            let generic_fn = build_generic_function("transform", k);
            let call_expr = build_generic_fn_call("transform", &concrete_types);
            let strategy = build_strategy(&call_expr);

            let source = format!("{}\n{}", generic_fn, strategy);

            let result = check_source(&source);
            prop_assert!(
                result.is_ok(),
                "Generic function with {} type params called with {} args should be accepted.\nSource:\n{}\nError: {:?}",
                k,
                concrete_types.len(),
                source,
                result.err()
            );
        }

        /// 9d: A generic function with K type params, when called with a
        /// different number of arguments, is rejected by the typechecker.
        #[test]
        fn prop_generic_fn_wrong_arg_count_rejected(
            k in arb_type_param_count(),
            delta in 1usize..=3,
            add_or_sub in any::<bool>(),
        ) {
            // Compute a wrong arg count
            let actual_count = if add_or_sub {
                k + delta
            } else {
                if k > delta { k - delta } else { 0 }
            };

            // actual_count must differ from k and must be >= 1 for valid call syntax
            prop_assume!(actual_count != k && actual_count >= 1);

            let generic_fn = build_generic_function("transform", k);

            // Build call with wrong number of arguments
            let arg_literals: Vec<&str> = (0..actual_count)
                .map(|i| match i % 4 {
                    0 => "3.14",
                    1 => "42",
                    2 => "true",
                    _ => "\"hello\"",
                })
                .collect();
            let call_expr = format!("transform({})", arg_literals.join(", "));
            let strategy = build_strategy(&call_expr);

            let source = format!("{}\n{}", generic_fn, strategy);

            let result = check_source(&source);
            prop_assert!(
                result.is_err(),
                "Generic function with {} type params called with {} args should be rejected.\nSource:\n{}",
                k,
                actual_count,
                source,
            );

            let err_msg = result.unwrap_err().to_string();
            prop_assert!(
                err_msg.contains("expects") && err_msg.contains("argument"),
                "Error should mention argument count mismatch, got: {}",
                err_msg
            );
        }
    }
}

// ============================================================================
// Property 7: Match Pattern Binding Types
// ============================================================================

#[cfg(test)]
mod match_pattern_binding_tests {
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

    /// A simple type for enum fields.
    #[derive(Debug, Clone)]
    enum FieldType {
        Int,
        Float,
        String,
        Bool,
    }

    impl FieldType {
        /// Returns the Flux source type annotation string.
        fn annotation(&self) -> &str {
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

    /// A generated enum variant with fields.
    #[derive(Debug, Clone)]
    struct GenVariant {
        name: String,
        fields: Vec<(String, FieldType)>,
    }

    /// Generate a data variant with 1-4 fields.
    fn arb_data_variant(idx: usize) -> impl Strategy<Value = GenVariant> {
        prop::collection::vec(arb_field_type(), 1..=4).prop_map(move |types| {
            let fields: Vec<(String, FieldType)> = types
                .into_iter()
                .enumerate()
                .map(|(i, ty)| (format!("f{}", i), ty))
                .collect();
            GenVariant {
                name: format!("Var{}", idx),
                fields,
            }
        })
    }

    /// Generate an enum with 1-3 data variants (all have fields for binding tests).
    fn arb_enum_with_data_variants() -> impl Strategy<Value = Vec<GenVariant>> {
        (1usize..=3).prop_flat_map(|n| {
            let strats: Vec<_> = (0..n).map(|i| arb_data_variant(i)).collect();
            strats
        })
    }

    // ========================================================================
    // Source Builders
    // ========================================================================

    /// Build a Flux enum definition source from generated variants.
    fn build_enum_def(enum_name: &str, variants: &[GenVariant]) -> String {
        let mut s = format!("enum {} {{\n", enum_name);
        for variant in variants {
            let fields: Vec<String> = variant
                .fields
                .iter()
                .map(|(name, ty)| format!("{}: {}", name, ty.annotation()))
                .collect();
            s.push_str(&format!("    {}({}),\n", variant.name, fields.join(", ")));
        }
        s.push_str("}\n");
        s
    }

    /// Build a match expression where one arm has a specific number of bindings.
    /// The target_variant_idx arm uses the specified bindings; all other arms use
    /// correct bindings. A wildcard is added for exhaustiveness if there are other
    /// variants not explicitly handled.
    fn build_match_with_bindings(
        enum_name: &str,
        variants: &[GenVariant],
        target_variant_idx: usize,
        binding_names: &[String],
    ) -> String {
        let target_variant = &variants[target_variant_idx];
        let bindings_str = binding_names.join(", ");

        let mut match_source = format!(
            "match val {{\n        {}.{}({}) => {{\n            x = 1.0\n        }}\n",
            enum_name, target_variant.name, bindings_str
        );

        // Add a wildcard arm for remaining variants (exhaustiveness)
        if variants.len() > 1 {
            match_source.push_str("        _ => {\n            x = 2.0\n        }\n");
        }

        match_source.push_str("    }");
        match_source
    }

    /// Build a match expression where the arm body uses the bound variable in a
    /// type-specific operation. This verifies the bound variable has the correct type.
    fn build_match_with_typed_usage(
        enum_name: &str,
        variants: &[GenVariant],
        target_variant_idx: usize,
    ) -> String {
        let target_variant = &variants[target_variant_idx];
        let field_count = target_variant.fields.len();

        let binding_names: Vec<String> = (0..field_count)
            .map(|i| format!("b{}", i))
            .collect();
        let bindings_str = binding_names.join(", ");

        // Use the first binding in a type-appropriate operation
        let first_field_type = &target_variant.fields[0].1;
        let usage_expr = match first_field_type {
            FieldType::Int => "result = b0 + 1",
            FieldType::Float => "result = b0 + 1.0",
            FieldType::String => "result = b0",
            FieldType::Bool => "result = b0",
        };

        let mut match_source = format!(
            "match val {{\n        {}.{}({}) => {{\n            {}\n        }}\n",
            enum_name, target_variant.name, bindings_str, usage_expr
        );

        // Add wildcard for exhaustiveness
        if variants.len() > 1 {
            match_source.push_str("        _ => {\n            result = 0.0\n        }\n");
        }

        match_source.push_str("    }");
        match_source
    }

    /// Build a complete Flux program with an enum definition, a value construction,
    /// and a match expression in the strategy on_bar block.
    fn build_program(enum_def: &str, enum_name: &str, variant: &GenVariant, match_expr: &str) -> String {
        // Construct a value of the target variant
        let args: Vec<&str> = variant
            .fields
            .iter()
            .map(|(_, ty)| ty.valid_literal())
            .collect();
        let construction = if args.is_empty() {
            format!("{}.{}", enum_name, variant.name)
        } else {
            format!("{}.{}({})", enum_name, variant.name, args.join(", "))
        };

        format!(
            "{}\nstrategy Test {{\n    on bar {{\n        val = {}\n    {}\n    }}\n}}\n",
            enum_def, construction, match_expr
        )
    }

    // ========================================================================
    // Property 7: Match Pattern Binding Types
    //
    // **Validates: Requirements 3.5, 3.7, 3.8**
    //
    // For any match arm with a variant pattern that binds variables:
    // - The typechecker SHALL introduce each bound variable into the arm body
    //   scope with the type matching the corresponding field in the enum variant
    //   definition.
    // - If the binding count differs from the field count, an error SHALL be
    //   reported.
    // ========================================================================

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// 7a: When a match arm binds the correct number of variables
        /// (matching the variant's field count), the typechecker accepts
        /// the match expression.
        #[test]
        fn prop_correct_binding_count_accepted(
            variants in arb_enum_with_data_variants(),
            target_idx_seed in 0usize..100,
        ) {
            let enum_name = "TestEnum";
            let target_idx = target_idx_seed % variants.len();
            let target_variant = &variants[target_idx];

            // Generate correct number of binding names
            let binding_names: Vec<String> = (0..target_variant.fields.len())
                .map(|i| format!("b{}", i))
                .collect();

            let enum_def = build_enum_def(enum_name, &variants);
            let match_expr = build_match_with_bindings(
                enum_name,
                &variants,
                target_idx,
                &binding_names,
            );
            let source = build_program(&enum_def, enum_name, target_variant, &match_expr);

            let result = check_source(&source);
            prop_assert!(
                result.is_ok(),
                "Correct binding count ({}) should be accepted.\nSource:\n{}\nError: {:?}",
                binding_names.len(),
                source,
                result.err()
            );
        }

        /// 7b: When a match arm binds more variables than the variant has fields,
        /// the typechecker rejects with a field count mismatch error.
        #[test]
        fn prop_too_many_bindings_rejected(
            variants in arb_enum_with_data_variants(),
            target_idx_seed in 0usize..100,
            extra in 1usize..=3,
        ) {
            let enum_name = "TestEnum";
            let target_idx = target_idx_seed % variants.len();
            let target_variant = &variants[target_idx];

            // Generate MORE binding names than fields
            let binding_count = target_variant.fields.len() + extra;
            let binding_names: Vec<String> = (0..binding_count)
                .map(|i| format!("b{}", i))
                .collect();

            let enum_def = build_enum_def(enum_name, &variants);
            let match_expr = build_match_with_bindings(
                enum_name,
                &variants,
                target_idx,
                &binding_names,
            );
            let source = build_program(&enum_def, enum_name, target_variant, &match_expr);

            let result = check_source(&source);
            prop_assert!(
                result.is_err(),
                "Too many bindings ({} for {} fields) should be rejected.\nSource:\n{}",
                binding_count,
                target_variant.fields.len(),
                source,
            );

            let err_msg = result.unwrap_err().to_string();
            prop_assert!(
                err_msg.contains("field") && err_msg.contains("binds"),
                "Error should mention field count vs binding mismatch, got: {}",
                err_msg
            );
        }

        /// 7c: When a match arm binds fewer variables than the variant has fields,
        /// the typechecker rejects with a field count mismatch error.
        #[test]
        fn prop_too_few_bindings_rejected(
            variants in arb_enum_with_data_variants(),
            target_idx_seed in 0usize..100,
        ) {
            let enum_name = "TestEnum";
            let target_idx = target_idx_seed % variants.len();
            let target_variant = &variants[target_idx];

            // Only test variants with 2+ fields so we can drop one
            prop_assume!(target_variant.fields.len() >= 2);

            // Generate FEWER binding names than fields (always at least 1 less)
            let binding_count = target_variant.fields.len() - 1;
            let binding_names: Vec<String> = (0..binding_count)
                .map(|i| format!("b{}", i))
                .collect();

            let enum_def = build_enum_def(enum_name, &variants);
            let match_expr = build_match_with_bindings(
                enum_name,
                &variants,
                target_idx,
                &binding_names,
            );
            let source = build_program(&enum_def, enum_name, target_variant, &match_expr);

            let result = check_source(&source);
            prop_assert!(
                result.is_err(),
                "Too few bindings ({} for {} fields) should be rejected.\nSource:\n{}",
                binding_count,
                target_variant.fields.len(),
                source,
            );

            let err_msg = result.unwrap_err().to_string();
            prop_assert!(
                err_msg.contains("field") && err_msg.contains("binds"),
                "Error should mention field count vs binding mismatch, got: {}",
                err_msg
            );
        }

        /// 7d: Bound variables are introduced with the correct types in the arm body.
        /// When the arm body uses a bound variable in a type-appropriate expression,
        /// the typechecker accepts. This validates that the binding types match
        /// the enum variant's field types.
        #[test]
        fn prop_bound_variables_have_correct_types(
            variants in arb_enum_with_data_variants(),
            target_idx_seed in 0usize..100,
        ) {
            let enum_name = "TestEnum";
            let target_idx = target_idx_seed % variants.len();
            let target_variant = &variants[target_idx];

            let enum_def = build_enum_def(enum_name, &variants);
            let match_expr = build_match_with_typed_usage(
                enum_name,
                &variants,
                target_idx,
            );
            let source = build_program(&enum_def, enum_name, target_variant, &match_expr);

            let result = check_source(&source);
            prop_assert!(
                result.is_ok(),
                "Bound variable usage with correct types should be accepted.\nSource:\n{}\nError: {:?}",
                source,
                result.err()
            );
        }
    }
}
