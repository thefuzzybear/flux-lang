//! Property-based tests for keyword-identifier disambiguation of `in`
//!
//! Feature: for-loop-iteration, Property 1: Identifier-keyword disambiguation
//!
//! **Validates: Requirements 1.1, 1.2**
//!
//! For any string that starts with "in" followed by one or more alphanumeric or
//! underscore characters, the lexer SHALL produce an `Ident` token (not an `In`
//! keyword token). The bare string "in" SHALL produce `Token::In`.

#[cfg(test)]
mod tests {
    use crate::lexer::{lex, Token};
    use proptest::prelude::*;

    // Feature: for-loop-iteration, Property 1: Identifier-keyword disambiguation
    // **Validates: Requirements 1.1, 1.2**

    /// The exact string "in" must always lex to Token::In
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn in_exact_produces_keyword_token(_dummy in 0..100u32) {
            // "in" is a fixed input, but we run it 100 times to confirm
            // determinism and satisfy the property-based testing requirement
            let tokens = lex("in").unwrap();
            prop_assert_eq!(tokens.len(), 2);
            prop_assert_eq!(&tokens[0], &Token::In);
            prop_assert_eq!(&tokens[1], &Token::Eof);
        }
    }

    /// Any string that starts with "in" followed by 1-10 alphanumeric or
    /// underscore characters must lex to Token::Ident (not Token::In)
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn in_prefix_with_suffix_produces_ident(
            suffix in "[a-zA-Z0-9_]{1,10}"
        ) {
            let input = format!("in{}", suffix);
            let tokens = lex(&input).unwrap();
            prop_assert_eq!(tokens.len(), 2,
                "Expected exactly 2 tokens (Ident + Eof) for input '{}', got {:?}",
                input, tokens);
            prop_assert_eq!(&tokens[0], &Token::Ident(input.clone()),
                "Expected Token::Ident(\"{}\") but got {:?}", input, tokens[0]);
            prop_assert_eq!(&tokens[1], &Token::Eof);
        }
    }
}
