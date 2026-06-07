use std::path::Path;

use crate::diagnostics;
use crate::error::{CliError, CompileErrorWithSpan};

/// Extract byte offset from a compiler error message.
///
/// Looks for the pattern "at byte N:" in the message string.
/// Returns 0 if the pattern is not found.
fn extract_byte_offset(message: &str) -> usize {
    if let Some(rest) = message.strip_prefix("at byte ") {
        if let Some(colon_pos) = rest.find(':') {
            if let Ok(offset) = rest[..colon_pos].trim().parse::<usize>() {
                return offset;
            }
        }
    }
    // Also try finding "at byte N:" anywhere in the message
    if let Some(idx) = message.find("at byte ") {
        let rest = &message[idx + "at byte ".len()..];
        if let Some(colon_pos) = rest.find(':') {
            if let Ok(offset) = rest[..colon_pos].trim().parse::<usize>() {
                return offset;
            }
        }
    }
    0
}

/// Convert a `CompileError` into a list of `CompileErrorWithSpan` for diagnostics.
fn compile_error_to_spans(err: &flux_compiler::CompileError) -> Vec<CompileErrorWithSpan> {
    let msg = err.to_string();
    // Split multi-line error messages into individual errors
    msg.lines()
        .map(|line| CompileErrorWithSpan {
            offset: extract_byte_offset(line),
            message: line.to_string(),
        })
        .collect()
}

pub fn run_build(file: &Path, output: Option<&Path>) -> Result<(), CliError> {
    let file_str = file.to_string_lossy();
    let source = std::fs::read_to_string(file).map_err(CliError::Io)?;

    // Lex
    let tokens = match flux_compiler::lexer::lex_with_spans(&source) {
        Ok(tokens) => tokens,
        Err(err) => {
            let compile_errors = compile_error_to_spans(&err);
            eprintln!("{}", diagnostics::format_errors(&file_str, &source, &compile_errors));
            return Err(CliError::Compile(compile_errors));
        }
    };

    // Parse
    let program = match flux_compiler::parser::parse(tokens) {
        Ok(program) => program,
        Err(err) => {
            let compile_errors = compile_error_to_spans(&err);
            eprintln!("{}", diagnostics::format_errors(&file_str, &source, &compile_errors));
            return Err(CliError::Compile(compile_errors));
        }
    };

    // Type check
    let typed_program = match flux_compiler::typeck::check(program) {
        Ok(typed) => typed,
        Err(err) => {
            let compile_errors = compile_error_to_spans(&err);
            eprintln!("{}", diagnostics::format_errors(&file_str, &source, &compile_errors));
            return Err(CliError::Compile(compile_errors));
        }
    };

    // Code generation
    let generated_code = match flux_compiler::codegen::generate(&typed_program) {
        Ok(code) => code,
        Err(err) => {
            let compile_errors = compile_error_to_spans(&err);
            eprintln!("{}", diagnostics::format_errors(&file_str, &source, &compile_errors));
            return Err(CliError::Compile(compile_errors));
        }
    };

    // Output
    match output {
        Some(output_path) => {
            std::fs::write(output_path, &generated_code).map_err(CliError::Io)?;
        }
        None => {
            print!("{}", generated_code);
        }
    }

    Ok(())
}
