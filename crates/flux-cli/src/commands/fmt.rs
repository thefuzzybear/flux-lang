//! The `flux fmt` command — formats Flux source files with optional colorization.
//!
//! Supports three modes:
//! - **stdout** (default): emit formatted source to stdout, with ANSI color if TTY
//! - **write** (`--write`): overwrite the source file with plain formatted text
//! - **check** (`--check`): compare formatted output with input, exit 0 if identical

use std::fs;
use std::path::Path;

use flux_compiler::lexer;
use flux_compiler::parser;
use flux_compiler::CompileError;

use crate::diagnostics::format_error_colored;
use crate::formatter::ansi::{colorize, should_colorize, ColorMode, ColorTheme};
use crate::formatter::Formatter;

/// Errors that can occur during the `fmt` command.
#[derive(Debug, thiserror::Error)]
pub enum FmtError {
    /// The source file could not be read.
    #[error("cannot open '{path}': {source}")]
    FileRead { path: String, source: std::io::Error },

    /// The source file could not be written (--write mode).
    #[error("cannot write '{path}': {source}")]
    FileWrite { path: String, source: std::io::Error },

    /// A lexer or parser error occurred, or check mode detected differences.
    #[error("{0}")]
    Compile(String),

    /// Two mutually exclusive flags were provided.
    #[error("flags '{0}' and '{1}' are mutually exclusive")]
    MutuallyExclusive(String, String),
}

/// Extract byte offset from a compiler error message string.
///
/// Recognized patterns:
/// - "at byte N: ..." (parser)
/// - "Lexer error at byte N: ..." (lexer)
///
/// Returns the extracted offset or 0 if no pattern matches.
fn extract_offset(error_msg: &str) -> usize {
    if let Some(pos) = error_msg.find("at byte ") {
        let after_prefix = &error_msg[pos + "at byte ".len()..];
        if let Some(colon_pos) = after_prefix.find(':') {
            let num_str = &after_prefix[..colon_pos];
            if let Ok(offset) = num_str.trim().parse::<usize>() {
                return offset;
            }
        }
    }
    0
}

/// Run the `fmt` command with the given options.
///
/// Orchestrates: read file → lex → parse → format → apply mode (stdout/write/check).
///
/// # Arguments
/// * `file` - Path to the `.flux` source file
/// * `color_mode` - Whether to apply ANSI coloring (Auto/Always/Never)
/// * `write_mode` - If true, overwrite the source file with formatted output
/// * `check_mode` - If true, compare and report whether file needs formatting
///
/// # Errors
/// Returns `FmtError` for I/O failures, compile errors, mutually exclusive flags,
/// or (in check mode) when the file needs formatting.
pub fn run_fmt(
    file: &Path,
    color_mode: ColorMode,
    write_mode: bool,
    check_mode: bool,
) -> Result<(), FmtError> {
    // 1. Check mutually exclusive flags
    if write_mode && check_mode {
        return Err(FmtError::MutuallyExclusive(
            "--write".to_string(),
            "--check".to_string(),
        ));
    }

    // 2. Read file
    let source = fs::read_to_string(file).map_err(|e| FmtError::FileRead {
        path: file.display().to_string(),
        source: e,
    })?;

    // 3. Lex
    let file_str = file.display().to_string();
    let tokens = lexer::lex_with_spans(&source).map_err(|e| {
        // Render colored diagnostic to stderr
        let use_color = should_colorize(color_mode);
        let msg = match &e {
            CompileError::Lexer(s) => s.clone(),
            other => other.to_string(),
        };
        // Handle multi-line lexer errors (one per line)
        for line in msg.lines() {
            let offset = extract_offset(line);
            let clean_msg = if let Some(pos) = line.find("at byte ") {
                let after = &line[pos + "at byte ".len()..];
                if let Some(colon_pos) = after.find(':') {
                    after[colon_pos + 1..].trim()
                } else {
                    line
                }
            } else {
                line
            };
            let diagnostic =
                format_error_colored(&file_str, &source, offset, clean_msg, use_color);
            eprintln!("{}", diagnostic);
        }
        FmtError::Compile(e.to_string())
    })?;

    // 4. Parse
    let ast = parser::parse(tokens).map_err(|e| {
        let use_color = should_colorize(color_mode);
        let msg = match &e {
            CompileError::Parser(s) => s.clone(),
            other => other.to_string(),
        };
        let offset = extract_offset(&msg);
        let clean_msg = if let Some(pos) = msg.find("at byte ") {
            let after = &msg[pos + "at byte ".len()..];
            if let Some(colon_pos) = after.find(':') {
                after[colon_pos + 1..].trim()
            } else {
                &msg
            }
        } else {
            &msg
        };
        let diagnostic = format_error_colored(&file_str, &source, offset, clean_msg, use_color);
        eprintln!("{}", diagnostic);
        FmtError::Compile(e.to_string())
    })?;

    // 5. Format
    let formatted = Formatter::format(&ast, &source);

    // 6. Apply mode
    if write_mode {
        // Write mode: overwrite file with plain formatted text (no color)
        if formatted != source {
            fs::write(file, &formatted).map_err(|e| FmtError::FileWrite {
                path: file.display().to_string(),
                source: e,
            })?;
        }
        Ok(())
    } else if check_mode {
        // Check mode: compare and report
        if formatted == source {
            Ok(())
        } else {
            println!("{}: needs formatting", file.display());
            Err(FmtError::Compile(format!(
                "{}: file needs formatting",
                file.display()
            )))
        }
    } else {
        // Stdout mode: colorize if appropriate
        let theme = ColorTheme::default_theme();
        let output = colorize(&formatted, &theme, color_mode);
        print!("{}", output);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Helper to create a temp file with given content and return its path.
    /// The caller is responsible for cleanup.
    fn temp_flux_file(content: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir();
        let id = std::process::id();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let path = dir.join(format!("flux_test_{}_{}.flux", id, ts));
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f.flush().unwrap();
        path
    }

    #[test]
    fn mutually_exclusive_write_and_check() {
        let path = temp_flux_file("strategy S {\n    on bar {\n        x = 1\n    }\n}\n");
        let result = run_fmt(&path, ColorMode::Never, true, true);
        let _ = fs::remove_file(&path);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"));
    }

    #[test]
    fn file_not_found_returns_file_read_error() {
        let result = run_fmt(
            Path::new("/nonexistent/file.flux"),
            ColorMode::Never,
            false,
            false,
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("cannot open"));
    }

    #[test]
    fn check_mode_returns_ok_for_formatted_file() {
        let content = "strategy S {\n    on bar {\n        x = 1\n    }\n}\n";
        let path = temp_flux_file(content);
        let result = run_fmt(&path, ColorMode::Never, false, true);
        let _ = fs::remove_file(&path);
        assert!(result.is_ok());
    }

    #[test]
    fn check_mode_returns_error_for_unformatted_file() {
        // Missing proper indentation
        let content = "strategy S {\non bar {\nx = 1\n}\n}\n";
        let path = temp_flux_file(content);
        let result = run_fmt(&path, ColorMode::Never, false, true);
        let _ = fs::remove_file(&path);
        assert!(result.is_err());
    }

    #[test]
    fn write_mode_formats_file_in_place() {
        let content = "strategy S {\non bar {\nx = 1\n}\n}\n";
        let path = temp_flux_file(content);
        let result = run_fmt(&path, ColorMode::Never, true, false);
        assert!(result.is_ok());
        let updated = fs::read_to_string(&path).unwrap();
        let _ = fs::remove_file(&path);
        assert!(updated.contains("    on bar {"));
        assert!(updated.contains("        x = 1"));
    }

    #[test]
    fn write_mode_leaves_formatted_file_unchanged() {
        let content = "strategy S {\n    on bar {\n        x = 1\n    }\n}\n";
        let path = temp_flux_file(content);
        let result = run_fmt(&path, ColorMode::Never, true, false);
        assert!(result.is_ok());
        let updated = fs::read_to_string(&path).unwrap();
        let _ = fs::remove_file(&path);
        assert_eq!(updated, content);
    }

    #[test]
    fn compile_error_renders_diagnostic_to_stderr() {
        let content = "strategy {";
        let path = temp_flux_file(content);
        let result = run_fmt(&path, ColorMode::Never, false, false);
        let _ = fs::remove_file(&path);
        assert!(result.is_err());
        match result.unwrap_err() {
            FmtError::Compile(_) => {} // expected
            other => panic!("Expected FmtError::Compile, got: {:?}", other),
        }
    }

    #[test]
    fn stdout_mode_emits_output() {
        let content = "strategy S {\n    on bar {\n        x = 1\n    }\n}\n";
        let path = temp_flux_file(content);
        // Just verify it doesn't error — stdout capture would need more infrastructure
        let result = run_fmt(&path, ColorMode::Never, false, false);
        let _ = fs::remove_file(&path);
        assert!(result.is_ok());
    }
}
