//! Comment Extractor
//!
//! Extracts comments from Flux source code as a pre-pass before formatting.
//! The Logos lexer skips comments (they are not in the token stream), so the
//! formatter needs this separate extraction to preserve comments in output.

/// A comment extracted from source with its position.
#[derive(Debug, Clone, PartialEq)]
pub struct Comment {
    /// The comment text including the `#` prefix
    pub text: String,
    /// Byte offset of the `#` character in source
    pub start: usize,
    /// Line number (1-based) where the comment appears
    pub line: usize,
    /// Column offset (0-based) of the `#` in its line
    pub column: usize,
    /// Whether this is a trailing comment (code precedes it on the same line)
    pub is_trailing: bool,
}

/// How a comment relates to the code around it.
#[derive(Debug, Clone, PartialEq)]
pub enum CommentPlacement {
    /// Comment on its own line, above the next code line
    Above { next_code_line: usize },
    /// Comment trailing code on the same line
    Trailing { code_line: usize },
    /// Comment at end of file with no following code
    EndOfFile,
}

/// Extract all comments from source code.
///
/// Scans for `#` characters outside of string literals and captures the
/// rest of the line (including the `#`) as comment text.
///
/// # Arguments
///
/// * `source` - Flux source code
///
/// # Returns
///
/// A vector of `Comment` structs in source order.
pub fn extract_comments(source: &str) -> Vec<Comment> {
    let mut comments = Vec::new();
    let mut in_string = false;
    let mut line: usize = 1;
    let mut line_start_offset: usize = 0;

    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let b = bytes[i];

        match b {
            b'\n' => {
                in_string = false; // Strings can't span lines in Flux
                line += 1;
                line_start_offset = i + 1;
                i += 1;
            }
            b'"' if !in_string => {
                // Enter a string literal — skip to closing quote
                in_string = true;
                i += 1;
                while i < len {
                    match bytes[i] {
                        b'\\' => {
                            // Skip escaped character
                            i += 2;
                        }
                        b'"' => {
                            in_string = false;
                            i += 1;
                            break;
                        }
                        b'\n' => {
                            // Unterminated string — newline ends it
                            in_string = false;
                            break;
                        }
                        _ => {
                            i += 1;
                        }
                    }
                }
            }
            b'#' if !in_string => {
                let start = i;
                let column = i - line_start_offset;

                // Capture from `#` to end of line
                let end = memchr_newline(bytes, i);
                let text = &source[start..end];

                // Determine if trailing: check if any non-whitespace exists
                // before the `#` on the same line
                let is_trailing = has_code_before(source, line_start_offset, start);

                comments.push(Comment {
                    text: text.to_string(),
                    start,
                    line,
                    column,
                    is_trailing,
                });

                i = end;
            }
            _ => {
                i += 1;
            }
        }
    }

    comments
}

/// Find the end of the current line (position of `\n` or end of source).
fn memchr_newline(bytes: &[u8], from: usize) -> usize {
    let mut i = from;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            return i;
        }
        i += 1;
    }
    bytes.len()
}

/// Check if there is any non-whitespace content between `line_start` and
/// `comment_start` (exclusive), which would make this a trailing comment.
fn has_code_before(source: &str, line_start: usize, comment_start: usize) -> bool {
    source[line_start..comment_start]
        .chars()
        .any(|c| !c.is_whitespace())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_single_line_comment() {
        let source = "# this is a comment\n";
        let comments = extract_comments(source);
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].text, "# this is a comment");
        assert_eq!(comments[0].start, 0);
        assert_eq!(comments[0].line, 1);
        assert_eq!(comments[0].column, 0);
        assert!(!comments[0].is_trailing);
    }

    #[test]
    fn extract_trailing_comment() {
        let source = "x = 42 # the answer\n";
        let comments = extract_comments(source);
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].text, "# the answer");
        assert_eq!(comments[0].start, 7);
        assert_eq!(comments[0].line, 1);
        assert_eq!(comments[0].column, 7);
        assert!(comments[0].is_trailing);
    }

    #[test]
    fn hash_inside_string_not_extracted() {
        let source = r#"msg = "hello # world""#;
        let comments = extract_comments(source);
        assert!(comments.is_empty());
    }

    #[test]
    fn multiple_comments() {
        let source = "# first\nx = 1\n# second\ny = 2 # inline\n";
        let comments = extract_comments(source);
        assert_eq!(comments.len(), 3);

        assert_eq!(comments[0].text, "# first");
        assert_eq!(comments[0].line, 1);
        assert!(!comments[0].is_trailing);

        assert_eq!(comments[1].text, "# second");
        assert_eq!(comments[1].line, 3);
        assert!(!comments[1].is_trailing);

        assert_eq!(comments[2].text, "# inline");
        assert_eq!(comments[2].line, 4);
        assert!(comments[2].is_trailing);
    }

    #[test]
    fn empty_source() {
        let comments = extract_comments("");
        assert!(comments.is_empty());
    }

    #[test]
    fn comment_at_end_without_newline() {
        let source = "x = 1\n# end";
        let comments = extract_comments(source);
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].text, "# end");
        assert_eq!(comments[0].line, 2);
        assert_eq!(comments[0].column, 0);
        assert!(!comments[0].is_trailing);
    }

    #[test]
    fn escaped_quote_in_string_does_not_end_string() {
        let source = r#"msg = "say \"hi\" # not comment""#;
        let comments = extract_comments(source);
        assert!(comments.is_empty());
    }

    #[test]
    fn comment_after_string_with_hash() {
        let source = "msg = \"hello\" # real comment\n";
        let comments = extract_comments(source);
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].text, "# real comment");
        assert!(comments[0].is_trailing);
    }

    #[test]
    fn indented_comment() {
        let source = "    # indented comment\n    x = 1\n";
        let comments = extract_comments(source);
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].column, 4);
        assert!(!comments[0].is_trailing);
    }

    #[test]
    fn comment_placement_above() {
        let source = "# above\nx = 1\n";
        let comments = extract_comments(source);
        assert_eq!(comments.len(), 1);
        assert!(!comments[0].is_trailing);
        // The comment is above the next code line
    }

    #[test]
    fn source_with_only_whitespace_before_comment() {
        let source = "   \t  # just whitespace before\n";
        let comments = extract_comments(source);
        assert_eq!(comments.len(), 1);
        assert!(!comments[0].is_trailing);
    }
}
