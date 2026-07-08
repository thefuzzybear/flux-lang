//! Unit tests for `fn` token lexing
//!
//! **Validates: Requirements 1.1, 1.2, 1.3**
//!
//! Tests that the lexer correctly produces Token::Fn for the exact keyword "fn",
//! produces Token::Ident for identifiers that start with "fn" but continue with
//! alphanumeric/underscore characters, and preserves correct span information.

use flux_compiler::lexer::{lex, lex_with_spans, Span, Token};

/// Test `fn` alone produces `Token::Fn` with correct span
#[test]
fn fn_alone_produces_fn_token() {
    let tokens = lex("fn").unwrap();
    assert_eq!(tokens, vec![Token::Fn, Token::Eof]);
}

/// Test `fn` alone has span [0, 2)
#[test]
fn fn_alone_has_correct_span() {
    let spanned = lex_with_spans("fn").unwrap();
    assert_eq!(spanned[0].token, Token::Fn);
    assert_eq!(spanned[0].span, Span::new(0, 2));
}

/// Test `fname` produces `Token::Ident("fname")` (not `Fn` + `Ident`)
#[test]
fn fname_produces_ident_not_fn() {
    let tokens = lex("fname").unwrap();
    assert_eq!(tokens, vec![Token::Ident("fname".to_string()), Token::Eof]);
}

/// Test `fn_helper` produces `Token::Ident("fn_helper")`
#[test]
fn fn_helper_produces_ident() {
    let tokens = lex("fn_helper").unwrap();
    assert_eq!(
        tokens,
        vec![Token::Ident("fn_helper".to_string()), Token::Eof]
    );
}

/// Test `fn(` produces `Token::Fn` followed by `Token::OpenParen`
#[test]
fn fn_followed_by_open_paren() {
    let tokens = lex("fn(").unwrap();
    assert_eq!(tokens, vec![Token::Fn, Token::OpenParen, Token::Eof]);
}

/// Test span for `Fn` token is exactly 2 bytes wide
#[test]
fn fn_span_exactly_2_bytes_wide() {
    // With leading whitespace to test non-zero offset
    let spanned = lex_with_spans("  fn").unwrap();
    let fn_token = &spanned[0];
    assert_eq!(fn_token.token, Token::Fn);
    assert_eq!(fn_token.span.len(), 2);
    assert_eq!(fn_token.span, Span::new(2, 4));
}
