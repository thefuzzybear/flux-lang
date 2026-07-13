//! `flux nucleus next` — suggest the next action based on project state.

use std::path::Path;

use super::config::{load_config, NucleusConfig, Phase};
use super::hypotheses::{parse_hypotheses, HypothesisDocument, VerdictStatus};
use super::NucleusError;

/// Format the 3-line structured suggestion output.
fn format_suggestion(phase: &str, action: &str, context: &str) -> String {
    format!("Phase: {}\nAction: {}\nContext: {}", phase, action, context)
}

/// Determine the suggested next action based on config and hypothesis document.
///
/// Returns `(phase, action, context)` triple. This function is CWD-independent
/// and used by `run_next` as well as unit tests.
pub fn suggest_next(config: &NucleusConfig, doc: &HypothesisDocument) -> (String, String, String) {
    let phase_str = config.nucleus.phase.to_string();

    match &config.nucleus.phase {
        Phase::Discovery => {
            if config.discovery.findings_count == 0 {
                (
                    phase_str,
                    "Write first discovery cell".to_string(),
                    "No findings generated yet. Create a Python script at discovery/cells/01_explore.py".to_string(),
                )
            } else if doc.active.is_empty() {
                (
                    phase_str,
                    "Formulate hypothesis".to_string(),
                    format!(
                        "{} findings generated. Add a hypothesis with predictions to hypotheses.md",
                        config.discovery.findings_count
                    ),
                )
            } else {
                let first_id = &doc.active[0].id;
                (
                    phase_str,
                    "Add predictions or promote".to_string(),
                    format!(
                        "Hypothesis {} defined. Add predictions or run 'flux nucleus promote'",
                        first_id
                    ),
                )
            }
        }
        Phase::Falsification => {
            // Find first untested prediction across active hypotheses
            for hyp in &doc.active {
                for pred in &hyp.predictions {
                    if pred.verdict == VerdictStatus::Pending {
                        let suggested_name = format!(
                            "{}_{}.py",
                            hyp.id.to_lowercase(),
                            pred.text
                                .split_whitespace()
                                .take(3)
                                .collect::<Vec<_>>()
                                .join("_")
                                .to_lowercase()
                                .chars()
                                .filter(|c| c.is_alphanumeric() || *c == '_')
                                .collect::<String>()
                        );
                        return (
                            phase_str,
                            format!("Write trial for prediction {}", pred.number),
                            format!(
                                "Prediction {} of {}: '{}' needs a trial at falsification/trials/{}",
                                pred.number, hyp.id, pred.text, suggested_name
                            ),
                        );
                    }
                }
            }
            // All predictions tested
            (
                phase_str,
                "Review and promote".to_string(),
                "All predictions tested. Run 'flux nucleus promote' to advance".to_string(),
            )
        }
        Phase::Strategy => {
            let strategy_path = Path::new("strategy/strategy.flux");
            if !strategy_path.exists() {
                (
                    phase_str,
                    "Create strategy.flux".to_string(),
                    "Write your strategy at strategy/strategy.flux using surviving hypotheses"
                        .to_string(),
                )
            } else {
                (
                    phase_str,
                    "Refine and promote".to_string(),
                    "strategy/strategy.flux exists. Run 'flux nucleus promote' when ready"
                        .to_string(),
                )
            }
        }
        Phase::Deployed => (
            phase_str,
            "Monitor".to_string(),
            "Strategy deployed. Monitor performance and iterate".to_string(),
        ),
    }
}

/// A testable version of strategy phase suggestion that accepts a flag
/// for whether the strategy file exists (avoids filesystem dependency in tests).
pub fn suggest_next_with_strategy_exists(
    config: &NucleusConfig,
    doc: &HypothesisDocument,
    strategy_file_exists: bool,
) -> (String, String, String) {
    let phase_str = config.nucleus.phase.to_string();

    match &config.nucleus.phase {
        Phase::Strategy => {
            if !strategy_file_exists {
                (
                    phase_str,
                    "Create strategy.flux".to_string(),
                    "Write your strategy at strategy/strategy.flux using surviving hypotheses"
                        .to_string(),
                )
            } else {
                (
                    phase_str,
                    "Refine and promote".to_string(),
                    "strategy/strategy.flux exists. Run 'flux nucleus promote' when ready"
                        .to_string(),
                )
            }
        }
        // For non-strategy phases, delegate to the normal function
        _ => suggest_next(config, doc),
    }
}

/// Determine and print the suggested next action.
pub fn run_next() -> Result<(), NucleusError> {
    let config = load_config()?;

    // Load hypotheses.md
    let cwd = std::env::current_dir().map_err(|e| NucleusError::Io {
        operation: "get current directory".to_string(),
        source: e,
    })?;
    let hyp_path = cwd.join("hypotheses.md");
    let hyp_content = std::fs::read_to_string(&hyp_path).map_err(|e| NucleusError::Io {
        operation: format!("read {}", hyp_path.display()),
        source: e,
    })?;
    let doc = parse_hypotheses(&hyp_content)?;

    let (phase, action, context) = suggest_next(&config, &doc);
    println!("{}", format_suggestion(&phase, &action, &context));

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::nucleus::config::*;
    use crate::commands::nucleus::hypotheses::*;

    /// Helper: create a minimal config in the given phase with given discovery stats.
    fn make_config(phase: Phase, findings_count: u32) -> NucleusConfig {
        NucleusConfig {
            nucleus: NucleusMeta {
                name: "test-project".to_string(),
                created: "2026-01-01".to_string(),
                phase,
            },
            discovery: DiscoveryState {
                cell_count: if findings_count > 0 { findings_count } else { 0 },
                findings_count,
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

    /// Helper: create an empty hypothesis document.
    fn empty_doc() -> HypothesisDocument {
        HypothesisDocument {
            active: vec![],
            killed: vec![],
        }
    }

    /// Helper: create a doc with one hypothesis with predictions.
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

    #[test]
    fn discovery_no_findings_suggests_first_cell() {
        let config = make_config(Phase::Discovery, 0);
        let doc = empty_doc();

        let (phase, action, context) = suggest_next(&config, &doc);

        assert_eq!(phase, "discovery");
        assert_eq!(action, "Write first discovery cell");
        assert!(context.contains("No findings generated yet"));
        assert!(context.contains("discovery/cells/01_explore.py"));
    }

    #[test]
    fn discovery_findings_no_hypotheses_suggests_formulate() {
        let config = make_config(Phase::Discovery, 3);
        let doc = empty_doc();

        let (phase, action, context) = suggest_next(&config, &doc);

        assert_eq!(phase, "discovery");
        assert_eq!(action, "Formulate hypothesis");
        assert!(context.contains("3 findings generated"));
        assert!(context.contains("hypotheses.md"));
    }

    #[test]
    fn discovery_findings_with_hypothesis_suggests_promote() {
        let config = make_config(Phase::Discovery, 2);
        let doc = doc_with_hypothesis(vec![Prediction {
            number: 1,
            text: "PF > 1.5".to_string(),
            trial: "trial_a".to_string(),
            result: String::new(),
            verdict: VerdictStatus::Pending,
        }]);

        let (phase, action, context) = suggest_next(&config, &doc);

        assert_eq!(phase, "discovery");
        assert_eq!(action, "Add predictions or promote");
        assert!(context.contains("H1"));
        assert!(context.contains("flux nucleus promote"));
    }

    #[test]
    fn falsification_pending_prediction_suggests_write_trial() {
        let config = make_config(Phase::Falsification, 2);
        let doc = doc_with_hypothesis(vec![
            Prediction {
                number: 1,
                text: "PF above threshold".to_string(),
                trial: "h1_pf".to_string(),
                result: "2.1".to_string(),
                verdict: VerdictStatus::Survived,
            },
            Prediction {
                number: 2,
                text: "Survives slippage".to_string(),
                trial: String::new(),
                result: String::new(),
                verdict: VerdictStatus::Pending,
            },
        ]);

        let (phase, action, context) = suggest_next(&config, &doc);

        assert_eq!(phase, "falsification");
        assert_eq!(action, "Write trial for prediction 2");
        assert!(context.contains("Prediction 2 of H1"));
        assert!(context.contains("Survives slippage"));
        assert!(context.contains("falsification/trials/"));
    }

    #[test]
    fn falsification_all_tested_suggests_promote() {
        let config = make_config(Phase::Falsification, 2);
        let doc = doc_with_hypothesis(vec![
            Prediction {
                number: 1,
                text: "PF above threshold".to_string(),
                trial: "h1_pf".to_string(),
                result: "2.1".to_string(),
                verdict: VerdictStatus::Survived,
            },
            Prediction {
                number: 2,
                text: "Survives slippage".to_string(),
                trial: "h1_slip".to_string(),
                result: "1.8".to_string(),
                verdict: VerdictStatus::Survived,
            },
        ]);

        let (phase, action, context) = suggest_next(&config, &doc);

        assert_eq!(phase, "falsification");
        assert_eq!(action, "Review and promote");
        assert!(context.contains("All predictions tested"));
        assert!(context.contains("flux nucleus promote"));
    }

    #[test]
    fn strategy_no_file_suggests_create() {
        let config = make_config(Phase::Strategy, 2);
        let doc = empty_doc();

        let (phase, action, context) =
            suggest_next_with_strategy_exists(&config, &doc, false);

        assert_eq!(phase, "strategy");
        assert_eq!(action, "Create strategy.flux");
        assert!(context.contains("strategy/strategy.flux"));
        assert!(context.contains("surviving hypotheses"));
    }

    #[test]
    fn strategy_file_exists_suggests_refine() {
        let config = make_config(Phase::Strategy, 2);
        let doc = empty_doc();

        let (phase, action, context) =
            suggest_next_with_strategy_exists(&config, &doc, true);

        assert_eq!(phase, "strategy");
        assert_eq!(action, "Refine and promote");
        assert!(context.contains("strategy/strategy.flux exists"));
        assert!(context.contains("flux nucleus promote"));
    }

    #[test]
    fn deployed_suggests_monitor() {
        let config = make_config(Phase::Deployed, 5);
        let doc = empty_doc();

        let (phase, action, context) = suggest_next(&config, &doc);

        assert_eq!(phase, "deployed");
        assert_eq!(action, "Monitor");
        assert!(context.contains("Strategy deployed"));
        assert!(context.contains("Monitor performance"));
    }

    #[test]
    fn format_suggestion_produces_correct_output() {
        let output = format_suggestion("discovery", "Write first discovery cell", "No findings");
        assert_eq!(
            output,
            "Phase: discovery\nAction: Write first discovery cell\nContext: No findings"
        );
    }
}
