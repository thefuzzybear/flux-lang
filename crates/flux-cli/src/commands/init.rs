use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum InitError {
    #[error("invalid project name '{name}': {reason}")]
    InvalidName { name: String, reason: String },

    #[error("directory is not empty: {0}")]
    DirectoryNotEmpty(PathBuf),

    #[error("cannot determine project name from current directory")]
    CannotDeriveName,

    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// Validates a project name according to Flux naming rules:
/// - Must not be empty
/// - Must be at most 64 characters
/// - Must contain only alphanumeric characters, hyphens, and underscores
pub fn validate_name(name: &str) -> Result<(), InitError> {
    if name.is_empty() {
        return Err(InitError::InvalidName {
            name: name.to_string(),
            reason: "must not be empty".to_string(),
        });
    }

    if name.len() > 64 {
        return Err(InitError::InvalidName {
            name: name.to_string(),
            reason: "must be at most 64 characters".to_string(),
        });
    }

    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(InitError::InvalidName {
            name: name.to_string(),
            reason: "must contain only alphanumeric characters, hyphens, and underscores"
                .to_string(),
        });
    }

    Ok(())
}

/// Resolves the project directory and name.
/// If `name` is provided, the target is `cwd/name`.
/// If `name` is `None`, uses the current directory and derives the name from its basename.
pub fn resolve_project_dir(name: Option<&str>) -> Result<(PathBuf, String), InitError> {
    let cwd = env::current_dir()?;
    match name {
        Some(n) => Ok((cwd.join(n), n.to_string())),
        None => {
            let dir_name = cwd
                .file_name()
                .and_then(|os| os.to_str())
                .ok_or(InitError::CannotDeriveName)?;
            Ok((cwd.clone(), dir_name.to_string()))
        }
    }
}

/// Ensures that a directory is empty (or does not yet exist).
/// Returns `Err(DirectoryNotEmpty)` if the directory exists and contains any entries.
pub fn ensure_empty(dir: &Path) -> Result<(), InitError> {
    if dir.exists() {
        let mut entries = fs::read_dir(dir)?;
        if entries.next().is_some() {
            return Err(InitError::DirectoryNotEmpty(dir.to_path_buf()));
        }
    }
    Ok(())
}

/// Renders the `flux.toml` manifest content for a given project name.
/// This is exposed as a separate function so property tests can verify
/// round-tripping without filesystem access.
pub fn render_manifest(project_name: &str) -> String {
    format!(
        r#"[project]
name = "{}"
version = "0.1.0"
strategies_dir = "strategies"
data_dir = "data"
"#,
        project_name
    )
}

/// Writes the `flux.toml` manifest file into the given directory.
pub fn write_manifest(dir: &Path, project_name: &str) -> Result<(), InitError> {
    let content = render_manifest(project_name);
    fs::write(dir.join("flux.toml"), content)?;
    Ok(())
}

/// Writes the example strategy file to `strategies/example.flux` within the given directory.
/// The strategy contains params, state, and an `on bar` handler as a working reference.
pub fn write_example_strategy(dir: &Path) -> Result<(), InitError> {
    let strategies_dir = dir.join("strategies");
    fs::create_dir_all(&strategies_dir)?;
    let content = r#"from indicators import {sma}

strategy SmaCrossover {
    params {
        period = 5
    }

    state {
        bar_count = 0
    }

    on bar {
        bar_count = bar_count + 1
        avg = sma(close, period)

        if close > avg and not in_position {
            OPEN(symbol, 100.0)
        } elif close < avg and in_position {
            CLOSE(symbol)
        }
    }
}
"#;
    fs::write(strategies_dir.join("example.flux"), content)?;
    Ok(())
}

/// Writes a sample data CSV file to `data/sample.csv` within the given directory.
/// Provides a small working dataset so new users can immediately run a backtest.
pub fn write_sample_data(dir: &Path) -> Result<(), InitError> {
    let data_dir = dir.join("data");
    fs::create_dir_all(&data_dir)?;
    let content = "timestamp,symbol,open,high,low,close,volume\n\
                   2024-01-02,AAPL,185.50,186.75,185.10,186.20,1200000\n\
                   2024-01-03,AAPL,186.20,186.90,185.80,186.50,980000\n\
                   2024-01-04,AAPL,186.50,187.10,186.30,186.80,750000\n\
                   2024-01-05,AAPL,186.80,187.25,186.60,187.00,620000\n\
                   2024-01-08,AAPL,187.00,187.50,186.90,187.30,830000\n\
                   2024-01-09,AAPL,187.30,187.60,186.80,186.90,540000\n\
                   2024-01-10,AAPL,186.90,187.20,186.50,187.10,670000\n\
                   2024-01-11,AAPL,187.10,187.40,186.70,186.60,720000\n\
                   2024-01-12,AAPL,186.60,186.80,185.90,186.00,890000\n\
                   2024-01-15,AAPL,186.00,186.50,185.50,186.30,950000\n";
    fs::write(data_dir.join("sample.csv"), content)?;
    Ok(())
}

/// Writes a project README with quick-start instructions.
pub fn write_project_readme(dir: &Path, project_name: &str) -> Result<(), InitError> {
    let content = format!(
        r#"# {}

A Flux trading strategy project.

## Quick Start

```bash
# Check your strategy for errors
flux check strategies/example.flux

# Run a backtest
flux backtest strategies/example.flux --data data/sample.csv --capital 10000
```

## Project Structure

```
{}/
├── flux.toml              # Project manifest
├── strategies/
│   └── example.flux       # Example SMA crossover strategy
├── data/
│   └── sample.csv         # Sample OHLCV data (AAPL)
└── .gitignore
```

## Writing Strategies

See `strategies/example.flux` for a working SMA crossover strategy.

Key concepts:
- `params {{ }}` — configurable constants
- `state {{ }}` — variables that persist across bars
- `on bar {{ }}` — event handler called once per bar
- `OPEN(symbol, qty)` — open a position
- `CLOSE(symbol)` — close entire position
- `sma(value, period)` — simple moving average indicator
- `in_position` — boolean: true if you have an open position

## CSV Data Format

Your data CSV needs these columns (case-insensitive):
```csv
timestamp,symbol,open,high,low,close,volume
```
"#,
        project_name, project_name
    );
    fs::write(dir.join("README.md"), content)?;
    Ok(())
}

/// Writes a `.gitignore` file to the given directory with patterns for build artifacts,
/// market data, and OS-generated files.
pub fn write_gitignore(dir: &Path) -> Result<(), InitError> {
    let content = "# Build artifacts\ntarget/\n\n# Large data files (sample.csv is tracked)\ndata/*.csv\n!data/sample.csv\n\n# OS files\n.DS_Store\nThumbs.db\n";
    fs::write(dir.join(".gitignore"), content)?;
    Ok(())
}

/// Formats the success message displayed after project initialization.
/// The message contains the project name and the path to the created directory.
pub fn format_success_message(project_name: &str, project_dir: &Path) -> String {
    format!(
        "Created Flux project '{}' at {}",
        project_name,
        project_dir.display()
    )
}

/// Orchestrates the full `flux init` workflow:
/// validate name → resolve dir → create dir if needed → ensure empty →
/// create subdirectories → write manifest → write example → write gitignore → print success.
pub fn run_init(name: Option<&str>) -> Result<(), InitError> {
    // 1. Validate name if provided
    if let Some(n) = name {
        validate_name(n)?;
    }

    // 2. Resolve project directory and name
    let (project_dir, project_name) = resolve_project_dir(name)?;

    // 3. If no name was given (in-place mode), validate the derived name
    if name.is_none() {
        validate_name(&project_name)?;
    }

    // 4. Create directory if it doesn't exist
    if !project_dir.exists() {
        fs::create_dir_all(&project_dir)?;
    }

    // 5. Ensure directory is empty
    ensure_empty(&project_dir)?;

    // 6. Create subdirectories
    fs::create_dir_all(project_dir.join("strategies"))?;
    fs::create_dir_all(project_dir.join("data"))?;

    // 7. Write files
    write_manifest(&project_dir, &project_name)?;
    write_example_strategy(&project_dir)?;
    write_sample_data(&project_dir)?;
    write_project_readme(&project_dir, &project_name)?;
    write_gitignore(&project_dir)?;

    // 8. Print success message
    println!("{}", format_success_message(&project_name, &project_dir));

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use regex::Regex;

    // Feature: flux-init, Property 1: Name validation correctly partitions all strings
    // **Validates: Requirements 1.1, 1.4**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]
        #[test]
        fn prop_name_validation_partitions_all_strings(s in "\\PC{0,128}") {
            let re = Regex::new(r"^[a-zA-Z0-9_-]{1,64}$").unwrap();
            let expected_valid = re.is_match(&s);
            let result = validate_name(&s);
            prop_assert_eq!(
                result.is_ok(),
                expected_valid,
                "validate_name({:?}) returned {:?}, but regex match was {}",
                s,
                result,
                expected_valid
            );
        }
    }

    // Feature: flux-init, Property 2: Manifest round-trip preserves project name
    // **Validates: Requirements 2.2, 2.3**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]
        #[test]
        fn prop_manifest_round_trip_preserves_name(name in "[a-zA-Z0-9_-]{1,64}") {
            let manifest = render_manifest(&name);
            let parsed: toml::Value = toml::from_str(&manifest).unwrap();
            let project_name = parsed["project"]["name"].as_str().unwrap();
            prop_assert_eq!(project_name, name.as_str());
        }
    }

    // Feature: flux-init, Property 3: Success message contains project name
    // **Validates: Requirements 5.1, 5.3**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]
        #[test]
        fn prop_success_message_contains_project_name(name in "[a-zA-Z0-9_-]{1,64}") {
            let path = PathBuf::from(format!("/tmp/{}", name));
            let message = format_success_message(&name, &path);
            prop_assert!(
                message.contains(&name),
                "Success message {:?} does not contain project name {:?}",
                message,
                name
            );
        }
    }

    #[test]
    fn valid_names() {
        assert!(validate_name("my-project").is_ok());
        assert!(validate_name("a").is_ok());
        assert!(validate_name("x_1-2").is_ok());
        assert!(validate_name("Hello_World-123").is_ok());
    }

    #[test]
    fn rejects_empty_name() {
        let err = validate_name("").unwrap_err();
        match err {
            InitError::InvalidName { name, reason } => {
                assert_eq!(name, "");
                assert_eq!(reason, "must not be empty");
            }
            _ => panic!("expected InvalidName"),
        }
    }

    #[test]
    fn rejects_name_too_long() {
        let long_name = "a".repeat(65);
        let err = validate_name(&long_name).unwrap_err();
        match err {
            InitError::InvalidName { reason, .. } => {
                assert_eq!(reason, "must be at most 64 characters");
            }
            _ => panic!("expected InvalidName"),
        }
    }

    #[test]
    fn accepts_name_at_max_length() {
        let name = "a".repeat(64);
        assert!(validate_name(&name).is_ok());
    }

    #[test]
    fn rejects_invalid_characters() {
        let cases = vec!["has space", "foo/bar", "foo@bar", "hello!", "a.b"];
        for case in cases {
            let err = validate_name(case).unwrap_err();
            match err {
                InitError::InvalidName { reason, .. } => {
                    assert_eq!(
                        reason,
                        "must contain only alphanumeric characters, hyphens, and underscores"
                    );
                }
                _ => panic!("expected InvalidName for '{}'", case),
            }
        }
    }
}
