//! Property-based test for library file acceptance.
//!
//! **Property 3: Library file acceptance**
//! **Validates: Requirements 3.1, 3.5**
//!
//! For any file containing zero or more `fn` definitions and zero or more
//! `import` statements (with no `strategy`, `data`, `connector`, or `state`
//! blocks), the parser SHALL produce a valid `Program` AST with an empty
//! strategy placeholder.

#[cfg(test)]
mod tests {
    use crate::lexer::lex_with_spans;
    use crate::parser::parse;
    use proptest::prelude::*;

    // ========================================================================
    // Helpers
    // ========================================================================

    fn is_keyword(s: &str) -> bool {
        matches!(
            s,
            "strategy" | "params" | "state" | "on" | "if" | "elif" | "else"
                | "for" | "while" | "return" | "fn" | "from" | "import" | "and" | "or"
                | "not" | "true" | "false" | "null" | "in" | "data" | "connector"
        )
    }

    // ========================================================================
    // Generators
    // ========================================================================

    /// Valid identifier: lowercase alpha start, not a keyword, doesn't start with "on_"
    fn arb_ident() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9_]{0,6}".prop_filter("not keyword or on_ prefix", |s| {
            !is_keyword(s) && !s.starts_with("on_")
        })
    }

    /// Module path segment for imports
    fn arb_module_segment() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9]{0,5}".prop_filter("not keyword", |s| !is_keyword(s))
    }

    /// Generate a simple expression suitable for function bodies
    fn arb_simple_expr() -> impl Strategy<Value = String> {
        prop_oneof![
            (1i64..1000).prop_map(|v| v.to_string()),
            (1u32..99, 1u32..9).prop_map(|(i, d)| format!("{}.{}", i, d)),
            Just("true".to_string()),
            Just("false".to_string()),
            arb_ident(),
        ]
    }

    /// Generate a simple statement for fn bodies (assignment or return)
    fn arb_simple_stmt() -> impl Strategy<Value = String> {
        prop_oneof![
            // Assignment: x = expr
            (arb_ident(), arb_simple_expr())
                .prop_map(|(name, expr)| format!("    {} = {}", name, expr)),
            // Return: return expr
            arb_simple_expr().prop_map(|expr| format!("    return {}", expr)),
            // Function call: foo(arg1, arg2)
            (arb_ident(), proptest::collection::vec(arb_simple_expr(), 0..3))
                .prop_map(|(name, args)| format!("    {}({})", name, args.join(", "))),
        ]
    }

    /// Generate a complete fn definition
    fn arb_fn_def_source() -> impl Strategy<Value = String> {
        (
            arb_ident(),
            proptest::collection::vec(arb_ident(), 0..4),
            proptest::collection::vec(arb_simple_stmt(), 1..4),
        )
            .prop_map(|(name, params, stmts)| {
                // Deduplicate params
                let mut seen = std::collections::HashSet::new();
                let unique_params: Vec<String> =
                    params.into_iter().filter(|p| seen.insert(p.clone())).collect();
                format!(
                    "fn {}({}) {{\n{}\n}}",
                    name,
                    unique_params.join(", "),
                    stmts.join("\n")
                )
            })
    }

    /// Generate an import statement (either dot-separated or :: separated)
    fn arb_import_source() -> impl Strategy<Value = String> {
        (
            proptest::collection::vec(arb_module_segment(), 1..4),
            prop_oneof![Just("."), Just("::")],
            proptest::collection::vec(arb_ident(), 1..3),
        )
            .prop_map(|(segments, sep, names)| {
                format!(
                    "from {} import {{{}}}",
                    segments.join(&sep),
                    names.join(", ")
                )
            })
    }

    /// Generate a complete library file source (imports + fn defs, no strategy)
    fn arb_library_file_source() -> impl Strategy<Value = String> {
        (
            proptest::collection::vec(arb_import_source(), 0..4),
            proptest::collection::vec(arb_fn_def_source(), 0..4),
        )
            .prop_map(|(imports, fns)| {
                let mut parts = Vec::new();
                for imp in &imports {
                    parts.push(imp.clone());
                }
                for f in &fns {
                    parts.push(f.clone());
                }
                parts.join("\n\n")
            })
    }

    // ========================================================================
    // Property Test
    // ========================================================================

    // Feature: flux-module-imports, Property 3: Library file acceptance
    // **Validates: Requirements 3.1, 3.5**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_library_file_acceptance(source in arb_library_file_source()) {
            // Lex the generated library file source
            let tokens = lex_with_spans(&source).expect(
                &format!("Library file should lex successfully.\nSource:\n{}", source)
            );

            // Parse should succeed for any valid library file
            let program = parse(tokens).expect(
                &format!("Library file should parse successfully.\nSource:\n{}", source)
            );

            // The strategy should be an empty placeholder (name == "")
            prop_assert_eq!(
                &program.strategy.name, "",
                "Library file should have empty strategy name.\nSource:\n{}\nGot strategy name: {:?}",
                source, program.strategy.name
            );

            // The strategy body should be empty
            prop_assert!(
                program.strategy.body.is_empty(),
                "Library file should have empty strategy body.\nSource:\n{}\nGot body: {:?}",
                source, program.strategy.body
            );

            // No data or connector blocks should be present
            prop_assert!(
                program.data_block.is_none(),
                "Library file should have no data block.\nSource:\n{}",
                source
            );
            prop_assert!(
                program.connector_block.is_none(),
                "Library file should have no connector block.\nSource:\n{}",
                source
            );
        }
    }
}
