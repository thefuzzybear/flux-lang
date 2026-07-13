//! End-to-end integration test for the `flux nucleus` subcommand lifecycle.
//!
//! Exercises the full Nucleus lifecycle by invoking the compiled `flux` binary:
//! init → status → create Python cell → run cell → manually add hypothesis →
//! promote → create trial → run trial → verdict → promote.
//!
//! **Validates: Requirements 1.1, 1.2, 1.3, 2.1, 3.1, 3.4, 3.5, 4.1, 4.2, 5.1, 5.2**

use std::fs;
use std::process::Command;

/// Get a Command pointing to the compiled `flux` binary.
fn flux_cmd() -> Command {
    Command::new(env!("CARGO_BIN_EXE_flux"))
}

/// Check if python3 is available on the system.
fn python3_available() -> bool {
    Command::new("python3")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Full nucleus lifecycle integration test.
///
/// Steps:
/// 1. Init project
/// 2. Status (discovery phase)
/// 3. Create and run a Python discovery cell
/// 4. Manually add a hypothesis with predictions to hypotheses.md
/// 5. Promote discovery → falsification
/// 6. Create and run a falsification trial (VERDICT: SURVIVED)
/// 7. Record verdict for second prediction
/// 8. Promote falsification → strategy (if majority survived)
#[test]
fn test_nucleus_full_lifecycle() {
    if !python3_available() {
        eprintln!("SKIPPING: python3 not found on PATH");
        return;
    }

    let tmp = tempfile::TempDir::new().expect("failed to create temp dir");
    let base = tmp.path();

    // =========================================================================
    // Step 1: Init
    // =========================================================================
    let output = flux_cmd()
        .args(["nucleus", "init", "test-nucleus"])
        .current_dir(base)
        .output()
        .expect("failed to execute flux nucleus init");

    assert_eq!(
        output.status.code(),
        Some(0),
        "nucleus init should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let project_dir = base.join("test-nucleus");
    assert!(project_dir.exists(), "project directory should be created");
    assert!(project_dir.join("nucleus.toml").is_file());
    assert!(project_dir.join("hypotheses.md").is_file());
    assert!(project_dir.join("discovery/cells").is_dir());
    assert!(project_dir.join("discovery/findings").is_dir());
    assert!(project_dir.join("falsification/trials").is_dir());

    // =========================================================================
    // Step 2: Status (should show discovery phase)
    // =========================================================================
    let output = flux_cmd()
        .args(["nucleus", "status"])
        .current_dir(&project_dir)
        .output()
        .expect("failed to execute flux nucleus status");

    assert_eq!(
        output.status.code(),
        Some(0),
        "nucleus status should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("discovery"),
        "status should show discovery phase, got: {}",
        stdout
    );

    // =========================================================================
    // Step 3: Create and run a Python discovery cell
    // =========================================================================
    let cell_path = project_dir.join("discovery/cells/01_explore.py");
    let findings_dir = project_dir.join("discovery/findings");
    let py_code = format!(
        r#"import os
findings_dir = r'{}'
os.makedirs(findings_dir, exist_ok=True)
with open(os.path.join(findings_dir, '01_explore.md'), 'w') as f:
    f.write('## Observations\n\nVolatility clustering detected in morning session.\n\n## Implications\n\nBreakout strategy may work in first hour.\n')
print('Discovery cell complete')
"#,
        findings_dir.display()
    );
    fs::write(&cell_path, &py_code).expect("failed to write discovery cell");

    let output = flux_cmd()
        .args(["nucleus", "run", "discovery/cells/01_explore.py"])
        .current_dir(&project_dir)
        .output()
        .expect("failed to execute flux nucleus run");

    assert_eq!(
        output.status.code(),
        Some(0),
        "nucleus run discovery cell should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify finding file was created
    assert!(
        findings_dir.join("01_explore.md").exists(),
        "finding file should be created by discovery cell"
    );

    // Verify cell_count was incremented in nucleus.toml
    let toml_content = fs::read_to_string(project_dir.join("nucleus.toml")).unwrap();
    assert!(
        toml_content.contains("cell_count = 1"),
        "cell_count should be 1 after running one cell, got: {}",
        toml_content
    );

    // =========================================================================
    // Step 4: Manually add hypothesis with predictions to hypotheses.md
    // =========================================================================
    let hypotheses_content = r#"# Hypotheses

## Active

### H1: Morning breakout-pullback has positive expectancy

| # | Prediction | Trial | Result | Verdict |
|---|-----------|-------|--------|----------|
| 1 | PF > 1.5 in trending regimes | h1_pf |  | pending |
| 2 | Survives $0.02 slippage | h1_slip |  | pending |

## Killed

(Empty — no hypotheses tested yet)
"#;
    fs::write(project_dir.join("hypotheses.md"), hypotheses_content)
        .expect("failed to write hypotheses.md");

    // =========================================================================
    // Step 5: Promote discovery → falsification
    // =========================================================================
    let output = flux_cmd()
        .args(["nucleus", "promote"])
        .current_dir(&project_dir)
        .output()
        .expect("failed to execute flux nucleus promote");

    assert_eq!(
        output.status.code(),
        Some(0),
        "nucleus promote (discovery→falsification) should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("falsification"),
        "promote output should mention new phase 'falsification', got: {}",
        stdout
    );

    // Verify phase changed in nucleus.toml
    let toml_content = fs::read_to_string(project_dir.join("nucleus.toml")).unwrap();
    assert!(
        toml_content.contains(r#"phase = "falsification""#),
        "nucleus.toml should have phase = falsification, got: {}",
        toml_content
    );

    // Verify status now shows falsification
    let output = flux_cmd()
        .args(["nucleus", "status"])
        .current_dir(&project_dir)
        .output()
        .expect("failed to execute flux nucleus status");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("falsification"),
        "status should show falsification phase, got: {}",
        stdout
    );

    // =========================================================================
    // Step 6: Create and run a falsification trial (prints VERDICT: SURVIVED)
    // =========================================================================
    let trial_path = project_dir.join("falsification/trials/h1_pf.py");
    let trial_code = r#"print('Running profit factor test...')
print('PF = 2.3 across trending regime sample')
print('VERDICT: SURVIVED')
"#;
    fs::write(&trial_path, trial_code).expect("failed to write trial file");

    let output = flux_cmd()
        .args(["nucleus", "run", "falsification/trials/h1_pf.py"])
        .current_dir(&project_dir)
        .output()
        .expect("failed to execute flux nucleus run trial");

    assert_eq!(
        output.status.code(),
        Some(0),
        "nucleus run trial should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify hypotheses.md was updated with the verdict
    let hyp_content = fs::read_to_string(project_dir.join("hypotheses.md")).unwrap();
    assert!(
        hyp_content.contains("survived"),
        "hypotheses.md should contain 'survived' after trial, got: {}",
        hyp_content
    );

    // =========================================================================
    // Step 7: Record verdict for second prediction via `verdict` command
    // =========================================================================
    // First, create the trial file so `verdict` can find it
    let trial2_path = project_dir.join("falsification/trials/h1_slip.py");
    let trial2_code = r#"print('Running slippage test...')
print('Slippage impact: minimal')
print('VERDICT: SURVIVED')
"#;
    fs::write(&trial2_path, trial2_code).expect("failed to write second trial file");

    let output = flux_cmd()
        .args(["nucleus", "verdict", "h1_slip", "survived"])
        .current_dir(&project_dir)
        .output()
        .expect("failed to execute flux nucleus verdict");

    assert_eq!(
        output.status.code(),
        Some(0),
        "nucleus verdict should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("h1_slip") && stdout.contains("survived"),
        "verdict output should confirm recording, got: {}",
        stdout
    );

    // Verify hypotheses.md now has both predictions as survived
    let hyp_content = fs::read_to_string(project_dir.join("hypotheses.md")).unwrap();
    // Both predictions should now show survived
    let survived_count = hyp_content.matches("survived").count();
    assert!(
        survived_count >= 2,
        "hypotheses.md should have at least 2 'survived' entries, found {}: {}",
        survived_count,
        hyp_content
    );

    // =========================================================================
    // Step 8: Promote falsification → strategy (majority survived)
    // =========================================================================
    let output = flux_cmd()
        .args(["nucleus", "promote"])
        .current_dir(&project_dir)
        .output()
        .expect("failed to execute flux nucleus promote");

    assert_eq!(
        output.status.code(),
        Some(0),
        "nucleus promote (falsification→strategy) should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("strategy"),
        "promote output should mention new phase 'strategy', got: {}",
        stdout
    );

    // Verify phase changed in nucleus.toml
    let toml_content = fs::read_to_string(project_dir.join("nucleus.toml")).unwrap();
    assert!(
        toml_content.contains(r#"phase = "strategy""#),
        "nucleus.toml should have phase = strategy, got: {}",
        toml_content
    );
}

/// Test that `flux nucleus init` rejects an existing directory.
#[test]
fn test_nucleus_init_rejects_existing_dir() {
    let tmp = tempfile::TempDir::new().expect("failed to create temp dir");
    let base = tmp.path();

    // Create the directory first
    fs::create_dir(base.join("existing-project")).unwrap();

    let output = flux_cmd()
        .args(["nucleus", "init", "existing-project"])
        .current_dir(base)
        .output()
        .expect("failed to execute flux nucleus init");

    assert_ne!(
        output.status.code(),
        Some(0),
        "nucleus init should fail for existing directory"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("already exists"),
        "error should mention directory already exists, got stderr: {}",
        stderr
    );
}

/// Test that `flux nucleus status` fails outside a Nucleus project.
#[test]
fn test_nucleus_status_outside_project() {
    let tmp = tempfile::TempDir::new().expect("failed to create temp dir");

    let output = flux_cmd()
        .args(["nucleus", "status"])
        .current_dir(tmp.path())
        .output()
        .expect("failed to execute flux nucleus status");

    assert_ne!(
        output.status.code(),
        Some(0),
        "nucleus status should fail outside a project"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("nucleus.toml") || stderr.contains("not a Nucleus project"),
        "error should mention nucleus.toml not found, got stderr: {}",
        stderr
    );
}

/// Test that `flux nucleus promote` fails gate check when no hypotheses exist.
#[test]
fn test_nucleus_promote_fails_gate() {
    let tmp = tempfile::TempDir::new().expect("failed to create temp dir");
    let base = tmp.path();

    // Init a project
    let output = flux_cmd()
        .args(["nucleus", "init", "gate-test"])
        .current_dir(base)
        .output()
        .expect("failed to execute flux nucleus init");
    assert_eq!(output.status.code(), Some(0));

    let project_dir = base.join("gate-test");

    // Try to promote without any hypotheses — should fail
    let output = flux_cmd()
        .args(["nucleus", "promote"])
        .current_dir(&project_dir)
        .output()
        .expect("failed to execute flux nucleus promote");

    assert_ne!(
        output.status.code(),
        Some(0),
        "nucleus promote should fail without hypotheses"
    );
}

/// Test that `flux nucleus init` rejects invalid names.
#[test]
fn test_nucleus_init_invalid_name() {
    let tmp = tempfile::TempDir::new().expect("failed to create temp dir");

    let output = flux_cmd()
        .args(["nucleus", "init", "bad name!"])
        .current_dir(tmp.path())
        .output()
        .expect("failed to execute flux nucleus init");

    assert_ne!(
        output.status.code(),
        Some(0),
        "nucleus init should reject invalid names"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid project name") || stderr.contains("alphanumeric"),
        "error should mention invalid name, got stderr: {}",
        stderr
    );
}

/// Test that `flux nucleus run` reports error for non-existent cell.
#[test]
fn test_nucleus_run_nonexistent_cell() {
    let tmp = tempfile::TempDir::new().expect("failed to create temp dir");
    let base = tmp.path();

    // Init project
    let output = flux_cmd()
        .args(["nucleus", "init", "run-test"])
        .current_dir(base)
        .output()
        .expect("failed to init");
    assert_eq!(output.status.code(), Some(0));

    let project_dir = base.join("run-test");

    let output = flux_cmd()
        .args(["nucleus", "run", "discovery/cells/nonexistent.py"])
        .current_dir(&project_dir)
        .output()
        .expect("failed to execute flux nucleus run");

    assert_ne!(
        output.status.code(),
        Some(0),
        "nucleus run should fail for non-existent cell"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not found") || stderr.contains("cell not found"),
        "error should mention file not found, got stderr: {}",
        stderr
    );
}

/// Test that `flux nucleus verdict` fails for non-existent trial.
#[test]
fn test_nucleus_verdict_trial_not_found() {
    if !python3_available() {
        eprintln!("SKIPPING: python3 not found on PATH");
        return;
    }

    let tmp = tempfile::TempDir::new().expect("failed to create temp dir");
    let base = tmp.path();

    // Init project
    let output = flux_cmd()
        .args(["nucleus", "init", "verdict-test"])
        .current_dir(base)
        .output()
        .expect("failed to init");
    assert_eq!(output.status.code(), Some(0));

    let project_dir = base.join("verdict-test");

    let output = flux_cmd()
        .args(["nucleus", "verdict", "nonexistent_trial", "survived"])
        .current_dir(&project_dir)
        .output()
        .expect("failed to execute flux nucleus verdict");

    assert_ne!(
        output.status.code(),
        Some(0),
        "nucleus verdict should fail for non-existent trial"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not found") || stderr.contains("trial"),
        "error should mention trial not found, got stderr: {}",
        stderr
    );
}
