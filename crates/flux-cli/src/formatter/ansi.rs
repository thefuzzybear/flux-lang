//! ANSI renderer — applies color codes to formatted Flux source for terminal display.
//!
//! This module provides token category classification, color theme configuration,
//! and ANSI escape code application for colorized terminal output of Flux source code.

use std::io::IsTerminal;

use flux_compiler::extract_comments;
use flux_compiler::lexer::token::Token;
use flux_compiler::lexer::lex_with_spans;

/// Token categories for color assignment.
///
/// Each token produced by the lexer is classified into one of these categories
/// to determine its color in terminal output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenCategory {
    /// Keywords: strategy, params, state, on, if, elif, else, for, while, return, from, import, and, or, not
    Keyword,
    /// Variable and function names that don't match other categories
    Identifier,
    /// Integer numeric literals
    IntegerLiteral,
    /// Floating-point numeric literals
    FloatLiteral,
    /// String literals including surrounding quotes
    StringLiteral,
    /// Comments (from `#` to end of line)
    Comment,
    /// Operators: +, -, *, /, %, ==, !=, <, <=, >, >=, =
    Operator,
    /// Delimiters: parentheses, braces, brackets, commas, dots, colons
    Delimiter,
    /// Signal functions: OPEN, CLOSE, CLOSE_QTY
    SignalFunction,
    /// Boolean literals: true, false (and null)
    BooleanLiteral,
}

/// ANSI escape codes for a single style.
///
/// A style consists of a prefix (the escape sequence to start the color)
/// and a suffix (the reset sequence). For unstyled tokens, both are empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnsiStyle {
    /// ANSI escape sequence to start the style, e.g., "\x1b[1;34m" for bold blue.
    /// Empty string for unstyled tokens.
    pub prefix: &'static str,
    /// ANSI escape sequence to reset styling, always "\x1b[0m" for styled tokens.
    /// Empty string for unstyled tokens.
    pub suffix: &'static str,
}

impl AnsiStyle {
    /// Create a styled AnsiStyle with the given prefix and the standard reset suffix.
    const fn styled(prefix: &'static str) -> Self {
        Self {
            prefix,
            suffix: "\x1b[0m",
        }
    }

    /// Create an unstyled AnsiStyle (no color applied).
    const fn unstyled() -> Self {
        Self {
            prefix: "",
            suffix: "",
        }
    }
}

/// Color configuration for ANSI rendering.
///
/// Maps each `TokenCategory` to an `AnsiStyle` with specific ANSI escape codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorTheme {
    /// Style for keywords (strategy, params, state, on, if, elif, else, for, while, return, from, import, and, or, not)
    pub keyword: AnsiStyle,
    /// Style for identifiers (no color by default)
    pub identifier: AnsiStyle,
    /// Style for integer literals
    pub integer_literal: AnsiStyle,
    /// Style for float literals
    pub float_literal: AnsiStyle,
    /// Style for string literals (including quotes)
    pub string_literal: AnsiStyle,
    /// Style for comments
    pub comment: AnsiStyle,
    /// Style for operators
    pub operator: AnsiStyle,
    /// Style for delimiters (no color by default)
    pub delimiter: AnsiStyle,
    /// Style for signal functions (OPEN, CLOSE, CLOSE_QTY)
    pub signal_function: AnsiStyle,
    /// Style for boolean literals (true, false) and null
    pub boolean_literal: AnsiStyle,
}

impl ColorTheme {
    /// Create the default color theme for Flux syntax highlighting.
    ///
    /// Color mapping:
    /// - Keywords: Bold Blue (`\x1b[1;34m`)
    /// - Identifiers: Default (no color)
    /// - Integer Literals: Cyan (`\x1b[36m`)
    /// - Float Literals: Cyan (`\x1b[36m`)
    /// - String Literals: Green (`\x1b[32m`)
    /// - Comments: Dim White/gray (`\x1b[2;37m`)
    /// - Operators: Yellow (`\x1b[33m`)
    /// - Delimiters: Default (no color)
    /// - Signal Functions: Bold Magenta (`\x1b[1;35m`)
    /// - Boolean Literals: Bold Blue (`\x1b[1;34m`) (same as keywords)
    pub fn default_theme() -> Self {
        Self {
            keyword: AnsiStyle::styled("\x1b[1;34m"),         // Bold Blue
            identifier: AnsiStyle::unstyled(),                 // Default (no color)
            integer_literal: AnsiStyle::styled("\x1b[36m"),   // Cyan
            float_literal: AnsiStyle::styled("\x1b[36m"),     // Cyan
            string_literal: AnsiStyle::styled("\x1b[32m"),    // Green
            comment: AnsiStyle::styled("\x1b[2;37m"),         // Dim White (gray)
            operator: AnsiStyle::styled("\x1b[33m"),          // Yellow
            delimiter: AnsiStyle::unstyled(),                   // Default (no color)
            signal_function: AnsiStyle::styled("\x1b[1;35m"), // Bold Magenta
            boolean_literal: AnsiStyle::styled("\x1b[1;34m"), // Bold Blue (same as keyword)
        }
    }

    /// Get the `AnsiStyle` for a given `TokenCategory`.
    pub fn style_for(&self, category: TokenCategory) -> &AnsiStyle {
        match category {
            TokenCategory::Keyword => &self.keyword,
            TokenCategory::Identifier => &self.identifier,
            TokenCategory::IntegerLiteral => &self.integer_literal,
            TokenCategory::FloatLiteral => &self.float_literal,
            TokenCategory::StringLiteral => &self.string_literal,
            TokenCategory::Comment => &self.comment,
            TokenCategory::Operator => &self.operator,
            TokenCategory::Delimiter => &self.delimiter,
            TokenCategory::SignalFunction => &self.signal_function,
            TokenCategory::BooleanLiteral => &self.boolean_literal,
        }
    }
}

/// Determines whether color output should be applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    /// Detect TTY and apply color only if stdout is a terminal.
    Auto,
    /// Always apply ANSI color codes (--color flag).
    Always,
    /// Never apply ANSI color codes (--no-color flag).
    Never,
}

/// Classify a lexer token into a `TokenCategory` for color assignment.
///
/// Maps each token variant to its semantic category, with special handling
/// for identifiers that are signal functions (OPEN, CLOSE, CLOSE_QTY).
pub fn classify_token(token: &Token) -> TokenCategory {
    match token {
        // Keywords
        Token::Strategy
        | Token::Params
        | Token::State
        | Token::On
        | Token::If
        | Token::Elif
        | Token::Else
        | Token::For
        | Token::While
        | Token::Return
        | Token::Fn
        | Token::From
        | Token::Import
        | Token::And
        | Token::Or
        | Token::Not
        | Token::Data
        | Token::Connector
        | Token::Struct
        | Token::Enum
        | Token::Match
        | Token::SelfKw
        | Token::Impl
        | Token::Trait
        | Token::In => TokenCategory::Keyword,

        // Boolean/Null literals
        Token::True | Token::False | Token::Null => TokenCategory::BooleanLiteral,

        // Numeric literals
        Token::Int(_) => TokenCategory::IntegerLiteral,
        Token::Float(_) => TokenCategory::FloatLiteral,

        // String literal
        Token::String(_) => TokenCategory::StringLiteral,

        // Operators
        Token::Plus
        | Token::Minus
        | Token::Star
        | Token::Slash
        | Token::Percent
        | Token::Eq
        | Token::Ne
        | Token::Lt
        | Token::Le
        | Token::Gt
        | Token::Ge
        | Token::Assign
        | Token::Bang
        | Token::AndAnd
        | Token::OrOr
        | Token::At
        | Token::Arrow
        | Token::FatArrow => TokenCategory::Operator,

        // Delimiters
        Token::OpenParen
        | Token::CloseParen
        | Token::OpenBrace
        | Token::CloseBrace
        | Token::OpenBracket
        | Token::CloseBracket
        | Token::Comma
        | Token::Dot
        | Token::Colon
        | Token::ColonColon
        | Token::Semicolon => TokenCategory::Delimiter,

        // Identifiers — check for signal functions and manifest keywords
        Token::Ident(name) => match name.as_str() {
            "OPEN" | "CLOSE" | "CLOSE_QTY" => TokenCategory::SignalFunction,
            "account" | "gateway" | "database" | "risk" | "products" | "strategies" | "env" => TokenCategory::Keyword,
            _ => TokenCategory::Identifier,
        },

        Token::Eof => TokenCategory::Identifier, // won't be rendered
    }
}

/// Determine whether colorization should be applied based on the color mode
/// and whether stdout is connected to a TTY.
pub fn should_colorize(mode: ColorMode) -> bool {
    match mode {
        ColorMode::Always => true,
        ColorMode::Never => false,
        ColorMode::Auto => std::io::stdout().is_terminal(),
    }
}

/// A span with its associated token category for colorization.
#[derive(Debug, Clone, Copy)]
struct ColorSpan {
    start: usize,
    end: usize,
    category: TokenCategory,
}

/// Apply ANSI color codes to formatted source text.
///
/// Re-lexes the input to identify token boundaries, then applies ANSI escape
/// codes span-by-span based on the theme's color mapping. Comments are handled
/// via a separate extraction pass since they are skipped by the lexer.
///
/// Returns the source unchanged if colorization is disabled (Never mode or
/// Auto mode without a TTY).
pub fn colorize(source: &str, theme: &ColorTheme, mode: ColorMode) -> String {
    if !should_colorize(mode) {
        return source.to_string();
    }

    // Build a list of (start, end, category) spans from lexer tokens
    let mut spans: Vec<ColorSpan> = Vec::new();

    // Extract comments (they're not in the token stream)
    let comments = extract_comments(source);
    for comment in &comments {
        let end = comment.start + comment.text.len();
        spans.push(ColorSpan {
            start: comment.start,
            end,
            category: TokenCategory::Comment,
        });
    }

    // Lex the source to get token spans
    if let Ok(spanned_tokens) = lex_with_spans(source) {
        for st in &spanned_tokens {
            if st.token == Token::Eof {
                continue;
            }
            spans.push(ColorSpan {
                start: st.span.start,
                end: st.span.end,
                category: classify_token(&st.token),
            });
        }
    }

    // Sort spans by start position (stable sort to preserve order for overlaps)
    spans.sort_by_key(|s| s.start);

    // Walk through the source, emitting colorized text
    let mut result = String::with_capacity(source.len() * 2);
    let mut pos: usize = 0;

    for span in &spans {
        // Skip spans that are before our current position (overlapping)
        if span.start < pos {
            continue;
        }

        // Emit any gap (whitespace/unrecognized chars) between last position and this span
        if span.start > pos {
            result.push_str(&source[pos..span.start]);
        }

        // Emit the token text with ANSI color codes
        let style = theme.style_for(span.category);
        let token_text = &source[span.start..span.end];
        result.push_str(style.prefix);
        result.push_str(token_text);
        result.push_str(style.suffix);

        pos = span.end;
    }

    // Emit any remaining text after the last span
    if pos < source.len() {
        result.push_str(&source[pos..]);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_theme_keywords_are_bold_blue() {
        let theme = ColorTheme::default_theme();
        assert_eq!(theme.keyword.prefix, "\x1b[1;34m");
        assert_eq!(theme.keyword.suffix, "\x1b[0m");
    }

    #[test]
    fn default_theme_identifiers_are_unstyled() {
        let theme = ColorTheme::default_theme();
        assert_eq!(theme.identifier.prefix, "");
        assert_eq!(theme.identifier.suffix, "");
    }

    #[test]
    fn default_theme_integers_are_cyan() {
        let theme = ColorTheme::default_theme();
        assert_eq!(theme.integer_literal.prefix, "\x1b[36m");
        assert_eq!(theme.integer_literal.suffix, "\x1b[0m");
    }

    #[test]
    fn default_theme_floats_are_cyan() {
        let theme = ColorTheme::default_theme();
        assert_eq!(theme.float_literal.prefix, "\x1b[36m");
        assert_eq!(theme.float_literal.suffix, "\x1b[0m");
    }

    #[test]
    fn default_theme_strings_are_green() {
        let theme = ColorTheme::default_theme();
        assert_eq!(theme.string_literal.prefix, "\x1b[32m");
        assert_eq!(theme.string_literal.suffix, "\x1b[0m");
    }

    #[test]
    fn default_theme_comments_are_dim_white() {
        let theme = ColorTheme::default_theme();
        assert_eq!(theme.comment.prefix, "\x1b[2;37m");
        assert_eq!(theme.comment.suffix, "\x1b[0m");
    }

    #[test]
    fn default_theme_operators_are_yellow() {
        let theme = ColorTheme::default_theme();
        assert_eq!(theme.operator.prefix, "\x1b[33m");
        assert_eq!(theme.operator.suffix, "\x1b[0m");
    }

    #[test]
    fn default_theme_delimiters_are_unstyled() {
        let theme = ColorTheme::default_theme();
        assert_eq!(theme.delimiter.prefix, "");
        assert_eq!(theme.delimiter.suffix, "");
    }

    #[test]
    fn default_theme_signal_functions_are_bold_magenta() {
        let theme = ColorTheme::default_theme();
        assert_eq!(theme.signal_function.prefix, "\x1b[1;35m");
        assert_eq!(theme.signal_function.suffix, "\x1b[0m");
    }

    #[test]
    fn default_theme_booleans_are_bold_blue() {
        let theme = ColorTheme::default_theme();
        assert_eq!(theme.boolean_literal.prefix, "\x1b[1;34m");
        assert_eq!(theme.boolean_literal.suffix, "\x1b[0m");
    }

    #[test]
    fn style_for_returns_correct_style() {
        let theme = ColorTheme::default_theme();
        assert_eq!(theme.style_for(TokenCategory::Keyword), &theme.keyword);
        assert_eq!(theme.style_for(TokenCategory::Identifier), &theme.identifier);
        assert_eq!(theme.style_for(TokenCategory::IntegerLiteral), &theme.integer_literal);
        assert_eq!(theme.style_for(TokenCategory::FloatLiteral), &theme.float_literal);
        assert_eq!(theme.style_for(TokenCategory::StringLiteral), &theme.string_literal);
        assert_eq!(theme.style_for(TokenCategory::Comment), &theme.comment);
        assert_eq!(theme.style_for(TokenCategory::Operator), &theme.operator);
        assert_eq!(theme.style_for(TokenCategory::Delimiter), &theme.delimiter);
        assert_eq!(theme.style_for(TokenCategory::SignalFunction), &theme.signal_function);
        assert_eq!(theme.style_for(TokenCategory::BooleanLiteral), &theme.boolean_literal);
    }

    #[test]
    fn colorize_never_mode_returns_source_unchanged() {
        let source = "x = sma(close, 20)";
        let theme = ColorTheme::default_theme();
        let result = colorize(source, &theme, ColorMode::Never);
        assert_eq!(result, source);
    }

    #[test]
    fn colorize_always_mode_applies_colors() {
        let source = "strategy Test {\n}\n";
        let theme = ColorTheme::default_theme();
        let result = colorize(source, &theme, ColorMode::Always);
        // "strategy" keyword should have bold blue prefix
        assert!(result.contains("\x1b[1;34mstrategy\x1b[0m"));
        // "Test" identifier should have no ANSI codes (unstyled)
        assert!(result.contains("Test"));
    }

    #[test]
    fn colorize_always_mode_colors_string_literal() {
        let source = "x = \"hello\"";
        let theme = ColorTheme::default_theme();
        let result = colorize(source, &theme, ColorMode::Always);
        // String literal including quotes should be green
        assert!(result.contains("\x1b[32m\"hello\"\x1b[0m"));
    }

    #[test]
    fn colorize_always_mode_colors_integer() {
        let source = "x = 42";
        let theme = ColorTheme::default_theme();
        let result = colorize(source, &theme, ColorMode::Always);
        // Integer should be cyan
        assert!(result.contains("\x1b[36m42\x1b[0m"));
    }

    #[test]
    fn colorize_always_mode_colors_float() {
        let source = "x = 3.14";
        let theme = ColorTheme::default_theme();
        let result = colorize(source, &theme, ColorMode::Always);
        // Float should be cyan
        assert!(result.contains("\x1b[36m3.14\x1b[0m"));
    }

    #[test]
    fn colorize_always_mode_colors_signal_function() {
        let source = "OPEN(symbol, 100)";
        let theme = ColorTheme::default_theme();
        let result = colorize(source, &theme, ColorMode::Always);
        // Signal function should be bold magenta
        assert!(result.contains("\x1b[1;35mOPEN\x1b[0m"));
    }

    #[test]
    fn colorize_always_mode_colors_boolean() {
        let source = "x = true";
        let theme = ColorTheme::default_theme();
        let result = colorize(source, &theme, ColorMode::Always);
        // Boolean should be bold blue
        assert!(result.contains("\x1b[1;34mtrue\x1b[0m"));
    }

    #[test]
    fn colorize_always_mode_colors_comment() {
        let source = "x = 1 # a comment";
        let theme = ColorTheme::default_theme();
        let result = colorize(source, &theme, ColorMode::Always);
        // Comment should be dim white
        assert!(result.contains("\x1b[2;37m# a comment\x1b[0m"));
    }

    #[test]
    fn colorize_always_mode_colors_operators() {
        let source = "x = a + b";
        let theme = ColorTheme::default_theme();
        let result = colorize(source, &theme, ColorMode::Always);
        // Operator '+' should be yellow
        assert!(result.contains("\x1b[33m+\x1b[0m"));
        // '=' is also an operator
        assert!(result.contains("\x1b[33m=\x1b[0m"));
    }

    #[test]
    fn colorize_preserves_whitespace_between_tokens() {
        let source = "x = 1";
        let theme = ColorTheme::default_theme();
        let result = colorize(source, &theme, ColorMode::Always);
        // Spaces between tokens should be preserved (not colored)
        // The result should contain the space characters between tokens
        assert!(result.contains(" "));
        // The overall structure should be: x <space> = <space> 1
        // with ANSI wrappers around the tokens
    }

    #[test]
    fn classify_token_keywords() {
        assert_eq!(classify_token(&Token::Strategy), TokenCategory::Keyword);
        assert_eq!(classify_token(&Token::If), TokenCategory::Keyword);
        assert_eq!(classify_token(&Token::Return), TokenCategory::Keyword);
        assert_eq!(classify_token(&Token::And), TokenCategory::Keyword);
    }

    #[test]
    fn classify_token_signal_functions() {
        assert_eq!(
            classify_token(&Token::Ident("OPEN".to_string())),
            TokenCategory::SignalFunction
        );
        assert_eq!(
            classify_token(&Token::Ident("CLOSE".to_string())),
            TokenCategory::SignalFunction
        );
        assert_eq!(
            classify_token(&Token::Ident("CLOSE_QTY".to_string())),
            TokenCategory::SignalFunction
        );
    }

    #[test]
    fn classify_token_regular_ident() {
        assert_eq!(
            classify_token(&Token::Ident("foo".to_string())),
            TokenCategory::Identifier
        );
    }

    #[test]
    fn classify_token_boolean_and_null() {
        assert_eq!(classify_token(&Token::True), TokenCategory::BooleanLiteral);
        assert_eq!(classify_token(&Token::False), TokenCategory::BooleanLiteral);
        assert_eq!(classify_token(&Token::Null), TokenCategory::BooleanLiteral);
    }

    #[test]
    fn classify_token_literals() {
        assert_eq!(classify_token(&Token::Int(42)), TokenCategory::IntegerLiteral);
        assert_eq!(classify_token(&Token::Float(3.14)), TokenCategory::FloatLiteral);
        assert_eq!(
            classify_token(&Token::String("hi".to_string())),
            TokenCategory::StringLiteral
        );
    }

    #[test]
    fn classify_token_operators() {
        assert_eq!(classify_token(&Token::Plus), TokenCategory::Operator);
        assert_eq!(classify_token(&Token::Eq), TokenCategory::Operator);
        assert_eq!(classify_token(&Token::Assign), TokenCategory::Operator);
    }

    #[test]
    fn classify_token_delimiters() {
        assert_eq!(classify_token(&Token::OpenParen), TokenCategory::Delimiter);
        assert_eq!(classify_token(&Token::CloseBrace), TokenCategory::Delimiter);
        assert_eq!(classify_token(&Token::Comma), TokenCategory::Delimiter);
    }

    #[test]
    fn should_colorize_always_returns_true() {
        assert!(should_colorize(ColorMode::Always));
    }

    #[test]
    fn should_colorize_never_returns_false() {
        assert!(!should_colorize(ColorMode::Never));
    }

    #[test]
    fn color_mode_variants_are_distinct() {
        assert_ne!(ColorMode::Auto, ColorMode::Always);
        assert_ne!(ColorMode::Auto, ColorMode::Never);
        assert_ne!(ColorMode::Always, ColorMode::Never);
    }

    #[test]
    fn token_category_all_variants() {
        // Verify all variants exist and are distinct
        let categories = [
            TokenCategory::Keyword,
            TokenCategory::Identifier,
            TokenCategory::IntegerLiteral,
            TokenCategory::FloatLiteral,
            TokenCategory::StringLiteral,
            TokenCategory::Comment,
            TokenCategory::Operator,
            TokenCategory::Delimiter,
            TokenCategory::SignalFunction,
            TokenCategory::BooleanLiteral,
        ];
        // Each pair should be distinct
        for i in 0..categories.len() {
            for j in (i + 1)..categories.len() {
                assert_ne!(categories[i], categories[j]);
            }
        }
    }

    #[test]
    fn ansi_style_styled_has_reset_suffix() {
        let style = AnsiStyle::styled("\x1b[1;34m");
        assert_eq!(style.suffix, "\x1b[0m");
    }

    #[test]
    fn ansi_style_unstyled_has_empty_prefix_and_suffix() {
        let style = AnsiStyle::unstyled();
        assert_eq!(style.prefix, "");
        assert_eq!(style.suffix, "");
    }
}
