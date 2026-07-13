//! Subprocess execution for discovery cells and falsification trials.
//!
//! Provides cell classification (by path extension and parent directory),
//! subprocess dispatch (python3, flux backtest, flux check), and verdict
//! extraction from stdout.

use std::path::Path;
use std::process::Command;

use super::NucleusError;

/// The type of a cell, determined by its file extension and location.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellType {
    /// A `.py` file in `discovery/cells/`
    PythonDiscovery,
    /// A `.flux` file in `discovery/cells/`
    FluxDiscovery,
    /// A `.py` file in `falsification/trials/`
    PythonTrial,
    /// A `.flux` file in `falsification/trials/`
    FluxTrial,
}

/// The result of executing a cell subprocess.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// The verdict extracted from a trial's stdout.
/// Defined locally to avoid circular dependencies with `hypotheses.rs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerdictResult {
    /// The hypothesis prediction survived the trial.
    Survived,
    /// The hypothesis prediction was killed, with a reason.
    Killed(String),
}

/// Determine the cell type from its path extension and parent directory.
///
/// The path must exist, have a recognized extension (`.py` or `.flux`),
/// and be located in either `discovery/cells/` or `falsification/trials/`.
pub fn classify_cell(cell_path: &Path) -> Result<CellType, NucleusError> {
    if !cell_path.exists() {
        return Err(NucleusError::FileNotFound(
            cell_path.display().to_string(),
        ));
    }

    let extension = cell_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    if extension != "py" && extension != "flux" {
        return Err(NucleusError::FileNotFound(format!(
            "unrecognized cell type (must be .py or .flux): {}",
            cell_path.display()
        )));
    }

    // Check parent directories to determine location.
    // We look for "discovery/cells" or "falsification/trials" in the path components.
    let path_str = cell_path.to_string_lossy();
    let normalized = path_str.replace('\\', "/");

    let in_discovery = normalized.contains("discovery/cells");
    let in_falsification = normalized.contains("falsification/trials");

    match (in_discovery, in_falsification, extension) {
        (true, false, "py") => Ok(CellType::PythonDiscovery),
        (true, false, "flux") => Ok(CellType::FluxDiscovery),
        (false, true, "py") => Ok(CellType::PythonTrial),
        (false, true, "flux") => Ok(CellType::FluxTrial),
        _ => Err(NucleusError::FileNotFound(format!(
            "cell not in a recognized location (must be in discovery/cells/ or falsification/trials/): {}",
            cell_path.display()
        ))),
    }
}

/// Execute a cell as a subprocess based on its type.
///
/// Dispatch rules:
/// - `PythonDiscovery` / `PythonTrial` → `python3 <cell_path>`
/// - `FluxTrial` → `flux backtest <cell_path>`
/// - `FluxDiscovery` → `flux check <cell_path>`
///
/// Returns `Python3NotFound` if python3 is not found on PATH.
/// Returns `SubprocessFailed` if the subprocess exits with a non-zero code.
pub fn execute_cell(cell_path: &Path, cell_type: CellType) -> Result<ExecutionResult, NucleusError> {
    let output = match cell_type {
        CellType::PythonDiscovery | CellType::PythonTrial => {
            Command::new("python3")
                .arg(cell_path)
                .output()
                .map_err(|e| {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        NucleusError::Python3NotFound
                    } else {
                        NucleusError::Io {
                            operation: format!("execute python3 {}", cell_path.display()),
                            source: e,
                        }
                    }
                })?
        }
        CellType::FluxTrial => {
            Command::new("flux")
                .arg("backtest")
                .arg(cell_path)
                .output()
                .map_err(|e| NucleusError::Io {
                    operation: format!("execute flux backtest {}", cell_path.display()),
                    source: e,
                })?
        }
        CellType::FluxDiscovery => {
            Command::new("flux")
                .arg("check")
                .arg(cell_path)
                .output()
                .map_err(|e| NucleusError::Io {
                    operation: format!("execute flux check {}", cell_path.display()),
                    source: e,
                })?
        }
    };

    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if exit_code != 0 {
        return Err(NucleusError::SubprocessFailed {
            code: exit_code,
            stderr: stderr.clone(),
        });
    }

    Ok(ExecutionResult {
        exit_code,
        stdout,
        stderr,
    })
}

/// Scan stdout for a verdict line.
///
/// Looks for lines matching:
/// - `VERDICT: SURVIVED` → `Some(VerdictResult::Survived)`
/// - `VERDICT: KILLED: <reason>` → `Some(VerdictResult::Killed(reason))`
///
/// Returns the first match found, or `None` if no verdict line is present.
pub fn extract_verdict(stdout: &str) -> Option<VerdictResult> {
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed == "VERDICT: SURVIVED" {
            return Some(VerdictResult::Survived);
        }
        if let Some(reason) = trimmed.strip_prefix("VERDICT: KILLED: ") {
            return Some(VerdictResult::Killed(reason.to_string()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // --- classify_cell tests ---

    #[test]
    fn classify_python_in_discovery_cells() {
        let tmp = TempDir::new().unwrap();
        let cells_dir = tmp.path().join("discovery").join("cells");
        fs::create_dir_all(&cells_dir).unwrap();
        let cell = cells_dir.join("explore.py");
        fs::write(&cell, "# python cell").unwrap();

        let result = classify_cell(&cell).unwrap();
        assert_eq!(result, CellType::PythonDiscovery);
    }

    #[test]
    fn classify_flux_in_discovery_cells() {
        let tmp = TempDir::new().unwrap();
        let cells_dir = tmp.path().join("discovery").join("cells");
        fs::create_dir_all(&cells_dir).unwrap();
        let cell = cells_dir.join("check_strat.flux");
        fs::write(&cell, "# flux cell").unwrap();

        let result = classify_cell(&cell).unwrap();
        assert_eq!(result, CellType::FluxDiscovery);
    }

    #[test]
    fn classify_python_in_falsification_trials() {
        let tmp = TempDir::new().unwrap();
        let trials_dir = tmp.path().join("falsification").join("trials");
        fs::create_dir_all(&trials_dir).unwrap();
        let cell = trials_dir.join("h1_cost.py");
        fs::write(&cell, "# trial").unwrap();

        let result = classify_cell(&cell).unwrap();
        assert_eq!(result, CellType::PythonTrial);
    }

    #[test]
    fn classify_flux_in_falsification_trials() {
        let tmp = TempDir::new().unwrap();
        let trials_dir = tmp.path().join("falsification").join("trials");
        fs::create_dir_all(&trials_dir).unwrap();
        let cell = trials_dir.join("h1_regime.flux");
        fs::write(&cell, "# flux trial").unwrap();

        let result = classify_cell(&cell).unwrap();
        assert_eq!(result, CellType::FluxTrial);
    }

    #[test]
    fn classify_file_not_found() {
        let path = Path::new("/nonexistent/discovery/cells/test.py");
        let result = classify_cell(path);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, NucleusError::FileNotFound(_)));
    }

    #[test]
    fn classify_unrecognized_location() {
        let tmp = TempDir::new().unwrap();
        let random_dir = tmp.path().join("somewhere").join("else");
        fs::create_dir_all(&random_dir).unwrap();
        let cell = random_dir.join("test.py");
        fs::write(&cell, "# lost file").unwrap();

        let result = classify_cell(&cell);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, NucleusError::FileNotFound(_)));
    }

    #[test]
    fn classify_unrecognized_extension() {
        let tmp = TempDir::new().unwrap();
        let cells_dir = tmp.path().join("discovery").join("cells");
        fs::create_dir_all(&cells_dir).unwrap();
        let cell = cells_dir.join("data.csv");
        fs::write(&cell, "a,b,c").unwrap();

        let result = classify_cell(&cell);
        assert!(result.is_err());
    }

    // --- extract_verdict tests ---

    #[test]
    fn extract_verdict_survived() {
        let stdout = "Running trial...\nVERDICT: SURVIVED\nDone.";
        let result = extract_verdict(stdout);
        assert_eq!(result, Some(VerdictResult::Survived));
    }

    #[test]
    fn extract_verdict_killed_with_reason() {
        let stdout = "Running trial...\nVERDICT: KILLED: low sharpe\nDone.";
        let result = extract_verdict(stdout);
        assert_eq!(result, Some(VerdictResult::Killed("low sharpe".to_string())));
    }

    #[test]
    fn extract_verdict_none_when_missing() {
        let stdout = "Running trial...\nCompleted successfully.\nNo issues found.";
        let result = extract_verdict(stdout);
        assert_eq!(result, None);
    }

    #[test]
    fn extract_verdict_multiline_finds_first() {
        let stdout = "Line 1\nLine 2\nVERDICT: SURVIVED\nLine 4\nVERDICT: KILLED: later";
        let result = extract_verdict(stdout);
        assert_eq!(result, Some(VerdictResult::Survived));
    }

    #[test]
    fn extract_verdict_killed_preserves_full_reason() {
        let stdout = "VERDICT: KILLED: PF < 0.8 across all lookback windows";
        let result = extract_verdict(stdout);
        assert_eq!(
            result,
            Some(VerdictResult::Killed(
                "PF < 0.8 across all lookback windows".to_string()
            ))
        );
    }

    #[test]
    fn extract_verdict_ignores_partial_match() {
        let stdout = "This is not a VERDICT: SURVIVED line\nVERDICT: something else";
        let result = extract_verdict(stdout);
        // "VERDICT: something else" doesn't match SURVIVED or KILLED: pattern
        assert_eq!(result, None);
    }

    #[test]
    fn extract_verdict_with_leading_whitespace() {
        let stdout = "  VERDICT: SURVIVED  ";
        let result = extract_verdict(stdout);
        assert_eq!(result, Some(VerdictResult::Survived));
    }

    #[test]
    fn extract_verdict_empty_stdout() {
        let result = extract_verdict("");
        assert_eq!(result, None);
    }
}
