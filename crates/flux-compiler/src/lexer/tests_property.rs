//! Property-based tests for the Flux lexer
//!
//! These tests use proptest to verify universal correctness properties
//! hold across many randomly generated inputs.

#[cfg(test)]
mod tests {
    use crate::lexer::{lex, lex_with_spans, Token};
    use proptest::prelude::*;

    // Feature: flux-lexer, Property 2: Identifier Round-Trip
    // **Validates: Requirements 2.1, 2.3**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn identifier_round_trip(
            ident in "[a-zA-Z_][a-zA-Z0-9_]{0,254}"
        ) {
            // Filter out exact keywords
            let keywords = ["strategy", "params", "state", "on", "if", "elif", "else",
                           "for", "while", "return", "from", "import", "and", "or",
                           "not", "true", "false", "null"];
            prop_assume!(!keywords.contains(&ident.as_str()));

            let tokens = lex(&ident).unwrap();
            assert_eq!(tokens.len(), 2);
            assert_eq!(tokens[0], Token::Ident(ident.clone()));
            assert_eq!(tokens[1], Token::Eof);
        }
    }

    // Feature: flux-lexer, Property 4: Float Parsing Round-Trip
    // **Validates: Requirements 3.2**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn float_parsing_round_trip(
            integer_part in "[0-9]{1,10}",
            decimal_part in "[0-9]{1,10}",
        ) {
            let source = format!("{}.{}", integer_part, decimal_part);
            let expected: f64 = source.parse().unwrap();
            // Skip if overflow to infinity
            prop_assume!(!expected.is_infinite());

            let tokens = lex(&source).unwrap();
            assert_eq!(tokens.len(), 2);
            match &tokens[0] {
                Token::Float(v) => assert_eq!(*v, expected),
                other => panic!("Expected Float, got {:?}", other),
            }
            assert_eq!(tokens[1], Token::Eof);
        }
    }

    // Feature: flux-lexer, Property 5: String Literal Round-Trip
    // **Validates: Requirements 4.1, 4.2, 4.3, 4.4, 4.5**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn string_literal_round_trip_simple(
            content in "[^\"\\\\\\n]{0,100}"  // printable chars excluding ", \, newline
        ) {
            let source = format!("\"{}\"", content);
            let tokens = lex(&source).unwrap();
            assert_eq!(tokens.len(), 2);
            assert_eq!(tokens[0], Token::String(content.clone()));
            assert_eq!(tokens[1], Token::Eof);
        }

        #[test]
        fn string_literal_round_trip_with_escapes(
            parts in prop::collection::vec("[^\"\\\\\\n]{0,20}", 1..5),
            escapes in prop::collection::vec(prop::sample::select(vec!["\\n", "\\t", "\\\"", "\\\\"]), 1..5),
        ) {
            // Build a source string with interleaved parts and escapes
            let mut source = String::from("\"");
            let mut expected = String::new();
            for (i, part) in parts.iter().enumerate() {
                source.push_str(part);
                expected.push_str(part);
                if i < escapes.len() {
                    source.push_str(escapes[i]);
                    match escapes[i] {
                        "\\n" => expected.push('\n'),
                        "\\t" => expected.push('\t'),
                        "\\\"" => expected.push('"'),
                        "\\\\" => expected.push('\\'),
                        _ => unreachable!(),
                    }
                }
            }
            source.push('"');

            let tokens = lex(&source).unwrap();
            assert_eq!(tokens.len(), 2);
            assert_eq!(tokens[0], Token::String(expected));
            assert_eq!(tokens[1], Token::Eof);
        }
    }

    // Feature: flux-lexer, Property 3: Integer Parsing Round-Trip
    // **Validates: Requirements 3.1**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn integer_parsing_round_trip(value in 0i64..=i64::MAX) {
            let source = value.to_string();
            let tokens = lex(&source).unwrap();
            assert_eq!(tokens.len(), 2);
            assert_eq!(tokens[0], Token::Int(value));
            assert_eq!(tokens[1], Token::Eof);
        }
    }

    // Helper strategy for generating valid Flux token strings
    fn valid_flux_token() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("strategy".to_string()),
            Just("if".to_string()),
            Just("else".to_string()),
            Just("true".to_string()),
            Just("false".to_string()),
            Just("42".to_string()),
            Just("3.14".to_string()),
            "[a-zA-Z_][a-zA-Z0-9_]{0,10}".prop_map(|s| s),
            Just("+".to_string()),
            Just("-".to_string()),
            Just("==".to_string()),
            Just("!=".to_string()),
            Just("(".to_string()),
            Just(")".to_string()),
            Just("{".to_string()),
            Just("}".to_string()),
            Just(",".to_string()),
            Just(".".to_string()),
        ]
    }

    // Feature: flux-lexer, Property 7: Span Bounds Invariant
    // **Validates: Requirements 9.1, 9.2, 9.3, 14.2**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn span_bounds_invariant(
            tokens_source in prop::collection::vec(valid_flux_token(), 1..20)
        ) {
            let source = tokens_source.join(" ");
            if let Ok(spanned_tokens) = lex_with_spans(&source) {
                for st in &spanned_tokens {
                    prop_assert!(st.span.start <= st.span.end,
                        "Span start ({}) > end ({})", st.span.start, st.span.end);
                    prop_assert!(st.span.end <= source.len(),
                        "Span end ({}) > source len ({})", st.span.end, source.len());
                    // Verify UTF-8 boundary
                    prop_assert!(source.is_char_boundary(st.span.start),
                        "Span start ({}) not on char boundary", st.span.start);
                    prop_assert!(source.is_char_boundary(st.span.end),
                        "Span end ({}) not on char boundary", st.span.end);
                }
            }
        }
    }

    // Feature: flux-lexer, Property 9: Span Concatenation Round-Trip
    // **Validates: Requirements 14.1, 14.4**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn span_concatenation_round_trip(
            tokens_source in prop::collection::vec(valid_flux_token(), 1..20)
        ) {
            let source = tokens_source.join(" ");
            if let Ok(spanned_tokens) = lex_with_spans(&source) {
                // Concatenate source slices at each non-EOF token's span
                let concatenated: String = spanned_tokens.iter()
                    .filter(|st| st.token != Token::Eof)
                    .map(|st| &source[st.span.start..st.span.end])
                    .collect();

                // The expected result is the source with whitespace removed
                // (our valid_flux_token() generator uses space-separated tokens
                //  and doesn't generate comments, so just remove spaces)
                let source_no_whitespace: String = source.chars()
                    .filter(|c| !c.is_whitespace())
                    .collect();

                prop_assert_eq!(concatenated, source_no_whitespace,
                    "Span concatenation should equal source with whitespace stripped");
            }
        }
    }
}

#[cfg(test)]
mod invalid_escape_tests {
    use crate::lexer::lex;
    use proptest::prelude::*;

    // Feature: flux-lexer, Property 6: Invalid Escape Rejection
    // **Validates: Requirements 4.7**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn invalid_escape_rejection(
            ch in "[^nt\"\\\\\\n]"  // any single char except n, t, ", \, newline
        ) {
            let source = format!("\"\\{}\"", ch);
            let result = lex(&source);
            prop_assert!(result.is_err(), "Expected error for escape \\{}, got: {:?}", ch, result);
        }
    }
}

#[cfg(test)]
mod span_ordering_tests {
    use crate::lexer::{lex_with_spans, Token};
    use proptest::prelude::*;

    /// Generate random valid Flux token source strings
    fn valid_flux_token() -> impl Strategy<Value = String> {
        prop_oneof![
            // Keywords
            Just("strategy".to_string()),
            Just("params".to_string()),
            Just("state".to_string()),
            Just("on".to_string()),
            Just("if".to_string()),
            Just("elif".to_string()),
            Just("else".to_string()),
            Just("for".to_string()),
            Just("while".to_string()),
            Just("return".to_string()),
            Just("from".to_string()),
            Just("import".to_string()),
            Just("and".to_string()),
            Just("or".to_string()),
            Just("not".to_string()),
            Just("true".to_string()),
            Just("false".to_string()),
            Just("null".to_string()),
            // Identifiers
            "[a-zA-Z_][a-zA-Z0-9_]{0,9}".prop_map(|s| s),
            // Integers
            "[0-9]{1,8}".prop_map(|s| s),
            // Floats
            "[0-9]{1,5}\\.[0-9]{1,5}".prop_map(|s| s),
            // Strings (simple, no escapes)
            "[^\"\\\\\\n]{0,10}".prop_map(|s| format!("\"{}\"", s)),
            // Operators
            Just("+".to_string()),
            Just("-".to_string()),
            Just("*".to_string()),
            Just("/".to_string()),
            Just("%".to_string()),
            Just("==".to_string()),
            Just("!=".to_string()),
            Just("<".to_string()),
            Just("<=".to_string()),
            Just(">".to_string()),
            Just(">=".to_string()),
            Just("&&".to_string()),
            Just("||".to_string()),
            Just("!".to_string()),
            Just("=".to_string()),
            // Delimiters
            Just("(".to_string()),
            Just(")".to_string()),
            Just("{".to_string()),
            Just("}".to_string()),
            Just("[".to_string()),
            Just("]".to_string()),
            Just(",".to_string()),
            Just(".".to_string()),
            Just(":".to_string()),
        ]
    }

    // Feature: flux-lexer, Property 8: Span Ordering (Non-Overlapping)
    // **Validates: Requirements 14.3**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn span_ordering_non_overlapping(
            tokens_source in prop::collection::vec(valid_flux_token(), 1..20)
        ) {
            let source = tokens_source.join(" ");
            if let Ok(spanned_tokens) = lex_with_spans(&source) {
                let non_eof: Vec<_> = spanned_tokens.iter()
                    .filter(|st| st.token != Token::Eof)
                    .collect();

                for window in non_eof.windows(2) {
                    prop_assert!(window[1].span.start >= window[0].span.end,
                        "Token spans overlap: {:?} (end {}) followed by {:?} (start {})",
                        window[0].token, window[0].span.end,
                        window[1].token, window[1].span.start);
                }
            }
        }
    }
}


#[cfg(test)]
mod whitespace_independence_tests {
    use crate::lexer::lex;
    use proptest::prelude::*;

    /// Generate random valid Flux token source strings
    fn valid_flux_token() -> impl Strategy<Value = String> {
        prop_oneof![
            // Keywords
            Just("strategy".to_string()),
            Just("params".to_string()),
            Just("state".to_string()),
            Just("on".to_string()),
            Just("if".to_string()),
            Just("elif".to_string()),
            Just("else".to_string()),
            Just("for".to_string()),
            Just("while".to_string()),
            Just("return".to_string()),
            Just("from".to_string()),
            Just("import".to_string()),
            Just("and".to_string()),
            Just("or".to_string()),
            Just("not".to_string()),
            Just("true".to_string()),
            Just("false".to_string()),
            Just("null".to_string()),
            // Identifiers
            "[a-zA-Z_][a-zA-Z0-9_]{0,9}".prop_map(|s| s),
            // Integers
            "[0-9]{1,8}".prop_map(|s| s),
            // Floats
            "[0-9]{1,5}\\.[0-9]{1,5}".prop_map(|s| s),
            // Strings (simple, no escapes)
            "[^\"\\\\\\n]{0,10}".prop_map(|s| format!("\"{}\"", s)),
            // Operators
            Just("+".to_string()),
            Just("-".to_string()),
            Just("*".to_string()),
            Just("/".to_string()),
            Just("%".to_string()),
            Just("==".to_string()),
            Just("!=".to_string()),
            Just("<".to_string()),
            Just("<=".to_string()),
            Just(">".to_string()),
            Just(">=".to_string()),
            Just("&&".to_string()),
            Just("||".to_string()),
            Just("!".to_string()),
            Just("=".to_string()),
            // Delimiters
            Just("(".to_string()),
            Just(")".to_string()),
            Just("{".to_string()),
            Just("}".to_string()),
            Just("[".to_string()),
            Just("]".to_string()),
            Just(",".to_string()),
            Just(".".to_string()),
            Just(":".to_string()),
        ]
    }

    /// Generate random whitespace strings (1-5 chars from space, tab, newline, carriage return)
    fn whitespace_strategy() -> impl Strategy<Value = String> {
        prop::collection::vec(prop::sample::select(vec![' ', '\t', '\n', '\r']), 1..5)
            .prop_map(|chars| chars.into_iter().collect::<String>())
    }

    // Feature: flux-lexer, Property 10: Whitespace Independence
    // **Validates: Requirements 8.1, 8.2**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn whitespace_independence(
            tokens_source in prop::collection::vec(valid_flux_token(), 2..10),
            whitespace in prop::collection::vec(whitespace_strategy(), 1..9),
        ) {
            // Join tokens with single space (baseline)
            let baseline_source = tokens_source.join(" ");
            let baseline_result = lex(&baseline_source);

            // Join tokens with random whitespace
            let mut varied_source = String::new();
            for (i, token) in tokens_source.iter().enumerate() {
                if i > 0 {
                    let ws_idx = (i - 1) % whitespace.len();
                    varied_source.push_str(&whitespace[ws_idx]);
                }
                varied_source.push_str(token);
            }
            let varied_result = lex(&varied_source);

            // Both should produce the same token sequence (or both error)
            match (baseline_result, varied_result) {
                (Ok(baseline_tokens), Ok(varied_tokens)) => {
                    prop_assert_eq!(baseline_tokens, varied_tokens,
                        "Different whitespace produced different tokens");
                }
                (Err(_), Err(_)) => {} // both error, that's fine
                (Ok(_), Err(e)) => prop_assert!(false, "Baseline succeeded but varied failed: {}", e),
                (Err(e), Ok(_)) => prop_assert!(false, "Baseline failed but varied succeeded: {}", e),
            }
        }
    }
}


#[cfg(test)]
mod comment_transparency_tests {
    use crate::lexer::lex;
    use proptest::prelude::*;

    /// Generate random valid Flux token source strings
    fn valid_flux_token() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("strategy".to_string()),
            Just("if".to_string()),
            Just("else".to_string()),
            Just("true".to_string()),
            Just("42".to_string()),
            Just("3.14".to_string()),
            "[a-zA-Z_][a-zA-Z0-9_]{0,10}".prop_map(|s| s),
            Just("+".to_string()),
            Just("==".to_string()),
            Just("(".to_string()),
            Just(")".to_string()),
        ]
    }

    // Feature: flux-lexer, Property 11: Comment Transparency
    // **Validates: Requirements 7.1, 7.2**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn comment_transparency(
            tokens_source in prop::collection::vec(valid_flux_token(), 2..8),
            comment_text in "[a-zA-Z0-9 ]{1,30}",
        ) {
            // Baseline: tokens separated by spaces
            let baseline_source = tokens_source.join(" ");
            let baseline_result = lex(&baseline_source);

            // With comment: put tokens on a line, append a comment, then put more tokens on next line
            let mid = tokens_source.len() / 2;
            let first_half = tokens_source[..mid].join(" ");
            let second_half = tokens_source[mid..].join(" ");
            let commented_source = format!("{} # {}\n{}", first_half, comment_text, second_half);
            let commented_result = lex(&commented_source);

            // Both should produce same token sequence
            match (baseline_result, commented_result) {
                (Ok(baseline_tokens), Ok(commented_tokens)) => {
                    prop_assert_eq!(baseline_tokens, commented_tokens,
                        "Comment changed token sequence");
                }
                (Err(_), Err(_)) => {} // both error, fine
                (Ok(_), Err(e)) => prop_assert!(false, "Baseline succeeded but commented failed: {}", e),
                (Err(e), Ok(_)) => prop_assert!(false, "Baseline failed but commented succeeded: {}", e),
            }
        }
    }
}


#[cfg(test)]
mod error_recovery_tests {
    use crate::lexer::lex;
    use proptest::prelude::*;

    /// Generate random valid Flux token source strings
    fn valid_flux_token() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("strategy".to_string()),
            Just("if".to_string()),
            Just("true".to_string()),
            Just("42".to_string()),
            Just("3.14".to_string()),
            "[a-zA-Z_][a-zA-Z0-9_]{0,10}".prop_map(|s| s),
            Just("+".to_string()),
            Just("(".to_string()),
            Just(")".to_string()),
        ]
    }

    // Feature: flux-lexer, Property 13: Error Recovery Continuation
    // **Validates: Requirements 10.1, 10.2, 10.3, 10.4**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn error_recovery_continuation(
            tokens_before in prop::collection::vec(valid_flux_token(), 1..5),
            tokens_after in prop::collection::vec(valid_flux_token(), 1..5),
            invalid_char in prop::sample::select(vec!['$', '~', '`']),
        ) {
            let before = tokens_before.join(" ");
            let after = tokens_after.join(" ");
            // Insert invalid char between the two parts
            let source = format!("{} {} {}", before, invalid_char, after);

            // Calculate expected byte offset of invalid character
            let expected_offset = before.len() + 1; // +1 for the space before invalid char

            let result = lex(&source);
            prop_assert!(result.is_err(), "Expected error for source with '{}' at byte {}", invalid_char, expected_offset);

            let err_msg = result.unwrap_err().to_string();
            prop_assert!(err_msg.contains(&format!("byte {}", expected_offset)),
                "Error should mention byte offset {}. Got: {}", expected_offset, err_msg);
        }
    }
}


#[cfg(test)]
mod eof_termination_tests {
    use crate::lexer::{lex, Token};
    use proptest::prelude::*;

    /// Generate random valid Flux token source strings
    fn valid_flux_token() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("strategy".to_string()),
            Just("if".to_string()),
            Just("true".to_string()),
            Just("42".to_string()),
            Just("3.14".to_string()),
            "[a-zA-Z_][a-zA-Z0-9_]{0,10}".prop_map(|s| s),
            Just("+".to_string()),
            Just("==".to_string()),
            Just("(".to_string()),
            Just(")".to_string()),
            Just("{".to_string()),
            Just("}".to_string()),
        ]
    }

    // Feature: flux-lexer, Property 12: EOF Termination Invariant
    // **Validates: Requirements 11.1, 11.2, 12.2**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn eof_termination_invariant(
            tokens_source in prop::collection::vec(valid_flux_token(), 0..20)
        ) {
            let source = tokens_source.join(" ");
            if let Ok(tokens) = lex(&source) {
                // Last token must be Eof
                prop_assert!(!tokens.is_empty(), "Token stream should not be empty");
                prop_assert_eq!(tokens.last().unwrap(), &Token::Eof,
                    "Last token should be Eof");

                // No other token should be Eof
                let non_last = &tokens[..tokens.len() - 1];
                for (i, token) in non_last.iter().enumerate() {
                    prop_assert_ne!(token, &Token::Eof,
                        "Token at index {} should not be Eof", i);
                }
            }
        }
    }
}
