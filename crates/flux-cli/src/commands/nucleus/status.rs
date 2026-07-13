//! `flux nucleus status` — display current project state and progress.

use super::config::{self, Phase};
use super::hypotheses;
use super::NucleusError;

/// Print the current project status.
///
/// Reads `nucleus.toml` and `hypotheses.md` from the current working directory,
/// then prints phase-specific formatted output.
pub fn run_status() -> Result<(), NucleusError> {
    // Load config — propagates ConfigNotFound if not in a Nucleus project
    let cfg = config::load_config()?;

    // Read hypotheses.md from CWD
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

    // Print phase-specific output
    let output = format_status(&cfg, &doc);
    print!("{}", output);

    Ok(())
}

/// Format status output based on phase. Separated for testability.
pub fn format_status(
    cfg: &config::NucleusConfig,
    doc: &hypotheses::HypothesisDocument,
) -> String {
    let name = &cfg.nucleus.name;

    match &cfg.nucleus.phase {
        Phase::Discovery => {
            format!(
                "Nucleus: {}\nPhase: discovery\nCells run: {}\nFindings: {}\n",
                name, cfg.discovery.cell_count, cfg.discovery.findings_count
            )
        }
        Phase::Falsification => {
            let (total, tested, survived) = hypotheses::prediction_stats(doc);
            format!(
                "Nucleus: {}\nPhase: falsification\nHypothesis: {}\nPredictions: {}/{} tested, {} survived\n",
                name, cfg.falsification.current_hypothesis, tested, total, survived
            )
        }
        Phase::Strategy => {
            format!(
                "Nucleus: {}\nPhase: strategy\nStrategy file: {}\nLast backtest: {}\n",
                name, cfg.strategy.file, cfg.strategy.last_backtest
            )
        }
        Phase::Deployed => {
            format!(
                "Nucleus: {}\nPhase: deployed\nStrategy file: {}\n",
                name, cfg.strategy.file
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::nucleus::config::{
        DiscoveryState, FalsificationState, HypothesesState, NucleusConfig, NucleusMeta,
        Phase, StrategyState,
    };
    use crate::commands::nucleus::hypotheses::{
        Hypothesis, HypothesisDocument, Prediction, VerdictStatus,
    };

    fn make_config(phase: Phase) -> NucleusConfig {
        NucleusConfig {
            nucleus: NucleusMeta {
                name: "test-project".to_string(),
                created: "2026-07-13".to_string(),
                phase,
            },
            discovery: DiscoveryState {
                cell_count: 5,
                findings_count: 3,
            },
            hypotheses: HypothesesState {
                active: vec!["H1".to_string()],
                killed: vec![],
            },
            falsification: FalsificationState {
                current_hypothesis: "H1".to_string(),
                predictions_total: 4,
                predictions_tested: 2,
                predictions_survived: 1,
            },
            strategy: StrategyState {
                file: "strategy.flux".to_string(),
                last_backtest: "2026-07-15".to_string(),
                fidelity: 90,
            },
        }
    }

    fn make_empty_doc() -> HypothesisDocument {
        HypothesisDocument {
            active: vec![],
            killed: vec![],
        }
    }

    fn make_falsification_doc() -> HypothesisDocument {
        HypothesisDocument {
            active: vec![Hypothesis {
                id: "H1".to_string(),
                claim: "Pivot breakout has positive expectancy".to_string(),
                predictions: vec![
                    Prediction {
                        number: 1,
                        text: "PF > 1.5".to_string(),
                        trial: "h1_pf".to_string(),
                        result: "2.1".to_string(),
                        verdict: VerdictStatus::Survived,
                    },
                    Prediction {
                        number: 2,
                        text: "Survives slippage".to_string(),
                        trial: "h1_slip".to_string(),
                        result: String::new(),
                        verdict: VerdictStatus::Pending,
                    },
                    Prediction {
                        number: 3,
                        text: "Max drawdown < 15%".to_string(),
                        trial: "h1_dd".to_string(),
                        result: "12%".to_string(),
                        verdict: VerdictStatus::Survived,
                    },
                ],
            }],
            killed: vec![],
        }
    }

    #[test]
    fn test_format_status_discovery() {
        let cfg = make_config(Phase::Discovery);
        let doc = make_empty_doc();

        let output = format_status(&cfg, &doc);
        assert_eq!(
            output,
            "Nucleus: test-project\nPhase: discovery\nCells run: 5\nFindings: 3\n"
        );
    }

    #[test]
    fn test_format_status_falsification() {
        let cfg = make_config(Phase::Falsification);
        let doc = make_falsification_doc();

        let output = format_status(&cfg, &doc);
        // prediction_stats returns (total=3, tested=2, survived=2)
        assert_eq!(
            output,
            "Nucleus: test-project\nPhase: falsification\nHypothesis: H1\nPredictions: 2/3 tested, 2 survived\n"
        );
    }

    #[test]
    fn test_format_status_strategy() {
        let cfg = make_config(Phase::Strategy);
        let doc = make_empty_doc();

        let output = format_status(&cfg, &doc);
        assert_eq!(
            output,
            "Nucleus: test-project\nPhase: strategy\nStrategy file: strategy.flux\nLast backtest: 2026-07-15\n"
        );
    }

    #[test]
    fn test_format_status_deployed() {
        let cfg = make_config(Phase::Deployed);
        let doc = make_empty_doc();

        let output = format_status(&cfg, &doc);
        assert_eq!(
            output,
            "Nucleus: test-project\nPhase: deployed\nStrategy file: strategy.flux\n"
        );
    }

    #[test]
    fn test_run_status_config_not_found() {
        // Use a temp dir with no nucleus.toml
        let tmp = tempfile::TempDir::new().unwrap();
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let result = run_status();

        std::env::set_current_dir(original_dir).unwrap();

        assert!(result.is_err());
        match result.unwrap_err() {
            NucleusError::ConfigNotFound => {} // expected
            other => panic!("expected ConfigNotFound, got: {other:?}"),
        }
    }
}
