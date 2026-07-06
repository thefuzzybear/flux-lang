//! Integration tests for the `flux init` subcommand.
//!
//! These tests invoke the compiled `flux` binary in isolated temp directories
//! and verify exit codes, stdout/stderr output, and filesystem side-effects.
//!
//! **Validates: Requirements 1.1, 1.2, 1.3, 1.4, 3.3, 5.1, 5.2**

use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// Get a Command for the compiled `flux` binary.
fn flux_cmd() -> Command {
    Command::new(env!("CARGO_BIN_EXE_flux"))
}

/// Create a unique temp directory for a test, returning its path.
/// The directory is created immediately so it exists for the test.
fn create_test_dir(test_name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("flux_init_tests")
        .join(test_name)
        .join(format!("{}", std::process::id()));
    // Clean up any previous run
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("failed to create test temp dir");
    dir
}

/// Clean up a test directory.
fn cleanup(dir: &PathBuf) {
    let _ = fs::remove_dir_all(dir);
}

// =============================================================================
// Test 1: flux init myproj — creates project in new directory
// =============================================================================

/// Validates: Requirements 1.1, 5.1, 5.2
/// `flux init myproj` in a temp dir should create the full project structure.
#[test]
fn init_creates_project_in_new_directory() {
    let tmp = create_test_dir("init_creates_project_in_new_directory");

    let output = flux_cmd()
        .arg("init")
        .arg("myproj")
        .current_dir(&tmp)
        .output()
        .expect("failed to execute");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(
        output.status.code(),
        Some(0),
        "Expected exit 0, stdout: {:?}, stderr: {:?}",
        stdout,
        stderr
    );

    let project_dir = tmp.join("myproj");

    // Directory exists
    assert!(project_dir.is_dir(), "myproj/ directory should exist");

    // flux.toml exists with correct content
    let manifest_path = project_dir.join("flux.toml");
    assert!(manifest_path.is_file(), "flux.toml should exist");
    let manifest = fs::read_to_string(&manifest_path).expect("read flux.toml");
    assert!(
        manifest.contains("name = \"myproj\""),
        "flux.toml should contain project name, got: {:?}",
        manifest
    );
    assert!(
        manifest.contains("version = \"0.1.0\""),
        "flux.toml should contain version"
    );
    assert!(
        manifest.contains("strategies_dir"),
        "flux.toml should contain strategies_dir"
    );
    assert!(
        manifest.contains("data_dir"),
        "flux.toml should contain data_dir"
    );

    // strategies/example.flux exists with strategy block
    let example_path = project_dir.join("strategies").join("example.flux");
    assert!(example_path.is_file(), "strategies/example.flux should exist");
    let example = fs::read_to_string(&example_path).expect("read example.flux");
    assert!(
        example.contains("strategy"),
        "example.flux should contain 'strategy' keyword"
    );

    // data/ directory exists
    assert!(
        project_dir.join("data").is_dir(),
        "data/ directory should exist"
    );

    // .gitignore exists with expected patterns
    let gitignore_path = project_dir.join(".gitignore");
    assert!(gitignore_path.is_file(), ".gitignore should exist");
    let gitignore = fs::read_to_string(&gitignore_path).expect("read .gitignore");
    assert!(
        gitignore.contains("target/"),
        ".gitignore should contain 'target/'"
    );
    assert!(
        gitignore.contains("data/*.csv"),
        ".gitignore should contain 'data/*.csv'"
    );
    assert!(
        gitignore.contains(".DS_Store"),
        ".gitignore should contain '.DS_Store'"
    );
    assert!(
        gitignore.contains("Thumbs.db"),
        ".gitignore should contain 'Thumbs.db'"
    );

    // stdout contains project name
    assert!(
        stdout.contains("myproj"),
        "stdout should contain project name 'myproj', got: {:?}",
        stdout
    );

    cleanup(&tmp);
}

// =============================================================================
// Test 2: flux init (no name) — in-place initialization
// =============================================================================

/// Validates: Requirements 1.2
/// `flux init` with no name in an empty directory should initialize in-place.
#[test]
fn init_in_place_no_name() {
    let tmp = create_test_dir("init_in_place_no_name");

    let output = flux_cmd()
        .arg("init")
        .current_dir(&tmp)
        .output()
        .expect("failed to execute");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(
        output.status.code(),
        Some(0),
        "Expected exit 0, stdout: {:?}, stderr: {:?}",
        stdout,
        stderr
    );

    // flux.toml exists in current dir
    assert!(
        tmp.join("flux.toml").is_file(),
        "flux.toml should exist in current directory"
    );

    // strategies/example.flux exists
    assert!(
        tmp.join("strategies").join("example.flux").is_file(),
        "strategies/example.flux should exist"
    );

    // data/ directory exists
    assert!(tmp.join("data").is_dir(), "data/ directory should exist");

    // .gitignore exists
    assert!(
        tmp.join(".gitignore").is_file(),
        ".gitignore should exist"
    );

    cleanup(&tmp);
}

// =============================================================================
// Test 3: flux init in non-empty directory — should fail
// =============================================================================

/// Validates: Requirement 1.3
/// `flux init` in a non-empty directory should exit with code 1 and report error.
#[test]
fn init_non_empty_directory_fails() {
    let tmp = create_test_dir("init_non_empty_directory_fails");

    // Create a file so the directory is non-empty
    fs::write(tmp.join("existing_file.txt"), "hello").expect("create file");

    let output = flux_cmd()
        .arg("init")
        .current_dir(&tmp)
        .output()
        .expect("failed to execute");

    assert_eq!(
        output.status.code(),
        Some(1),
        "Expected exit 1 for non-empty directory"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not empty"),
        "stderr should contain 'not empty', got: {:?}",
        stderr
    );

    cleanup(&tmp);
}

// =============================================================================
// Test 4: flux init "bad name!" — invalid name should fail
// =============================================================================

/// Validates: Requirement 1.4
/// `flux init "bad name!"` should exit with code 1 and report invalid name.
#[test]
fn init_invalid_name_fails() {
    let tmp = create_test_dir("init_invalid_name_fails");

    let output = flux_cmd()
        .arg("init")
        .arg("bad name!")
        .current_dir(&tmp)
        .output()
        .expect("failed to execute");

    assert_eq!(
        output.status.code(),
        Some(1),
        "Expected exit 1 for invalid project name"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid project name"),
        "stderr should contain 'invalid project name', got: {:?}",
        stderr
    );

    cleanup(&tmp);
}

// =============================================================================
// Test 5: flux check on generated example compiles successfully
// =============================================================================

/// Validates: Requirement 3.3
/// The generated example strategy should compile without errors via `flux check`.
#[test]
fn init_generated_example_compiles() {
    let tmp = create_test_dir("init_generated_example_compiles");

    // First, init a project
    let init_output = flux_cmd()
        .arg("init")
        .arg("testproj")
        .current_dir(&tmp)
        .output()
        .expect("failed to execute init");

    assert_eq!(
        init_output.status.code(),
        Some(0),
        "Init should succeed, stderr: {}",
        String::from_utf8_lossy(&init_output.stderr)
    );

    // Now run flux check on the generated example strategy
    let example_path = tmp.join("testproj").join("strategies").join("example.flux");
    assert!(example_path.is_file(), "example.flux should exist after init");

    let check_output = flux_cmd()
        .arg("check")
        .arg(&example_path)
        .output()
        .expect("failed to execute check");

    let check_stdout = String::from_utf8_lossy(&check_output.stdout);
    let check_stderr = String::from_utf8_lossy(&check_output.stderr);

    assert_eq!(
        check_output.status.code(),
        Some(0),
        "flux check on example.flux should exit 0, stdout: {:?}, stderr: {:?}",
        check_stdout,
        check_stderr
    );

    cleanup(&tmp);
}
