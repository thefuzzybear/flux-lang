//! Integration tests for the `flux fmt` CLI command.
//!
//! These tests invoke the built binary via `std::process::Command` and verify
//! exit codes, stdout, and stderr output for various flag combinations and modes.
//!
//! **Validates: Requirements 1.4, 1.5, 1.6, 1.7, 4.1, 4.2, 4.3, 5.1, 5.2, 5.3**

use std::fs;
use std::process::Command;

/// Get a Command for the compiled `flux` binary.
fn flux_cmd() -> Command {
    Command::new(env!("CARGO_BIN_EXE_flux"))
}

/// Create a temporary .flux file with given content and return its path.
/// Uses a unique name based on PID and a counter suffix.
fn temp_flux_file(content: &str, suffix: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir();
    let id = std::process::id();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    let path = dir.join(format!("flux_fmt_test_{}_{}_{}.flux", id, ts, suffix));
    fs::write(&path, content).expect("failed to write temp file");
    path
}

// =============================================================================
// Mutually exclusive flags
// =============================================================================

/// Validates: Requirements 1.6
/// `flux fmt file --color --no-color` should exit with code 2 and report mutually exclusive.
#[test]
fn fmt_color_and_no_color_are_mutually_exclusive() {
    let path = temp_flux_file("strategy S {\n    on bar {\n        x = 1\n    }\n}\n", "excl1");

    let output = flux_cmd()
        .arg("fmt")
        .arg(&path)
        .arg("--color")
        .arg("--no-color")
        .output()
        .expect("failed to execute");

    let _ = fs::remove_file(&path);

    assert_eq!(
        output.status.code(),
        Some(2),
        "Expected exit code 2 for mutually exclusive flags, got: {:?}, stderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("mutually exclusive"),
        "Expected 'mutually exclusive' in stderr, got: {:?}",
        stderr
    );
}

/// Validates: Requirements 4.6, 5.5
/// `flux fmt file --write --check` should exit with code 2 and report mutually exclusive.
#[test]
fn fmt_write_and_check_are_mutually_exclusive() {
    let path = temp_flux_file("strategy S {\n    on bar {\n        x = 1\n    }\n}\n", "excl2");

    let output = flux_cmd()
        .arg("fmt")
        .arg(&path)
        .arg("--write")
        .arg("--check")
        .output()
        .expect("failed to execute");

    let _ = fs::remove_file(&path);

    assert_eq!(
        output.status.code(),
        Some(2),
        "Expected exit code 2 for mutually exclusive --write and --check, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("mutually exclusive"),
        "Expected 'mutually exclusive' in stderr, got: {:?}",
        stderr
    );
}

// =============================================================================
// File not found
// =============================================================================

/// Validates: Requirements 1.7
/// `flux fmt /nonexistent/file.flux` should exit with code 1 and print error to stderr.
#[test]
fn fmt_file_not_found_exits_1() {
    let output = flux_cmd()
        .arg("fmt")
        .arg("/nonexistent/path/missing.flux")
        .output()
        .expect("failed to execute");

    assert_eq!(
        output.status.code(),
        Some(1),
        "Expected exit code 1 for missing file"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot open") || stderr.contains("No such file"),
        "Expected file-not-found error in stderr, got: {:?}",
        stderr
    );
}

// =============================================================================
// --write mode
// =============================================================================

/// Validates: Requirements 4.1, 4.3
/// `flux fmt tempfile.flux --write` should reformat the file in place.
#[test]
fn fmt_write_mode_reformats_file() {
    let unformatted = "strategy S {\non bar {\nx = 1\n}\n}\n";
    let path = temp_flux_file(unformatted, "write1");

    let output = flux_cmd()
        .arg("fmt")
        .arg(&path)
        .arg("--write")
        .output()
        .expect("failed to execute");

    assert_eq!(
        output.status.code(),
        Some(0),
        "Expected exit code 0 for --write, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let content = fs::read_to_string(&path).expect("failed to read back file");
    let _ = fs::remove_file(&path);

    // Verify proper indentation was applied
    assert!(
        content.contains("    on bar {"),
        "Expected indented 'on bar {{', got:\n{}",
        content
    );
    assert!(
        content.contains("        x = 1"),
        "Expected double-indented 'x = 1', got:\n{}",
        content
    );
}

/// Validates: Requirements 4.2
/// `flux fmt tempfile.flux --write` with already-formatted content leaves file unchanged.
#[test]
fn fmt_write_mode_leaves_formatted_file_unchanged() {
    let formatted = "strategy S {\n    on bar {\n        x = 1\n    }\n}\n";
    let path = temp_flux_file(formatted, "write2");

    let output = flux_cmd()
        .arg("fmt")
        .arg(&path)
        .arg("--write")
        .output()
        .expect("failed to execute");

    assert_eq!(
        output.status.code(),
        Some(0),
        "Expected exit code 0 for --write on formatted file, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let content = fs::read_to_string(&path).expect("failed to read back file");
    let _ = fs::remove_file(&path);

    assert_eq!(
        content, formatted,
        "Formatted file should be unchanged after --write"
    );
}

// =============================================================================
// --check mode
// =============================================================================

/// Validates: Requirements 5.2
/// `flux fmt tempfile.flux --check` with formatted file should exit 0.
#[test]
fn fmt_check_mode_formatted_file_exits_0() {
    let formatted = "strategy S {\n    on bar {\n        x = 1\n    }\n}\n";
    let path = temp_flux_file(formatted, "check1");

    let output = flux_cmd()
        .arg("fmt")
        .arg(&path)
        .arg("--check")
        .output()
        .expect("failed to execute");

    let _ = fs::remove_file(&path);

    assert_eq!(
        output.status.code(),
        Some(0),
        "Expected exit code 0 for --check on formatted file, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Validates: Requirements 5.3
/// `flux fmt tempfile.flux --check` with unformatted file should exit 1.
#[test]
fn fmt_check_mode_unformatted_file_exits_1() {
    let unformatted = "strategy S {\non bar {\nx = 1\n}\n}\n";
    let path = temp_flux_file(unformatted, "check2");

    let output = flux_cmd()
        .arg("fmt")
        .arg(&path)
        .arg("--check")
        .output()
        .expect("failed to execute");

    let _ = fs::remove_file(&path);

    assert_eq!(
        output.status.code(),
        Some(1),
        "Expected exit code 1 for --check on unformatted file"
    );
}

// =============================================================================
// Piped output (--no-color)
// =============================================================================

/// Validates: Requirements 1.4, 5.1
/// `flux fmt file --no-color` should produce output without ANSI escape codes.
#[test]
fn fmt_no_color_output_has_no_ansi_codes() {
    let content = "strategy S {\n    on bar {\n        x = 1\n    }\n}\n";
    let path = temp_flux_file(content, "nocolor");

    let output = flux_cmd()
        .arg("fmt")
        .arg(&path)
        .arg("--no-color")
        .output()
        .expect("failed to execute");

    let _ = fs::remove_file(&path);

    assert_eq!(
        output.status.code(),
        Some(0),
        "Expected exit code 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("\x1b["),
        "Expected no ANSI escape codes in --no-color output, got: {:?}",
        stdout
    );
    // Should still have content
    assert!(
        !stdout.is_empty(),
        "Expected non-empty stdout from fmt"
    );
}

// =============================================================================
// Compile error handling
// =============================================================================

/// Validates: Requirements 1.8
/// `flux fmt` with invalid syntax should exit 1 and print diagnostic to stderr.
#[test]
fn fmt_compile_error_exits_1_with_diagnostic() {
    let invalid = "strategy {";
    let path = temp_flux_file(invalid, "compileerr");

    let output = flux_cmd()
        .arg("fmt")
        .arg(&path)
        .arg("--no-color")
        .output()
        .expect("failed to execute");

    let _ = fs::remove_file(&path);

    assert_eq!(
        output.status.code(),
        Some(1),
        "Expected exit code 1 for compile error"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("error[") || stderr.contains("error:"),
        "Expected error diagnostic in stderr, got: {:?}",
        stderr
    );
}
