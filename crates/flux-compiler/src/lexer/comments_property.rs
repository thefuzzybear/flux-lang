//! Property-based tests for the Comment Extractor
//!
//! Feature: flux-fmt-syntax-highlight, Property 8: Comment Preservation
//!
//! Validates that `extract_comments` correctly identifies and extracts all
//! comments from Flux source code, preserving their text content, line numbers,
//! and column positions.

#[cfg(test)]
mod tests {
    use crate::lexer::comments::extract_comments;
    use proptest::prelude::*;

    // =========================================================================
    // Generator strategies for building source strings with comments
    // =========================================================================

    /// Generate a comment text (content after `#`, no newlines)
    fn comment_text() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9 _.,!?:;'+=/-]{0,40}".prop_map(|s| s)
    }

    /// Generate a simple code line (an assignment like `x = 1`)
    fn code_line() -> impl Strategy<Value = String> {
        ("[a-z][a-z0-9_]{0,8}", 0i32..1000).prop_map(|(name, val)| format!("{} = {}", name, val))
    }

    /// A line type: code-only, comment-only, code with trailing comment, or empty
    #[derive(Debug, Clone)]
    enum LineKind {
        CodeOnly(String),
        CommentOnly(String),
        CodeWithTrailing(String, String),
        Empty,
        StringWithHash(String),
    }

    /// Generate a source line of various kinds
    fn source_line() -> impl Strategy<Value = LineKind> {
        prop_oneof![
            3 => code_line().prop_map(LineKind::CodeOnly),
            3 => comment_text().prop_map(|t| LineKind::CommentOnly(t)),
            3 => (code_line(), comment_text())
                .prop_map(|(code, comment)| LineKind::CodeWithTrailing(code, comment)),
            1 => Just(LineKind::Empty),
            2 => "[a-zA-Z0-9 .,!?]{1,20}".prop_map(|content| LineKind::StringWithHash(content)),
        ]
    }

    /// Build source text from a Vec of LineKinds and return the expected comments
    fn build_source(lines: &[LineKind]) -> (String, Vec<ExpectedComment>) {
        let mut source = String::new();
        let mut expected_comments = Vec::new();
        let mut current_line: usize = 1;

        for line_kind in lines {
            match line_kind {
                LineKind::CodeOnly(code) => {
                    source.push_str(code);
                    source.push('\n');
                }
                LineKind::CommentOnly(text) => {
                    let comment_text = format!("# {}", text);
                    let column = 0;
                    expected_comments.push(ExpectedComment {
                        text: comment_text.clone(),
                        line: current_line,
                        column,
                        is_trailing: false,
                    });
                    source.push_str(&comment_text);
                    source.push('\n');
                }
                LineKind::CodeWithTrailing(code, text) => {
                    let comment_text = format!("# {}", text);
                    let column = code.len() + 1; // space before #
                    expected_comments.push(ExpectedComment {
                        text: comment_text.clone(),
                        line: current_line,
                        column,
                        is_trailing: true,
                    });
                    source.push_str(code);
                    source.push(' ');
                    source.push_str(&comment_text);
                    source.push('\n');
                }
                LineKind::Empty => {
                    source.push('\n');
                }
                LineKind::StringWithHash(content) => {
                    // A string literal containing a #, should NOT be extracted
                    let line = format!("msg = \"hello {} # world\"", content);
                    source.push_str(&line);
                    source.push('\n');
                }
            }
            current_line += 1;
        }

        (source, expected_comments)
    }

    #[derive(Debug, Clone)]
    struct ExpectedComment {
        text: String,
        line: usize,
        column: usize,
        is_trailing: bool,
    }

    // =========================================================================
    // Property 8: Comment Preservation
    // =========================================================================

    // Feature: flux-fmt-syntax-highlight, Property 8: Comment Preservation
    // **Validates: Requirements 3.6**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// For any source string containing comments at various positions,
        /// extract_comments returns a comment for each `#` outside string literals,
        /// with preserved text content, correct line numbers, and correct columns.
        #[test]
        fn comment_preservation(
            lines in prop::collection::vec(source_line(), 1..15)
        ) {
            let (source, expected) = build_source(&lines);
            let actual = extract_comments(&source);

            // 1. Every comment from the input appears in extract_comments output
            prop_assert_eq!(
                actual.len(),
                expected.len(),
                "Expected {} comments but got {}.\nSource:\n{}\nActual comments: {:?}",
                expected.len(),
                actual.len(),
                source,
                actual
            );

            // 2. Each returned comment's text field contains the exact text from # to end-of-line
            for (i, (act, exp)) in actual.iter().zip(expected.iter()).enumerate() {
                prop_assert_eq!(
                    &act.text, &exp.text,
                    "Comment {} text mismatch.\nExpected: {:?}\nActual: {:?}\nSource:\n{}",
                    i, exp.text, act.text, source
                );

                // 3. Line numbers match the actual line position in source
                prop_assert_eq!(
                    act.line, exp.line,
                    "Comment {} line mismatch. Expected line {}, got {}.\nComment: {:?}\nSource:\n{}",
                    i, exp.line, act.line, act, source
                );

                // 4. Column values match the position of # within its line
                prop_assert_eq!(
                    act.column, exp.column,
                    "Comment {} column mismatch. Expected col {}, got {}.\nComment: {:?}\nSource:\n{}",
                    i, exp.column, act.column, act, source
                );

                // 5. Trailing flag is correctly set
                prop_assert_eq!(
                    act.is_trailing, exp.is_trailing,
                    "Comment {} is_trailing mismatch. Expected {}, got {}.\nComment: {:?}\nSource:\n{}",
                    i, exp.is_trailing, act.is_trailing, act, source
                );
            }
        }

        /// Comments appear in sequential line order
        #[test]
        fn comment_line_ordering(
            lines in prop::collection::vec(source_line(), 2..15)
        ) {
            let (source, _) = build_source(&lines);
            let comments = extract_comments(&source);

            // Comments should be in source order (non-decreasing line numbers)
            for window in comments.windows(2) {
                prop_assert!(
                    window[0].line <= window[1].line,
                    "Comments not in line order: line {} followed by line {}.\nSource:\n{}",
                    window[0].line, window[1].line, source
                );
            }
        }

        /// Hash characters inside string literals are not extracted as comments
        #[test]
        fn hash_in_string_not_extracted(
            content_before in "[a-zA-Z0-9 ]{0,10}",
            content_after in "[a-zA-Z0-9 ]{0,10}",
        ) {
            // Build a source with only a string containing #
            let source = format!("msg = \"{}#{}\"", content_before, content_after);
            let comments = extract_comments(&source);
            prop_assert!(
                comments.is_empty(),
                "Hash inside string should not produce comments.\nSource: {:?}\nGot: {:?}",
                source, comments
            );
        }

        /// Every comment text starts with '#'
        #[test]
        fn comment_text_starts_with_hash(
            lines in prop::collection::vec(source_line(), 1..10)
        ) {
            let (source, _) = build_source(&lines);
            let comments = extract_comments(&source);

            for (i, comment) in comments.iter().enumerate() {
                prop_assert!(
                    comment.text.starts_with('#'),
                    "Comment {} text does not start with '#': {:?}\nSource:\n{}",
                    i, comment.text, source
                );
            }
        }
    }
}
