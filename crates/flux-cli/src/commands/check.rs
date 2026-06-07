use std::path::Path;

use crate::diagnostics;
use crate::error::{CliError, CompileErrorWithSpan};

/// Extract byte offset from a compiler error message string.
///
/// Recognized patterns:
/// - "at byte N: ..." (parser, typeck)
/// - "Lexer error at byte N: ..." (lexer)
///
/// Returns (offset, cleaned_message). If no pattern matches, returns (0, original_message).
fn extract_offset_and_message(error_msg: &str) -> (usize, String) {
    // Try "at byte N:" pattern (may appear at the start, or after "Lexer error ")
    if let Some(pos) = error_msg.find("at byte ") {
        let after_prefix = &error_msg[pos + "at byte ".len()..];
        if let Some(colon_pos) = after_prefix.find(':') {
            let num_str = &after_prefix[..colon_pos];
            if let Ok(offset) = num_str.trim().parse::<usize>() {
                // The message is everything after "at byte N: "
                let message = after_prefix[colon_pos + 1..].trim().to_string();
                return (offset, message);
            }
        }
    }
    (0, error_msg.to_string())
}

/// Convert a `CompileError` into a list of `CompileErrorWithSpan`.
///
/// Each line of a multi-line error message (e.g., from the lexer which may
/// report multiple errors) is parsed individually.
fn compile_error_to_spans(error: &flux_compiler::CompileError) -> Vec<CompileErrorWithSpan> {
    let msg = match error {
        flux_compiler::CompileError::Lexer(s) => s.clone(),
        flux_compiler::CompileError::Parser(s) => s.clone(),
        flux_compiler::CompileError::Type(s) => s.clone(),
        _ => error.to_string(),
    };

    // The lexer may produce multi-line error messages (one error per line)
    msg.lines()
        .map(|line| {
            let (offset, message) = extract_offset_and_message(line);
            CompileErrorWithSpan { offset, message }
        })
        .collect()
}

/// Run the check command: lex, parse, and type-check a Flux source file.
///
/// On success, prints "{file}: ok" to stdout.
/// On failure, formats errors via the diagnostics module and writes to stderr.
pub fn run_check(file: &Path) -> Result<(), CliError> {
    let source = std::fs::read_to_string(file).map_err(CliError::Io)?;

    let file_display = file.display().to_string();

    // Lex
    let tokens = match flux_compiler::lexer::lex_with_spans(&source) {
        Ok(tokens) => tokens,
        Err(err) => {
            let errors = compile_error_to_spans(&err);
            let formatted = diagnostics::format_errors(&file_display, &source, &errors);
            eprintln!("{}", formatted);
            return Err(CliError::Compile(errors));
        }
    };

    // Parse
    let ast = match flux_compiler::parser::parse(tokens) {
        Ok(ast) => ast,
        Err(err) => {
            let errors = compile_error_to_spans(&err);
            let formatted = diagnostics::format_errors(&file_display, &source, &errors);
            eprintln!("{}", formatted);
            return Err(CliError::Compile(errors));
        }
    };

    // Type check
    match flux_compiler::typeck::check(ast) {
        Ok(_typed_program) => {
            println!("{}: ok", file.display());
            Ok(())
        }
        Err(err) => {
            let errors = compile_error_to_spans(&err);
            let formatted = diagnostics::format_errors(&file_display, &source, &errors);
            eprintln!("{}", formatted);
            Err(CliError::Compile(errors))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_offset_parser_format() {
        let (offset, msg) = extract_offset_and_message("at byte 42: expected identifier");
        assert_eq!(offset, 42);
        assert_eq!(msg, "expected identifier");
    }

    #[test]
    fn extract_offset_lexer_format() {
        let (offset, msg) =
            extract_offset_and_message("Lexer error at byte 15: unexpected character '@'");
        assert_eq!(offset, 15);
        assert_eq!(msg, "unexpected character '@'");
    }

    #[test]
    fn extract_offset_type_format() {
        let (offset, msg) =
            extract_offset_and_message("at byte 100: expected Bool, found Int");
        assert_eq!(offset, 100);
        assert_eq!(msg, "expected Bool, found Int");
    }

    #[test]
    fn extract_offset_no_pattern() {
        let (offset, msg) = extract_offset_and_message("something went wrong");
        assert_eq!(offset, 0);
        assert_eq!(msg, "something went wrong");
    }

    #[test]
    fn compile_error_to_spans_multiline_lexer() {
        let error = flux_compiler::CompileError::Lexer(
            "Lexer error at byte 5: unexpected character '@'\n\
             Lexer error at byte 12: unterminated string literal"
                .to_string(),
        );
        let spans = compile_error_to_spans(&error);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].offset, 5);
        assert_eq!(spans[0].message, "unexpected character '@'");
        assert_eq!(spans[1].offset, 12);
        assert_eq!(spans[1].message, "unterminated string literal");
    }

    #[test]
    fn run_check_missing_file() {
        let result = run_check(Path::new("/nonexistent/file.flux"));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CliError::Io(_)));
    }
}
