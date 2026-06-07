//! Compiler error types

use thiserror::Error;

/// Compiler errors
#[derive(Error, Debug)]
pub enum CompileError {
    /// Feature not yet implemented
    #[error("Not yet implemented: {feature}")]
    NotImplemented { feature: String },

    /// Lexer error
    #[error("Lexer error: {0}")]
    Lexer(String),

    /// Parser error
    #[error("Parser error: {0}")]
    Parser(String),

    /// Type checking error
    #[error("Type error: {0}")]
    Type(String),

    /// Code generation error
    #[error("Code generation error: {0}")]
    Codegen(String),

    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Compiler result type
pub type Result<T> = std::result::Result<T, CompileError>;
