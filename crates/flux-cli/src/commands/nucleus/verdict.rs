//! `flux nucleus verdict` — record a trial outcome.

use std::path::Path;

use super::config;
use super::hypotheses::{self, VerdictStatus};
use super::NucleusError;

/// Record the verdict for a falsification trial.
///
/// Validates the outcome, checks the trial file exists, updates hypotheses.md
/// and nucleus.toml counters, and moves killed hypotheses as needed.
pub fn run_verdict(
    trial_name: &str,
    outcome: &str,
    reason: Option<&str>,
) -> Result<(), NucleusError> {
    // 1. Validate outcome (case-insensitive)
    let outcome_lower = outcome.to_lowercase();
    if outcome_lower != "survived" && outcome_lower != "killed" {
        return Err(NucleusError::ConfigParseError(format!(
            "invalid outcome '{}' (must be 'survived' or 'killed')",
            outcome
        )));
    }

    // 2. If killed, validate reason is present and non-empty
    if outcome_lower == "killed" {
        match reason {
            None => {
                return Err(NucleusError::ConfigParseError(
                    "reason is required when outcome is 'killed'".to_string(),
                ));
            }
            Some(r) if r.trim().is_empty() => {
                return Err(NucleusError::ConfigParseError(
                    "reason is required when outcome is 'killed'".to_string(),
                ));
            }
            _ => {}
        }
    }

    // 3. Validate trial file exists in falsification/trials/
    let cwd = std::env::current_dir().map_err(|e| NucleusError::Io {
        operation: "get current directory".to_string(),
        source: e,
    })?;
    let trials_dir = cwd.join("falsification").join("trials");

    let py_path = trials_dir.join(format!("{}.py", trial_name));
    let flux_path = trials_dir.join(format!("{}.flux", trial_name));

    if !py_path.exists() && !flux_path.exists() {
        return Err(NucleusError::TrialNotFound(trial_name.to_string()));
    }

    // 4. Load hypotheses.md
    let hyp_path = cwd.join("hypotheses.md");
    let hyp_content = std::fs::read_to_string(&hyp_path).map_err(|e| NucleusError::Io {
        operation: format!("read {}", hyp_path.display()),
        source: e,
    })?;
    let mut doc = hypotheses::parse_hypotheses(&hyp_content)?;

    // 5. Construct VerdictStatus
    let verdict = match outcome_lower.as_str() {
        "survived" => VerdictStatus::Survived,
        "killed" => VerdictStatus::Killed(reason.unwrap().to_string()),
        _ => unreachable!(),
    };

    // 6. Update the prediction verdict
    hypotheses::update_prediction_verdict(&mut doc, trial_name, verdict)?;

    // 7. Check and move killed hypotheses
    hypotheses::check_and_move_killed(&mut doc);

    // 8. Serialize and write hypotheses.md back
    let updated_hyp = hypotheses::serialize_hypotheses(&doc);
    std::fs::write(&hyp_path, &updated_hyp).map_err(|e| NucleusError::Io {
        operation: format!("write {}", hyp_path.display()),
        source: e,
    })?;

    // 9. Load config, update counters, save
    let mut cfg = config::load_config()?;
    cfg.falsification.predictions_tested += 1;
    if outcome_lower == "survived" {
        cfg.falsification.predictions_survived += 1;
    }
    config::save_config(&cwd, &cfg)?;

    // 10. Print confirmation
    println!("Recorded: {} → {}", trial_name, outcome_lower);

    Ok(())
}

/// Validate the outcome string (case-insensitive).
/// Returns Ok(()) if "survived" or "killed", error otherwise.
pub fn validate_outcome(outcome: &str) -> Result<(), NucleusError> {
    let lower = outcome.to_lowercase();
    if lower != "survived" && lower != "killed" {
        return Err(NucleusError::ConfigParseError(format!(
            "invalid outcome '{}' (must be 'survived' or 'killed')",
            outcome
        )));
    }
    Ok(())
}

/// Check that a trial file exists in the given trials directory.
/// Returns Ok(()) if a `.py` or `.flux` file with the given name exists.
pub fn validate_trial_exists(trials_dir: &Path, trial_name: &str) -> Result<(), NucleusError> {
    let py_path = trials_dir.join(format!("{}.py", trial_name));
    let flux_path = trials_dir.join(format!("{}.flux", trial_name));

    if !py_path.exists() && !flux_path.exists() {
        return Err(NucleusError::TrialNotFound(trial_name.to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::nucleus::config::{
        self as cfg_mod, DiscoveryState, FalsificationState, HypothesesState, NucleusConfig,
        NucleusMeta, Phase, StrategyState,
    };
    use crate::commands::nucleus::hypotheses::{
        serialize_hypotheses, Hypothesis, HypothesisDocument, Prediction,
    };
    use std::fs;
    use std::sync::Mutex;
    use tempfile::TempDir;

    // Mutex to serialize tests that change CWD (process-global state).
    static CWD_LOCK: Mutex<()> = Mutex::new(());

    /// Helper to create a minimal nucleus project in a temp directory.
    fn setup_project(
        tmp: &TempDir,
        doc: &HypothesisDocument,
        cfg: &NucleusConfig,
        trial_files: &[&str],
    ) {
        let root = tmp.path();

        // Create falsification/trials/ directory
        let trials_dir = root.join("falsification").join("trials");
        fs::create_dir_all(&trials_dir).unwrap();

        // Create trial files
        for trial_file in trial_files {
            fs::write(trials_dir.join(trial_file), "# trial stub").unwrap();
        }

        // Write hypotheses.md
        let hyp_content = serialize_hypotheses(doc);
        fs::write(root.join("hypotheses.md"), hyp_content).unwrap();

        // Write nucleus.toml
        let cfg_content = cfg_mod::serialize_config(cfg);
        fs::write(root.join("nucleus.toml"), cfg_content).unwrap();
    }

    fn sample_config() -> NucleusConfig {
        NucleusConfig {
            nucleus: NucleusMeta {
                name: "test-project".to_string(),
                created: "2026-07-13".to_string(),
                phase: Phase::Falsification,
            },
            discovery: DiscoveryState {
                cell_count: 1,
                findings_count: 1,
            },
            hypotheses: HypothesesState {
                active: vec!["H1".to_string()],
                killed: vec![],
            },
            falsification: FalsificationState {
                current_hypothesis: "H1".to_string(),
                predictions_total: 2,
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

    fn sample_doc() -> HypothesisDocument {
        HypothesisDocument {
            active: vec![Hypothesis {
                id: "H1".to_string(),
                claim: "Test hypothesis".to_string(),
                predictions: vec![
                    Prediction {
                        number: 1,
                        text: "Prediction one".to_string(),
                        trial: "trial_a".to_string(),
                        result: String::new(),
                        verdict: VerdictStatus::Pending,
                    },
                    Prediction {
                        number: 2,
                        text: "Prediction two".to_string(),
                        trial: "trial_b".to_string(),
                        result: String::new(),
                        verdict: VerdictStatus::Pending,
                    },
                ],
            }],
            killed: vec![],
        }
    }

    /// Run `run_verdict` inside a temp dir with CWD locked.
    /// Returns (result, hypotheses.md content, nucleus.toml content).
    fn run_in_project(
        tmp: &TempDir,
        trial_name: &str,
        outcome: &str,
        reason: Option<&str>,
    ) -> (Result<(), NucleusError>, String, String) {
        let _lock = CWD_LOCK.lock().unwrap();
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let result = run_verdict(trial_name, outcome, reason);

        // Read results while still in tmp dir
        let hyp_content = fs::read_to_string(tmp.path().join("hypotheses.md"))
            .unwrap_or_default();
        let cfg_content = fs::read_to_string(tmp.path().join("nucleus.toml"))
            .unwrap_or_default();

        std::env::set_current_dir(&original_dir).unwrap();

        (result, hyp_content, cfg_content)
    }

    #[test]
    fn test_valid_survived_outcome() {
        let tmp = TempDir::new().unwrap();
        let doc = sample_doc();
        let cfg = sample_config();
        setup_project(&tmp, &doc, &cfg, &["trial_a.py"]);

        let (result, hyp_content, cfg_content) =
            run_in_project(&tmp, "trial_a", "survived", None);

        assert!(result.is_ok());

        // Verify hypotheses.md was updated
        let parsed = hypotheses::parse_hypotheses(&hyp_content).unwrap();
        assert_eq!(parsed.active[0].predictions[0].verdict, VerdictStatus::Survived);

        // Verify config counters updated
        let parsed_cfg = cfg_mod::parse_config(&cfg_content).unwrap();
        assert_eq!(parsed_cfg.falsification.predictions_tested, 1);
        assert_eq!(parsed_cfg.falsification.predictions_survived, 1);
    }

    #[test]
    fn test_valid_killed_with_reason() {
        let tmp = TempDir::new().unwrap();
        let doc = sample_doc();
        let cfg = sample_config();
        setup_project(&tmp, &doc, &cfg, &["trial_a.py"]);

        let (result, hyp_content, cfg_content) =
            run_in_project(&tmp, "trial_a", "killed", Some("PF below threshold"));

        assert!(result.is_ok());

        // Verify hypotheses.md was updated with killed + reason
        // Note: the hypotheses parser lowercases the verdict line, so reason comes back lowercase
        let parsed = hypotheses::parse_hypotheses(&hyp_content).unwrap();
        assert_eq!(
            parsed.active[0].predictions[0].verdict,
            VerdictStatus::Killed("pf below threshold".to_string())
        );

        // Verify config counters — tested incremented, survived NOT incremented
        let parsed_cfg = cfg_mod::parse_config(&cfg_content).unwrap();
        assert_eq!(parsed_cfg.falsification.predictions_tested, 1);
        assert_eq!(parsed_cfg.falsification.predictions_survived, 0);
    }

    #[test]
    fn test_invalid_outcome_returns_error() {
        let tmp = TempDir::new().unwrap();
        let doc = sample_doc();
        let cfg = sample_config();
        setup_project(&tmp, &doc, &cfg, &["trial_a.py"]);

        let (result, _, _) = run_in_project(&tmp, "trial_a", "unknown", None);

        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("invalid outcome"));
    }

    #[test]
    fn test_killed_without_reason_returns_error() {
        let tmp = TempDir::new().unwrap();
        let doc = sample_doc();
        let cfg = sample_config();
        setup_project(&tmp, &doc, &cfg, &["trial_a.py"]);

        let (result, _, _) = run_in_project(&tmp, "trial_a", "killed", None);

        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("reason is required"));
    }

    #[test]
    fn test_killed_with_empty_reason_returns_error() {
        let tmp = TempDir::new().unwrap();
        let doc = sample_doc();
        let cfg = sample_config();
        setup_project(&tmp, &doc, &cfg, &["trial_a.py"]);

        let (result, _, _) = run_in_project(&tmp, "trial_a", "killed", Some("  "));

        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("reason is required"));
    }

    #[test]
    fn test_trial_not_found_returns_error() {
        let tmp = TempDir::new().unwrap();
        let doc = sample_doc();
        let cfg = sample_config();
        // No trial files created
        setup_project(&tmp, &doc, &cfg, &[]);

        let (result, _, _) = run_in_project(&tmp, "nonexistent_trial", "survived", None);

        assert!(result.is_err());
        match result.unwrap_err() {
            NucleusError::TrialNotFound(name) => assert_eq!(name, "nonexistent_trial"),
            other => panic!("expected TrialNotFound, got: {:?}", other),
        }
    }

    #[test]
    fn test_flux_trial_file_accepted() {
        let tmp = TempDir::new().unwrap();
        let doc = sample_doc();
        let cfg = sample_config();
        // Create a .flux trial file instead of .py
        setup_project(&tmp, &doc, &cfg, &["trial_a.flux"]);

        let (result, _, _) = run_in_project(&tmp, "trial_a", "survived", None);

        assert!(result.is_ok());
    }

    #[test]
    fn test_case_insensitive_outcome() {
        let tmp = TempDir::new().unwrap();
        let doc = sample_doc();
        let cfg = sample_config();
        setup_project(&tmp, &doc, &cfg, &["trial_a.py"]);

        // "SURVIVED" should work (case-insensitive)
        let (result, _, _) = run_in_project(&tmp, "trial_a", "SURVIVED", None);

        assert!(result.is_ok());
    }

    #[test]
    fn test_check_and_move_killed_triggered() {
        let tmp = TempDir::new().unwrap();

        // Create a document where hypothesis has only one prediction
        let doc = HypothesisDocument {
            active: vec![Hypothesis {
                id: "H1".to_string(),
                claim: "Doomed hypothesis".to_string(),
                predictions: vec![Prediction {
                    number: 1,
                    text: "Single prediction".to_string(),
                    trial: "trial_doom".to_string(),
                    result: String::new(),
                    verdict: VerdictStatus::Pending,
                }],
            }],
            killed: vec![],
        };
        let cfg = sample_config();
        setup_project(&tmp, &doc, &cfg, &["trial_doom.py"]);

        let (result, hyp_content, _) =
            run_in_project(&tmp, "trial_doom", "killed", Some("failed badly"));

        assert!(result.is_ok());

        // Verify hypothesis moved to killed section
        let parsed = hypotheses::parse_hypotheses(&hyp_content).unwrap();
        assert!(parsed.active.is_empty(), "active should be empty after all predictions killed");
        assert_eq!(parsed.killed.len(), 1);
        assert_eq!(parsed.killed[0].id, "H1");
        // Note: the hypotheses parser lowercases verdict reasons
        assert_eq!(parsed.killed[0].reason, "failed badly");
    }

    #[test]
    fn test_validate_outcome_valid() {
        assert!(validate_outcome("survived").is_ok());
        assert!(validate_outcome("killed").is_ok());
        assert!(validate_outcome("Survived").is_ok());
        assert!(validate_outcome("KILLED").is_ok());
    }

    #[test]
    fn test_validate_outcome_invalid() {
        assert!(validate_outcome("unknown").is_err());
        assert!(validate_outcome("").is_err());
        assert!(validate_outcome("pass").is_err());
    }

    #[test]
    fn test_validate_trial_exists() {
        let tmp = TempDir::new().unwrap();
        let trials_dir = tmp.path().join("falsification").join("trials");
        fs::create_dir_all(&trials_dir).unwrap();
        fs::write(trials_dir.join("my_trial.py"), "# stub").unwrap();

        assert!(validate_trial_exists(&trials_dir, "my_trial").is_ok());
        assert!(validate_trial_exists(&trials_dir, "nonexistent").is_err());
    }
}
