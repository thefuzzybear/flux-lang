//! `flux nucleus promote` — advance to the next phase when gate criteria are met.

use std::path::Path;

use super::config::{self, NucleusConfig, Phase};
use super::hypotheses::{self, HypothesisDocument};
use super::NucleusError;

/// Check whether the current phase's gate criteria are met.
///
/// Returns `Ok(())` if all criteria are satisfied, or `Err(criteria)` with a list
/// of human-readable strings describing unmet criteria.
pub fn check_gate(config: &NucleusConfig, doc: &HypothesisDocument) -> Result<(), Vec<String>> {
    match config.nucleus.phase {
        Phase::Discovery => check_discovery_gate(doc),
        Phase::Falsification => check_falsification_gate(doc),
        Phase::Strategy => check_strategy_gate(),
        Phase::Deployed => Err(vec![
            "Already at deployed phase — no further promotion".to_string(),
        ]),
    }
}

/// Discovery → Falsification: at least one hypothesis with at least one prediction.
fn check_discovery_gate(doc: &HypothesisDocument) -> Result<(), Vec<String>> {
    let has_hypothesis_with_prediction = doc
        .active
        .iter()
        .any(|h| !h.predictions.is_empty());

    if has_hypothesis_with_prediction {
        Ok(())
    } else {
        Err(vec![
            "At least one hypothesis with at least one prediction is required".to_string(),
        ])
    }
}

/// Falsification → Strategy: all predictions tested AND majority survived.
fn check_falsification_gate(doc: &HypothesisDocument) -> Result<(), Vec<String>> {
    let (total, tested, survived) = hypotheses::prediction_stats(doc);
    let mut unmet = Vec::new();

    if tested < total {
        unmet.push(format!(
            "All predictions must be tested ({}/{} tested)",
            tested, total
        ));
    }

    if total > 0 && survived <= total / 2 {
        unmet.push(format!(
            "Majority of predictions must survive ({}/{} survived)",
            survived, total
        ));
    }

    if unmet.is_empty() {
        Ok(())
    } else {
        Err(unmet)
    }
}

/// Strategy → Deployed: strategy/strategy.flux exists AND passes `flux check`.
fn check_strategy_gate() -> Result<(), Vec<String>> {
    let mut unmet = Vec::new();

    let strategy_path = Path::new("strategy/strategy.flux");
    if !strategy_path.exists() {
        unmet.push("strategy/strategy.flux must exist".to_string());
    } else {
        // Run `flux check strategy/strategy.flux`
        match std::process::Command::new("flux")
            .args(["check", "strategy/strategy.flux"])
            .output()
        {
            Ok(output) => {
                if !output.status.success() {
                    unmet.push("strategy/strategy.flux must pass flux check".to_string());
                }
            }
            Err(_) => {
                unmet.push("strategy/strategy.flux must pass flux check".to_string());
            }
        }
    }

    if unmet.is_empty() {
        Ok(())
    } else {
        Err(unmet)
    }
}

/// Advance the phase to the next one. Returns the new phase.
fn advance_phase(phase: &Phase) -> Phase {
    match phase {
        Phase::Discovery => Phase::Falsification,
        Phase::Falsification => Phase::Strategy,
        Phase::Strategy => Phase::Deployed,
        Phase::Deployed => Phase::Deployed, // Should not reach here due to gate check
    }
}

/// Check phase gate and promote to the next phase if criteria are met.
pub fn run_promote() -> Result<(), NucleusError> {
    // 1. Load config from CWD
    let mut cfg = config::load_config()?;

    // 2. Load hypotheses.md from CWD and parse
    let cwd = std::env::current_dir().map_err(|e| NucleusError::Io {
        operation: "get current directory".to_string(),
        source: e,
    })?;
    let hyp_path = cwd.join("hypotheses.md");
    let hyp_content = std::fs::read_to_string(&hyp_path).map_err(|e| NucleusError::Io {
        operation: format!("read {}", hyp_path.display()),
        source: e,
    })?;
    let doc = hypotheses::parse_hypotheses(&hyp_content)?;

    // 3. Check gate
    match check_gate(&cfg, &doc) {
        Ok(()) => {
            // 5. Advance phase and save
            let new_phase = advance_phase(&cfg.nucleus.phase);
            cfg.nucleus.phase = new_phase.clone();
            config::save_config(&cwd, &cfg)?;
            println!("Promoted to {}", new_phase);
            Ok(())
        }
        Err(criteria) => {
            // 4. Print each unmet criterion
            for criterion in &criteria {
                println!("  - {}", criterion);
            }
            Err(NucleusError::GateNotMet(criteria))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::nucleus::config::*;
    use crate::commands::nucleus::hypotheses::*;

    /// Helper: create a config with a given phase.
    fn config_with_phase(phase: Phase) -> NucleusConfig {
        NucleusConfig {
            nucleus: NucleusMeta {
                name: "test-project".to_string(),
                created: "2026-01-01".to_string(),
                phase,
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

    /// Helper: create an empty hypothesis document (no hypotheses).
    fn empty_doc() -> HypothesisDocument {
        HypothesisDocument {
            active: vec![],
            killed: vec![],
        }
    }

    /// Helper: create a doc with one hypothesis that has predictions.
    fn doc_with_hypothesis(predictions: Vec<Prediction>) -> HypothesisDocument {
        HypothesisDocument {
            active: vec![Hypothesis {
                id: "H1".to_string(),
                claim: "Test hypothesis".to_string(),
                predictions,
            }],
            killed: vec![],
        }
    }

    // --- Test 1: Gate: discovery with no hypotheses → fails ---
    #[test]
    fn gate_discovery_no_hypotheses_fails() {
        let cfg = config_with_phase(Phase::Discovery);
        let doc = empty_doc();

        let result = check_gate(&cfg, &doc);
        assert!(result.is_err());
        let criteria = result.unwrap_err();
        assert_eq!(criteria.len(), 1);
        assert_eq!(
            criteria[0],
            "At least one hypothesis with at least one prediction is required"
        );
    }

    // --- Test 2: Gate: discovery with hypothesis that has a prediction → passes ---
    #[test]
    fn gate_discovery_with_prediction_passes() {
        let cfg = config_with_phase(Phase::Discovery);
        let doc = doc_with_hypothesis(vec![Prediction {
            number: 1,
            text: "PF > 1.5".to_string(),
            trial: "trial_a".to_string(),
            result: String::new(),
            verdict: VerdictStatus::Pending,
        }]);

        let result = check_gate(&cfg, &doc);
        assert!(result.is_ok());
    }

    // --- Test 3: Gate: falsification with untested predictions → fails ---
    #[test]
    fn gate_falsification_untested_predictions_fails() {
        let cfg = config_with_phase(Phase::Falsification);
        let doc = doc_with_hypothesis(vec![
            Prediction {
                number: 1,
                text: "Pred A".to_string(),
                trial: "t1".to_string(),
                result: String::new(),
                verdict: VerdictStatus::Survived,
            },
            Prediction {
                number: 2,
                text: "Pred B".to_string(),
                trial: "t2".to_string(),
                result: String::new(),
                verdict: VerdictStatus::Pending, // untested
            },
        ]);

        let result = check_gate(&cfg, &doc);
        assert!(result.is_err());
        let criteria = result.unwrap_err();
        assert!(criteria
            .iter()
            .any(|c| c.contains("All predictions must be tested")));
    }

    // --- Test 4: Gate: falsification with all tested, majority survived → passes ---
    #[test]
    fn gate_falsification_all_tested_majority_survived_passes() {
        let cfg = config_with_phase(Phase::Falsification);
        let doc = doc_with_hypothesis(vec![
            Prediction {
                number: 1,
                text: "Pred A".to_string(),
                trial: "t1".to_string(),
                result: "2.3".to_string(),
                verdict: VerdictStatus::Survived,
            },
            Prediction {
                number: 2,
                text: "Pred B".to_string(),
                trial: "t2".to_string(),
                result: "1.8".to_string(),
                verdict: VerdictStatus::Survived,
            },
            Prediction {
                number: 3,
                text: "Pred C".to_string(),
                trial: "t3".to_string(),
                result: "0.5".to_string(),
                verdict: VerdictStatus::Killed("failed".to_string()),
            },
        ]);

        let result = check_gate(&cfg, &doc);
        assert!(result.is_ok());
    }

    // --- Test 5: Gate: falsification with all tested, majority killed → fails ---
    #[test]
    fn gate_falsification_all_tested_majority_killed_fails() {
        let cfg = config_with_phase(Phase::Falsification);
        let doc = doc_with_hypothesis(vec![
            Prediction {
                number: 1,
                text: "Pred A".to_string(),
                trial: "t1".to_string(),
                result: "0.5".to_string(),
                verdict: VerdictStatus::Killed("failed A".to_string()),
            },
            Prediction {
                number: 2,
                text: "Pred B".to_string(),
                trial: "t2".to_string(),
                result: "0.3".to_string(),
                verdict: VerdictStatus::Killed("failed B".to_string()),
            },
            Prediction {
                number: 3,
                text: "Pred C".to_string(),
                trial: "t3".to_string(),
                result: "1.8".to_string(),
                verdict: VerdictStatus::Survived,
            },
        ]);

        let result = check_gate(&cfg, &doc);
        assert!(result.is_err());
        let criteria = result.unwrap_err();
        assert!(criteria
            .iter()
            .any(|c| c.contains("Majority of predictions must survive")));
    }

    // --- Test 6: Gate: strategy with no strategy.flux → fails ---
    #[test]
    fn gate_strategy_no_file_fails() {
        // Run in a temp dir that has no strategy/strategy.flux
        let tmp = tempfile::TempDir::new().unwrap();
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let cfg = config_with_phase(Phase::Strategy);
        let doc = empty_doc();

        let result = check_gate(&cfg, &doc);

        std::env::set_current_dir(original_dir).unwrap();

        assert!(result.is_err());
        let criteria = result.unwrap_err();
        assert!(criteria
            .iter()
            .any(|c| c.contains("strategy/strategy.flux must exist")));
    }

    // --- Test 7: Gate: deployed → fails (cannot promote further) ---
    #[test]
    fn gate_deployed_cannot_promote() {
        let cfg = config_with_phase(Phase::Deployed);
        let doc = empty_doc();

        let result = check_gate(&cfg, &doc);
        assert!(result.is_err());
        let criteria = result.unwrap_err();
        assert_eq!(criteria.len(), 1);
        assert_eq!(
            criteria[0],
            "Already at deployed phase — no further promotion"
        );
    }

    // --- Additional: discovery with hypothesis but no predictions → fails ---
    #[test]
    fn gate_discovery_hypothesis_without_predictions_fails() {
        let cfg = config_with_phase(Phase::Discovery);
        let doc = HypothesisDocument {
            active: vec![Hypothesis {
                id: "H1".to_string(),
                claim: "Test hypothesis".to_string(),
                predictions: vec![], // no predictions
            }],
            killed: vec![],
        };

        let result = check_gate(&cfg, &doc);
        assert!(result.is_err());
    }

    // --- Additional: falsification with exactly half survived → fails (must be strict majority) ---
    #[test]
    fn gate_falsification_exactly_half_survived_fails() {
        let cfg = config_with_phase(Phase::Falsification);
        let doc = doc_with_hypothesis(vec![
            Prediction {
                number: 1,
                text: "Pred A".to_string(),
                trial: "t1".to_string(),
                result: "1.8".to_string(),
                verdict: VerdictStatus::Survived,
            },
            Prediction {
                number: 2,
                text: "Pred B".to_string(),
                trial: "t2".to_string(),
                result: "0.3".to_string(),
                verdict: VerdictStatus::Killed("failed".to_string()),
            },
        ]);

        let result = check_gate(&cfg, &doc);
        assert!(result.is_err());
        let criteria = result.unwrap_err();
        assert!(criteria
            .iter()
            .any(|c| c.contains("Majority of predictions must survive")));
    }
}
