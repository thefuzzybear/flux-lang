//! Property-based tests for unrecognized data block keys rejection.
//!
//! Feature: flux-run-harness, Property 4: Unrecognized data block keys are rejected
//!
//! **Validates: Requirements 2.5**
//!
//! For any identifier string that is not one of {symbols, period, interval, source},
//! a data block containing that key SHALL produce a parse error whose message lists
//! the valid keys.

#[cfg(test)]
mod tests {
    use crate::lexer::lex_with_spans;
    use crate::parser::parse;
    use proptest::prelude::*;

    /// The set of valid data block keys.
    const VALID_KEYS: &[&str] = &["symbols", "period", "interval", "source"];

    /// All Flux keywords that would lex as keyword tokens rather than identifiers.
    /// These must be filtered out because the lexer won't produce an Ident token for them,
    /// causing a different parse error (not the "unrecognized key" error we're testing).
    const KEYWORDS: &[&str] = &[
        "strategy", "params", "state", "on", "if", "elif", "else", "for", "while",
        "return", "from", "import", "and", "or", "not", "true", "false", "null", "data",
    ];

    /// Generate identifier-legal strings that are NOT valid data block keys and NOT keywords.
    fn arb_invalid_key() -> impl Strategy<Value = String> {
        "[a-zA-Z_][a-zA-Z0-9_]{0,11}".prop_filter(
            "must not be a valid data block key or a Flux keyword",
            |s| {
                !VALID_KEYS.contains(&s.as_str()) && !KEYWORDS.contains(&s.as_str())
            },
        )
    }

    /// Build a source string with an invalid key inside a data block.
    fn build_source_with_key(key: &str) -> String {
        format!(
            "data {{\n    {} = \"value\"\n}}\nstrategy Test {{\n    on bar {{\n    }}\n}}\n",
            key
        )
    }

    // ========================================================================
    // Property Tests
    // ========================================================================

    // Feature: flux-run-harness, Property 4: Unrecognized data block keys are rejected
    // **Validates: Requirements 2.5**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        /// Property: For any identifier string that is not one of {symbols, period,
        /// interval, source}, a data block containing that key produces a parse error
        /// whose message lists the valid keys.
        #[test]
        fn prop_unrecognized_data_block_keys_are_rejected(key in arb_invalid_key()) {
            let src = build_source_with_key(&key);

            // Lexing should succeed — the key is a valid identifier
            let tokens = lex_with_spans(&src).unwrap_or_else(|e| {
                panic!("Lexing should succeed for source:\n{}\nError: {}", src, e)
            });

            // Parsing should fail with an error about unrecognized key
            let result = parse(tokens);
            prop_assert!(
                result.is_err(),
                "Expected parse error for unrecognized key '{}', but parsing succeeded.\nSource:\n{}",
                key,
                src
            );

            let err_msg = match result.unwrap_err() {
                crate::error::CompileError::Parser(msg) => msg,
                other => panic!("Expected CompileError::Parser, got: {:?}", other),
            };

            // The error message must list all valid keys
            prop_assert!(
                err_msg.contains("symbols"),
                "Error message should contain 'symbols'. Got: {}",
                err_msg
            );
            prop_assert!(
                err_msg.contains("period"),
                "Error message should contain 'period'. Got: {}",
                err_msg
            );
            prop_assert!(
                err_msg.contains("interval"),
                "Error message should contain 'interval'. Got: {}",
                err_msg
            );
            prop_assert!(
                err_msg.contains("source"),
                "Error message should contain 'source'. Got: {}",
                err_msg
            );

            // The error message must contain the invalid key name
            prop_assert!(
                err_msg.contains(&key),
                "Error message should contain the invalid key '{}'. Got: {}",
                key,
                err_msg
            );
        }
    }
}
