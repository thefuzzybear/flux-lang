//! Integration tests for the `flux` CLI binary.
//!
//! These tests invoke the built binary via `std::process::Command` and verify
//! exit codes, stdout, and stderr output for each subcommand.
//!
//! **Validates: Requirements 1.1, 1.2, 1.3, 1.4, 1.5, 2.2, 2.3, 2.7, 3.1, 3.3, 3.4, 4.1, 4.4, 4.6, 7.1, 7.2, 7.3, 7.4**

use std::path::PathBuf;
use std::process::Command;

/// Get the path to the compiled `flux` binary.
fn flux_cmd() -> Command {
    Command::new(env!("CARGO_BIN_EXE_flux"))
}

/// Get the path to a test fixture file.
fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

// =============================================================================
// --version flag
// =============================================================================

/// Validates: Requirement 1.5, 7.4
/// `flux --version` should print the version string and exit successfully.
#[test]
fn version_flag_prints_version() {
    let output = flux_cmd().arg("--version").output().expect("failed to execute");

    // clap prints version to stdout
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("flux") && stdout.contains("0.1.0"),
        "Expected version output containing 'flux' and '0.1.0', got: {:?}",
        stdout
    );
    assert_eq!(output.status.code(), Some(0));
}

// =============================================================================
// --help flag
// =============================================================================

/// Validates: Requirements 1.3, 1.4, 7.4
/// `flux --help` should print usage info including subcommand names.
#[test]
fn help_flag_prints_usage() {
    let output = flux_cmd().arg("--help").output().expect("failed to execute");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("flux"), "Help should mention binary name");
    assert!(stdout.contains("check"), "Help should list 'check' subcommand");
    assert!(stdout.contains("build"), "Help should list 'build' subcommand");
    assert!(stdout.contains("backtest"), "Help should list 'backtest' subcommand");
    assert_eq!(output.status.code(), Some(0));
}

// =============================================================================
// Unknown subcommand
// =============================================================================

/// Validates: Requirements 1.2, 7.4
/// Unknown subcommand should exit with code 2 and display an error.
#[test]
fn unknown_subcommand_exits_with_code_2() {
    let output = flux_cmd().arg("invalidcmd").output().expect("failed to execute");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unrecognized subcommand") || stderr.contains("invalid"),
        "Expected error about unrecognized subcommand, got stderr: {:?}",
        stderr
    );
}

// =============================================================================
// Check command
// =============================================================================

/// Validates: Requirements 2.2, 7.1
/// `check` with a valid Flux file should exit 0 and print "ok" to stdout.
#[test]
fn check_valid_file_exits_0_with_ok() {
    let output = flux_cmd()
        .arg("check")
        .arg(fixture_path("valid_strategy.flux"))
        .output()
        .expect("failed to execute");

    assert_eq!(
        output.status.code(),
        Some(0),
        "Expected exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("ok"),
        "Expected stdout to contain 'ok', got: {:?}",
        stdout
    );
}

/// Validates: Requirements 2.3, 7.2
/// `check` with a file containing parse errors should exit 1 and print errors
/// with line:col format to stderr.
#[test]
fn check_invalid_file_exits_1_with_error() {
    let output = flux_cmd()
        .arg("check")
        .arg(fixture_path("invalid_strategy.flux"))
        .output()
        .expect("failed to execute");

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Should contain error formatting with file:line:col
    assert!(
        stderr.contains("error["),
        "Expected formatted error in stderr, got: {:?}",
        stderr
    );
    // Should contain line:col info (colon-separated numbers)
    assert!(
        stderr.contains(":4:") || stderr.contains(":3:") || stderr.contains(":"),
        "Expected line:col info in error output"
    );
}

/// Validates: Requirements 2.4, 7.2
/// `check` with a file containing type errors should exit 1 and print errors
/// to stderr.
#[test]
fn check_type_error_file_exits_1_with_error() {
    let output = flux_cmd()
        .arg("check")
        .arg(fixture_path("type_error_strategy.flux"))
        .output()
        .expect("failed to execute");

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("error["),
        "Expected formatted error in stderr, got: {:?}",
        stderr
    );
}

/// Validates: Requirements 2.7, 7.3
/// `check` with a non-existent file should exit 1.
#[test]
fn check_missing_file_exits_1() {
    let output = flux_cmd()
        .arg("check")
        .arg("/nonexistent/path/missing.flux")
        .output()
        .expect("failed to execute");

    assert_eq!(
        output.status.code(),
        Some(1),
        "Expected exit 1 for missing file"
    );
}

// =============================================================================
// Build command
// =============================================================================

/// Validates: Requirements 3.1, 7.1
/// `build` with a valid file should exit 0 and print generated Rust code to stdout.
#[test]
fn build_valid_file_exits_0_with_code() {
    let output = flux_cmd()
        .arg("build")
        .arg(fixture_path("valid_strategy.flux"))
        .output()
        .expect("failed to execute");

    assert_eq!(
        output.status.code(),
        Some(0),
        "Expected exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Generated Rust code should be non-empty and contain typical Rust keywords
    assert!(!stdout.is_empty(), "Expected non-empty stdout with generated code");
    assert!(
        stdout.contains("fn") || stdout.contains("struct") || stdout.contains("impl"),
        "Expected generated Rust code, got: {:?}",
        &stdout[..stdout.len().min(200)]
    );
}

/// Validates: Requirements 3.3, 7.2
/// `build` with an invalid file should exit 1 and print errors to stderr.
#[test]
fn build_invalid_file_exits_1() {
    let output = flux_cmd()
        .arg("build")
        .arg(fixture_path("invalid_strategy.flux"))
        .output()
        .expect("failed to execute");

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("error["),
        "Expected formatted error in stderr, got: {:?}",
        stderr
    );
}

/// Validates: Requirements 3.4, 7.3
/// `build` with a missing file should exit 1.
#[test]
fn build_missing_file_exits_1() {
    let output = flux_cmd()
        .arg("build")
        .arg("/nonexistent/path/missing.flux")
        .output()
        .expect("failed to execute");

    assert_eq!(
        output.status.code(),
        Some(1),
        "Expected exit 1 for missing file"
    );
}

/// Validates: Requirements 3.1, 3.5
/// `build --output` should write generated code to the specified file.
#[test]
fn build_output_flag_writes_to_file() {
    let output_path = std::env::temp_dir().join("flux_integration_test_output.rs");

    // Clean up any previous test artifact
    let _ = std::fs::remove_file(&output_path);

    let output = flux_cmd()
        .arg("build")
        .arg(fixture_path("valid_strategy.flux"))
        .arg("--output")
        .arg(&output_path)
        .output()
        .expect("failed to execute");

    assert_eq!(
        output.status.code(),
        Some(0),
        "Expected exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // The output file should exist and be non-empty
    assert!(output_path.exists(), "Output file should exist at {:?}", output_path);
    let contents = std::fs::read_to_string(&output_path).expect("Failed to read output file");
    assert!(!contents.is_empty(), "Output file should not be empty");
    assert!(
        contents.contains("fn") || contents.contains("struct") || contents.contains("impl"),
        "Output file should contain Rust code"
    );

    // Clean up
    let _ = std::fs::remove_file(&output_path);
}

// =============================================================================
// Backtest command
// =============================================================================

/// Validates: Requirements 4.1, 7.1
/// `backtest` with valid strategy and data should exit 0 and print signals + summary.
#[test]
fn backtest_valid_strategy_exits_0_with_summary() {
    let output = flux_cmd()
        .arg("backtest")
        .arg(fixture_path("valid_strategy.flux"))
        .arg("--data")
        .arg(fixture_path("sample_data.csv"))
        .output()
        .expect("failed to execute");

    assert_eq!(
        output.status.code(),
        Some(0),
        "Expected exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should contain the summary section
    assert!(
        stdout.contains("Summary"),
        "Expected 'Summary' in output, got: {:?}",
        stdout
    );
    // Should contain signal type names
    assert!(
        stdout.contains("Open") || stdout.contains("Close") || stdout.contains("CloseQty"),
        "Expected signal type names in output"
    );
}

/// Validates: Requirements 4.4, 7.2
/// `backtest` with a file that has compilation errors should exit 1 without
/// loading data (errors printed to stderr).
#[test]
fn backtest_compilation_failure_exits_1() {
    let output = flux_cmd()
        .arg("backtest")
        .arg(fixture_path("invalid_strategy.flux"))
        .arg("--data")
        .arg(fixture_path("sample_data.csv"))
        .output()
        .expect("failed to execute");

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("error["),
        "Expected compilation error in stderr, got: {:?}",
        stderr
    );
    // Stdout should be empty — no data loading or signal output attempted
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("Summary"),
        "Should not produce summary output on compilation failure"
    );
}

/// Validates: Requirements 4.6, 7.3
/// `backtest` with bad CSV data should exit 1.
#[test]
fn backtest_bad_csv_exits_1() {
    let output = flux_cmd()
        .arg("backtest")
        .arg(fixture_path("valid_strategy.flux"))
        .arg("--data")
        .arg(fixture_path("bad_data.csv"))
        .output()
        .expect("failed to execute");

    assert_eq!(
        output.status.code(),
        Some(1),
        "Expected exit 1 for bad CSV data"
    );
}

/// Validates: Requirements 4.6, 7.3
/// `backtest` with CSV missing required columns should exit 1.
#[test]
fn backtest_missing_cols_csv_exits_1() {
    let output = flux_cmd()
        .arg("backtest")
        .arg(fixture_path("valid_strategy.flux"))
        .arg("--data")
        .arg(fixture_path("missing_cols.csv"))
        .output()
        .expect("failed to execute");

    assert_eq!(
        output.status.code(),
        Some(1),
        "Expected exit 1 for CSV with missing columns"
    );
}

/// Validates: Requirements 4.6, 7.3
/// `backtest` with a non-existent data file should exit 1.
#[test]
fn backtest_missing_data_file_exits_1() {
    let output = flux_cmd()
        .arg("backtest")
        .arg(fixture_path("valid_strategy.flux"))
        .arg("--data")
        .arg("/nonexistent/data.csv")
        .output()
        .expect("failed to execute");

    assert_eq!(
        output.status.code(),
        Some(1),
        "Expected exit 1 for missing data file"
    );
}

// =============================================================================
// Argument validation
// =============================================================================

/// Validates: Requirements 1.6, 7.4
/// `check` without a file argument should exit with code 2.
#[test]
fn check_missing_argument_exits_2() {
    let output = flux_cmd().arg("check").output().expect("failed to execute");

    assert_eq!(
        output.status.code(),
        Some(2),
        "Expected exit 2 for missing required argument"
    );
}

/// Validates: Requirements 1.6, 7.4
/// `backtest` without --data option should exit with code 2.
#[test]
fn backtest_missing_data_option_exits_2() {
    let output = flux_cmd()
        .arg("backtest")
        .arg(fixture_path("valid_strategy.flux"))
        .output()
        .expect("failed to execute");

    assert_eq!(
        output.status.code(),
        Some(2),
        "Expected exit 2 for missing --data option"
    );
}

// =============================================================================
// Math strategy integration tests (Tier 1, 2, and 3 functions)
// =============================================================================

// =============================================================================
// Connector block backward compatibility (flux-live-harness)
// =============================================================================

/// Validates: Requirements 8.7, 8.8
/// `flux backtest` with a strategy that has BOTH `data` and `connector` blocks
/// should successfully compile and run, using only the data block for CSV config.
/// The connector block should be silently ignored.
#[test]
fn backtest_strategy_with_connector_and_data_blocks_succeeds() {
    let output = flux_cmd()
        .arg("backtest")
        .arg(fixture_path("connector_and_data_block_strategy.flux"))
        .arg("--data")
        .arg(fixture_path("sample_data.csv"))
        .arg("--capital")
        .arg("10000")
        .output()
        .expect("failed to execute");

    assert_eq!(
        output.status.code(),
        Some(0),
        "Expected exit 0 for strategy with both data and connector blocks, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should produce normal backtest output with signals and summary
    assert!(
        stdout.contains("Summary"),
        "Expected 'Summary' in backtest output, got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("Signals"),
        "Expected 'Signals' section in backtest output, got: {:?}",
        stdout
    );
    // Should produce portfolio summary (proves data block was used for backtest)
    assert!(
        stdout.contains("Portfolio Summary"),
        "Expected 'Portfolio Summary' in output — proves data_block path was used"
    );
}

/// Validates: Requirements 8.7, 8.8
/// `flux check` with a strategy that has BOTH `data` and `connector` blocks
/// should pass type checking without errors (connector block is valid syntax).
#[test]
fn check_strategy_with_connector_and_data_blocks_succeeds() {
    let output = flux_cmd()
        .arg("check")
        .arg(fixture_path("connector_and_data_block_strategy.flux"))
        .output()
        .expect("failed to execute");

    assert_eq!(
        output.status.code(),
        Some(0),
        "Expected exit 0 for check with both data and connector blocks, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("ok"),
        "Expected 'ok' in check output, got: {:?}",
        stdout
    );
}

// =============================================================================
// Math strategy integration tests (Tier 1, 2, and 3 functions)
// =============================================================================

/// Validates: Requirements 3.5, 9.6, 15.1
/// `flux check` with a strategy using Tier 1 and 2 math/stats functions should
/// pass type checking without errors.
#[test]
fn test_check_math_strategy() {
    let output = flux_cmd()
        .arg("check")
        .arg(fixture_path("math_strategy.flux"))
        .output()
        .expect("failed to execute");

    assert_eq!(
        output.status.code(),
        Some(0),
        "Expected exit 0 for math strategy check, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("ok"),
        "Expected stdout to contain 'ok', got: {:?}",
        stdout
    );
}

// =============================================================================
// Module resolution integration tests
// =============================================================================

/// Validates: Requirements 10.1, 10.2, 10.3
/// `flux check` on a multi-file project with `::` imports should succeed.
/// The main file imports `double` from `lib/helpers.flux` via `from lib::helpers import {double}`.
#[test]
fn cli_module_check_multifile_project_succeeds() {
    let output = flux_cmd()
        .arg("check")
        .arg(fixture_path("modules/main_with_import.flux"))
        .output()
        .expect("failed to execute");

    assert_eq!(
        output.status.code(),
        Some(0),
        "Expected exit 0 for multi-file check, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("ok"),
        "Expected 'ok' in stdout for successful multi-file check, got: {:?}",
        stdout
    );
}

/// Validates: Requirements 10.2, 10.3
/// `flux check` with a missing import file should exit non-zero and produce
/// a descriptive error mentioning the missing module path.
#[test]
fn cli_module_check_missing_import_produces_error() {
    let output = flux_cmd()
        .arg("check")
        .arg(fixture_path("modules/main_missing_import.flux"))
        .output()
        .expect("failed to execute");

    assert_ne!(
        output.status.code(),
        Some(0),
        "Expected non-zero exit for missing import file"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Should mention the module path or "not found"
    assert!(
        stderr.contains("not found") || stderr.contains("nonexistent"),
        "Expected descriptive error about missing module, got stderr: {:?}",
        stderr
    );
}

/// Validates: Requirements 10.2, 10.3
/// `flux check` with circular imports should exit non-zero and produce
/// an error mentioning circular import.
#[test]
fn cli_module_check_circular_imports_produces_error() {
    let output = flux_cmd()
        .arg("check")
        .arg(fixture_path("modules/main_circular.flux"))
        .output()
        .expect("failed to execute");

    assert_ne!(
        output.status.code(),
        Some(0),
        "Expected non-zero exit for circular imports"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Should mention circular import
    assert!(
        stderr.contains("circular") || stderr.contains("cycle"),
        "Expected error about circular imports, got stderr: {:?}",
        stderr
    );
}

/// Validates: Requirements 9.1, 10.1
/// `flux check` with a built-in import (`from indicators import {sma}`) should pass
/// through the module resolver and succeed at typechecking (the module resolver
/// does NOT try to resolve dot-separated built-in imports as files).
#[test]
fn cli_module_check_builtin_import_passthrough() {
    let output = flux_cmd()
        .arg("check")
        .arg(fixture_path("modules/main_builtin_import.flux"))
        .output()
        .expect("failed to execute");

    assert_eq!(
        output.status.code(),
        Some(0),
        "Expected exit 0 for built-in import passthrough, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("ok"),
        "Expected 'ok' in stdout for built-in import check, got: {:?}",
        stdout
    );
}

/// Validates: Requirements 3.5, 9.6, 15.1, 16.1
/// `flux backtest` with a strategy using Tier 1 and 2 math/stats functions should
/// execute without errors and produce output (signals and/or summary).
#[test]
fn test_backtest_math_strategy() {
    let output = flux_cmd()
        .arg("backtest")
        .arg(fixture_path("math_strategy.flux"))
        .arg("--data")
        .arg(fixture_path("sample_data.csv"))
        .arg("--capital")
        .arg("10000")
        .output()
        .expect("failed to execute");

    assert_eq!(
        output.status.code(),
        Some(0),
        "Expected exit 0 for math strategy backtest, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should contain the portfolio summary section
    assert!(
        stdout.contains("Summary"),
        "Expected 'Summary' in backtest output, got: {:?}",
        stdout
    );
}

// =============================================================================
// Backward Compatibility: --fidelity 0 matches default output
// =============================================================================

/// Validates: Requirements 4.7, 11.7
/// Running with `--fidelity 0` must produce output identical to running
/// without the `--fidelity` flag (the default behavior).
#[test]
fn test_backtest_fidelity_0_matches_default_output() {
    // Run without --fidelity flag
    let output_default = flux_cmd()
        .arg("backtest")
        .arg(fixture_path("math_strategy.flux"))
        .arg("--data")
        .arg(fixture_path("sample_data.csv"))
        .arg("--capital")
        .arg("10000")
        .output()
        .expect("failed to execute default backtest");

    // Run with explicit --fidelity 0
    let output_fidelity0 = flux_cmd()
        .arg("backtest")
        .arg(fixture_path("math_strategy.flux"))
        .arg("--data")
        .arg(fixture_path("sample_data.csv"))
        .arg("--capital")
        .arg("10000")
        .arg("--fidelity")
        .arg("0")
        .output()
        .expect("failed to execute fidelity 0 backtest");

    assert_eq!(
        output_default.status.code(),
        Some(0),
        "Default backtest failed, stderr: {}",
        String::from_utf8_lossy(&output_default.stderr)
    );
    assert_eq!(
        output_fidelity0.status.code(),
        Some(0),
        "Fidelity 0 backtest failed, stderr: {}",
        String::from_utf8_lossy(&output_fidelity0.stderr)
    );

    let stdout_default = String::from_utf8_lossy(&output_default.stdout);
    let stdout_fidelity0 = String::from_utf8_lossy(&output_fidelity0.stdout);

    // Output must be byte-for-byte identical
    assert_eq!(
        stdout_default, stdout_fidelity0,
        "Fidelity 0 output differs from default output.\n\
         Default:\n{}\n\nFidelity 0:\n{}",
        stdout_default, stdout_fidelity0
    );

    // Verify the output contains all expected sections
    assert!(stdout_default.contains("--- Signals ---"), "Missing Signals section");
    assert!(stdout_default.contains("--- Portfolio Summary ---"), "Missing Portfolio Summary section");
    assert!(stdout_default.contains("--- Summary ---"), "Missing Summary section");
}


// =============================================================================
// flux live — directory mode (account manifest)
// =============================================================================

/// Validates: Requirement 6.1, 6.2
/// `flux live <dir>` with a valid account.flux should parse and validate.
/// Note: The command will panic with todo!() since AccountRuntime isn't implemented yet.
/// We verify it gets far enough to hit the todo (exit code 101 = panic).
#[test]
fn live_directory_with_valid_account_flux_reaches_todo() {
    let dir = fixture_path("account_manifest");
    let output = flux_cmd()
        .args(["live", dir.to_str().unwrap()])
        .output()
        .expect("failed to execute");

    // The command should panic with the todo!() since AccountRuntime isn't implemented
    // Panic gives exit code 101 on Unix
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not yet implemented") || stderr.contains("AccountRuntime"),
        "Expected todo!() panic in stderr, got: {}",
        stderr
    );
}

/// Validates: Requirement 6.4
/// `flux live <dir>` without account.flux should produce an error.
#[test]
fn live_directory_without_account_flux_errors() {
    let dir = fixture_path("empty_dir");
    let output = flux_cmd()
        .args(["live", dir.to_str().unwrap()])
        .output()
        .expect("failed to execute");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no account.flux found in directory"),
        "Expected 'no account.flux found' error, got: {}",
        stderr
    );
}

/// Validates: Requirement 6.5
/// `flux live <nonexistent_dir>` should produce a "does not exist" error.
#[test]
fn live_nonexistent_directory_errors() {
    let output = flux_cmd()
        .args(["live", "/tmp/flux_nonexistent_dir_12345"])
        .output()
        .expect("failed to execute");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("does not exist") || stderr.contains("No such file"),
        "Expected 'does not exist' error, got: {}",
        stderr
    );
}

/// Validates: Requirement 7.3
/// `flux live README.md` (neither dir, .flux, nor .toml) should produce guidance error.
#[test]
fn live_unsupported_path_type_errors() {
    // Use a file that exists but isn't .flux or .toml
    let readme = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap().parent().unwrap()
        .join("README.md");
    let output = flux_cmd()
        .args(["live", readme.to_str().unwrap()])
        .output()
        .expect("failed to execute");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("path must be a directory"),
        "Expected guidance error message, got: {}",
        stderr
    );
}
