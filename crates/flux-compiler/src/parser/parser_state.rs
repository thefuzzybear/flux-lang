//! Parser state: a cursor over the token stream with helper methods.

use crate::error::{CompileError, Result};
use crate::lexer::{Span, SpannedToken, Token};

/// Parser state: a cursor over the token stream with helper methods.
pub(crate) struct ParserState {
    tokens: Vec<SpannedToken>,
    pos: usize,
}

impl ParserState {
    /// Create a new parser from a token stream.
    /// Returns an error if the token stream is empty (no tokens at all).
    pub fn new(tokens: Vec<SpannedToken>) -> Result<Self> {
        if tokens.is_empty() {
            return Err(CompileError::Parser(
                "unexpected end of input: expected at least one token".to_string(),
            ));
        }
        Ok(Self { tokens, pos: 0 })
    }

    /// Peek at the current token without consuming it.
    pub fn peek(&self) -> &Token {
        &self.tokens[self.pos].token
    }

    /// Peek at the current SpannedToken without consuming it.
    pub fn peek_spanned(&self) -> &SpannedToken {
        &self.tokens[self.pos]
    }

    /// Get the span of the current token.
    pub fn current_span(&self) -> Span {
        self.tokens[self.pos].span
    }

    /// Advance the cursor by one token, returning the consumed SpannedToken.
    pub fn advance(&mut self) -> &SpannedToken {
        let tok = &self.tokens[self.pos];
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        tok
    }

    /// Consume the current token if it matches the expected kind.
    /// Returns the span of the consumed token on success.
    pub fn expect(&mut self, expected: &Token) -> Result<Span> {
        let current = self.peek_spanned();
        if std::mem::discriminant(&current.token) == std::mem::discriminant(expected) {
            let span = current.span;
            self.advance();
            Ok(span)
        } else {
            Err(self.error_expected(&format!("{:?}", expected)))
        }
    }

    /// Consume an identifier token, returning (name, span).
    pub fn expect_ident(&mut self) -> Result<(String, Span)> {
        let current = self.peek_spanned().clone();
        match &current.token {
            Token::Ident(name) => {
                let name = name.clone();
                let span = current.span;
                self.advance();
                Ok((name, span))
            }
            _ => Err(self.error_expected("identifier")),
        }
    }

    /// Consume a string literal token, returning (value, span).
    pub fn expect_string(&mut self) -> Result<(String, Span)> {
        let current = self.peek_spanned().clone();
        match &current.token {
            Token::String(value) => {
                let value = value.clone();
                let span = current.span;
                self.advance();
                Ok((value, span))
            }
            _ => Err(self.error_expected("string literal")),
        }
    }

    /// Check if the current token matches (by discriminant) without consuming.
    pub fn check(&self, expected: &Token) -> bool {
        std::mem::discriminant(self.peek()) == std::mem::discriminant(expected)
    }

    /// Check if at end of input.
    pub fn at_eof(&self) -> bool {
        matches!(self.peek(), Token::Eof)
    }

    /// Create a parser error with byte offset and expectation.
    pub fn error_expected(&self, expected: &str) -> CompileError {
        let span = self.current_span();
        let found = format!("{:?}", self.peek());
        CompileError::Parser(format!(
            "at byte {}: expected {}, found {}",
            span.start, expected, found
        ))
    }

    /// Create a span that covers from `start` to the end of the previous token.
    pub fn span_from(&self, start: Span) -> Span {
        let end = if self.pos > 0 {
            self.tokens[self.pos - 1].span.end
        } else {
            start.end
        };
        Span::new(start.start, end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spanned(token: Token, start: usize, end: usize) -> SpannedToken {
        SpannedToken { token, span: Span::new(start, end) }
    }

    // --- ParserState::new ---

    #[test]
    fn new_with_empty_vec_returns_error() {
        let result = ParserState::new(vec![]);
        assert!(result.is_err());
        match result {
            Err(CompileError::Parser(msg)) => {
                assert!(msg.contains("unexpected end of input"));
            }
            Err(other) => panic!("Expected CompileError::Parser, got: {other:?}"),
            Ok(_) => panic!("Expected error, got Ok"),
        }
    }

    // --- peek and advance ---

    #[test]
    fn peek_returns_current_token_without_advancing() {
        let tokens = vec![
            spanned(Token::OpenBrace, 0, 1),
            spanned(Token::CloseBrace, 1, 2),
            spanned(Token::Eof, 2, 2),
        ];
        let state = ParserState::new(tokens).unwrap();
        assert_eq!(state.peek(), &Token::OpenBrace);
        // Peek again to ensure cursor didn't move
        assert_eq!(state.peek(), &Token::OpenBrace);
    }

    #[test]
    fn advance_moves_cursor_and_returns_consumed_token() {
        let tokens = vec![
            spanned(Token::OpenBrace, 0, 1),
            spanned(Token::CloseBrace, 1, 2),
            spanned(Token::Eof, 2, 2),
        ];
        let mut state = ParserState::new(tokens).unwrap();
        let consumed = state.advance();
        assert_eq!(consumed.token, Token::OpenBrace);
        assert_eq!(state.peek(), &Token::CloseBrace);
    }

    #[test]
    fn advance_on_eof_does_not_move_past_last_token() {
        let tokens = vec![spanned(Token::Eof, 5, 5)];
        let mut state = ParserState::new(tokens).unwrap();
        // Advance on the only (Eof) token should not panic or move past it
        let consumed = state.advance();
        assert_eq!(consumed.token, Token::Eof);
        // Still at Eof
        assert_eq!(state.peek(), &Token::Eof);
    }

    // --- expect ---

    #[test]
    fn expect_succeeds_when_current_token_matches() {
        let tokens = vec![
            spanned(Token::OpenBrace, 0, 1),
            spanned(Token::Eof, 1, 1),
        ];
        let mut state = ParserState::new(tokens).unwrap();
        let span = state.expect(&Token::OpenBrace).unwrap();
        assert_eq!(span, Span::new(0, 1));
        // Cursor should have advanced past the OpenBrace
        assert_eq!(state.peek(), &Token::Eof);
    }

    #[test]
    fn expect_fails_when_current_token_does_not_match() {
        let tokens = vec![
            spanned(Token::CloseBrace, 3, 4),
            spanned(Token::Eof, 4, 4),
        ];
        let mut state = ParserState::new(tokens).unwrap();
        let result = state.expect(&Token::OpenBrace);
        assert!(result.is_err());
        match result.unwrap_err() {
            CompileError::Parser(msg) => {
                assert!(msg.contains("expected"));
                assert!(msg.contains("OpenBrace"));
            }
            other => panic!("Expected CompileError::Parser, got: {other:?}"),
        }
    }

    // --- expect_ident ---

    #[test]
    fn expect_ident_returns_name_and_span_for_ident_token() {
        let tokens = vec![
            spanned(Token::Ident("foo".to_string()), 10, 13),
            spanned(Token::Eof, 13, 13),
        ];
        let mut state = ParserState::new(tokens).unwrap();
        let (name, span) = state.expect_ident().unwrap();
        assert_eq!(name, "foo");
        assert_eq!(span, Span::new(10, 13));
        assert_eq!(state.peek(), &Token::Eof);
    }

    #[test]
    fn expect_ident_returns_error_for_non_ident_token() {
        let tokens = vec![
            spanned(Token::Int(42), 0, 2),
            spanned(Token::Eof, 2, 2),
        ];
        let mut state = ParserState::new(tokens).unwrap();
        let result = state.expect_ident();
        assert!(result.is_err());
        match result.unwrap_err() {
            CompileError::Parser(msg) => {
                assert!(msg.contains("identifier"));
            }
            other => panic!("Expected CompileError::Parser, got: {other:?}"),
        }
    }

    // --- error_expected ---

    #[test]
    fn error_expected_message_includes_byte_offset() {
        let tokens = vec![
            spanned(Token::Plus, 7, 8),
            spanned(Token::Eof, 8, 8),
        ];
        let state = ParserState::new(tokens).unwrap();
        let err = state.error_expected("expression");
        match err {
            CompileError::Parser(msg) => {
                assert!(msg.contains("at byte 7"), "Expected 'at byte 7' in: {msg}");
                assert!(msg.contains("expected expression"));
                assert!(msg.contains("Plus"));
            }
            other => panic!("Expected CompileError::Parser, got: {other:?}"),
        }
    }

    // --- check ---

    #[test]
    fn check_returns_true_without_consuming() {
        let tokens = vec![
            spanned(Token::OpenParen, 0, 1),
            spanned(Token::Eof, 1, 1),
        ];
        let state = ParserState::new(tokens).unwrap();
        assert!(state.check(&Token::OpenParen));
        // Cursor should not have moved
        assert_eq!(state.peek(), &Token::OpenParen);
    }

    #[test]
    fn check_returns_false_when_no_match() {
        let tokens = vec![
            spanned(Token::OpenParen, 0, 1),
            spanned(Token::Eof, 1, 1),
        ];
        let state = ParserState::new(tokens).unwrap();
        assert!(!state.check(&Token::CloseParen));
    }

    // --- at_eof ---

    #[test]
    fn at_eof_returns_true_when_at_eof_token() {
        let tokens = vec![spanned(Token::Eof, 10, 10)];
        let state = ParserState::new(tokens).unwrap();
        assert!(state.at_eof());
    }

    #[test]
    fn at_eof_returns_false_when_not_at_eof() {
        let tokens = vec![
            spanned(Token::Strategy, 0, 8),
            spanned(Token::Eof, 8, 8),
        ];
        let state = ParserState::new(tokens).unwrap();
        assert!(!state.at_eof());
    }

    // --- span_from ---

    #[test]
    fn span_from_computes_correct_composite_span() {
        let tokens = vec![
            spanned(Token::OpenBrace, 0, 1),
            spanned(Token::Ident("x".to_string()), 2, 3),
            spanned(Token::CloseBrace, 4, 5),
            spanned(Token::Eof, 5, 5),
        ];
        let mut state = ParserState::new(tokens).unwrap();
        let start_span = state.current_span(); // Span(0, 1)
        state.advance(); // consume OpenBrace, cursor at Ident
        state.advance(); // consume Ident, cursor at CloseBrace

        // span_from(start) should cover from start.start to previous token's end
        let composite = state.span_from(start_span);
        assert_eq!(composite, Span::new(0, 3));
    }

    #[test]
    fn span_from_at_position_zero_uses_start_end() {
        let tokens = vec![
            spanned(Token::Eof, 0, 0),
        ];
        let state = ParserState::new(tokens).unwrap();
        let start_span = Span::new(0, 5);
        // pos is 0, so span_from uses start.end as end
        let composite = state.span_from(start_span);
        assert_eq!(composite, Span::new(0, 5));
    }
}
