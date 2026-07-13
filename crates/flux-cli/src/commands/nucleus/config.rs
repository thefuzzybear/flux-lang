//! Nucleus configuration parsing and serialization (`nucleus.toml`).

use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::NucleusError;

/// The complete nucleus.toml configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NucleusConfig {
    pub nucleus: NucleusMeta,
    pub discovery: DiscoveryState,
    pub hypotheses: HypothesesState,
    pub falsification: FalsificationState,
    pub strategy: StrategyState,
}

/// Nucleus project metadata (`[nucleus]` section).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NucleusMeta {
    pub name: String,
    pub created: String,
    pub phase: Phase,
}

/// Project lifecycle phase.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Discovery,
    Falsification,
    Strategy,
    Deployed,
}

impl fmt::Display for Phase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(phase_to_str(self))
    }
}

impl Phase {
    /// Parse a phase string into a Phase enum variant.
    pub fn from_str(s: &str) -> Result<Self, NucleusError> {
        parse_phase(s)
    }
}

/// Discovery phase state counters (`[discovery]` section).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscoveryState {
    pub cell_count: u32,
    pub findings_count: u32,
}

/// Hypotheses tracking state (`[hypotheses]` section).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HypothesesState {
    pub active: Vec<String>,
    pub killed: Vec<String>,
}

/// Falsification phase state (`[falsification]` section).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FalsificationState {
    pub current_hypothesis: String,
    pub predictions_total: u32,
    pub predictions_tested: u32,
    pub predictions_survived: u32,
}

/// Strategy phase state (`[strategy]` section).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrategyState {
    pub file: String,
    pub last_backtest: String,
    pub fidelity: u32,
}

/// Parse a nucleus.toml string into a `NucleusConfig`.
///
/// Returns `NucleusError::InvalidPhase` if the phase value is not recognized,
/// or `NucleusError::ConfigParseError` for other TOML issues.
pub fn parse_config(content: &str) -> Result<NucleusConfig, NucleusError> {
    // First try to detect invalid phase values before full deserialization,
    // so we can return a specific InvalidPhase error.
    if let Ok(raw) = content.parse::<toml::Table>() {
        if let Some(nucleus_table) = raw.get("nucleus").and_then(|v| v.as_table()) {
            if let Some(phase_val) = nucleus_table.get("phase").and_then(|v| v.as_str()) {
                // Validate the phase value
                parse_phase(phase_val)?;
            }
        }
    }

    toml::from_str(content).map_err(|e| NucleusError::ConfigParseError(e.to_string()))
}

/// Serialize a `NucleusConfig` to a TOML string.
pub fn serialize_config(config: &NucleusConfig) -> String {
    toml::to_string(config).expect("NucleusConfig should always serialize to valid TOML")
}

/// Load `nucleus.toml` from the current working directory.
///
/// Returns `NucleusError::ConfigNotFound` if the file does not exist.
pub fn load_config() -> Result<NucleusConfig, NucleusError> {
    let cwd = std::env::current_dir().map_err(|e| NucleusError::Io {
        operation: "get current directory".to_string(),
        source: e,
    })?;
    let path = cwd.join("nucleus.toml");
    if !path.exists() {
        return Err(NucleusError::ConfigNotFound);
    }
    let content = std::fs::read_to_string(&path).map_err(|e| NucleusError::Io {
        operation: format!("read {}", path.display()),
        source: e,
    })?;
    parse_config(&content)
}

/// Write a `NucleusConfig` to `nucleus.toml` in the given directory.
pub fn save_config(dir: &Path, config: &NucleusConfig) -> Result<(), NucleusError> {
    let path = dir.join("nucleus.toml");
    let content = serialize_config(config);
    std::fs::write(&path, content).map_err(|e| NucleusError::Io {
        operation: format!("write {}", path.display()),
        source: e,
    })
}

/// Parse a phase string into a `Phase` enum variant.
///
/// Valid values: "discovery", "falsification", "strategy", "deployed".
pub fn parse_phase(s: &str) -> Result<Phase, NucleusError> {
    match s {
        "discovery" => Ok(Phase::Discovery),
        "falsification" => Ok(Phase::Falsification),
        "strategy" => Ok(Phase::Strategy),
        "deployed" => Ok(Phase::Deployed),
        other => Err(NucleusError::InvalidPhase(other.to_string())),
    }
}

/// Convert a `Phase` to its lowercase string representation.
pub fn phase_to_str(phase: &Phase) -> &'static str {
    match phase {
        Phase::Discovery => "discovery",
        Phase::Falsification => "falsification",
        Phase::Strategy => "strategy",
        Phase::Deployed => "deployed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Helper: create a default config for testing.
    fn sample_config() -> NucleusConfig {
        NucleusConfig {
            nucleus: NucleusMeta {
                name: "pivot-master".to_string(),
                created: "2026-07-13".to_string(),
                phase: Phase::Discovery,
            },
            discovery: DiscoveryState {
                cell_count: 3,
                findings_count: 2,
            },
            hypotheses: HypothesesState {
                active: vec!["H1".to_string()],
                killed: vec!["H0".to_string()],
            },
            falsification: FalsificationState {
                current_hypothesis: "H1".to_string(),
                predictions_total: 4,
                predictions_tested: 2,
                predictions_survived: 1,
            },
            strategy: StrategyState {
                file: "strategy.flux".to_string(),
                last_backtest: "2026-07-14".to_string(),
                fidelity: 85,
            },
        }
    }

    #[test]
    fn round_trip_serialize_then_parse() {
        let config = sample_config();
        let serialized = serialize_config(&config);
        let parsed = parse_config(&serialized).expect("round-trip parse should succeed");
        assert_eq!(config, parsed);
    }

    #[test]
    fn round_trip_all_phases() {
        for phase in [
            Phase::Discovery,
            Phase::Falsification,
            Phase::Strategy,
            Phase::Deployed,
        ] {
            let mut config = sample_config();
            config.nucleus.phase = phase.clone();
            let serialized = serialize_config(&config);
            let parsed = parse_config(&serialized).expect("round-trip parse should succeed");
            assert_eq!(config, parsed);
        }
    }

    #[test]
    fn invalid_phase_returns_error() {
        let toml_content = r#"
[nucleus]
name = "test"
created = "2026-01-01"
phase = "bogus"

[discovery]
cell_count = 0
findings_count = 0

[hypotheses]
active = []
killed = []

[falsification]
current_hypothesis = ""
predictions_total = 0
predictions_tested = 0
predictions_survived = 0

[strategy]
file = ""
last_backtest = ""
fidelity = 0
"#;
        let result = parse_config(toml_content);
        assert!(result.is_err());
        match result.unwrap_err() {
            NucleusError::InvalidPhase(val) => assert_eq!(val, "bogus"),
            other => panic!("expected InvalidPhase, got: {other:?}"),
        }
    }

    #[test]
    fn missing_file_returns_config_not_found() {
        // Use a temp dir that definitely has no nucleus.toml.
        let tmp = TempDir::new().unwrap();
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let result = load_config();

        // Restore CWD before asserting (so test cleanup works).
        std::env::set_current_dir(original_dir).unwrap();

        assert!(result.is_err());
        match result.unwrap_err() {
            NucleusError::ConfigNotFound => {} // expected
            other => panic!("expected ConfigNotFound, got: {other:?}"),
        }
    }

    #[test]
    fn save_and_load_config() {
        let tmp = TempDir::new().unwrap();
        let config = sample_config();

        save_config(tmp.path(), &config).expect("save should succeed");

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let loaded = load_config().expect("load should succeed");

        std::env::set_current_dir(original_dir).unwrap();

        assert_eq!(config, loaded);
    }

    #[test]
    fn parse_phase_valid_values() {
        assert_eq!(parse_phase("discovery").unwrap(), Phase::Discovery);
        assert_eq!(parse_phase("falsification").unwrap(), Phase::Falsification);
        assert_eq!(parse_phase("strategy").unwrap(), Phase::Strategy);
        assert_eq!(parse_phase("deployed").unwrap(), Phase::Deployed);
    }

    #[test]
    fn parse_phase_invalid_value() {
        let result = parse_phase("invalid");
        assert!(result.is_err());
        match result.unwrap_err() {
            NucleusError::InvalidPhase(val) => assert_eq!(val, "invalid"),
            other => panic!("expected InvalidPhase, got: {other:?}"),
        }
    }

    #[test]
    fn phase_display() {
        assert_eq!(Phase::Discovery.to_string(), "discovery");
        assert_eq!(Phase::Falsification.to_string(), "falsification");
        assert_eq!(Phase::Strategy.to_string(), "strategy");
        assert_eq!(Phase::Deployed.to_string(), "deployed");
    }

    #[test]
    fn parse_config_with_empty_collections() {
        let toml_content = r#"
[nucleus]
name = "empty-test"
created = "2026-01-01"
phase = "discovery"

[discovery]
cell_count = 0
findings_count = 0

[hypotheses]
active = []
killed = []

[falsification]
current_hypothesis = ""
predictions_total = 0
predictions_tested = 0
predictions_survived = 0

[strategy]
file = ""
last_backtest = ""
fidelity = 0
"#;
        let config = parse_config(toml_content).expect("should parse empty config");
        assert_eq!(config.nucleus.name, "empty-test");
        assert_eq!(config.nucleus.phase, Phase::Discovery);
        assert!(config.hypotheses.active.is_empty());
        assert!(config.hypotheses.killed.is_empty());
    }

    #[test]
    fn parse_config_malformed_toml() {
        let result = parse_config("this is not valid toml [[[");
        assert!(result.is_err());
    }
}
