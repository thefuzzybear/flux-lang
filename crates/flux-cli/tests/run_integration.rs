//! Integration tests for the `flux run` command and data block support
//! in `flux check` and `flux fmt`.
//!
//! These tests invoke the built binary via `std::process::Command` and verify
//! exit codes, stdout, and stderr output for the run harness scenarios.
//!
//! **Validates: Requirements 3.8, 3.9, 5.2, 6.2, 6.3, 6.4, 9.3, 9.4**

use std::fs;
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

/// Create a temporary .flux file with given content and return its path.
fn temp_flux_file(content: &str, suffix: &str) -> PathBuf {
    let dir = std::env::temp_dir();
    let id = std::process::id();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    let path = dir.join(format!("flux_run_test_{}_{}_{}.flux", id, ts, suffix));
    fs::write(&path, content).expect("failed to write temp file");
    path
}

// =============================================================================
// flux run: Valid strategy + data block compiles and invokes pipeline
// =============================================================================

/// Validates: Requirements 3.8, 5.2
/// `flux run` with a valid strategy + data block should get past compilation
/// successfully. Since this would try to fetch real data, we verify the
/// compilation and fetching phases are reached by checking stderr output.
/// The test may fail at the fetch phase (network) but the key assertion is
/// that compilation succeeds and "Fetching" appears in stderr.
#[test]
fn run_valid_strategy_with_data_block_compiles_and_reaches_fetch() {
    let output = flux_cmd()
        .arg("run")
        .arg(fixture_path("valid_data_block_strategy.flux"))
        .output()
        .expect("failed to execute");

    let stderr = String::from_utf8_lossy(&output.stderr);

    // The run command should print "Compiling" to stderr
    assert!(
        stderr.contains("Compiling"),
        "Expected 'Compiling' in stderr, got: {:?}",
        stderr
    );

    // After successful compilation, it should attempt to fetch
    // (may fail due to network, but "Fetching" proves compile passed)
    assert!(
        stderr.contains("Fetching"),
        "Expected 'Fetching' in stderr (proves compilation succeeded), got: {:?}",
        stderr
    );

    // Should NOT contain any compile error
    assert!(
        !stderr.contains("error["),
        "Should not contain compile errors, got: {:?}",
        stderr
    );
}

// =============================================================================
// flux run: Invalid strategy shows compile error, exits before fetch
// =============================================================================

/// Validates: Requirements 6.2, 6.3
/// `flux run` with syntax errors should show compile error and exit
/// WITHOUT showing "Fetching" (proves fail-fast before network).
#[test]
fn run_invalid_strategy_shows_compile_error_before_fetch() {
    let output = flux_cmd()
        .arg("run")
        .arg(fixture_path("invalid_data_block_syntax.flux"))
        .output()
        .expect("failed to execute");

    assert_eq!(
        output.status.code(),
        Some(1),
        "Expected exit code 1 for compile error"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);

    // Should contain a compile error
    assert!(
        stderr.contains("error["),
        "Expected formatted compile error in stderr, got: {:?}",
        stderr
    );

    // Should NOT contain "Fetching" — proves fail-fast before network
    assert!(
        !stderr.contains("Fetching"),
        "Should NOT reach fetch phase on compile error, got: {:?}",
        stderr
    );
}

// =============================================================================
// flux run: No data block + no --symbols = usage error
// =============================================================================

/// Validates: Requirements 3.9
/// `flux run` with no data block and no --symbols flag should exit with
/// a usage error mentioning "no symbols".
#[test]
fn run_no_data_block_no_symbols_shows_usage_error() {
    let output = flux_cmd()
        .arg("run")
        .arg(fixture_path("no_data_block_strategy.flux"))
        .output()
        .expect("failed to execute");

    assert_eq!(
        output.status.code(),
        Some(2),
        "Expected exit code 2 (usage error) when no symbols available, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);

    // Should mention "no symbols" in the error
    assert!(
        stderr.contains("no symbols"),
        "Expected 'no symbols' in usage error, got: {:?}",
        stderr
    );
}

// =============================================================================
// flux run: CLI overrides take precedence over data block values
// =============================================================================

/// Validates: Requirements 3.8, 3.9
/// `flux run` with --symbols override should use CLI symbols instead of
/// data block symbols. We verify the override is applied by checking
/// the "Fetching" line mentions the overridden symbols.
#[test]
fn run_cli_overrides_take_precedence_over_data_block() {
    let output = flux_cmd()
        .arg("run")
        .arg(fixture_path("data_block_override_test.flux"))
        .arg("--symbols")
        .arg("MSFT,GOOG")
        .arg("--period")
        .arg("3mo")
        .output()
        .expect("failed to execute");

    let stderr = String::from_utf8_lossy(&output.stderr);

    // Compilation should succeed
    assert!(
        stderr.contains("Compiling"),
        "Expected 'Compiling' in stderr, got: {:?}",
        stderr
    );

    // The Fetching line should show CLI-overridden symbols (MSFT, GOOG)
    // not the data block symbols (AAPL)
    if stderr.contains("Fetching") {
        assert!(
            stderr.contains("MSFT") && stderr.contains("GOOG"),
            "Expected CLI override symbols MSFT,GOOG in Fetching output, got: {:?}",
            stderr
        );
        assert!(
            !stderr.contains("[AAPL]"),
            "Data block symbol AAPL should be overridden, got: {:?}",
            stderr
        );
        // Period override should show "3mo"
        assert!(
            stderr.contains("3mo"),
            "Expected CLI override period '3mo' in Fetching output, got: {:?}",
            stderr
        );
    }
}

// =============================================================================
// flux check: Reports data block type errors with source spans
// =============================================================================

/// Validates: Requirements 6.4, 9.3
/// `flux check` with an invalid period in the data block should report
/// the error with valid options listed.
#[test]
fn check_reports_data_block_type_errors_with_source_spans() {
    let output = flux_cmd()
        .arg("check")
        .arg(fixture_path("data_block_type_error.flux"))
        .output()
        .expect("failed to execute");

    assert_eq!(
        output.status.code(),
        Some(1),
        "Expected exit code 1 for data block type error"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);

    // Should contain a formatted error with error[ prefix
    assert!(
        stderr.contains("error["),
        "Expected formatted error in stderr, got: {:?}",
        stderr
    );

    // Should mention invalid period
    assert!(
        stderr.contains("invalid period") || stderr.contains("2y2"),
        "Expected error about invalid period '2y2', got: {:?}",
        stderr
    );

    // Should list valid options
    assert!(
        stderr.contains("1y") && stderr.contains("6mo") && stderr.contains("1d"),
        "Expected valid period options in error message, got: {:?}",
        stderr
    );
}

// =============================================================================
// flux fmt: Formats data blocks idempotently
// =============================================================================

/// Validates: Requirements 9.4
/// `flux fmt` should properly format a messy data block, and the result
/// should be idempotent on second formatting.
#[test]
fn fmt_formats_data_blocks_idempotently() {
    // Copy the messy fixture to a temp file for --write mode
    let messy_content = fs::read_to_string(fixture_path("messy_data_block.flux"))
        .expect("failed to read messy fixture");
    let path = temp_flux_file(&messy_content, "fmt_data_block");

    // First format: --write to reformat in place
    let output = flux_cmd()
        .arg("fmt")
        .arg(&path)
        .arg("--write")
        .output()
        .expect("failed to execute");

    assert_eq!(
        output.status.code(),
        Some(0),
        "Expected exit code 0 for fmt --write, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Read the formatted content
    let formatted = fs::read_to_string(&path).expect("failed to read formatted file");

    // Verify data block is properly formatted:
    // - "data {" on its own line
    // - Fields indented with 4 spaces
    // - Proper spacing around = and in lists
    assert!(
        formatted.contains("data {\n"),
        "Expected 'data {{' on its own line, got:\n{}",
        formatted
    );
    assert!(
        formatted.contains("    symbols = [\"AAPL\", \"MSFT\"]"),
        "Expected properly formatted symbols list, got:\n{}",
        formatted
    );
    assert!(
        formatted.contains("    period = \"1y\""),
        "Expected properly formatted period, got:\n{}",
        formatted
    );
    assert!(
        formatted.contains("    interval = \"1d\""),
        "Expected properly formatted interval, got:\n{}",
        formatted
    );
    assert!(
        formatted.contains("    source = \"yahoo\""),
        "Expected properly formatted source, got:\n{}",
        formatted
    );

    // Second format: verify idempotence with --check
    let output2 = flux_cmd()
        .arg("fmt")
        .arg(&path)
        .arg("--check")
        .output()
        .expect("failed to execute");

    assert_eq!(
        output2.status.code(),
        Some(0),
        "Expected exit code 0 for --check on already-formatted file (idempotence), stderr: {}",
        String::from_utf8_lossy(&output2.stderr)
    );

    // Clean up
    let _ = fs::remove_file(&path);
}

/// Validates: Requirements 9.4
/// `flux fmt --no-color` on a file with a data block should produce
/// formatted output to stdout that includes the data block.
#[test]
fn fmt_data_block_output_includes_formatted_block() {
    let output = flux_cmd()
        .arg("fmt")
        .arg(fixture_path("valid_data_block_strategy.flux"))
        .arg("--no-color")
        .output()
        .expect("failed to execute");

    assert_eq!(
        output.status.code(),
        Some(0),
        "Expected exit code 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Formatted output should contain the data block
    assert!(
        stdout.contains("data {"),
        "Expected 'data {{' in formatted output, got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("symbols = [\"AAPL\"]"),
        "Expected symbols in formatted output"
    );
    assert!(
        stdout.contains("strategy SimpleData"),
        "Expected strategy in formatted output"
    );
}
