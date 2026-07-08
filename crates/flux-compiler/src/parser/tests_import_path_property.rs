//! Property-based tests for import path separator preservation (round-trip).
//!
//! Feature: flux-module-imports, Property 2: Import path separator preservation (round-trip)
//!
//! **Validates: Requirements 2.1, 2.2, 2.3, 9.2**
//!
//! For any import statement with `::` path separators, parsing produces an Import AST
//! with `module_path` containing `::` between segments. For any import statement with
//! `.` path separators, parsing produces an Import AST with `.` between segments.

#[cfg(test)]
mod tests {
    use crate::lexer::lex_with_spans;
    use crate::parser::{parse, pretty_print_program};
    use proptest::prelude::*;

    // ========================================================================
    // Generators
    // ========================================================================

    /// Generate a valid Flux identifier that is not a keyword and does not start with "on_".
    fn arb_ident() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9_]{0,6}".prop_filter("not keyword or on_ prefix", |s| {
            let keywords = [
                "strategy", "params", "state", "on", "if", "elif", "else",
                "for", "while", "return", "from", "import", "and", "or",
                "not", "true", "false", "null", "data", "connector", "fn", "in",
            ];
            !keywords.contains(&s.as_str()) && !s.starts_with("on_")
        })
    }

    /// Generate a valid module path segment (short identifier, not a keyword).
    fn arb_module_segment() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9]{0,5}".prop_filter("not keyword", |s| {
            let keywords = [
                "strategy", "params", "state", "on", "if", "elif", "else",
                "for", "while", "return", "from", "import", "and", "or",
                "not", "true", "false", "null", "data", "connector", "fn", "in",
            ];
            !keywords.contains(&s.as_str())
        })
    }

    /// Generate a list of unique import names (1..4 identifiers).
    fn arb_import_names() -> impl Strategy<Value = Vec<String>> {
        proptest::collection::vec(arb_ident(), 1..4).prop_map(|names| {
            let mut seen = std::collections::HashSet::new();
            let unique: Vec<String> = names.into_iter().filter(|n| seen.insert(n.clone())).collect();
            unique
        }).prop_filter("at least one name", |v: &Vec<String>| !v.is_empty())
    }

    // ========================================================================
    // Helper: parse a library file source and return the first Import
    // ========================================================================

    /// Wraps an import statement in a minimal library file (with a dummy fn def)
    /// so the parser can handle it as a complete program.
    fn parse_import_from_source(source: &str) -> crate::error::Result<crate::parser::ast::Import> {
        // Wrap the import in a library file with a dummy function
        let full_source = format!("{}\nfn _dummy() {{\n    return 1\n}}\n", source);
        let tokens = lex_with_spans(&full_source)?;
        let program = parse(tokens)?;
        assert!(!program.imports.is_empty(), "Expected at least one import in parsed program");
        Ok(program.imports[0].clone())
    }

    // ========================================================================
    // Property Tests
    // ========================================================================

    // Feature: flux-module-imports, Property 2: Import path separator preservation (round-trip)
    // **Validates: Requirements 2.1, 2.2, 2.3, 9.2**

    // For any import with `::` separators, the parsed module_path preserves `::` between segments.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn colon_colon_separator_preserved_in_module_path(
            segments in proptest::collection::vec(arb_module_segment(), 2..6),
            names in arb_import_names(),
        ) {
            let module_path = segments.join("::");
            let names_str = names.join(", ");
            let source = format!("from {} import {{{}}}", module_path, names_str);

            let import = parse_import_from_source(&source).unwrap();

            // The parsed module_path should exactly match our generated path with :: separators
            prop_assert_eq!(&import.module_path, &module_path,
                "Expected module_path '{}' but got '{}' from source '{}'",
                module_path, import.module_path, source);

            // Verify each segment is present and separated by ::
            let parsed_segments: Vec<&str> = import.module_path.split("::").collect();
            prop_assert_eq!(parsed_segments.len(), segments.len(),
                "Expected {} segments but got {} in module_path '{}'",
                segments.len(), parsed_segments.len(), import.module_path);

            for (expected, actual) in segments.iter().zip(parsed_segments.iter()) {
                prop_assert_eq!(expected.as_str(), *actual,
                    "Segment mismatch: expected '{}', got '{}'", expected, actual);
            }

            // Verify import names are preserved
            prop_assert_eq!(import.names.len(), names.len(),
                "Expected {} import names but got {}", names.len(), import.names.len());
            for (expected, actual) in names.iter().zip(import.names.iter()) {
                prop_assert_eq!(expected, actual,
                    "Import name mismatch: expected '{}', got '{}'", expected, actual);
            }
        }
    }

    // For any import with `.` separators, the parsed module_path preserves `.` between segments.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn dot_separator_preserved_in_module_path(
            segments in proptest::collection::vec(arb_module_segment(), 1..5),
            names in arb_import_names(),
        ) {
            let module_path = segments.join(".");
            let names_str = names.join(", ");
            let source = format!("from {} import {{{}}}", module_path, names_str);

            let import = parse_import_from_source(&source).unwrap();

            // The parsed module_path should exactly match our generated path with . separators
            prop_assert_eq!(&import.module_path, &module_path,
                "Expected module_path '{}' but got '{}' from source '{}'",
                module_path, import.module_path, source);

            // Verify no :: appears in dot-separated paths
            prop_assert!(!import.module_path.contains("::"),
                "Dot-separated path should not contain '::': '{}'", import.module_path);

            // For multi-segment paths, verify segments are separated by .
            if segments.len() > 1 {
                let parsed_segments: Vec<&str> = import.module_path.split('.').collect();
                prop_assert_eq!(parsed_segments.len(), segments.len(),
                    "Expected {} segments but got {} in module_path '{}'",
                    segments.len(), parsed_segments.len(), import.module_path);

                for (expected, actual) in segments.iter().zip(parsed_segments.iter()) {
                    prop_assert_eq!(expected.as_str(), *actual,
                        "Segment mismatch: expected '{}', got '{}'", expected, actual);
                }
            }
        }
    }

    // Round-trip: pretty-printing and re-parsing an import with :: produces the same module_path.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn colon_colon_import_round_trip(
            segments in proptest::collection::vec(arb_module_segment(), 2..5),
            names in arb_import_names(),
        ) {
            let module_path = segments.join("::");
            let names_str = names.join(", ");
            // Use a full strategy program for round-trip so pretty-print produces valid output
            let source = format!(
                "from {} import {{{}}}\nstrategy Test {{\n    on bar {{\n        return 1\n    }}\n}}\n",
                module_path, names_str
            );

            // First parse: source → AST
            let tokens = lex_with_spans(&source).unwrap();
            let program = parse(tokens).unwrap();

            // Pretty-print the program back to source
            let printed = pretty_print_program(&program);

            // Re-parse the pretty-printed output
            let tokens2 = lex_with_spans(&printed).unwrap();
            let program2 = parse(tokens2).unwrap();

            // The module_path should survive the round-trip
            prop_assert!(!program2.imports.is_empty(),
                "Re-parsed program should have imports");
            prop_assert_eq!(&program2.imports[0].module_path, &module_path,
                "Round-trip module_path mismatch: expected '{}', got '{}'",
                module_path, program2.imports[0].module_path);

            // Import names should also survive
            prop_assert_eq!(&program2.imports[0].names, &names,
                "Round-trip import names mismatch");
        }
    }

    // Round-trip: pretty-printing and re-parsing an import with . produces the same module_path.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn dot_import_round_trip(
            segments in proptest::collection::vec(arb_module_segment(), 1..5),
            names in arb_import_names(),
        ) {
            let module_path = segments.join(".");
            let names_str = names.join(", ");
            // Use a full strategy program for round-trip so pretty-print produces valid output
            let source = format!(
                "from {} import {{{}}}\nstrategy Test {{\n    on bar {{\n        return 1\n    }}\n}}\n",
                module_path, names_str
            );

            // First parse: source → AST
            let tokens = lex_with_spans(&source).unwrap();
            let program = parse(tokens).unwrap();

            // Pretty-print the program back to source
            let printed = pretty_print_program(&program);

            // Re-parse the pretty-printed output
            let tokens2 = lex_with_spans(&printed).unwrap();
            let program2 = parse(tokens2).unwrap();

            // The module_path should survive the round-trip
            prop_assert!(!program2.imports.is_empty(),
                "Re-parsed program should have imports");
            prop_assert_eq!(&program2.imports[0].module_path, &module_path,
                "Round-trip module_path mismatch: expected '{}', got '{}'",
                module_path, program2.imports[0].module_path);

            // Import names should also survive
            prop_assert_eq!(&program2.imports[0].names, &names,
                "Round-trip import names mismatch");
        }
    }
}
