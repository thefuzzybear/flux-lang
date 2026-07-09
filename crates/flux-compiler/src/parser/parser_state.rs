//! Parser state: a cursor over the token stream with helper methods.

use crate::error::{CompileError, Result};
use crate::lexer::{Span, SpannedToken, Token};

use super::ast::{Decorator, DecoratorArg, TypeAnnotation};

/// Parser state: a cursor over the token stream with helper methods.
pub(crate) struct ParserState {
    tokens: Vec<SpannedToken>,
    pos: usize,
    /// When true, an identifier immediately followed by `{` is parsed as a
    /// plain `Ident` expression rather than the start of a struct literal.
    ///
    /// This disambiguates `if cond { ... }` / `while cond { ... }` / `for x
    /// in iterable { ... }` (where `{` opens the body block) from
    /// `StructName { field = value }` (where `{` opens a struct literal) —
    /// both share the same `Ident` `OpenBrace` token prefix. The flag is set
    /// while parsing a condition/iterable and lifted again inside any
    /// enclosing delimiter (`(...)`, `[...]`) where the following `{` can no
    /// longer be confused with a block.
    forbid_struct_literal: bool,
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
        Ok(Self {
            tokens,
            pos: 0,
            forbid_struct_literal: false,
        })
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

    /// Consume an integer literal token, returning (value, span).
    pub fn expect_int(&mut self) -> Result<(i64, Span)> {
        let current = self.peek_spanned().clone();
        match &current.token {
            Token::Int(value) => {
                let value = *value;
                let span = current.span;
                self.advance();
                Ok((value, span))
            }
            _ => Err(self.error_expected("integer literal")),
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

    /// Returns whether struct literal parsing is currently suppressed
    /// (see `forbid_struct_literal` field docs).
    pub fn struct_literal_forbidden(&self) -> bool {
        self.forbid_struct_literal
    }

    /// Run `f` with struct-literal parsing suppressed, restoring the
    /// previous setting afterward. Used when parsing `if`/`while`/`for`
    /// conditions and iterables, where a trailing `{` opens the statement
    /// body rather than a struct literal.
    pub fn with_struct_literal_forbidden<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let previous = self.forbid_struct_literal;
        self.forbid_struct_literal = true;
        let result = f(self);
        self.forbid_struct_literal = previous;
        result
    }

    /// Run `f` with struct-literal parsing allowed, restoring the previous
    /// setting afterward. Used inside enclosing delimiters (parens,
    /// brackets, argument lists) where a `{` can no longer be confused with
    /// a statement body.
    pub fn with_struct_literal_allowed<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let previous = self.forbid_struct_literal;
        self.forbid_struct_literal = false;
        let result = f(self);
        self.forbid_struct_literal = previous;
        result
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

    /// Parse zero or more decorator annotations: `@name` or `@name(IntLiteral)`.
    ///
    /// Decorators are returned in declaration order. This is used both for
    /// struct-level decorators (preceding `struct Name { ... }`) and
    /// field-level decorators (preceding a field within a struct body).
    pub fn parse_decorators(&mut self) -> Result<Vec<Decorator>> {
        let mut decorators = Vec::new();

        while self.check(&Token::At) {
            let start_span = self.current_span();
            self.advance(); // consume `@`

            let (name, _) = self.expect_ident()?;

            let arg = if self.check(&Token::OpenParen) {
                self.advance(); // consume `(`
                let (value, _) = self.expect_int()?;
                self.expect(&Token::CloseParen)?;
                Some(DecoratorArg::Int(value))
            } else {
                None
            };

            let span = self.span_from(start_span);
            decorators.push(Decorator { name, arg, span });
        }

        Ok(decorators)
    }

    /// Parse a type annotation used in struct fields and function signatures.
    ///
    /// Resolves:
    /// - `f64`, `int`, `bool`, `str` to their scalar `TypeAnnotation` variants
    /// - `int(N)` to `TypeAnnotation::BitInt(N)` (for `@bitfield` structs)
    /// - any other identifier to `TypeAnnotation::Named(String)`
    /// - `[Type; N]` to `TypeAnnotation::FixedArray(Box<Type>, N)`
    pub fn parse_type_annotation(&mut self) -> Result<TypeAnnotation> {
        match self.peek().clone() {
            Token::Ident(name) => {
                self.advance(); // consume identifier
                match name.as_str() {
                    "int" => {
                        // Check for `int(N)` bitfield syntax.
                        if self.check(&Token::OpenParen) {
                            self.advance(); // consume `(`
                            let (width, width_span) = self.expect_int()?;
                            if width <= 0 {
                                return Err(CompileError::Parser(format!(
                                    "at byte {}: int(N) bit width must be positive, got {}",
                                    width_span.start, width
                                )));
                            }
                            self.expect(&Token::CloseParen)?;
                            Ok(TypeAnnotation::BitInt(width as usize))
                        } else {
                            Ok(TypeAnnotation::Int)
                        }
                    }
                    "f64" => Ok(TypeAnnotation::F64),
                    "bool" => Ok(TypeAnnotation::Bool),
                    "str" => Ok(TypeAnnotation::Str),
                    _ => Ok(TypeAnnotation::Named(name)),
                }
            }
            Token::OpenBracket => {
                self.advance(); // consume `[`
                let element_type = self.parse_type_annotation()?;
                self.expect(&Token::Semicolon)?;
                let (size, size_span) = self.expect_int()?;
                if size <= 0 {
                    return Err(CompileError::Parser(format!(
                        "at byte {}: fixed array size must be positive, got {}",
                        size_span.start, size
                    )));
                }
                self.expect(&Token::CloseBracket)?;
                Ok(TypeAnnotation::FixedArray(Box::new(element_type), size as usize))
            }
            _ => Err(self.error_expected("type annotation")),
        }
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

    // --- parse_type_annotation ---

    #[test]
    fn parse_type_annotation_f64() {
        let tokens = vec![
            spanned(Token::Ident("f64".to_string()), 0, 3),
            spanned(Token::Eof, 3, 3),
        ];
        let mut state = ParserState::new(tokens).unwrap();
        let ty = state.parse_type_annotation().unwrap();
        assert_eq!(ty, TypeAnnotation::F64);
    }

    #[test]
    fn parse_type_annotation_int_scalar() {
        let tokens = vec![
            spanned(Token::Ident("int".to_string()), 0, 3),
            spanned(Token::Eof, 3, 3),
        ];
        let mut state = ParserState::new(tokens).unwrap();
        let ty = state.parse_type_annotation().unwrap();
        assert_eq!(ty, TypeAnnotation::Int);
    }

    #[test]
    fn parse_type_annotation_bool() {
        let tokens = vec![
            spanned(Token::Ident("bool".to_string()), 0, 4),
            spanned(Token::Eof, 4, 4),
        ];
        let mut state = ParserState::new(tokens).unwrap();
        let ty = state.parse_type_annotation().unwrap();
        assert_eq!(ty, TypeAnnotation::Bool);
    }

    #[test]
    fn parse_type_annotation_str() {
        let tokens = vec![
            spanned(Token::Ident("str".to_string()), 0, 3),
            spanned(Token::Eof, 3, 3),
        ];
        let mut state = ParserState::new(tokens).unwrap();
        let ty = state.parse_type_annotation().unwrap();
        assert_eq!(ty, TypeAnnotation::Str);
    }

    #[test]
    fn parse_type_annotation_named_struct() {
        let tokens = vec![
            spanned(Token::Ident("Quote".to_string()), 0, 5),
            spanned(Token::Eof, 5, 5),
        ];
        let mut state = ParserState::new(tokens).unwrap();
        let ty = state.parse_type_annotation().unwrap();
        assert_eq!(ty, TypeAnnotation::Named("Quote".to_string()));
    }

    #[test]
    fn parse_type_annotation_bitint() {
        // int(8)
        let tokens = vec![
            spanned(Token::Ident("int".to_string()), 0, 3),
            spanned(Token::OpenParen, 3, 4),
            spanned(Token::Int(8), 4, 5),
            spanned(Token::CloseParen, 5, 6),
            spanned(Token::Eof, 6, 6),
        ];
        let mut state = ParserState::new(tokens).unwrap();
        let ty = state.parse_type_annotation().unwrap();
        assert_eq!(ty, TypeAnnotation::BitInt(8));
    }

    #[test]
    fn parse_type_annotation_bitint_zero_width_errors() {
        // int(0)
        let tokens = vec![
            spanned(Token::Ident("int".to_string()), 0, 3),
            spanned(Token::OpenParen, 3, 4),
            spanned(Token::Int(0), 4, 5),
            spanned(Token::CloseParen, 5, 6),
            spanned(Token::Eof, 6, 6),
        ];
        let mut state = ParserState::new(tokens).unwrap();
        let result = state.parse_type_annotation();
        assert!(result.is_err());
    }

    #[test]
    fn parse_type_annotation_fixed_array() {
        // [f64; 20]
        let tokens = vec![
            spanned(Token::OpenBracket, 0, 1),
            spanned(Token::Ident("f64".to_string()), 1, 4),
            spanned(Token::Semicolon, 4, 5),
            spanned(Token::Int(20), 5, 7),
            spanned(Token::CloseBracket, 7, 8),
            spanned(Token::Eof, 8, 8),
        ];
        let mut state = ParserState::new(tokens).unwrap();
        let ty = state.parse_type_annotation().unwrap();
        assert_eq!(
            ty,
            TypeAnnotation::FixedArray(Box::new(TypeAnnotation::F64), 20)
        );
    }

    #[test]
    fn parse_type_annotation_nested_fixed_array_of_named_struct() {
        // [Level; 20]
        let tokens = vec![
            spanned(Token::OpenBracket, 0, 1),
            spanned(Token::Ident("Level".to_string()), 1, 6),
            spanned(Token::Semicolon, 6, 7),
            spanned(Token::Int(20), 7, 9),
            spanned(Token::CloseBracket, 9, 10),
            spanned(Token::Eof, 10, 10),
        ];
        let mut state = ParserState::new(tokens).unwrap();
        let ty = state.parse_type_annotation().unwrap();
        assert_eq!(
            ty,
            TypeAnnotation::FixedArray(Box::new(TypeAnnotation::Named("Level".to_string())), 20)
        );
    }

    #[test]
    fn parse_type_annotation_fixed_array_zero_size_errors() {
        // [f64; 0]
        let tokens = vec![
            spanned(Token::OpenBracket, 0, 1),
            spanned(Token::Ident("f64".to_string()), 1, 4),
            spanned(Token::Semicolon, 4, 5),
            spanned(Token::Int(0), 5, 6),
            spanned(Token::CloseBracket, 6, 7),
            spanned(Token::Eof, 7, 7),
        ];
        let mut state = ParserState::new(tokens).unwrap();
        let result = state.parse_type_annotation();
        assert!(result.is_err());
    }

    #[test]
    fn parse_type_annotation_invalid_token_errors() {
        let tokens = vec![
            spanned(Token::Int(5), 0, 1),
            spanned(Token::Eof, 1, 1),
        ];
        let mut state = ParserState::new(tokens).unwrap();
        let result = state.parse_type_annotation();
        assert!(result.is_err());
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

    // --- expect_int ---

    #[test]
    fn expect_int_returns_value_and_span_for_int_token() {
        let tokens = vec![
            spanned(Token::Int(64), 5, 7),
            spanned(Token::Eof, 7, 7),
        ];
        let mut state = ParserState::new(tokens).unwrap();
        let (value, span) = state.expect_int().unwrap();
        assert_eq!(value, 64);
        assert_eq!(span, Span::new(5, 7));
        assert_eq!(state.peek(), &Token::Eof);
    }

    #[test]
    fn expect_int_returns_error_for_non_int_token() {
        let tokens = vec![
            spanned(Token::Ident("foo".to_string()), 0, 3),
            spanned(Token::Eof, 3, 3),
        ];
        let mut state = ParserState::new(tokens).unwrap();
        let result = state.expect_int();
        assert!(result.is_err());
        match result.unwrap_err() {
            CompileError::Parser(msg) => {
                assert!(msg.contains("integer literal"));
            }
            other => panic!("Expected CompileError::Parser, got: {other:?}"),
        }
    }

    // --- parse_decorators ---

    #[test]
    fn parse_decorators_returns_empty_vec_when_no_at_token() {
        let tokens = vec![
            spanned(Token::Struct, 0, 6),
            spanned(Token::Eof, 6, 6),
        ];
        let mut state = ParserState::new(tokens).unwrap();
        let decorators = state.parse_decorators().unwrap();
        assert!(decorators.is_empty());
        // Cursor should not have moved
        assert_eq!(state.peek(), &Token::Struct);
    }

    #[test]
    fn parse_decorators_parses_single_decorator_without_arg() {
        // @packed
        let tokens = vec![
            spanned(Token::At, 0, 1),
            spanned(Token::Ident("packed".to_string()), 1, 7),
            spanned(Token::Eof, 7, 7),
        ];
        let mut state = ParserState::new(tokens).unwrap();
        let decorators = state.parse_decorators().unwrap();
        assert_eq!(decorators.len(), 1);
        assert_eq!(decorators[0].name, "packed");
        assert_eq!(decorators[0].arg, None);
    }

    #[test]
    fn parse_decorators_parses_single_decorator_with_int_arg() {
        // @aligned(64)
        let tokens = vec![
            spanned(Token::At, 0, 1),
            spanned(Token::Ident("aligned".to_string()), 1, 8),
            spanned(Token::OpenParen, 8, 9),
            spanned(Token::Int(64), 9, 11),
            spanned(Token::CloseParen, 11, 12),
            spanned(Token::Eof, 12, 12),
        ];
        let mut state = ParserState::new(tokens).unwrap();
        let decorators = state.parse_decorators().unwrap();
        assert_eq!(decorators.len(), 1);
        assert_eq!(decorators[0].name, "aligned");
        assert_eq!(decorators[0].arg, Some(DecoratorArg::Int(64)));
    }

    #[test]
    fn parse_decorators_preserves_declaration_order_for_multiple_decorators() {
        // @aligned(64) @packed @hot
        let tokens = vec![
            spanned(Token::At, 0, 1),
            spanned(Token::Ident("aligned".to_string()), 1, 8),
            spanned(Token::OpenParen, 8, 9),
            spanned(Token::Int(64), 9, 11),
            spanned(Token::CloseParen, 11, 12),
            spanned(Token::At, 13, 14),
            spanned(Token::Ident("packed".to_string()), 14, 20),
            spanned(Token::At, 21, 22),
            spanned(Token::Ident("hot".to_string()), 22, 25),
            spanned(Token::Eof, 25, 25),
        ];
        let mut state = ParserState::new(tokens).unwrap();
        let decorators = state.parse_decorators().unwrap();
        assert_eq!(decorators.len(), 3);
        assert_eq!(decorators[0].name, "aligned");
        assert_eq!(decorators[0].arg, Some(DecoratorArg::Int(64)));
        assert_eq!(decorators[1].name, "packed");
        assert_eq!(decorators[1].arg, None);
        assert_eq!(decorators[2].name, "hot");
        assert_eq!(decorators[2].arg, None);
    }

    #[test]
    fn parse_decorators_stops_at_non_at_token() {
        // @stack struct Foo { ... } -- decorators should stop before `struct`
        let tokens = vec![
            spanned(Token::At, 0, 1),
            spanned(Token::Ident("stack".to_string()), 1, 6),
            spanned(Token::Struct, 7, 13),
            spanned(Token::Eof, 13, 13),
        ];
        let mut state = ParserState::new(tokens).unwrap();
        let decorators = state.parse_decorators().unwrap();
        assert_eq!(decorators.len(), 1);
        assert_eq!(decorators[0].name, "stack");
        assert_eq!(state.peek(), &Token::Struct);
    }

    #[test]
    fn parse_decorators_errors_on_missing_ident_after_at() {
        let tokens = vec![
            spanned(Token::At, 0, 1),
            spanned(Token::Int(5), 1, 2),
            spanned(Token::Eof, 2, 2),
        ];
        let mut state = ParserState::new(tokens).unwrap();
        let result = state.parse_decorators();
        assert!(result.is_err());
    }

    #[test]
    fn parse_decorators_errors_on_non_int_arg() {
        // @aligned(foo) -- arg must be an int literal
        let tokens = vec![
            spanned(Token::At, 0, 1),
            spanned(Token::Ident("aligned".to_string()), 1, 8),
            spanned(Token::OpenParen, 8, 9),
            spanned(Token::Ident("foo".to_string()), 9, 12),
            spanned(Token::CloseParen, 12, 13),
            spanned(Token::Eof, 13, 13),
        ];
        let mut state = ParserState::new(tokens).unwrap();
        let result = state.parse_decorators();
        assert!(result.is_err());
    }

    #[test]
    fn parse_decorators_errors_on_missing_close_paren() {
        // @aligned(64 -- missing closing paren
        let tokens = vec![
            spanned(Token::At, 0, 1),
            spanned(Token::Ident("aligned".to_string()), 1, 8),
            spanned(Token::OpenParen, 8, 9),
            spanned(Token::Int(64), 9, 11),
            spanned(Token::Eof, 11, 11),
        ];
        let mut state = ParserState::new(tokens).unwrap();
        let result = state.parse_decorators();
        assert!(result.is_err());
    }
}
