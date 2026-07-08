//! Property-based tests for ColonColon vs Colon token discrimination
//!
//! Feature: flux-module-imports, Property 1: ColonColon vs Colon token discrimination
//!
//! **Validates: Requirements 1.1, 1.2, 1.3**
//!
//! For any source text containing `::`, the lexer should emit exactly one
//! ColonColon token for that sequence (not two Colon tokens). For any single
//! `:` not followed by another `:`, the lexer should emit a Colon token.

#[cfg(test)]
mod tests {
    use crate::lexer::{lex, Token};
    use proptest::prelude::*;

    // ========================================================================
    // Generators
    // ========================================================================

    /// Generate a valid Flux identifier (not a keyword)
    fn arb_ident() -> impl Strategy<Value = String> {
        "[a-zA-Z_][a-zA-Z0-9_]{0,20}".prop_filter("must not be a keyword", |s| {
            let keywords = [
                "strategy", "params", "state", "on", "if", "elif", "else",
                "for", "while", "return", "from", "import", "and", "or",
                "not", "true", "false", "null", "data", "connector", "fn",
            ];
            !keywords.contains(&s.as_str())
        })
    }

    // ========================================================================
    // Property Tests
    // ========================================================================

    // Feature: flux-module-imports, Property 1: ColonColon vs Colon token discrimination
    // **Validates: Requirements 1.1, 1.2, 1.3**

    // For any two identifiers `a` and `b`, the source `a::b` should produce
    // exactly [Ident(a), ColonColon, Ident(b), Eof] — never two Colon tokens.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn double_colon_emits_colon_colon_token(
            left in arb_ident(),
            right in arb_ident(),
        ) {
            let source = format!("{}::{}", left, right);
            let tokens = lex(&source).unwrap();

            // Should produce exactly 4 tokens: Ident, ColonColon, Ident, Eof
            prop_assert_eq!(tokens.len(), 4,
                "Expected 4 tokens for '{}', got {:?}", source, tokens);
            prop_assert_eq!(&tokens[0], &Token::Ident(left.clone()),
                "First token should be Ident('{}'), got {:?}", left, tokens[0]);
            prop_assert_eq!(&tokens[1], &Token::ColonColon,
                "Second token should be ColonColon, got {:?}", tokens[1]);
            prop_assert_eq!(&tokens[2], &Token::Ident(right.clone()),
                "Third token should be Ident('{}'), got {:?}", right, tokens[2]);
            prop_assert_eq!(&tokens[3], &Token::Eof);

            // Verify no Colon tokens appear (ColonColon must not be split)
            for (i, tok) in tokens.iter().enumerate() {
                prop_assert_ne!(tok, &Token::Colon,
                    "Unexpected Colon token at index {} in source '{}'", i, source);
            }
        }
    }

    // For any two identifiers `a` and `b`, the source `a:b` (single colon)
    // should produce [Ident(a), Colon, Ident(b), Eof] — never ColonColon.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn single_colon_emits_colon_token(
            left in arb_ident(),
            right in arb_ident(),
        ) {
            let source = format!("{}:{}", left, right);
            let tokens = lex(&source).unwrap();

            // Should produce exactly 4 tokens: Ident, Colon, Ident, Eof
            prop_assert_eq!(tokens.len(), 4,
                "Expected 4 tokens for '{}', got {:?}", source, tokens);
            prop_assert_eq!(&tokens[0], &Token::Ident(left.clone()),
                "First token should be Ident('{}'), got {:?}", left, tokens[0]);
            prop_assert_eq!(&tokens[1], &Token::Colon,
                "Second token should be Colon, got {:?}", tokens[1]);
            prop_assert_eq!(&tokens[2], &Token::Ident(right.clone()),
                "Third token should be Ident('{}'), got {:?}", right, tokens[2]);
            prop_assert_eq!(&tokens[3], &Token::Eof);

            // Verify no ColonColon tokens appear
            for (i, tok) in tokens.iter().enumerate() {
                prop_assert_ne!(tok, &Token::ColonColon,
                    "Unexpected ColonColon token at index {} in source '{}'", i, source);
            }
        }
    }

    // For a multi-segment path `a::b::c`, each `::` should produce exactly
    // one ColonColon token (longest-match semantics).
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn multi_segment_path_emits_correct_colon_colon_count(
            segments in prop::collection::vec(arb_ident(), 2..6),
        ) {
            let source = segments.join("::");
            let tokens = lex(&source).unwrap();

            // Count ColonColon tokens — should equal (segments - 1)
            let colon_colon_count = tokens.iter()
                .filter(|t| **t == Token::ColonColon)
                .count();
            let expected_separators = segments.len() - 1;

            prop_assert_eq!(colon_colon_count, expected_separators,
                "Expected {} ColonColon tokens for {} segments in '{}', got {}",
                expected_separators, segments.len(), source, colon_colon_count);

            // No plain Colon tokens should appear
            let colon_count = tokens.iter()
                .filter(|t| **t == Token::Colon)
                .count();
            prop_assert_eq!(colon_count, 0,
                "Expected 0 Colon tokens in '{}', got {}", source, colon_count);

            // Verify Ident tokens match the segments
            let ident_tokens: Vec<&Token> = tokens.iter()
                .filter(|t| matches!(t, Token::Ident(_)))
                .collect();
            prop_assert_eq!(ident_tokens.len(), segments.len(),
                "Expected {} Ident tokens in '{}', got {}", segments.len(), source, ident_tokens.len());

            for (ident_tok, expected_name) in ident_tokens.iter().zip(segments.iter()) {
                prop_assert_eq!(*ident_tok, &Token::Ident(expected_name.clone()),
                    "Ident mismatch in source '{}'", source);
            }
        }
    }

    // A single `:` followed by a space (not another `:`) should always be Colon.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn colon_followed_by_non_colon_emits_colon(
            left in arb_ident(),
            right in arb_ident(),
        ) {
            // Use space to separate the colon from the next identifier
            let source = format!("{}: {}", left, right);
            let tokens = lex(&source).unwrap();

            prop_assert_eq!(tokens.len(), 4,
                "Expected 4 tokens for '{}', got {:?}", source, tokens);
            prop_assert_eq!(&tokens[1], &Token::Colon,
                "Expected Colon after '{}' in '{}', got {:?}", left, source, tokens[1]);

            // Confirm no ColonColon
            for (i, tok) in tokens.iter().enumerate() {
                prop_assert_ne!(tok, &Token::ColonColon,
                    "Unexpected ColonColon at index {} in source '{}'", i, source);
            }
        }
    }
}
