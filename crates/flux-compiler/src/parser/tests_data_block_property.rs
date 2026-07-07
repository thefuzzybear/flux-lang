//! Property-based tests for data block parse round-trip.
//!
//! Feature: flux-run-harness, Property 2: Data block parse round-trip
//!
//! **Validates: Requirements 1.2, 1.3, 1.4, 1.8, 9.2**
//!
//! For any valid combination of data block fields (symbols list of 1+ non-empty
//! strings, period/interval/source as arbitrary string literals), constructing the
//! source text `data { symbols = [...] period = "..." interval = "..." source = "..." }`,
//! parsing it into a `DataBlock` AST node, and then reading back the field values
//! SHALL produce the original values with matching field content.

#[cfg(test)]
mod tests {
    use crate::lexer::lex_with_spans;
    use crate::parser::ast::DataBlock;
    use crate::parser::parse;
    use proptest::prelude::*;

    // ========================================================================
    // Generators
    // ========================================================================

    /// Generate a safe string value (alphanumeric + underscore, no quotes/backslashes)
    fn arb_safe_string() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9_]{1,12}"
    }

    /// Generate a non-empty vector of 1-5 symbol strings
    fn arb_symbols() -> impl Strategy<Value = Vec<String>> {
        proptest::collection::vec(arb_safe_string(), 1..=5)
    }

    // ========================================================================
    // Source construction helpers
    // ========================================================================

    /// Build a symbols list literal: `["SYM1", "SYM2"]`
    fn format_symbols_list(symbols: &[String]) -> String {
        let items: Vec<String> = symbols.iter().map(|s| format!("\"{}\"", s)).collect();
        format!("[{}]", items.join(", "))
    }

    /// Build a complete valid .flux source with a data block and minimal strategy.
    ///
    /// The strategy wrapping is required because `parse()` expects a complete program.
    fn build_source(
        symbols: Option<&Vec<String>>,
        period: Option<&str>,
        interval: Option<&str>,
        source: Option<&str>,
    ) -> String {
        let mut data_body = String::new();

        if let Some(syms) = symbols {
            data_body.push_str(&format!("    symbols = {}\n", format_symbols_list(syms)));
        }
        if let Some(p) = period {
            data_body.push_str(&format!("    period = \"{}\"\n", p));
        }
        if let Some(i) = interval {
            data_body.push_str(&format!("    interval = \"{}\"\n", i));
        }
        if let Some(s) = source {
            data_body.push_str(&format!("    source = \"{}\"\n", s));
        }

        format!(
            "data {{\n{}}}\n\nstrategy Test {{\n    on bar {{\n    }}\n}}\n",
            data_body
        )
    }

    // ========================================================================
    // Assertion helpers
    // ========================================================================

    fn assert_data_block_fields(
        block: &DataBlock,
        expected_symbols: Option<&Vec<String>>,
        expected_period: Option<&str>,
        expected_interval: Option<&str>,
        expected_source: Option<&str>,
    ) {
        // Check symbols
        match (expected_symbols, &block.symbols) {
            (Some(expected), Some(field)) => {
                assert_eq!(
                    &field.value, expected,
                    "Symbols mismatch: expected {:?}, got {:?}",
                    expected, field.value
                );
            }
            (None, None) => {}
            (Some(expected), None) => {
                panic!("Expected symbols {:?} but data block has None", expected);
            }
            (None, Some(field)) => {
                panic!(
                    "Expected no symbols but data block has {:?}",
                    field.value
                );
            }
        }

        // Check period
        match (expected_period, &block.period) {
            (Some(expected), Some(field)) => {
                assert_eq!(
                    field.value, expected,
                    "Period mismatch: expected {:?}, got {:?}",
                    expected, field.value
                );
            }
            (None, None) => {}
            (Some(expected), None) => {
                panic!("Expected period {:?} but data block has None", expected);
            }
            (None, Some(field)) => {
                panic!("Expected no period but data block has {:?}", field.value);
            }
        }

        // Check interval
        match (expected_interval, &block.interval) {
            (Some(expected), Some(field)) => {
                assert_eq!(
                    field.value, expected,
                    "Interval mismatch: expected {:?}, got {:?}",
                    expected, field.value
                );
            }
            (None, None) => {}
            (Some(expected), None) => {
                panic!("Expected interval {:?} but data block has None", expected);
            }
            (None, Some(field)) => {
                panic!(
                    "Expected no interval but data block has {:?}",
                    field.value
                );
            }
        }

        // Check source
        match (expected_source, &block.source) {
            (Some(expected), Some(field)) => {
                assert_eq!(
                    field.value, expected,
                    "Source mismatch: expected {:?}, got {:?}",
                    expected, field.value
                );
            }
            (None, None) => {}
            (Some(expected), None) => {
                panic!("Expected source {:?} but data block has None", expected);
            }
            (None, Some(field)) => {
                panic!("Expected no source but data block has {:?}", field.value);
            }
        }
    }

    // ========================================================================
    // Property Tests
    // ========================================================================

    // Feature: flux-run-harness, Property 2: Data block parse round-trip
    // **Validates: Requirements 1.2, 1.3, 1.4, 1.8, 9.2**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// Property: For any valid combination of data block fields, constructing
        /// source text, parsing it, and reading back field values produces the
        /// original values.
        #[test]
        fn prop_data_block_parse_round_trip_all_fields(
            symbols in arb_symbols(),
            period in arb_safe_string(),
            interval in arb_safe_string(),
            source in arb_safe_string(),
        ) {
            let src = build_source(
                Some(&symbols),
                Some(&period),
                Some(&interval),
                Some(&source),
            );

            let tokens = lex_with_spans(&src).unwrap_or_else(|e| {
                panic!("Lexing failed for source:\n{}\nError: {}", src, e)
            });
            let program = parse(tokens).unwrap_or_else(|e| {
                panic!("Parsing failed for source:\n{}\nError: {}", src, e)
            });

            let data_block = program.data_block.as_ref().unwrap_or_else(|| {
                panic!("Expected data_block to be Some after parsing:\n{}", src)
            });

            assert_data_block_fields(
                data_block,
                Some(&symbols),
                Some(&period),
                Some(&interval),
                Some(&source),
            );
        }

        /// Property: Parsing succeeds with only the symbols field present.
        #[test]
        fn prop_data_block_parse_round_trip_symbols_only(
            symbols in arb_symbols(),
        ) {
            let src = build_source(Some(&symbols), None, None, None);

            let tokens = lex_with_spans(&src).unwrap_or_else(|e| {
                panic!("Lexing failed for source:\n{}\nError: {}", src, e)
            });
            let program = parse(tokens).unwrap_or_else(|e| {
                panic!("Parsing failed for source:\n{}\nError: {}", src, e)
            });

            let data_block = program.data_block.as_ref().unwrap_or_else(|| {
                panic!("Expected data_block to be Some after parsing:\n{}", src)
            });

            assert_data_block_fields(data_block, Some(&symbols), None, None, None);
        }

        /// Property: Parsing succeeds with a subset of string fields (period + source).
        #[test]
        fn prop_data_block_parse_round_trip_partial_fields(
            symbols in arb_symbols(),
            period in arb_safe_string(),
            source in arb_safe_string(),
        ) {
            let src = build_source(Some(&symbols), Some(&period), None, Some(&source));

            let tokens = lex_with_spans(&src).unwrap_or_else(|e| {
                panic!("Lexing failed for source:\n{}\nError: {}", src, e)
            });
            let program = parse(tokens).unwrap_or_else(|e| {
                panic!("Parsing failed for source:\n{}\nError: {}", src, e)
            });

            let data_block = program.data_block.as_ref().unwrap_or_else(|| {
                panic!("Expected data_block to be Some after parsing:\n{}", src)
            });

            assert_data_block_fields(
                data_block,
                Some(&symbols),
                Some(&period),
                None,
                Some(&source),
            );
        }
    }
}
