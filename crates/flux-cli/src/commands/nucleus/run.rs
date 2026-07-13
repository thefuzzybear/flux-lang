//! `flux nucleus run` — execute a discovery cell or falsification trial.
//!
//! Classifies the cell by extension/location, executes it via subprocess,
//! then updates project state (nucleus.toml, hypotheses.md) based on the result.

use std::path::Path;

use super::config::{self, NucleusConfig};
use super::executor::{self, CellType, VerdictResult};
use super::hypotheses::{self, VerdictStatus};
use super::NucleusError;

/// Execute a cell at the given path: classify, run, update state.
pub fn run_cell(cell_path: &Path) -> Result<(), NucleusError> {
    // 1. Classify the cell (propagates FileNotFound if path doesn't exist)
    let cell_type = executor::classify_cell(cell_path)?;

    // 2. Execute the cell as a subprocess
    let result = match executor::execute_cell(cell_path, cell_type) {
        Ok(r) => r,
        Err(NucleusError::SubprocessFailed { code, stderr }) => {
            // Print stderr to the user, then propagate the error
            if !stderr.is_empty() {
                eprintln!("{}", stderr);
            }
            return Err(NucleusError::SubprocessFailed { code, stderr });
        }
        Err(e) => return Err(e),
    };

    // 3. Print stdout to user
    if !result.stdout.is_empty() {
        print!("{}", result.stdout);
    }

    // 4. Handle based on cell type
    match cell_type {
        CellType::PythonDiscovery | CellType::FluxDiscovery => {
            handle_discovery(cell_path)?;
        }
        CellType::PythonTrial | CellType::FluxTrial => {
            handle_trial(cell_path, &result.stdout)?;
        }
    }

    Ok(())
}

/// Handle post-execution logic for discovery cells.
///
/// - Check that a finding file was produced
/// - Warn if missing
/// - Increment cell_count in config
fn handle_discovery(cell_path: &Path) -> Result<(), NucleusError> {
    // Derive the finding name from the cell filename (without extension)
    let cell_name = cell_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");

    // Check if the corresponding finding file exists
    let cwd = std::env::current_dir().map_err(|e| NucleusError::Io {
        operation: "get current directory".to_string(),
        source: e,
    })?;
    let finding_path = cwd.join("discovery").join("findings").join(format!("{}.md", cell_name));

    if !finding_path.exists() {
        println!(
            "warning: cell did not produce a finding at discovery/findings/{}.md",
            cell_name
        );
    }

    // Load config, increment cell_count, save
    let mut config = config::load_config()?;
    config.discovery.cell_count += 1;
    config::save_config(&cwd, &config)?;

    Ok(())
}

/// Handle post-execution logic for falsification trials.
///
/// - Extract verdict from stdout
/// - If found: update hypotheses.md and config counters
/// - If not found: print warning
fn handle_trial(cell_path: &Path, stdout: &str) -> Result<(), NucleusError> {
    // Derive trial name from filename without extension
    let trial_name = cell_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");

    // Extract verdict from stdout
    let verdict_result = executor::extract_verdict(stdout);

    match verdict_result {
        Some(vr) => {
            // Convert VerdictResult to VerdictStatus
            let verdict_status = verdict_result_to_status(vr);

            // Load, update, and save hypotheses.md
            let cwd = std::env::current_dir().map_err(|e| NucleusError::Io {
                operation: "get current directory".to_string(),
                source: e,
            })?;

            let hyp_path = cwd.join("hypotheses.md");
            let hyp_content = std::fs::read_to_string(&hyp_path).map_err(|e| NucleusError::Io {
                operation: format!("read {}", hyp_path.display()),
                source: e,
            })?;

            let mut doc = hypotheses::parse_hypotheses(&hyp_content)?;

            // Update the prediction verdict (non-fatal if trial not found in predictions)
            if let Err(_) = hypotheses::update_prediction_verdict(&mut doc, trial_name, verdict_status) {
                // Trial name not found in predictions — this is OK, just warn
                println!("warning: trial '{}' not found in any prediction table", trial_name);
            }

            // Check if any hypothesis should be moved to killed
            hypotheses::check_and_move_killed(&mut doc);

            // Write updated hypotheses.md
            let serialized = hypotheses::serialize_hypotheses(&doc);
            std::fs::write(&hyp_path, serialized).map_err(|e| NucleusError::Io {
                operation: format!("write {}", hyp_path.display()),
                source: e,
            })?;

            // Update config falsification counters
            let mut config = config::load_config()?;
            update_falsification_counters(&mut config, &doc);
            config::save_config(&cwd, &config)?;
        }
        None => {
            println!("warning: no verdict found in trial output");
        }
    }

    Ok(())
}

/// Convert an executor `VerdictResult` to a hypotheses `VerdictStatus`.
fn verdict_result_to_status(vr: VerdictResult) -> VerdictStatus {
    match vr {
        VerdictResult::Survived => VerdictStatus::Survived,
        VerdictResult::Killed(reason) => VerdictStatus::Killed(reason),
    }
}

/// Update the falsification counters in config based on the current hypothesis document state.
fn update_falsification_counters(config: &mut NucleusConfig, doc: &hypotheses::HypothesisDocument) {
    let (total, tested, survived) = hypotheses::prediction_stats(doc);
    config.falsification.predictions_total = total;
    config.falsification.predictions_tested = tested;
    config.falsification.predictions_survived = survived;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Mutex;
    use tempfile::TempDir;

    use crate::commands::nucleus::config::{
        DiscoveryState, FalsificationState, HypothesesState, NucleusConfig, NucleusMeta, Phase,
        StrategyState,
    };

    /// Mutex to serialize tests that modify CWD (process-global state).
    /// We use `into_inner` on poison to prevent cascading failures.
    static CWD_MUTEX: Mutex<()> = Mutex::new(());

    fn lock_cwd() -> std::sync::MutexGuard<'static, ()> {
        CWD_MUTEX.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Create a minimal nucleus.toml config for testing.
    fn sample_config(name: &str) -> NucleusConfig {
        NucleusConfig {
            nucleus: NucleusMeta {
                name: name.to_string(),
                created: "2026-01-01".to_string(),
                phase: Phase::Discovery,
            },
            discovery: DiscoveryState {
                cell_count: 0,
                findings_count: 0,
            },
            hypotheses: HypothesesState {
                active: vec![],
                killed: vec![],
            },
            falsification: FalsificationState {
                current_hypothesis: String::new(),
                predictions_total: 0,
                predictions_tested: 0,
                predictions_survived: 0,
            },
            strategy: StrategyState {
                file: String::new(),
                last_backtest: String::new(),
                fidelity: 0,
            },
        }
    }

    fn sample_hypotheses_md() -> &'static str {
        "# Hypotheses\n\n\
         ## Active\n\n\
         (None yet — run discovery cells to generate hypotheses)\n\n\
         ## Killed\n\n\
         (Empty — no hypotheses tested yet)\n"
    }

    fn hypotheses_with_prediction() -> String {
        "# Hypotheses\n\n\
         ## Active\n\n\
         ### H1: Test hypothesis\n\n\
         | # | Prediction | Trial | Result | Verdict |\n\
         |---|-----------|-------|--------|----------|\n\
         | 1 | PF > 1.0 | h1_trial |  | pending |\n\n\
         ## Killed\n\n\
         (Empty — no hypotheses tested yet)\n"
            .to_string()
    }

    /// Set up a temp directory that looks like a nucleus project.
    fn setup_nucleus_project(tmp: &TempDir) -> std::path::PathBuf {
        let root = tmp.path().to_path_buf();
        fs::create_dir_all(root.join("discovery/cells")).unwrap();
        fs::create_dir_all(root.join("discovery/findings")).unwrap();
        fs::create_dir_all(root.join("falsification/trials")).unwrap();
        fs::create_dir_all(root.join("falsification/verdicts")).unwrap();

        // Write nucleus.toml
        let config = sample_config("test-project");
        config::save_config(&root, &config).unwrap();

        // Write hypotheses.md
        fs::write(root.join("hypotheses.md"), sample_hypotheses_md()).unwrap();

        root
    }

    #[test]
    fn test_verdict_result_to_status_survived() {
        let status = verdict_result_to_status(VerdictResult::Survived);
        assert_eq!(status, VerdictStatus::Survived);
    }

    #[test]
    fn test_verdict_result_to_status_killed() {
        let status = verdict_result_to_status(VerdictResult::Killed("low sharpe".to_string()));
        assert_eq!(status, VerdictStatus::Killed("low sharpe".to_string()));
    }

    #[test]
    fn test_update_falsification_counters() {
        let mut config = sample_config("test");
        let doc = hypotheses::HypothesisDocument {
            active: vec![hypotheses::Hypothesis {
                id: "H1".to_string(),
                claim: "Test".to_string(),
                predictions: vec![
                    hypotheses::Prediction {
                        number: 1,
                        text: "A".to_string(),
                        trial: "t1".to_string(),
                        result: String::new(),
                        verdict: VerdictStatus::Survived,
                    },
                    hypotheses::Prediction {
                        number: 2,
                        text: "B".to_string(),
                        trial: "t2".to_string(),
                        result: String::new(),
                        verdict: VerdictStatus::Killed("bad".to_string()),
                    },
                    hypotheses::Prediction {
                        number: 3,
                        text: "C".to_string(),
                        trial: "t3".to_string(),
                        result: String::new(),
                        verdict: VerdictStatus::Pending,
                    },
                ],
            }],
            killed: vec![],
        };

        update_falsification_counters(&mut config, &doc);

        assert_eq!(config.falsification.predictions_total, 3);
        assert_eq!(config.falsification.predictions_tested, 2);
        assert_eq!(config.falsification.predictions_survived, 1);
    }

    #[test]
    fn test_run_cell_file_not_found() {
        let result = run_cell(Path::new("/nonexistent/discovery/cells/test.py"));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), NucleusError::FileNotFound(_)));
    }

    #[test]
    fn test_run_cell_discovery_python_with_finding() {
        let _lock = lock_cwd();
        let tmp = TempDir::new().unwrap();
        let root = setup_nucleus_project(&tmp);

        // Create a Python cell that writes a finding using absolute path
        let cell_path = root.join("discovery/cells/explore_vol.py");
        let findings_dir = root.join("discovery/findings");
        let py_code = format!(
            r#"import os
findings_dir = r'{}'
os.makedirs(findings_dir, exist_ok=True)
with open(os.path.join(findings_dir, 'explore_vol.md'), 'w') as f:
    f.write('## Observations\n\nHigh vol detected.\n\n## Implications\n\nGood for breakout.\n')
print('Exploration complete')
"#,
            findings_dir.display()
        );
        fs::write(&cell_path, py_code).unwrap();

        // Change to project directory so load_config works
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(&root).unwrap();

        let result = run_cell(&cell_path);

        // Restore CWD before any assertions
        std::env::set_current_dir(&original_dir).unwrap();

        // If python3 is not available, skip the test gracefully
        match &result {
            Err(NucleusError::Python3NotFound) => return,
            _ => {}
        }

        result.expect("run_cell should succeed");

        // Verify cell_count was incremented
        let content = fs::read_to_string(root.join("nucleus.toml")).unwrap();
        let config = config::parse_config(&content).unwrap();
        assert_eq!(config.discovery.cell_count, 1);

        // Verify finding file exists
        assert!(root.join("discovery/findings/explore_vol.md").exists());
    }

    #[test]
    fn test_run_cell_discovery_python_no_finding_warns() {
        let _lock = lock_cwd();
        let tmp = TempDir::new().unwrap();
        let root = setup_nucleus_project(&tmp);

        // Create a Python cell that does NOT write a finding
        let cell_path = root.join("discovery/cells/lazy_cell.py");
        fs::write(&cell_path, "print('I did nothing useful')\n").unwrap();

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(&root).unwrap();

        let result = run_cell(&cell_path);

        std::env::set_current_dir(&original_dir).unwrap();

        match &result {
            Err(NucleusError::Python3NotFound) => return,
            _ => {}
        }

        // Should succeed but cell_count still incremented
        result.expect("run_cell should succeed even without finding");

        let content = fs::read_to_string(root.join("nucleus.toml")).unwrap();
        let config = config::parse_config(&content).unwrap();
        assert_eq!(config.discovery.cell_count, 1);
    }

    #[test]
    fn test_run_cell_trial_with_verdict() {
        let _lock = lock_cwd();
        let tmp = TempDir::new().unwrap();
        let root = setup_nucleus_project(&tmp);

        // Write hypotheses.md with a prediction expecting "h1_trial"
        fs::write(root.join("hypotheses.md"), hypotheses_with_prediction()).unwrap();

        // Create a Python trial that prints a VERDICT line
        let cell_path = root.join("falsification/trials/h1_trial.py");
        fs::write(&cell_path, "print('Running test...')\nprint('VERDICT: SURVIVED')\n").unwrap();

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(&root).unwrap();

        let result = run_cell(&cell_path);

        std::env::set_current_dir(&original_dir).unwrap();

        match &result {
            Err(NucleusError::Python3NotFound) => return,
            _ => {}
        }

        result.expect("run_cell should succeed");

        // Verify hypotheses.md was updated
        let hyp_content = fs::read_to_string(root.join("hypotheses.md")).unwrap();
        assert!(
            hyp_content.contains("survived"),
            "hypotheses.md should contain 'survived'"
        );

        // Verify config counters updated
        let config_content = fs::read_to_string(root.join("nucleus.toml")).unwrap();
        let config = config::parse_config(&config_content).unwrap();
        assert_eq!(config.falsification.predictions_tested, 1);
        assert_eq!(config.falsification.predictions_survived, 1);
    }

    #[test]
    fn test_run_cell_trial_no_verdict_warns() {
        let _lock = lock_cwd();
        let tmp = TempDir::new().unwrap();
        let root = setup_nucleus_project(&tmp);

        // Create a Python trial that does NOT print a verdict
        let cell_path = root.join("falsification/trials/h1_noisy.py");
        fs::write(&cell_path, "print('Running analysis...')\nprint('Done.')\n").unwrap();

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(&root).unwrap();

        let result = run_cell(&cell_path);

        std::env::set_current_dir(&original_dir).unwrap();

        match &result {
            Err(NucleusError::Python3NotFound) => return,
            _ => {}
        }

        // Should succeed (no verdict is a warning, not an error)
        result.expect("run_cell should succeed without verdict");
    }

    #[test]
    fn test_run_cell_subprocess_failure() {
        let _lock = lock_cwd();
        let tmp = TempDir::new().unwrap();
        let root = setup_nucleus_project(&tmp);

        // Create a Python cell that exits with non-zero
        let cell_path = root.join("discovery/cells/bad_cell.py");
        fs::write(&cell_path, "import sys\nprint('oops', file=sys.stderr)\nsys.exit(1)\n").unwrap();

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(&root).unwrap();

        let result = run_cell(&cell_path);

        std::env::set_current_dir(&original_dir).unwrap();

        match &result {
            Err(NucleusError::Python3NotFound) => return,
            _ => {}
        }

        // Should fail with SubprocessFailed
        assert!(result.is_err());
        match result.unwrap_err() {
            NucleusError::SubprocessFailed { code, stderr } => {
                assert_eq!(code, 1);
                assert!(stderr.contains("oops"));
            }
            other => panic!("expected SubprocessFailed, got: {:?}", other),
        }
    }
}
