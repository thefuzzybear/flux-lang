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
// pub mod typeck;  // TODO: Implement
// pub mod codegen; // TODO: Implement

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
    // TODO: Implement full compilation pipeline
    //
    // let tokens = lexer::lex(source)?;
    // let ast = parser::parse(tokens)?;
    // let typed_ast = typeck::check(ast)?;
    // let rust_code = codegen::generate(typed_ast)?;
    // Ok(rust_code)

    Err(CompileError::NotImplemented {
        feature: "compilation pipeline".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_not_yet_implemented() {
        let source = "strategy Test {}";
        let result = compile(source);
        assert!(result.is_err());
    }
}
