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

pub mod token;

pub use token::Token;

use crate::error::{CompileError, Result};

/// Lex Flux source code into tokens
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
    // TODO: Implement lexer using logos
    //
    // Use logos crate for fast, zero-cost tokenization:
    // - Define Token enum with #[derive(Logos)]
    // - Add regex patterns for each token
    // - Handle whitespace and comments
    // - Track spans (start, end positions)

    Err(CompileError::NotImplemented {
        feature: "lexer".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lex_not_yet_implemented() {
        let source = "strategy Test {}";
        let result = lex(source);
        assert!(result.is_err());
    }

    // TODO: Add tests when lexer is implemented
    //
    // #[test]
    // fn lex_strategy_keyword() {
    //     let source = "strategy";
    //     let tokens = lex(source).unwrap();
    //     assert_eq!(tokens.len(), 1);
    //     assert_eq!(tokens[0], Token::Strategy);
    // }
    //
    // #[test]
    // fn lex_identifier() {
    //     let source = "my_strategy";
    //     let tokens = lex(source).unwrap();
    //     assert_eq!(tokens.len(), 1);
    //     matches!(tokens[0], Token::Ident(_));
    // }
}
