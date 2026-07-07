//! Parser module for the Flux language.
//!
//! Transforms a token stream into an Abstract Syntax Tree (AST).

pub mod ast;
mod parser_state;
mod expr;
mod stmt;
mod top_level;
mod pretty_print;

#[cfg(test)]
mod tests_property;

#[cfg(test)]
mod tests_data_block_property;

#[cfg(test)]
mod tests_data_block_keys_property;

pub use ast::*;

use crate::error::Result;
use crate::lexer::SpannedToken;

/// Parse a token stream into a Program AST.
///
/// # Arguments
/// * `tokens` - Spanned tokens from the lexer (must end with Token::Eof)
///
/// # Returns
/// A `Program` AST node on success
///
/// # Errors
/// Returns `CompileError::Parser` with byte offset and expectation on syntax error
pub fn parse(tokens: Vec<SpannedToken>) -> Result<Program> {
    let mut state = parser_state::ParserState::new(tokens)?;
    state.parse_program()
}

/// Pretty-print a Program AST back to Flux source text.
pub fn pretty_print_program(program: &Program) -> String {
    pretty_print::format_program(program)
}
