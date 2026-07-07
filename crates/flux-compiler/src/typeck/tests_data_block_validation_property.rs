//! Property-based tests for data block type validation.
//!
//! Feature: flux-run-harness, Property 3: Invalid data block field values are rejected with valid options
//!
//! **Validates: Requirements 2.2, 2.3, 2.4**
//!
//! For any string that is not a member of the valid set for a data block field
//! (period not in {1d, 5d, 1mo, 3mo, 6mo, 1y, 2y, 5y, max};
//! interval not in {1m, 5m, 15m, 1h, 1d, 1wk, 1mo};
//! source not in {yahoo}),
//! the typechecker SHALL return an error whose message contains all valid options
//! for that field.

#[cfg(test)]
mod tests {
    use crate::error::CompileError;
    use crate::lexer::lex_with_spans;
    use crate::parser::parse;
    use crate::typeck::check;
    use proptest::prelude::*;

    // ========================================================================
    // Valid sets (must match the typechecker implementation)
    // ========================================================================

    const VALID_PERIODS: &[&str] = &["1d", "5d", "1mo", "3mo", "6mo", "1y", "2y", "5y", "max"];
    const VALID_INTERVALS: &[&str] = &["1m", "5m", "15m", "1h", "1d", "1wk", "1mo"];
    const VALID_SOURCES: &[&str] = &["yahoo"];

    // ========================================================================
    // Generators
    // ========================================================================

    /// Generate a string of safe characters (alphanumeric + underscore) that is
    /// NOT in the given valid set.
    fn arb_invalid_value(valid_set: &'static [&'static str]) -> impl Strategy<Value = String> {
        "[a-zA-Z0-9_]{1,12}".prop_filter("must not be in valid set", move |s| {
            !valid_set.contains(&s.as_str())
        })
    }

    // ========================================================================
    // Source construction helpers
    // ========================================================================

    /// Build a complete .flux source with a data block containing an invalid period.
    fn build_source_with_period(period: &str) -> String {
        format!(
            "data {{\n    symbols = [\"AAPL\"]\n    period = \"{}\"\n}}\n\nstrategy Test {{\n    on bar {{\n    }}\n}}\n",
            period
        )
    }

    /// Build a complete .flux source with a data block containing an invalid interval.
    fn build_source_with_interval(interval: &str) -> String {
        format!(
            "data {{\n    symbols = [\"AAPL\"]\n    interval = \"{}\"\n}}\n\nstrategy Test {{\n    on bar {{\n    }}\n}}\n",
            interval
        )
    }

    /// Build a complete .flux source with a data block containing an invalid source.
    fn build_source_with_source(source: &str) -> String {
        format!(
            "data {{\n    symbols = [\"AAPL\"]\n    source = \"{}\"\n}}\n\nstrategy Test {{\n    on bar {{\n    }}\n}}\n",
            source
        )
    }

    /// Helper: lex → parse → typecheck a source string.
    fn typecheck_source(source: &str) -> Result<(), CompileError> {
        let tokens = lex_with_spans(source).map_err(|e| {
            panic!("Lexing failed for source:\n{}\nError: {}", source, e);
        }).unwrap();
        let program = parse(tokens).map_err(|e| {
            panic!("Parsing failed for source:\n{}\nError: {}", source, e);
        }).unwrap();
        check(program).map(|_| ())
    }

    // ========================================================================
    // Property Tests
    // ========================================================================

    // Feature: flux-run-harness, Property 3: Invalid data block field values are rejected with valid options
    // **Validates: Requirements 2.2, 2.3, 2.4**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// Property: For any string not in the valid period set, the typechecker
        /// returns an error whose message contains all valid period options.
        ///
        /// **Validates: Requirements 2.2**
        #[test]
        fn prop_invalid_period_rejected_with_valid_options(
            invalid_period in arb_invalid_value(VALID_PERIODS)
        ) {
            let src = build_source_with_period(&invalid_period);
            let result = typecheck_source(&src);

            // Must be an error
            prop_assert!(result.is_err(),
                "Typechecker should reject invalid period '{}', but got Ok", invalid_period);

            let err = result.unwrap_err();
            match &err {
                CompileError::Type(msg) => {
                    // Error must contain all valid period options
                    for valid in VALID_PERIODS {
                        prop_assert!(
                            msg.contains(valid),
                            "Error message should list valid option '{}' for period. Got: {}",
                            valid, msg
                        );
                    }
                    // Error should mention the invalid value
                    prop_assert!(
                        msg.contains(&invalid_period),
                        "Error message should mention the invalid value '{}'. Got: {}",
                        invalid_period, msg
                    );
                }
                other => {
                    prop_assert!(false,
                        "Expected CompileError::Type for invalid period '{}', got: {:?}",
                        invalid_period, other);
                }
            }
        }

        /// Property: For any string not in the valid interval set, the typechecker
        /// returns an error whose message contains all valid interval options.
        ///
        /// **Validates: Requirements 2.3**
        #[test]
        fn prop_invalid_interval_rejected_with_valid_options(
            invalid_interval in arb_invalid_value(VALID_INTERVALS)
        ) {
            let src = build_source_with_interval(&invalid_interval);
            let result = typecheck_source(&src);

            // Must be an error
            prop_assert!(result.is_err(),
                "Typechecker should reject invalid interval '{}', but got Ok", invalid_interval);

            let err = result.unwrap_err();
            match &err {
                CompileError::Type(msg) => {
                    // Error must contain all valid interval options
                    for valid in VALID_INTERVALS {
                        prop_assert!(
                            msg.contains(valid),
                            "Error message should list valid option '{}' for interval. Got: {}",
                            valid, msg
                        );
                    }
                    // Error should mention the invalid value
                    prop_assert!(
                        msg.contains(&invalid_interval),
                        "Error message should mention the invalid value '{}'. Got: {}",
                        invalid_interval, msg
                    );
                }
                other => {
                    prop_assert!(false,
                        "Expected CompileError::Type for invalid interval '{}', got: {:?}",
                        invalid_interval, other);
                }
            }
        }

        /// Property: For any string not in the valid source set, the typechecker
        /// returns an error whose message contains all valid source options.
        ///
        /// **Validates: Requirements 2.4**
        #[test]
        fn prop_invalid_source_rejected_with_valid_options(
            invalid_source in arb_invalid_value(VALID_SOURCES)
        ) {
            let src = build_source_with_source(&invalid_source);
            let result = typecheck_source(&src);

            // Must be an error
            prop_assert!(result.is_err(),
                "Typechecker should reject invalid source '{}', but got Ok", invalid_source);

            let err = result.unwrap_err();
            match &err {
                CompileError::Type(msg) => {
                    // Error must contain all valid source options
                    for valid in VALID_SOURCES {
                        prop_assert!(
                            msg.contains(valid),
                            "Error message should list valid option '{}' for source. Got: {}",
                            valid, msg
                        );
                    }
                    // Error should mention the invalid value
                    prop_assert!(
                        msg.contains(&invalid_source),
                        "Error message should mention the invalid value '{}'. Got: {}",
                        invalid_source, msg
                    );
                }
                other => {
                    prop_assert!(false,
                        "Expected CompileError::Type for invalid source '{}', got: {:?}",
                        invalid_source, other);
                }
            }
        }
    }
}
