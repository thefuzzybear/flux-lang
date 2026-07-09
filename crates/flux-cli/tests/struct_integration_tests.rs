//! End-to-end integration tests for struct support in the Flux compiler pipeline.
//!
//! Task 19.1 — Validates requirements:
//! 17.1, 17.2, 17.3, 17.4, 17.5, 18.1, 18.2, 18.3, 18.4, 18.5,
//! 19.1, 19.2, 19.3, 20.1, 20.2, 20.3, 20.4
//!
//! Tests:
//! 1. Full pipeline (lex → parse → typecheck → codegen) on a strategy with structs and decorators
//! 2. Interpreter run on a strategy that constructs struct literals, accesses fields, passes structs to functions
//! 3. `flux check` on a file with struct errors produces correct diagnostics
//! 4. `flux fmt` on a file with struct definitions produces correctly formatted output

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
// Test 1: Full compile pipeline — structs with decorators produce valid Rust output
// =============================================================================

/// Validates: Requirements 17.1, 17.2, 17.3, 17.4, 17.5, 20.1, 20.2, 20.3
/// Full pipeline (lex → parse → typecheck → codegen) on a strategy using stdlib
/// structs and decorators produces valid Rust output with struct declarations and
/// field access code.
#[test]
fn full_pipeline_struct_codegen_produces_valid_rust() {
    let output = flux_cmd()
        .arg("build")
        .arg(fixture_path("struct_strategy.flux"))
        .output()
        .expect("failed to execute");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(
        output.status.code(),
        Some(0),
        "Expected exit 0 for struct strategy build, stderr: {}",
        stderr
    );

    // Verify the output contains Rust struct declarations
    assert!(
        stdout.contains("#[derive(Clone, Copy)]"),
        "Expected #[derive(Clone, Copy)] in codegen output, got: {:?}",
        &stdout[..stdout.len().min(500)]
    );

    // Verify struct definitions are emitted
    assert!(
        stdout.contains("pub struct Quote"),
        "Expected 'pub struct Quote' in codegen output"
    );

    // Verify field access code is present (e.g., accessing .bid or .ask)
    assert!(
        stdout.contains(".bid") || stdout.contains(".ask") || stdout.contains(".price"),
        "Expected field access expressions in codegen output"
    );

    // Verify struct literal construction syntax is present
    assert!(
        stdout.contains("Quote {") || stdout.contains("Quote{"),
        "Expected struct literal construction in codegen output"
    );
}

/// Validates: Requirements 17.5
/// Structs are emitted in dependency order — Quote must appear before MarketSnapshot
/// since MarketSnapshot contains a Quote field.
#[test]
fn full_pipeline_struct_dependency_order() {
    let output = flux_cmd()
        .arg("build")
        .arg(fixture_path("struct_strategy.flux"))
        .output()
        .expect("failed to execute");

    let stdout = String::from_utf8_lossy(&output.stdout);

    assert_eq!(
        output.status.code(),
        Some(0),
        "Expected exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Quote must be emitted before any struct that references it
    if stdout.contains("pub struct MarketSnapshot") {
        let quote_pos = stdout.find("pub struct Quote").expect("Quote should be in output");
        let snapshot_pos = stdout
            .find("pub struct MarketSnapshot")
            .expect("MarketSnapshot should be in output");
        assert!(
            quote_pos < snapshot_pos,
            "Quote should be emitted before MarketSnapshot (dependency order)"
        );
    }
}

// =============================================================================
// Test 2: Interpreter — struct construction, field access, and function passing
// =============================================================================

/// Validates: Requirements 18.1, 18.2, 18.3, 18.4, 18.5
/// The interpreter evaluates struct literals, accesses fields, and passes structs
/// to functions correctly during a backtest run.
#[test]
fn interpreter_struct_construction_and_field_access() {
    let output = flux_cmd()
        .arg("backtest")
        .arg(fixture_path("struct_strategy.flux"))
        .arg("--data")
        .arg(fixture_path("sample_data.csv"))
        .arg("--capital")
        .arg("10000")
        .output()
        .expect("failed to execute");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(
        output.status.code(),
        Some(0),
        "Expected exit 0 for struct backtest, stderr: {}",
        stderr
    );

    // The strategy should complete and produce a summary (signals or portfolio)
    assert!(
        stdout.contains("Summary") || stdout.contains("Signals"),
        "Expected backtest output with Summary or Signals section, got: {:?}",
        stdout
    );
}

// =============================================================================
// Test 3: `flux check` with struct errors produces correct diagnostics
// =============================================================================

/// Validates: Requirements 20.4
/// `flux check` on a file with struct errors (missing fields, type mismatches)
/// produces correct error diagnostics.
#[test]
fn check_struct_errors_produces_diagnostics() {
    let output = flux_cmd()
        .arg("check")
        .arg(fixture_path("struct_errors.flux"))
        .output()
        .expect("failed to execute");

    assert_eq!(
        output.status.code(),
        Some(1),
        "Expected exit 1 for struct errors, stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    // Should contain error formatting
    assert!(
        stderr.contains("error["),
        "Expected formatted error diagnostics in stderr, got: {:?}",
        stderr
    );
}

/// Validates: Requirements 2.3, 2.4
/// Using a struct literal with missing fields should produce a clear error message.
#[test]
fn check_struct_missing_fields_error() {
    let output = flux_cmd()
        .arg("check")
        .arg(fixture_path("struct_missing_fields.flux"))
        .output()
        .expect("failed to execute");

    assert_eq!(
        output.status.code(),
        Some(1),
        "Expected exit 1 for missing fields error"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("error[") || stderr.contains("missing"),
        "Expected error about missing fields, got stderr: {:?}",
        stderr
    );
}

// =============================================================================
// Test 4: `flux fmt` on struct definitions produces correctly formatted output
// =============================================================================

/// Validates: Requirements 19.1, 19.2
/// `flux fmt` on a file with struct definitions produces correctly formatted output
/// with one field per line and consistent indentation.
#[test]
fn fmt_struct_definitions_formatted_correctly() {
    let output = flux_cmd()
        .arg("fmt")
        .arg("--no-color")
        .arg(fixture_path("struct_format_test.flux"))
        .output()
        .expect("failed to execute");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(
        output.status.code(),
        Some(0),
        "Expected exit 0 for fmt on struct file, stderr: {}",
        stderr
    );

    // Verify struct definition is formatted with fields on separate lines
    assert!(
        stdout.contains("struct Quote {"),
        "Expected 'struct Quote {{' in formatted output, got: {:?}",
        stdout
    );

    // Verify consistent indentation (4 spaces) for fields
    assert!(
        stdout.contains("    bid: f64"),
        "Expected indented field 'bid: f64' in formatted output, got: {:?}",
        stdout
    );
}

/// Validates: Requirements 19.3
/// Formatting a struct file, then formatting the output again, produces identical
/// output (idempotent round-trip). This verifies parse → format → parse round-trip.
#[test]
fn fmt_struct_round_trip_idempotent() {
    // First format the file
    let fmt_output = flux_cmd()
        .arg("fmt")
        .arg("--no-color")
        .arg(fixture_path("struct_format_test.flux"))
        .output()
        .expect("failed to execute fmt");

    assert_eq!(
        fmt_output.status.code(),
        Some(0),
        "Expected exit 0 for first fmt, stderr: {}",
        String::from_utf8_lossy(&fmt_output.stderr)
    );

    let formatted = String::from_utf8_lossy(&fmt_output.stdout);

    // Write formatted output to a temp file and format it again
    let temp_dir = std::env::temp_dir();
    let temp_file = temp_dir.join("flux_struct_roundtrip_test.flux");
    std::fs::write(&temp_file, formatted.as_ref()).expect("Failed to write temp file");

    let fmt_output2 = flux_cmd()
        .arg("fmt")
        .arg("--no-color")
        .arg(&temp_file)
        .output()
        .expect("failed to execute second fmt");

    let _ = std::fs::remove_file(&temp_file);

    assert_eq!(
        fmt_output2.status.code(),
        Some(0),
        "Expected exit 0 for second fmt, stderr: {}",
        String::from_utf8_lossy(&fmt_output2.stderr)
    );

    let formatted2 = String::from_utf8_lossy(&fmt_output2.stdout);

    // The second format should produce identical output (idempotent)
    assert_eq!(
        formatted, formatted2,
        "Formatting should be idempotent — format(format(x)) == format(x)"
    );
}

// =============================================================================
// Test 5: Struct type checking with stdlib imports
// =============================================================================

/// Validates: Requirements 20.1, 20.2, 20.3
/// `flux check` passes on a strategy that imports stdlib structs and uses them
/// with helper functions.
#[test]
fn check_stdlib_struct_imports_succeed() {
    let output = flux_cmd()
        .arg("check")
        .arg(fixture_path("struct_strategy.flux"))
        .output()
        .expect("failed to execute");

    assert_eq!(
        output.status.code(),
        Some(0),
        "Expected exit 0 for stdlib struct import check, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("ok"),
        "Expected 'ok' in check output, got: {:?}",
        stdout
    );
}
