//! Property-based tests for keyword-identifier disambiguation of `data`
//!
//! Feature: flux-run-harness, Property 1: Keyword-identifier disambiguation for `data`
//!
//! **Validates: Requirements 1.1, 9.1**
//!
//! For any identifier-legal string S, if S is exactly `"data"` then lexing
//! SHALL produce `Token::Data`, and if S starts with `"data"` but is longer
//! (e.g., `"data_source"`, `"database"`) then lexing SHALL produce `Token::Ident(S)`.

#[cfg(test)]
mod tests {
    use crate::lexer::{lex, Token};
    use proptest::prelude::*;

    // Feature: flux-run-harness, Property 1: Keyword-identifier disambiguation for `data`
    // **Validates: Requirements 1.1, 9.1**

    /// The exact string "data" must always lex to Token::Data
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn data_exact_produces_keyword_token(_dummy in 0..100u32) {
            // "data" is a fixed input, but we run it 100 times to satisfy the
            // property-based testing requirement and confirm determinism
            let tokens = lex("data").unwrap();
            prop_assert_eq!(tokens.len(), 2);
            prop_assert_eq!(&tokens[0], &Token::Data);
            prop_assert_eq!(&tokens[1], &Token::Eof);
        }
    }

    /// Any identifier-legal string that starts with "data" but is longer must
    /// lex to Token::Ident (not Token::Data)
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn data_prefix_with_suffix_produces_ident(
            suffix in "[a-zA-Z0-9_]{1,50}"
        ) {
            let input = format!("data{}", suffix);
            let tokens = lex(&input).unwrap();
            prop_assert_eq!(tokens.len(), 2,
                "Expected exactly 2 tokens (Ident + Eof) for input '{}', got {:?}",
                input, tokens);
            prop_assert_eq!(&tokens[0], &Token::Ident(input.clone()),
                "Expected Token::Ident(\"{}\") but got {:?}", input, tokens[0]);
            prop_assert_eq!(&tokens[1], &Token::Eof);
        }
    }

    /// Any identifier-legal string that does NOT start with "data" and is not
    /// a keyword must lex to Token::Ident
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn non_data_identifier_produces_ident(
            ident in "[a-zA-Z_][a-zA-Z0-9_]{0,49}"
        ) {
            // Filter out all keywords (including "data")
            let keywords = [
                "strategy", "params", "state", "on", "if", "elif", "else",
                "for", "while", "return", "from", "import", "and", "or",
                "not", "true", "false", "null", "data",
            ];
            prop_assume!(!keywords.contains(&ident.as_str()));
            // Also filter out identifiers longer than 255 chars (max length)
            prop_assume!(ident.len() <= 255);

            let tokens = lex(&ident).unwrap();
            prop_assert_eq!(tokens.len(), 2,
                "Expected exactly 2 tokens for '{}', got {:?}", ident, tokens);
            prop_assert_eq!(&tokens[0], &Token::Ident(ident.clone()),
                "Expected Token::Ident(\"{}\") but got {:?}", ident, tokens[0]);
            prop_assert_eq!(&tokens[1], &Token::Eof);
        }
    }
}
