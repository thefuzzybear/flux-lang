//! `flux nucleus init` — scaffold a new Nucleus project.

use std::fs;
use std::path::Path;

use chrono::Local;

use super::NucleusError;

/// Validate that a nucleus project name contains only alphanumeric characters,
/// hyphens, and underscores (and is non-empty).
pub fn validate_name(name: &str) -> Result<(), NucleusError> {
    if name.is_empty() {
        return Err(NucleusError::InvalidName(name.to_string()));
    }
    for ch in name.chars() {
        if !ch.is_ascii_alphanumeric() && ch != '-' && ch != '_' {
            return Err(NucleusError::InvalidName(name.to_string()));
        }
    }
    Ok(())
}

/// Render the starter `nucleus.toml` content for a new project.
pub fn render_nucleus_toml(name: &str, date: &str) -> String {
    format!(
        r#"[nucleus]
name = "{name}"
created = "{date}"
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
"#
    )
}

/// Render the starter `hypotheses.md` content for a new project.
pub fn render_hypotheses_md() -> String {
    r#"# Hypotheses

## Active

(None yet — run discovery cells to generate hypotheses)

## Killed

(Empty — no hypotheses tested yet)
"#
    .to_string()
}

/// Create a new Nucleus project with the given name in the current directory.
///
/// Validates the name, checks that the directory does not already exist,
/// creates the full directory tree, and writes starter files.
pub fn run_init(name: &str) -> Result<(), NucleusError> {
    let cwd = std::env::current_dir().map_err(|e| NucleusError::Io {
        operation: "get current directory".to_string(),
        source: e,
    })?;
    run_init_in(&cwd, name)
}

/// Create a new Nucleus project with the given name under `base_dir`.
///
/// This is the internal implementation that avoids CWD dependency,
/// making it safe to call from tests running in parallel.
pub(crate) fn run_init_in(base_dir: &Path, name: &str) -> Result<(), NucleusError> {
    validate_name(name)?;

    let project_dir = base_dir.join(name);
    if project_dir.exists() {
        return Err(NucleusError::DirectoryExists(name.to_string()));
    }

    // Create directory tree
    let directories = [
        "",
        "discovery/cells",
        "discovery/findings",
        "discovery/data",
        "falsification/trials",
        "falsification/verdicts",
        "strategy",
        "strategy/lib",
        "artifacts",
    ];

    for dir in &directories {
        let path = if dir.is_empty() {
            project_dir.clone()
        } else {
            project_dir.join(dir)
        };
        fs::create_dir_all(&path).map_err(|e| NucleusError::Io {
            operation: format!("create directory {}", path.display()),
            source: e,
        })?;
    }

    // Write nucleus.toml
    let date = Local::now().format("%Y-%m-%d").to_string();
    let toml_content = render_nucleus_toml(name, &date);
    let toml_path = project_dir.join("nucleus.toml");
    fs::write(&toml_path, &toml_content).map_err(|e| NucleusError::Io {
        operation: format!("write {}", toml_path.display()),
        source: e,
    })?;

    // Write hypotheses.md
    let hypotheses_content = render_hypotheses_md();
    let hypotheses_path = project_dir.join("hypotheses.md");
    fs::write(&hypotheses_path, &hypotheses_content).map_err(|e| NucleusError::Io {
        operation: format!("write {}", hypotheses_path.display()),
        source: e,
    })?;

    println!("Created Nucleus project '{name}'");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // --- validate_name unit tests ---

    #[test]
    fn validate_name_accepts_valid_names() {
        assert!(validate_name("my-project").is_ok());
        assert!(validate_name("test_123").is_ok());
        assert!(validate_name("abc").is_ok());
        assert!(validate_name("A").is_ok());
        assert!(validate_name("hello-world_99").is_ok());
    }

    #[test]
    fn validate_name_rejects_empty() {
        let result = validate_name("");
        assert!(result.is_err());
        match result.unwrap_err() {
            NucleusError::InvalidName(n) => assert_eq!(n, ""),
            other => panic!("expected InvalidName, got: {other:?}"),
        }
    }

    #[test]
    fn validate_name_rejects_spaces() {
        let result = validate_name("my project");
        assert!(result.is_err());
        match result.unwrap_err() {
            NucleusError::InvalidName(n) => assert_eq!(n, "my project"),
            other => panic!("expected InvalidName, got: {other:?}"),
        }
    }

    #[test]
    fn validate_name_rejects_slashes() {
        let result = validate_name("foo/bar");
        assert!(result.is_err());
        match result.unwrap_err() {
            NucleusError::InvalidName(n) => assert_eq!(n, "foo/bar"),
            other => panic!("expected InvalidName, got: {other:?}"),
        }
    }

    #[test]
    fn validate_name_rejects_special_chars() {
        let result = validate_name("hello!");
        assert!(result.is_err());
        match result.unwrap_err() {
            NucleusError::InvalidName(n) => assert_eq!(n, "hello!"),
            other => panic!("expected InvalidName, got: {other:?}"),
        }
    }

    #[test]
    fn validate_name_rejects_dots() {
        assert!(validate_name("my.project").is_err());
    }

    // --- render function tests ---

    #[test]
    fn render_nucleus_toml_contains_name_and_date() {
        let content = render_nucleus_toml("test-proj", "2026-07-15");
        assert!(content.contains(r#"name = "test-proj""#));
        assert!(content.contains(r#"created = "2026-07-15""#));
        assert!(content.contains(r#"phase = "discovery""#));
        assert!(content.contains("cell_count = 0"));
        assert!(content.contains("findings_count = 0"));
    }

    #[test]
    fn render_nucleus_toml_is_parseable() {
        let content = render_nucleus_toml("my-project", "2026-01-01");
        let config: super::super::config::NucleusConfig =
            toml::from_str(&content).expect("rendered TOML should be valid");
        assert_eq!(config.nucleus.name, "my-project");
        assert_eq!(config.nucleus.created, "2026-01-01");
        assert_eq!(config.nucleus.phase, super::super::config::Phase::Discovery);
        assert_eq!(config.discovery.cell_count, 0);
    }

    #[test]
    fn render_hypotheses_md_has_sections() {
        let content = render_hypotheses_md();
        assert!(content.contains("# Hypotheses"));
        assert!(content.contains("## Active"));
        assert!(content.contains("## Killed"));
        assert!(content.contains("(None yet"));
        assert!(content.contains("(Empty"));
    }

    #[test]
    fn render_hypotheses_md_is_parseable() {
        let content = render_hypotheses_md();
        let doc = super::super::hypotheses::parse_hypotheses(&content)
            .expect("rendered hypotheses.md should be parseable");
        assert!(doc.active.is_empty());
        assert!(doc.killed.is_empty());
    }

    // --- run_init integration tests ---

    #[test]
    fn run_init_creates_full_scaffold() {
        let tmp = TempDir::new().unwrap();

        let result = run_init_in(tmp.path(), "my-nucleus");
        result.expect("run_init should succeed");

        let base = tmp.path().join("my-nucleus");
        assert!(base.exists(), "project directory should exist");
        assert!(base.join("nucleus.toml").is_file());
        assert!(base.join("hypotheses.md").is_file());
        assert!(base.join("discovery/cells").is_dir());
        assert!(base.join("discovery/findings").is_dir());
        assert!(base.join("discovery/data").is_dir());
        assert!(base.join("falsification/trials").is_dir());
        assert!(base.join("falsification/verdicts").is_dir());
        assert!(base.join("strategy").is_dir());
        assert!(base.join("strategy/lib").is_dir());
        assert!(base.join("artifacts").is_dir());

        // Verify nucleus.toml content
        let toml_content = std::fs::read_to_string(base.join("nucleus.toml")).unwrap();
        assert!(toml_content.contains(r#"name = "my-nucleus""#));
        assert!(toml_content.contains(r#"phase = "discovery""#));

        // Verify hypotheses.md content
        let hyp_content = std::fs::read_to_string(base.join("hypotheses.md")).unwrap();
        assert!(hyp_content.contains("## Active"));
        assert!(hyp_content.contains("## Killed"));
    }

    #[test]
    fn run_init_rejects_existing_directory() {
        let tmp = TempDir::new().unwrap();

        // Create the directory first
        std::fs::create_dir(tmp.path().join("existing-proj")).unwrap();

        let result = run_init_in(tmp.path(), "existing-proj");

        assert!(result.is_err());
        match result.unwrap_err() {
            NucleusError::DirectoryExists(name) => assert_eq!(name, "existing-proj"),
            other => panic!("expected DirectoryExists, got: {other:?}"),
        }
    }

    #[test]
    fn run_init_rejects_invalid_name() {
        let tmp = TempDir::new().unwrap();
        let result = run_init_in(tmp.path(), "bad name!");
        assert!(result.is_err());
        match result.unwrap_err() {
            NucleusError::InvalidName(n) => assert_eq!(n, "bad name!"),
            other => panic!("expected InvalidName, got: {other:?}"),
        }
    }
}
