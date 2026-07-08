//! Lexer - Tokenizes Flux source code
//!
//! The lexer transforms raw source code into a stream of tokens.
//!
//! # Example
//!
//! ```
//! use flux_compiler::lexer::lex;
//!
//! let source = "strategy Simple { }";
//! let tokens = lex(source);
//! // tokens contains: [Strategy, Ident("Simple"), OpenBrace, CloseBrace]
//! ```

pub mod comments;
pub(crate) mod logos_token;
pub mod span;
pub mod token;

pub use comments::{Comment, CommentPlacement, extract_comments};
pub use span::{Span, SpannedToken};
pub use token::Token;

use crate::error::{CompileError, Result};
use logos::Logos;
use logos_token::LogosToken;

/// Internal lexer error type used during tokenization.
/// Not exposed in the public API — errors are accumulated and formatted
/// into a single `CompileError::Lexer(String)` message before returning.
#[derive(Debug)]
enum LexError {
    UnexpectedChar { ch: char, offset: usize },
    UnterminatedString { span: Span },
    InvalidEscape { character: char, span: Span },
    IntegerOverflow { span: Span },
    FloatOverflow { span: Span },
    IdentifierTooLong { span: Span, length: usize },
}

/// Parse a string literal, processing escape sequences.
///
/// Input: the raw source slice including surrounding quotes (e.g., `"hello\nworld"`)
/// Returns: the processed string content or a LexError
fn parse_string_literal(raw: &str, span: Span) -> std::result::Result<String, LexError> {
    let inner = &raw[1..raw.len() - 1]; // Strip quotes
    let mut result = String::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some('"') => result.push('"'),
                Some('\\') => result.push('\\'),
                Some(other) => {
                    return Err(LexError::InvalidEscape {
                        character: other,
                        span,
                    });
                }
                None => {
                    return Err(LexError::UnterminatedString { span });
                }
            }
        } else if ch == '\n' {
            return Err(LexError::UnterminatedString { span });
        } else {
            result.push(ch);
        }
    }
    Ok(result)
}

/// Format all accumulated lexer errors into a human-readable string.
///
/// The `_source` parameter enables future enhancements such as
/// line/column display; it is currently unused.
fn format_errors(errors: &[LexError], _source: &str) -> String {
    errors
        .iter()
        .map(|err| match err {
            LexError::UnexpectedChar { ch, offset } => {
                format!("Lexer error at byte {offset}: unexpected character '{ch}'")
            }
            LexError::UnterminatedString { span } => {
                format!(
                    "Lexer error at byte {}: unterminated string literal",
                    span.start
                )
            }
            LexError::InvalidEscape { character, span } => {
                format!(
                    "Lexer error at byte {}: unrecognized escape sequence '\\{character}'",
                    span.start
                )
            }
            LexError::IntegerOverflow { span } => {
                format!(
                    "Lexer error at byte {}: integer literal overflows i64",
                    span.start
                )
            }
            LexError::FloatOverflow { span } => {
                format!(
                    "Lexer error at byte {}: float literal overflows f64",
                    span.start
                )
            }
            LexError::IdentifierTooLong { span, length } => {
                format!(
                    "Lexer error at byte {}: identifier exceeds maximum length ({})",
                    span.start, length
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Convert a LogosToken into a Token, handling value parsing for literals.
fn convert_token(
    logos_token: LogosToken,
    slice: &str,
    span: Span,
) -> std::result::Result<Token, LexError> {
    match logos_token {
        // Keywords
        LogosToken::Strategy => Ok(Token::Strategy),
        LogosToken::Params => Ok(Token::Params),
        LogosToken::State => Ok(Token::State),
        LogosToken::On => Ok(Token::On),
        LogosToken::If => Ok(Token::If),
        LogosToken::Elif => Ok(Token::Elif),
        LogosToken::Else => Ok(Token::Else),
        LogosToken::For => Ok(Token::For),
        LogosToken::While => Ok(Token::While),
        LogosToken::Return => Ok(Token::Return),
        LogosToken::Fn => Ok(Token::Fn),
        LogosToken::From => Ok(Token::From),
        LogosToken::Import => Ok(Token::Import),
        LogosToken::And => Ok(Token::And),
        LogosToken::Or => Ok(Token::Or),
        LogosToken::Not => Ok(Token::Not),
        LogosToken::True => Ok(Token::True),
        LogosToken::False => Ok(Token::False),
        LogosToken::Null => Ok(Token::Null),
        LogosToken::Data => Ok(Token::Data),
        LogosToken::Connector => Ok(Token::Connector),

        // Operators
        LogosToken::Eq => Ok(Token::Eq),
        LogosToken::Ne => Ok(Token::Ne),
        LogosToken::Le => Ok(Token::Le),
        LogosToken::Ge => Ok(Token::Ge),
        LogosToken::AndAnd => Ok(Token::AndAnd),
        LogosToken::OrOr => Ok(Token::OrOr),
        LogosToken::Plus => Ok(Token::Plus),
        LogosToken::Minus => Ok(Token::Minus),
        LogosToken::Star => Ok(Token::Star),
        LogosToken::Slash => Ok(Token::Slash),
        LogosToken::Percent => Ok(Token::Percent),
        LogosToken::Lt => Ok(Token::Lt),
        LogosToken::Gt => Ok(Token::Gt),
        LogosToken::Bang => Ok(Token::Bang),
        LogosToken::Assign => Ok(Token::Assign),

        // Delimiters
        LogosToken::OpenParen => Ok(Token::OpenParen),
        LogosToken::CloseParen => Ok(Token::CloseParen),
        LogosToken::OpenBrace => Ok(Token::OpenBrace),
        LogosToken::CloseBrace => Ok(Token::CloseBrace),
        LogosToken::OpenBracket => Ok(Token::OpenBracket),
        LogosToken::CloseBracket => Ok(Token::CloseBracket),
        LogosToken::Comma => Ok(Token::Comma),
        LogosToken::Dot => Ok(Token::Dot),
        LogosToken::Colon => Ok(Token::Colon),

        // Identifiers
        LogosToken::Ident => {
            if slice.len() > 255 {
                Err(LexError::IdentifierTooLong {
                    span,
                    length: slice.len(),
                })
            } else {
                Ok(Token::Ident(slice.to_string()))
            }
        }

        // Integer literals
        LogosToken::Int => {
            let value = slice.parse::<i64>().map_err(|_| LexError::IntegerOverflow { span })?;
            Ok(Token::Int(value))
        }

        // Float literals
        LogosToken::Float => {
            let value = slice.parse::<f64>().map_err(|_| LexError::FloatOverflow { span })?;
            if value.is_infinite() {
                return Err(LexError::FloatOverflow { span });
            }
            Ok(Token::Float(value))
        }

        // String literals
        LogosToken::StringLiteral => {
            let value = parse_string_literal(slice, span)?;
            Ok(Token::String(value))
        }
    }
}

/// Lex Flux source code into spanned tokens (tokens paired with source locations)
///
/// # Arguments
///
/// * `source` - Flux source code
///
/// # Returns
///
/// Vector of `SpannedToken`s, ending with `Token::Eof`
///
/// # Errors
///
/// Returns `CompileError::Lexer` if source contains invalid tokens
pub fn lex_with_spans(source: &str) -> Result<Vec<SpannedToken>> {
    let mut logos_lexer = LogosToken::lexer(source);
    let mut tokens: Vec<SpannedToken> = Vec::new();
    let mut errors: Vec<LexError> = Vec::new();

    while let Some(result) = logos_lexer.next() {
        let span = Span::new(logos_lexer.span().start, logos_lexer.span().end);
        let slice = logos_lexer.slice();

        match result {
            Ok(logos_token) => match convert_token(logos_token, slice, span) {
                Ok(token) => tokens.push(SpannedToken { token, span }),
                Err(err) => errors.push(err),
            },
            Err(()) => {
                errors.push(LexError::UnexpectedChar {
                    ch: source[span.start..span.end].chars().next().unwrap_or('\0'),
                    offset: span.start,
                });
                if errors.len() >= 100 {
                    break;
                }
            }
        }
    }

    if !errors.is_empty() {
        return Err(CompileError::Lexer(format_errors(&errors, source)));
    }

    let eof_span = Span::new(source.len(), source.len());
    tokens.push(SpannedToken {
        token: Token::Eof,
        span: eof_span,
    });
    Ok(tokens)
}

/// Lex Flux source code into tokens (without span information)
///
/// # Arguments
///
/// * `source` - Flux source code
///
/// # Returns
///
/// Vector of tokens
///
/// # Errors
///
/// Returns `CompileError::Lexer` if source contains invalid tokens
pub fn lex(source: &str) -> Result<Vec<Token>> {
    let spanned = lex_with_spans(source)?;
    Ok(spanned.into_iter().map(|st| st.token).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_string_simple() {
        let span = Span::new(0, 7);
        let result = parse_string_literal("\"hello\"", span).unwrap();
        assert_eq!(result, "hello");
    }

    #[test]
    fn parse_string_empty() {
        let span = Span::new(0, 2);
        let result = parse_string_literal("\"\"", span).unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn parse_string_escape_newline() {
        let span = Span::new(0, 6);
        let result = parse_string_literal("\"a\\nb\"", span).unwrap();
        assert_eq!(result, "a\nb");
    }

    #[test]
    fn parse_string_escape_tab() {
        let span = Span::new(0, 6);
        let result = parse_string_literal("\"a\\tb\"", span).unwrap();
        assert_eq!(result, "a\tb");
    }

    #[test]
    fn parse_string_escape_quote() {
        let span = Span::new(0, 6);
        let result = parse_string_literal("\"a\\\"b\"", span).unwrap();
        assert_eq!(result, "a\"b");
    }

    #[test]
    fn parse_string_escape_backslash() {
        let span = Span::new(0, 6);
        let result = parse_string_literal("\"a\\\\b\"", span).unwrap();
        assert_eq!(result, "a\\b");
    }

    #[test]
    fn parse_string_invalid_escape() {
        let span = Span::new(0, 6);
        let result = parse_string_literal("\"a\\qb\"", span);
        assert!(matches!(
            result,
            Err(LexError::InvalidEscape { character: 'q', .. })
        ));
    }

    #[test]
    fn parse_string_literal_newline_unterminated() {
        let span = Span::new(0, 5);
        let result = parse_string_literal("\"a\nb\"", span);
        assert!(matches!(result, Err(LexError::UnterminatedString { .. })));
    }

    #[test]
    fn parse_string_trailing_backslash() {
        let span = Span::new(0, 4);
        let result = parse_string_literal("\"a\\\"", span);
        assert!(matches!(result, Err(LexError::UnterminatedString { .. })));
    }

    #[test]
    fn format_errors_single_unexpected_char() {
        let errors = vec![LexError::UnexpectedChar { ch: '@', offset: 15 }];
        let output = format_errors(&errors, "");
        assert_eq!(output, "Lexer error at byte 15: unexpected character '@'");
    }

    #[test]
    fn format_errors_multiple() {
        let errors = vec![
            LexError::UnexpectedChar { ch: '@', offset: 15 },
            LexError::UnterminatedString {
                span: Span::new(23, 30),
            },
            LexError::InvalidEscape {
                character: 'q',
                span: Span::new(45, 50),
            },
            LexError::IntegerOverflow {
                span: Span::new(67, 90),
            },
        ];
        let output = format_errors(&errors, "");
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], "Lexer error at byte 15: unexpected character '@'");
        assert_eq!(lines[1], "Lexer error at byte 23: unterminated string literal");
        assert_eq!(
            lines[2],
            "Lexer error at byte 45: unrecognized escape sequence '\\q'"
        );
        assert_eq!(lines[3], "Lexer error at byte 67: integer literal overflows i64");
    }

    #[test]
    fn format_errors_float_overflow() {
        let errors = vec![LexError::FloatOverflow {
            span: Span::new(10, 20),
        }];
        let output = format_errors(&errors, "");
        assert_eq!(output, "Lexer error at byte 10: float literal overflows f64");
    }

    #[test]
    fn format_errors_identifier_too_long() {
        let errors = vec![LexError::IdentifierTooLong {
            span: Span::new(5, 270),
            length: 265,
        }];
        let output = format_errors(&errors, "");
        assert_eq!(
            output,
            "Lexer error at byte 5: identifier exceeds maximum length (265)"
        );
    }

    // ===== Public API (lex) unit tests =====
    // Validates: Requirements 1.1–1.18, 2.1, 3.1, 3.2, 4.1–4.5, 5.1–5.16, 6.1–6.9, 7.1–7.4, 8.1–8.4, 11.1–11.4, 12.1–12.4

    #[test]
    fn lex_empty_input() {
        let tokens = lex("").unwrap();
        assert_eq!(tokens, vec![Token::Eof]);
    }

    // --- Keyword tests ---

    #[test]
    fn lex_keyword_strategy() {
        let tokens = lex("strategy").unwrap();
        assert_eq!(tokens, vec![Token::Strategy, Token::Eof]);
    }

    #[test]
    fn lex_keyword_params() {
        let tokens = lex("params").unwrap();
        assert_eq!(tokens, vec![Token::Params, Token::Eof]);
    }

    #[test]
    fn lex_keyword_state() {
        let tokens = lex("state").unwrap();
        assert_eq!(tokens, vec![Token::State, Token::Eof]);
    }

    #[test]
    fn lex_keyword_on() {
        let tokens = lex("on").unwrap();
        assert_eq!(tokens, vec![Token::On, Token::Eof]);
    }

    #[test]
    fn lex_keyword_if() {
        let tokens = lex("if").unwrap();
        assert_eq!(tokens, vec![Token::If, Token::Eof]);
    }

    #[test]
    fn lex_keyword_elif() {
        let tokens = lex("elif").unwrap();
        assert_eq!(tokens, vec![Token::Elif, Token::Eof]);
    }

    #[test]
    fn lex_keyword_else() {
        let tokens = lex("else").unwrap();
        assert_eq!(tokens, vec![Token::Else, Token::Eof]);
    }

    #[test]
    fn lex_keyword_for() {
        let tokens = lex("for").unwrap();
        assert_eq!(tokens, vec![Token::For, Token::Eof]);
    }

    #[test]
    fn lex_keyword_while() {
        let tokens = lex("while").unwrap();
        assert_eq!(tokens, vec![Token::While, Token::Eof]);
    }

    #[test]
    fn lex_keyword_return() {
        let tokens = lex("return").unwrap();
        assert_eq!(tokens, vec![Token::Return, Token::Eof]);
    }

    #[test]
    fn lex_keyword_from() {
        let tokens = lex("from").unwrap();
        assert_eq!(tokens, vec![Token::From, Token::Eof]);
    }

    #[test]
    fn lex_keyword_import() {
        let tokens = lex("import").unwrap();
        assert_eq!(tokens, vec![Token::Import, Token::Eof]);
    }

    #[test]
    fn lex_keyword_and() {
        let tokens = lex("and").unwrap();
        assert_eq!(tokens, vec![Token::And, Token::Eof]);
    }

    #[test]
    fn lex_keyword_or() {
        let tokens = lex("or").unwrap();
        assert_eq!(tokens, vec![Token::Or, Token::Eof]);
    }

    #[test]
    fn lex_keyword_not() {
        let tokens = lex("not").unwrap();
        assert_eq!(tokens, vec![Token::Not, Token::Eof]);
    }

    #[test]
    fn lex_keyword_true() {
        let tokens = lex("true").unwrap();
        assert_eq!(tokens, vec![Token::True, Token::Eof]);
    }

    #[test]
    fn lex_keyword_false() {
        let tokens = lex("false").unwrap();
        assert_eq!(tokens, vec![Token::False, Token::Eof]);
    }

    #[test]
    fn lex_keyword_null() {
        let tokens = lex("null").unwrap();
        assert_eq!(tokens, vec![Token::Null, Token::Eof]);
    }

    // --- Identifier tests ---

    #[test]
    fn lex_identifier_simple() {
        let tokens = lex("foo").unwrap();
        assert_eq!(tokens, vec![Token::Ident("foo".to_string()), Token::Eof]);
    }

    #[test]
    fn lex_identifier_with_underscore() {
        let tokens = lex("my_var").unwrap();
        assert_eq!(tokens, vec![Token::Ident("my_var".to_string()), Token::Eof]);
    }

    #[test]
    fn lex_identifier_starts_with_underscore() {
        let tokens = lex("_private").unwrap();
        assert_eq!(tokens, vec![Token::Ident("_private".to_string()), Token::Eof]);
    }

    #[test]
    fn lex_identifier_with_digits() {
        let tokens = lex("x123").unwrap();
        assert_eq!(tokens, vec![Token::Ident("x123".to_string()), Token::Eof]);
    }

    #[test]
    fn lex_identifier_preserves_case() {
        let tokens = lex("CamelCase").unwrap();
        assert_eq!(tokens, vec![Token::Ident("CamelCase".to_string()), Token::Eof]);
    }

    // --- Integer tests ---

    #[test]
    fn lex_integer_42() {
        let tokens = lex("42").unwrap();
        assert_eq!(tokens, vec![Token::Int(42), Token::Eof]);
    }

    #[test]
    fn lex_integer_leading_zeros() {
        let tokens = lex("007").unwrap();
        assert_eq!(tokens, vec![Token::Int(7), Token::Eof]);
    }

    #[test]
    fn lex_integer_zero() {
        let tokens = lex("0").unwrap();
        assert_eq!(tokens, vec![Token::Int(0), Token::Eof]);
    }

    // --- Float tests ---

    #[test]
    fn lex_float_3_14() {
        let tokens = lex("3.14").unwrap();
        assert_eq!(tokens, vec![Token::Float(3.14), Token::Eof]);
    }

    #[test]
    fn lex_float_0_5() {
        let tokens = lex("0.5").unwrap();
        assert_eq!(tokens, vec![Token::Float(0.5), Token::Eof]);
    }

    #[test]
    fn lex_float_2_0() {
        let tokens = lex("2.0").unwrap();
        assert_eq!(tokens, vec![Token::Float(2.0), Token::Eof]);
    }

    // --- String literal tests ---

    #[test]
    fn lex_string_hello() {
        let tokens = lex(r#""hello""#).unwrap();
        assert_eq!(tokens, vec![Token::String("hello".to_string()), Token::Eof]);
    }

    #[test]
    fn lex_string_empty() {
        let tokens = lex(r#""""#).unwrap();
        assert_eq!(tokens, vec![Token::String("".to_string()), Token::Eof]);
    }

    #[test]
    fn lex_string_escape_newline_via_lex() {
        let tokens = lex(r#""a\nb""#).unwrap();
        assert_eq!(tokens, vec![Token::String("a\nb".to_string()), Token::Eof]);
    }

    #[test]
    fn lex_string_escape_tab_via_lex() {
        let tokens = lex(r#""a\tb""#).unwrap();
        assert_eq!(tokens, vec![Token::String("a\tb".to_string()), Token::Eof]);
    }

    #[test]
    fn lex_string_escape_quote_via_lex() {
        let tokens = lex(r#""a\"b""#).unwrap();
        assert_eq!(tokens, vec![Token::String("a\"b".to_string()), Token::Eof]);
    }

    #[test]
    fn lex_string_escape_backslash_via_lex() {
        let tokens = lex(r#""a\\b""#).unwrap();
        assert_eq!(tokens, vec![Token::String("a\\b".to_string()), Token::Eof]);
    }

    // --- Operator tests (single-char) ---

    #[test]
    fn lex_operator_plus() {
        let tokens = lex("+").unwrap();
        assert_eq!(tokens, vec![Token::Plus, Token::Eof]);
    }

    #[test]
    fn lex_operator_minus() {
        let tokens = lex("-").unwrap();
        assert_eq!(tokens, vec![Token::Minus, Token::Eof]);
    }

    #[test]
    fn lex_operator_star() {
        let tokens = lex("*").unwrap();
        assert_eq!(tokens, vec![Token::Star, Token::Eof]);
    }

    #[test]
    fn lex_operator_slash() {
        let tokens = lex("/").unwrap();
        assert_eq!(tokens, vec![Token::Slash, Token::Eof]);
    }

    #[test]
    fn lex_operator_percent() {
        let tokens = lex("%").unwrap();
        assert_eq!(tokens, vec![Token::Percent, Token::Eof]);
    }

    #[test]
    fn lex_operator_lt() {
        let tokens = lex("<").unwrap();
        assert_eq!(tokens, vec![Token::Lt, Token::Eof]);
    }

    #[test]
    fn lex_operator_gt() {
        let tokens = lex(">").unwrap();
        assert_eq!(tokens, vec![Token::Gt, Token::Eof]);
    }

    #[test]
    fn lex_operator_bang() {
        let tokens = lex("!").unwrap();
        assert_eq!(tokens, vec![Token::Bang, Token::Eof]);
    }

    #[test]
    fn lex_operator_assign() {
        let tokens = lex("=").unwrap();
        assert_eq!(tokens, vec![Token::Assign, Token::Eof]);
    }

    // --- Operator tests (multi-char) ---

    #[test]
    fn lex_operator_eq() {
        let tokens = lex("==").unwrap();
        assert_eq!(tokens, vec![Token::Eq, Token::Eof]);
    }

    #[test]
    fn lex_operator_ne() {
        let tokens = lex("!=").unwrap();
        assert_eq!(tokens, vec![Token::Ne, Token::Eof]);
    }

    #[test]
    fn lex_operator_le() {
        let tokens = lex("<=").unwrap();
        assert_eq!(tokens, vec![Token::Le, Token::Eof]);
    }

    #[test]
    fn lex_operator_ge() {
        let tokens = lex(">=").unwrap();
        assert_eq!(tokens, vec![Token::Ge, Token::Eof]);
    }

    #[test]
    fn lex_operator_and_and() {
        let tokens = lex("&&").unwrap();
        assert_eq!(tokens, vec![Token::AndAnd, Token::Eof]);
    }

    #[test]
    fn lex_operator_or_or() {
        let tokens = lex("||").unwrap();
        assert_eq!(tokens, vec![Token::OrOr, Token::Eof]);
    }

    // --- Delimiter tests ---

    #[test]
    fn lex_delimiter_open_paren() {
        let tokens = lex("(").unwrap();
        assert_eq!(tokens, vec![Token::OpenParen, Token::Eof]);
    }

    #[test]
    fn lex_delimiter_close_paren() {
        let tokens = lex(")").unwrap();
        assert_eq!(tokens, vec![Token::CloseParen, Token::Eof]);
    }

    #[test]
    fn lex_delimiter_open_brace() {
        let tokens = lex("{").unwrap();
        assert_eq!(tokens, vec![Token::OpenBrace, Token::Eof]);
    }

    #[test]
    fn lex_delimiter_close_brace() {
        let tokens = lex("}").unwrap();
        assert_eq!(tokens, vec![Token::CloseBrace, Token::Eof]);
    }

    #[test]
    fn lex_delimiter_open_bracket() {
        let tokens = lex("[").unwrap();
        assert_eq!(tokens, vec![Token::OpenBracket, Token::Eof]);
    }

    #[test]
    fn lex_delimiter_close_bracket() {
        let tokens = lex("]").unwrap();
        assert_eq!(tokens, vec![Token::CloseBracket, Token::Eof]);
    }

    #[test]
    fn lex_delimiter_comma() {
        let tokens = lex(",").unwrap();
        assert_eq!(tokens, vec![Token::Comma, Token::Eof]);
    }

    #[test]
    fn lex_delimiter_dot() {
        let tokens = lex(".").unwrap();
        assert_eq!(tokens, vec![Token::Dot, Token::Eof]);
    }

    #[test]
    fn lex_delimiter_colon() {
        let tokens = lex(":").unwrap();
        assert_eq!(tokens, vec![Token::Colon, Token::Eof]);
    }

    // --- Comment handling tests ---

    #[test]
    fn lex_comment_skipped() {
        let tokens = lex("strategy # comment\nparams").unwrap();
        assert_eq!(tokens, vec![Token::Strategy, Token::Params, Token::Eof]);
    }

    #[test]
    fn lex_comment_only() {
        let tokens = lex("# this is a comment").unwrap();
        assert_eq!(tokens, vec![Token::Eof]);
    }

    #[test]
    fn lex_comment_at_end_of_input() {
        let tokens = lex("true # trailing").unwrap();
        assert_eq!(tokens, vec![Token::True, Token::Eof]);
    }

    // --- Whitespace handling tests ---

    #[test]
    fn lex_whitespace_spaces_between_tokens() {
        let tokens = lex("if   else").unwrap();
        assert_eq!(tokens, vec![Token::If, Token::Else, Token::Eof]);
    }

    #[test]
    fn lex_whitespace_tabs_and_newlines() {
        let tokens = lex("for\n\twhile").unwrap();
        assert_eq!(tokens, vec![Token::For, Token::While, Token::Eof]);
    }

    #[test]
    fn lex_whitespace_only() {
        let tokens = lex("   \t\n\r  ").unwrap();
        assert_eq!(tokens, vec![Token::Eof]);
    }

    #[test]
    fn lex_tokens_without_whitespace() {
        let tokens = lex("a+b").unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Ident("a".to_string()),
                Token::Plus,
                Token::Ident("b".to_string()),
                Token::Eof,
            ]
        );
    }

    // Edge case tests for numeric literals and operators
    // Validates: Requirements 3.3, 3.4, 5.17, 5.18, 13.7, 13.8

    #[test]
    fn dot_followed_by_digits_produces_dot_and_int() {
        // `.5` should produce [Dot, Int(5), Eof] — not Float
        // The float regex `[0-9]+\.[0-9]+` requires leading digits
        let tokens = lex(".5").unwrap();
        assert_eq!(tokens, vec![Token::Dot, Token::Int(5), Token::Eof]);
    }

    #[test]
    fn digits_followed_by_dot_produces_int_and_dot() {
        // `3.` should produce [Int(3), Dot, Eof] — not Float
        // The float regex `[0-9]+\.[0-9]+` requires trailing digits
        let tokens = lex("3.").unwrap();
        assert_eq!(tokens, vec![Token::Int(3), Token::Dot, Token::Eof]);
    }

    #[test]
    fn triple_equals_produces_eq_and_assign() {
        // `===` should produce [Eq, Assign, Eof] via maximal munch
        // `==` is consumed first, leaving `=` as Assign
        let tokens = lex("===").unwrap();
        assert_eq!(tokens, vec![Token::Eq, Token::Assign, Token::Eof]);
    }

    #[test]
    fn not_equals_equals_produces_ne_and_assign() {
        // `!==` should produce [Ne, Assign, Eof] via maximal munch
        // `!=` is consumed first, leaving `=` as Assign
        let tokens = lex("!==").unwrap();
        assert_eq!(tokens, vec![Token::Ne, Token::Assign, Token::Eof]);
    }

    #[test]
    fn lone_ampersand_produces_unexpected_char_error() {
        // A single `&` (not followed by another `&`) should produce an error
        // since logos only has `&&` as a token, not `&`
        let result = lex("&");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("unexpected character"),
            "Expected 'unexpected character' in error, got: {err_msg}"
        );
    }

    #[test]
    fn lone_pipe_produces_unexpected_char_error() {
        // A single `|` (not followed by another `|`) should produce an error
        // since logos only has `||` as a token, not `|`
        let result = lex("|");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("unexpected character"),
            "Expected 'unexpected character' in error, got: {err_msg}"
        );
    }

    // --- Identifier-keyword disambiguation tests (Task 7.2) ---
    // Validates: Requirements 1.19, 1.20, 2.1, 2.2, 2.4

    #[test]
    fn ident_keyword_prefix_with_suffix() {
        // "strategy_name" should be a single Ident, not Strategy + _name
        let tokens = lex("strategy_name").unwrap();
        assert_eq!(
            tokens,
            vec![Token::Ident("strategy_name".to_string()), Token::Eof]
        );
    }

    #[test]
    fn ident_keyword_with_digit_suffix() {
        // "import2" should be a single Ident, not Import + Int(2)
        let tokens = lex("import2").unwrap();
        assert_eq!(
            tokens,
            vec![Token::Ident("import2".to_string()), Token::Eof]
        );
    }

    #[test]
    fn ident_uppercase_keyword() {
        // "Strategy" (uppercase S) should be Ident, not the keyword
        let tokens = lex("Strategy").unwrap();
        assert_eq!(
            tokens,
            vec![Token::Ident("Strategy".to_string()), Token::Eof]
        );
    }

    #[test]
    fn ident_all_uppercase_keyword() {
        // "TRUE" should be Ident, not the True keyword
        let tokens = lex("TRUE").unwrap();
        assert_eq!(
            tokens,
            vec![Token::Ident("TRUE".to_string()), Token::Eof]
        );
    }

    #[test]
    fn ident_at_max_length() {
        // Identifier at exactly 255 chars should succeed
        let long_ident = "a".repeat(255);
        let tokens = lex(&long_ident).unwrap();
        assert_eq!(
            tokens,
            vec![Token::Ident(long_ident), Token::Eof]
        );
    }

    #[test]
    fn ident_exceeds_max_length() {
        // Identifier at 256 chars should produce an error
        let too_long = "a".repeat(256);
        let result = lex(&too_long);
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            CompileError::Lexer(msg) => {
                assert!(
                    msg.contains("identifier exceeds maximum length"),
                    "Expected 'identifier exceeds maximum length' in error, got: {msg}"
                );
            }
            _ => panic!("Expected CompileError::Lexer, got: {err:?}"),
        }
    }

    // --- String error handling tests (Task 7.3) ---
    // Validates: Requirements 4.6, 4.7, 4.8, 6.10, 7.4

    #[test]
    fn lex_unterminated_string_no_closing_quote() {
        // Unterminated string: logos regex won't match without closing quote,
        // producing unexpected character errors for `"` and subsequent chars.
        let result = lex(r#""hello"#);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("unexpected character"),
            "Expected 'unexpected character' in error, got: {err_msg}"
        );
    }

    #[test]
    fn lex_string_with_literal_newline() {
        // Literal newline inside a string: logos regex matches (since \n is
        // not `"` or `\`), but parse_string_literal rejects it as unterminated.
        let result = lex("\"hello\nworld\"");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("unterminated string literal"),
            "Expected 'unterminated string literal' in error, got: {err_msg}"
        );
    }

    #[test]
    fn lex_string_invalid_escape_sequence() {
        // Invalid escape \q: logos regex matches (\\. matches \q),
        // but parse_string_literal rejects the unrecognized escape.
        let result = lex(r#""\q""#);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("unrecognized escape sequence"),
            "Expected 'unrecognized escape sequence' in error, got: {err_msg}"
        );
    }

    #[test]
    fn lex_hash_inside_string_not_comment() {
        // # inside a string is part of the string value, not a comment start.
        let result = lex(r#""hello # world""#).unwrap();
        assert_eq!(
            result,
            vec![Token::String("hello # world".to_string()), Token::Eof]
        );
    }

    #[test]
    fn lex_delimiters_inside_string_no_delimiter_tokens() {
        // Delimiters inside a string produce a single String token,
        // not individual delimiter tokens.
        let result = lex(r#""({[]})""#).unwrap();
        assert_eq!(
            result,
            vec![Token::String("({[]})".to_string()), Token::Eof]
        );
    }

    // --- Error accumulation and recovery tests (Task 7.4) ---
    // Validates: Requirements 10.1, 10.2, 10.3, 10.4, 3.5, 3.6, 9.5

    #[test]
    fn error_multiple_invalid_chars_each_produce_errors() {
        // Multiple invalid characters should each produce their own error
        let result = lex("@ $");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("byte 0: unexpected character '@'"),
            "Expected '@' error at byte 0, got: {err_msg}"
        );
        assert!(
            err_msg.contains("byte 2: unexpected character '$'"),
            "Expected '$' error at byte 2, got: {err_msg}"
        );
    }

    #[test]
    fn error_recovery_continues_scanning_after_invalid_chars() {
        // The lexer should report the error at byte 6 (position of @)
        // even though valid tokens surround it
        let result = lex("valid @ identifier");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("byte 6"),
            "Expected error mentioning byte 6, got: {err_msg}"
        );
    }

    #[test]
    fn error_count_capped_at_100() {
        // Create input with 101 invalid '@' characters
        let input = "@".repeat(101);
        let result = lex(&input);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        let line_count = err_msg.lines().count();
        assert_eq!(
            line_count, 100,
            "Expected exactly 100 error lines (capped), got: {line_count}"
        );
    }

    #[test]
    fn error_integer_overflow_produces_appropriate_error() {
        // A number exceeding i64 range should produce an overflow error
        let result = lex("99999999999999999999");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("integer literal overflows i64"),
            "Expected integer overflow error, got: {err_msg}"
        );
    }

    #[test]
    fn error_messages_include_byte_offsets() {
        // Integer overflow error should include byte 0 as the offset
        let result = lex("99999999999999999999");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("byte 0"),
            "Expected error to mention byte 0, got: {err_msg}"
        );
    }

    // --- Data keyword tests (Task 1.1) ---
    // Validates: Requirements 1.1, 9.1

    #[test]
    fn lex_keyword_data() {
        let tokens = lex("data").unwrap();
        assert_eq!(tokens, vec![Token::Data, Token::Eof]);
    }

    #[test]
    fn lex_data_source_as_ident() {
        // "data_source" should be a single Ident, not Data + _source
        let tokens = lex("data_source").unwrap();
        assert_eq!(
            tokens,
            vec![Token::Ident("data_source".to_string()), Token::Eof]
        );
    }

    #[test]
    fn lex_database_as_ident() {
        // "database" should be a single Ident, not Data + base
        let tokens = lex("database").unwrap();
        assert_eq!(
            tokens,
            vec![Token::Ident("database".to_string()), Token::Eof]
        );
    }

    // --- fn keyword tests (Task 1.2) ---
    // Validates: Requirements 1.1, 1.2, 1.3

    #[test]
    fn lex_keyword_fn_alone() {
        // "fn" alone should produce Token::Fn
        let tokens = lex("fn").unwrap();
        assert_eq!(tokens, vec![Token::Fn, Token::Eof]);
    }

    #[test]
    fn lex_fn_alone_correct_span() {
        // "fn" should have span [0, 2)
        let spanned = lex_with_spans("fn").unwrap();
        assert_eq!(spanned[0].token, Token::Fn);
        assert_eq!(spanned[0].span, Span::new(0, 2));
    }

    #[test]
    fn lex_fname_produces_ident() {
        // "fname" should be a single Ident, not Fn + Ident("ame")
        let tokens = lex("fname").unwrap();
        assert_eq!(
            tokens,
            vec![Token::Ident("fname".to_string()), Token::Eof]
        );
    }

    #[test]
    fn lex_fn_helper_produces_ident() {
        // "fn_helper" should be a single Ident, not Fn + Ident("_helper")
        let tokens = lex("fn_helper").unwrap();
        assert_eq!(
            tokens,
            vec![Token::Ident("fn_helper".to_string()), Token::Eof]
        );
    }

    #[test]
    fn lex_fn_followed_by_open_paren() {
        // "fn(" should produce Token::Fn followed by Token::OpenParen
        let tokens = lex("fn(").unwrap();
        assert_eq!(tokens, vec![Token::Fn, Token::OpenParen, Token::Eof]);
    }

    #[test]
    fn lex_fn_span_exactly_2_bytes() {
        // Fn token span should be exactly 2 bytes wide regardless of position
        let spanned = lex_with_spans("  fn").unwrap();
        let fn_token = &spanned[0];
        assert_eq!(fn_token.token, Token::Fn);
        assert_eq!(fn_token.span.len(), 2);
        assert_eq!(fn_token.span, Span::new(2, 4));
    }

    // --- connector keyword tests ---
    // Validates: Requirements 8.1

    #[test]
    fn lex_keyword_connector() {
        let tokens = lex("connector").unwrap();
        assert_eq!(tokens, vec![Token::Connector, Token::Eof]);
    }

    #[test]
    fn lex_connectors_as_ident() {
        // "connectors" should be a single Ident, not Connector + s
        let tokens = lex("connectors").unwrap();
        assert_eq!(
            tokens,
            vec![Token::Ident("connectors".to_string()), Token::Eof]
        );
    }

    #[test]
    fn lex_connector_type_as_ident() {
        // "connector_type" should be a single Ident, not Connector + _type
        let tokens = lex("connector_type").unwrap();
        assert_eq!(
            tokens,
            vec![Token::Ident("connector_type".to_string()), Token::Eof]
        );
    }
}

#[cfg(test)]
mod tests_property;

#[cfg(test)]
mod tests_data_keyword_property;

#[cfg(test)]
mod comments_property;
