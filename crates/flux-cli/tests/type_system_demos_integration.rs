//! Integration tests for the type system demo strategies.
//!
//! These tests validate that all four demo strategies (pairs_trading, regime_detector,
//! order_book, live_connector) pass type-checking and execute successfully in backtest mode.
//! They also verify the demo directory structure and CSV data file requirements.
//!
//! **Validates: Requirements 6.3, 10.2, 15.2, 19.1, 19.2, 19.3, 19.4, 19.5, 19.6**

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The four type system demo names.
const DEMOS: &[&str] = &["pairs_trading", "regime_detector", "order_book", "live_connector"];

/// Get the workspace root (two levels up from CARGO_MANIFEST_DIR for flux-cli crate).
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("failed to get crates/ dir")
        .parent()
        .expect("failed to get workspace root")
        .to_path_buf()
}

/// Get the path to a demo directory.
fn demo_dir(name: &str) -> PathBuf {
    workspace_root().join("demos").join(name)
}

/// Get the path to a demo's strategy file.
fn strategy_path(name: &str) -> PathBuf {
    demo_dir(name).join("strategy.flux")
}

/// Get the path to a demo's data CSV file.
fn data_path(name: &str) -> PathBuf {
    demo_dir(name).join("data.csv")
}

/// Run `cargo run -p flux-cli -- <args>` and return the output.
fn run_flux(args: &[&str]) -> std::process::Output {
    Command::new("cargo")
        .args(["run", "-p", "flux-cli", "--"])
        .args(args)
        .current_dir(workspace_root())
        .output()
        .expect("failed to execute flux CLI")
}

// =============================================================================
// Directory Structure Tests
// =============================================================================

/// Validates: Requirement 19.1, 19.2, 19.3, 19.4
/// All four demo directories exist at the expected paths.
#[test]
fn test_demo_directories_exist() {
    for demo in DEMOS {
        let dir = demo_dir(demo);
        assert!(
            dir.exists() && dir.is_dir(),
            "Demo directory missing: {}",
            dir.display()
        );
    }
}

/// Validates: Requirement 19.1, 19.2, 19.3, 19.4
/// All four demo strategy files exist with correct naming.
#[test]
fn test_strategy_files_exist() {
    for demo in DEMOS {
        let path = strategy_path(demo);
        assert!(
            path.exists() && path.is_file(),
            "Strategy file missing: {}",
            path.display()
        );
    }
}

/// Validates: Requirement 19.5
/// All four demo CSV data files exist with correct naming (data.csv).
#[test]
fn test_csv_files_exist() {
    for demo in DEMOS {
        let path = data_path(demo);
        assert!(
            path.exists() && path.is_file(),
            "Data CSV file missing: {}",
            path.display()
        );
    }
}

/// Validates: Requirement 19.6
/// All four demo README files exist with correct naming (README.md).
#[test]
fn test_readme_files_exist() {
    for demo in DEMOS {
        let path = demo_dir(demo).join("README.md");
        assert!(
            path.exists() && path.is_file(),
            "README.md missing: {}",
            path.display()
        );
    }
}

// =============================================================================
// CSV Row Count Tests
// =============================================================================

/// Count data rows in a CSV file (total lines minus the header line).
fn csv_data_row_count(path: &Path) -> usize {
    let content = fs::read_to_string(path).expect("failed to read CSV file");
    let line_count = content.lines().count();
    // Subtract 1 for the header row
    if line_count > 0 {
        line_count - 1
    } else {
        0
    }
}

/// Validates: Requirements 6.1, 10.1, 15.1
/// All CSV data files have at least 100 data rows (excluding the header).
#[test]
fn test_csv_files_have_100_plus_rows() {
    for demo in DEMOS {
        let path = data_path(demo);
        let row_count = csv_data_row_count(&path);
        assert!(
            row_count >= 100,
            "CSV for '{}' has only {} data rows (expected at least 100): {}",
            demo,
            row_count,
            path.display()
        );
    }
}

// =============================================================================
// Type Check Tests (flux check)
// =============================================================================

/// Validates: Requirements 1.1, 2.1, 3.2, 4.1, 5.1, 7.1, 8.3, 11.3, 12.1, 13.1, 16.1, 17.1-17.5
/// All four demo strategy files pass `flux check` with exit code 0.
#[test]
fn test_all_demos_typecheck_cleanly() {
    for demo in DEMOS {
        let path = strategy_path(demo);
        let output = run_flux(&["check", path.to_str().unwrap()]);

        assert_eq!(
            output.status.code(),
            Some(0),
            "`flux check` failed for '{}', stderr: {}",
            demo,
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("ok"),
            "`flux check` for '{}' should print 'ok', got: {:?}",
            demo,
            stdout
        );
    }
}

/// Validates: Requirement 19.1
/// pairs_trading passes type-check independently.
#[test]
fn test_pairs_trading_typecheck() {
    let output = run_flux(&["check", strategy_path("pairs_trading").to_str().unwrap()]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "pairs_trading check failed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Validates: Requirement 19.2
/// regime_detector passes type-check independently.
#[test]
fn test_regime_detector_typecheck() {
    let output = run_flux(&["check", strategy_path("regime_detector").to_str().unwrap()]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "regime_detector check failed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Validates: Requirement 19.3
/// order_book passes type-check independently.
#[test]
fn test_order_book_typecheck() {
    let output = run_flux(&["check", strategy_path("order_book").to_str().unwrap()]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "order_book check failed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Validates: Requirement 19.4
/// live_connector passes type-check independently.
#[test]
fn test_live_connector_typecheck() {
    let output = run_flux(&["check", strategy_path("live_connector").to_str().unwrap()]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "live_connector check failed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// =============================================================================
// Backtest Execution Tests (flux backtest)
// =============================================================================

/// Validates: Requirements 6.3, 10.2, 15.2
/// All four demos execute successfully in backtest mode (exit code 0).
#[test]
fn test_all_demos_backtest_successfully() {
    for demo in DEMOS {
        let strat = strategy_path(demo);
        let data = data_path(demo);
        let output = run_flux(&[
            "backtest",
            strat.to_str().unwrap(),
            "--data",
            data.to_str().unwrap(),
            "--capital",
            "100000",
        ]);

        assert_eq!(
            output.status.code(),
            Some(0),
            "`flux backtest` failed for '{}', stderr: {}",
            demo,
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("Summary"),
            "Backtest for '{}' should produce Summary output, got: {:?}",
            demo,
            stdout
        );
    }
}

/// Validates: Requirement 6.3
/// pairs_trading backtest executes and produces output with signal summary.
#[test]
fn test_pairs_trading_backtest_produces_output() {
    let output = run_flux(&[
        "backtest",
        strategy_path("pairs_trading").to_str().unwrap(),
        "--data",
        data_path("pairs_trading").to_str().unwrap(),
        "--capital",
        "100000",
    ]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "pairs_trading backtest failed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should contain Portfolio Summary section
    assert!(
        stdout.contains("Portfolio Summary"),
        "Expected 'Portfolio Summary' in pairs_trading output"
    );
    // Should contain signal summary section
    assert!(
        stdout.contains("Summary"),
        "Expected 'Summary' section in pairs_trading output"
    );
}

/// Validates: Requirement 15.2
/// order_book backtest executes and produces output with signal summary.
#[test]
fn test_order_book_backtest_produces_output() {
    let output = run_flux(&[
        "backtest",
        strategy_path("order_book").to_str().unwrap(),
        "--data",
        data_path("order_book").to_str().unwrap(),
        "--capital",
        "100000",
    ]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "order_book backtest failed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Portfolio Summary"),
        "Expected 'Portfolio Summary' in order_book output"
    );
    assert!(
        stdout.contains("Summary"),
        "Expected 'Summary' section in order_book output"
    );
}

/// Validates: Requirement 10.2
/// regime_detector backtest runs successfully and produces output.
#[test]
fn test_regime_detector_backtest_runs() {
    let output = run_flux(&[
        "backtest",
        strategy_path("regime_detector").to_str().unwrap(),
        "--data",
        data_path("regime_detector").to_str().unwrap(),
        "--capital",
        "100000",
    ]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "regime_detector backtest failed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should produce portfolio summary output
    assert!(
        stdout.contains("Portfolio Summary"),
        "Expected 'Portfolio Summary' in regime_detector output"
    );
}

/// Validates: Requirement 16.1, 16.2
/// live_connector works in backtest mode via its data block.
#[test]
fn test_live_connector_backtest_mode() {
    let output = run_flux(&[
        "backtest",
        strategy_path("live_connector").to_str().unwrap(),
        "--data",
        data_path("live_connector").to_str().unwrap(),
        "--capital",
        "100000",
    ]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "live_connector backtest failed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Portfolio Summary"),
        "Expected 'Portfolio Summary' in live_connector backtest output"
    );
    assert!(
        stdout.contains("Summary"),
        "Expected signal 'Summary' section in live_connector output"
    );
}

// =============================================================================
// Signal Production Tests
// =============================================================================

/// Validates: Requirement 6.3
/// pairs_trading should produce signal-related output when backtested.
/// Note: Full signal production depends on interpreter support for struct constructors.
/// This test verifies the strategy runs end-to-end and produces the expected output structure.
#[test]
fn test_pairs_trading_signal_output_structure() {
    let output = run_flux(&[
        "backtest",
        strategy_path("pairs_trading").to_str().unwrap(),
        "--data",
        data_path("pairs_trading").to_str().unwrap(),
        "--capital",
        "100000",
    ]);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Verify the output contains all expected sections
    assert!(stdout.contains("Signals"), "Expected 'Signals' section header");
    assert!(
        stdout.contains("Total signals:"),
        "Expected 'Total signals:' in summary"
    );
    assert!(stdout.contains("Open:"), "Expected 'Open:' count in summary");
    assert!(stdout.contains("Close:"), "Expected 'Close:' count in summary");
}

/// Validates: Requirement 15.2
/// order_book should produce signal-related output when backtested.
#[test]
fn test_order_book_signal_output_structure() {
    let output = run_flux(&[
        "backtest",
        strategy_path("order_book").to_str().unwrap(),
        "--data",
        data_path("order_book").to_str().unwrap(),
        "--capital",
        "100000",
    ]);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Verify the output contains all expected sections
    assert!(stdout.contains("Signals"), "Expected 'Signals' section header");
    assert!(
        stdout.contains("Total signals:"),
        "Expected 'Total signals:' in summary"
    );
    assert!(stdout.contains("Open:"), "Expected 'Open:' count in summary");
    assert!(stdout.contains("Close:"), "Expected 'Close:' count in summary");
}

/// Validates: Requirement 10.2
/// regime_detector backtest produces output structure indicating execution completed.
#[test]
fn test_regime_detector_output_structure() {
    let output = run_flux(&[
        "backtest",
        strategy_path("regime_detector").to_str().unwrap(),
        "--data",
        data_path("regime_detector").to_str().unwrap(),
        "--capital",
        "100000",
    ]);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Verify the output has the complete backtest structure
    assert!(
        stdout.contains("Initial Capital:"),
        "Expected 'Initial Capital:' in portfolio output"
    );
    assert!(
        stdout.contains("Total signals:"),
        "Expected 'Total signals:' in summary output"
    );
}

// =============================================================================
// Complete Directory Structure Validation
// =============================================================================

/// Validates: Requirements 19.1-19.6
/// Each demo directory has exactly the expected files (strategy.flux, data.csv, README.md).
#[test]
fn test_demo_complete_file_structure() {
    let expected_files = ["strategy.flux", "data.csv", "README.md"];

    for demo in DEMOS {
        for file in &expected_files {
            let path = demo_dir(demo).join(file);
            assert!(
                path.exists(),
                "Missing file '{}' in demo '{}': {}",
                file,
                demo,
                path.display()
            );
        }
    }
}

// =============================================================================
// Trading Signal Production Tests (Interpreter Type System)
// =============================================================================

/// Validates: Requirement 9.1 (interpreter type system producing signals)
///
/// This test verifies that the pairs_trading demo strategy actually produces
/// OPEN or CLOSE trading signals during backtest — not just that it exits
/// successfully. A backtest that exits 0 with zero signals means the interpreter
/// is silently failing on some type system construct (structs, enums, match, etc.).
#[test]
fn test_pairs_trading_produces_trading_signals() {
    let output = run_flux(&[
        "backtest",
        strategy_path("pairs_trading").to_str().unwrap(),
        "--data",
        data_path("pairs_trading").to_str().unwrap(),
        "--capital",
        "100000",
    ]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "pairs_trading backtest exited with non-zero code, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);

    // The key assertion: the interpreter must have evaluated the type system
    // constructs (structs, enums, match expressions, HashMap ops) to actually
    // produce trading signals. If we get exit 0 but no Open/Close lines,
    // the interpreter is silently skipping type system AST nodes.
    let has_open_signal = stdout.lines().any(|line| line.contains(" Open "));
    let has_close_signal = stdout.lines().any(|line| line.contains(" Close "));

    assert!(
        has_open_signal || has_close_signal,
        "pairs_trading backtest produced no trading signals (no Open or Close in output).\n\
         This means the interpreter failed to evaluate type system constructs.\n\
         stdout:\n{}",
        stdout
    );
}

// =============================================================================
// Signal Production Integration Tests (interpreter-type-system spec)
// =============================================================================

/// Validates: Requirement 9.3
///
/// The order_book demo exercises nested structs and multi-field match destructuring
/// (OrderBook with best_bid/best_ask sub-structs). If this test fails, it likely means
/// nested struct field access (`book.best_bid.price`) or match with multiple bindings
/// isn't working in the interpreter.
#[test]
fn test_order_book_produces_trading_signals() {
    let strat = strategy_path("order_book");
    let data = data_path("order_book");
    let output = run_flux(&[
        "backtest",
        strat.to_str().unwrap(),
        "--data",
        data.to_str().unwrap(),
        "--capital",
        "100000",
    ]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "`flux backtest` failed for order_book, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);

    // The backtest must produce at least one Open or Close signal line,
    // confirming the interpreter handles nested struct field access and
    // multi-field match destructuring correctly.
    let has_signal = stdout.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.contains(" Open ") || trimmed.contains(" Close ")
    });

    assert!(
        has_signal,
        "order_book backtest produced no Open or Close signals.\n\
         This likely means nested struct field access (book.best_bid.price) or \
         match with multiple bindings is not working.\n\
         Full stdout:\n{}",
        stdout
    );
}

// =============================================================================
// Signal Production Verification Tests
// =============================================================================

/// Validates: Requirement 9.2
///
/// The regime_detector demo exercises trait-bounded generics (e.g.,
/// `fn detect_regime[T: RegimeDetector](detector: T, ...)`) and must produce
/// at least one OPEN or CLOSE signal when backtested against its data.csv.
/// If this test fails, it likely means trait method dispatch via generic
/// functions isn't working correctly in the interpreter.
#[test]
fn test_regime_detector_produces_trading_signals() {
    let output = run_flux(&[
        "backtest",
        strategy_path("regime_detector").to_str().unwrap(),
        "--data",
        data_path("regime_detector").to_str().unwrap(),
        "--capital",
        "100000",
    ]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "regime_detector backtest failed with non-zero exit code, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);

    // The backtest output contains signal lines in the format:
    //   "{bar_index} Open {symbol} {qty}" or "{bar_index} Close {symbol}"
    // At least one such signal must be present for the strategy to be meaningful.
    let has_open_signal = stdout.lines().any(|line| line.contains(" Open "));
    let has_close_signal = stdout.lines().any(|line| line.contains(" Close "));

    assert!(
        has_open_signal || has_close_signal,
        "regime_detector backtest produced no trading signals (no Open or Close lines found).\n\
         This likely indicates trait method dispatch via generic functions is broken.\n\
         stdout:\n{}",
        stdout
    );
}

/// Validates: Requirement 9.4
///
/// The live_connector demo exercises trait impls and conditional signals
/// (AlertLevel enum with match expression, SessionState struct with impl block,
/// DataFilter trait, and HashMap lookups). If this test fails, it likely means
/// trait method dispatch or some conditional control flow with type system
/// constructs isn't working in the interpreter.
#[test]
fn test_live_connector_produces_trading_signals() {
    let output = run_flux(&[
        "backtest",
        strategy_path("live_connector").to_str().unwrap(),
        "--data",
        data_path("live_connector").to_str().unwrap(),
        "--capital",
        "100000",
    ]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "live_connector backtest failed with non-zero exit code, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);

    // The backtest must produce at least one OPEN or CLOSE signal line,
    // confirming the interpreter handles trait impls (DataFilter), enum match
    // expressions (AlertLevel), struct instance methods (SessionState.update),
    // and HashMap operations correctly in a combined strategy.
    let has_open_signal = stdout.lines().any(|line| line.contains(" Open ") || line.contains("OPEN"));
    let has_close_signal = stdout.lines().any(|line| line.contains(" Close ") || line.contains("CLOSE"));

    assert!(
        has_open_signal || has_close_signal,
        "live_connector backtest produced no trading signals (no Open/OPEN or Close/CLOSE lines found).\n\
         This likely indicates trait method dispatch or conditional control flow with \
         type system constructs (enum match, struct methods, HashMap) is broken.\n\
         stdout:\n{}",
        stdout
    );
}
