//! Property-based test: Connector keyword-identifier disambiguation
//!
//! **Validates: Requirements 8.1**
//!
//! Property 9: The exact string "connector" lexes as Token::Connector,
//! while any longer identifier-legal string starting with "connector"
//! lexes as Token::Ident. Other random identifiers also lex as Token::Ident.

use flux_compiler::lexer::{lex, Token};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// The exact string "connector" must lex as Token::Connector.
    /// This is a fixed assertion run once per proptest invocation to anchor the property.
    #[test]
    fn connector_exact_keyword_lexes_as_connector(_dummy in 0..1u8) {
        let tokens = lex("connector").unwrap();
        prop_assert_eq!(tokens.len(), 2);
        prop_assert_eq!(&tokens[0], &Token::Connector);
        prop_assert_eq!(&tokens[1], &Token::Eof);
    }

    /// Strings that start with "connector" but are longer must lex as Token::Ident.
    /// Generate a non-empty suffix of identifier-legal characters.
    #[test]
    fn connector_prefix_with_suffix_lexes_as_ident(
        suffix in "[a-zA-Z0-9_]{1,50}"
    ) {
        let input = format!("connector{}", suffix);
        let tokens = lex(&input).unwrap();
        prop_assert_eq!(tokens.len(), 2, "Expected 2 tokens for input {:?}, got {:?}", input, tokens);
        prop_assert_eq!(&tokens[0], &Token::Ident(input.clone()),
            "Expected Ident({:?}) but got {:?}", input, tokens[0]);
        prop_assert_eq!(&tokens[1], &Token::Eof);
    }

    /// Random valid identifiers (that are not any keyword) lex as Token::Ident.
    #[test]
    fn random_identifiers_lex_as_ident(
        ident in "[a-zA-Z_][a-zA-Z0-9_]{0,100}"
    ) {
        // Filter out all keywords (including "connector" and "data")
        let keywords = [
            "strategy", "params", "state", "on", "if", "elif", "else",
            "for", "while", "return", "from", "import", "and", "or",
            "not", "true", "false", "null", "data", "connector",
        ];
        prop_assume!(!keywords.contains(&ident.as_str()));
        // Skip identifiers that exceed max length
        prop_assume!(ident.len() <= 255);

        let tokens = lex(&ident).unwrap();
        prop_assert_eq!(tokens.len(), 2);
        prop_assert_eq!(&tokens[0], &Token::Ident(ident.clone()),
            "Expected Ident({:?}) but got {:?}", ident, tokens[0]);
        prop_assert_eq!(&tokens[1], &Token::Eof);
    }
}
