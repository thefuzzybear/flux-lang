//! Flux Compiler
//!
//! Compiles Flux source code to Rust code.
//!
//! # Architecture
//!
//! ```text
//! Flux Source (.flux)
//!     ↓ Lexer
//! Tokens
//!     ↓ Parser
//! Abstract Syntax Tree (AST)
//!     ↓ Type Checker
//! Typed AST
//!     ↓ Code Generator
//! Rust Code (.rs)
//! ```
//!
//! # Example
//!
//! ```no_run
//! use flux_compiler::compile;
//!
//! let flux_source = r#"
//!     strategy Simple {
//!         on_bar {
//!             if close > open {
//!                 OPEN(symbol, 100)
//!             }
//!         }
//!     }
//! "#;
//!
//! let rust_code = compile(flux_source).expect("Compilation failed");
//! // rust_code contains generated Rust code
//! ```

pub mod error;
pub mod lexer;
pub mod parser;
pub mod typeck;
pub mod codegen;

pub use error::{CompileError, Result};

/// Compile Flux source code to Rust code.
///
/// This is the main entry point for the compiler. It runs all compilation
/// phases: lexing, parsing, type checking, and code generation.
///
/// # Arguments
///
/// * `source` - Flux source code
///
/// # Returns
///
/// Generated Rust code as a String
///
/// # Errors
///
/// Returns `CompileError` if any compilation phase fails.
pub fn compile(source: &str) -> Result<String> {
    let tokens = lexer::lex_with_spans(source)?;
    let ast = parser::parse(tokens)?;
    let typed_ast = typeck::check(ast)?;
    let code = codegen::generate(&typed_ast)?;
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_valid_strategy() {
        let source = r#"strategy Simple {
    on_bar {
        if close > open {
            OPEN(symbol, 100)
        }
    }
}"#;
        let result = compile(source);
        assert!(result.is_ok(), "Expected Ok, got: {:?}", result.err());
        let code = result.unwrap();
        assert!(code.contains("struct Simple"), "Generated code should contain the strategy struct");
    }

    #[test]
    fn compile_syntax_error_returns_parser_error() {
        let source = "strategy {"; // missing name
        let result = compile(source);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CompileError::Parser(_)));
    }

    #[test]
    fn compile_type_error_returns_type_error() {
        let source = r#"strategy Bad {
    on_bar {
        if 42 {
            OPEN(symbol, 100)
        }
    }
}"#;
        let result = compile(source);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CompileError::Type(_)));
    }
}
