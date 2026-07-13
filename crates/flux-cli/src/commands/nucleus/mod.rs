//! The `flux nucleus` command group: hypothesis-driven strategy development.
//!
//! Provides subcommands for managing Nucleus projects through the scientific
//! development lifecycle: discovery → falsification → strategy → deployed.

pub mod config;
pub mod executor;
pub mod hypotheses;
pub mod init;
pub mod next;
pub mod promote;
pub mod run;
pub mod status;
pub mod verdict;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Hypothesis-driven strategy development framework.
#[derive(Parser, Debug)]
pub struct NucleusArgs {
    #[command(subcommand)]
    pub command: NucleusSubcommand,
}

/// Nucleus subcommands for managing the scientific development lifecycle.
#[derive(Subcommand, Debug)]
pub enum NucleusSubcommand {
    /// Create a new Nucleus project
    Init {
        /// Project name (alphanumeric, hyphens, underscores)
        name: String,
    },
    /// Show project state and progress
    Status,
    /// Execute a discovery cell or falsification trial
    Run {
        /// Path to the cell or trial file
        cell_path: PathBuf,
    },
    /// Advance to the next phase
    Promote,
    /// Record a trial outcome
    Verdict {
        /// Name of the trial file (without path)
        trial_name: String,
        /// Outcome: "survived" or "killed"
        outcome: String,
        /// Reason (required when outcome is "killed")
        reason: Option<String>,
    },
    /// Suggest next action based on current project state
    Next,
}

/// Errors that can occur during Nucleus operations.
#[derive(Debug, thiserror::Error)]
pub enum NucleusError {
    #[error("not a Nucleus project (nucleus.toml not found in current directory)")]
    ConfigNotFound,

    #[error("failed to parse nucleus.toml: {0}")]
    ConfigParseError(String),

    #[error("invalid phase '{0}' in nucleus.toml (valid: discovery, falsification, strategy, deployed)")]
    InvalidPhase(String),

    #[error("failed to parse hypotheses.md: {0}")]
    HypothesesParseError(String),

    #[error("directory '{0}' already exists")]
    DirectoryExists(String),

    #[error("invalid project name '{0}' (only alphanumeric, hyphens, underscores allowed)")]
    InvalidName(String),

    #[error("permission denied: {0}")]
    PermissionDenied(String),

    #[error("cell not found: {0}")]
    FileNotFound(String),

    #[error("python3 not found on PATH (required for .py cells)")]
    Python3NotFound,

    #[error("cell exited with code {code}")]
    SubprocessFailed { code: i32, stderr: String },

    #[error("cannot promote — unmet criteria")]
    GateNotMet(Vec<String>),

    #[error("trial '{0}' not found in falsification/trials/")]
    TrialNotFound(String),

    #[error("{operation} failed: {source}")]
    Io {
        operation: String,
        source: std::io::Error,
    },
}

/// Dispatch a nucleus subcommand to the appropriate handler.
pub fn run_nucleus(args: NucleusArgs) -> Result<(), NucleusError> {
    match args.command {
        NucleusSubcommand::Init { name } => init::run_init(&name),
        NucleusSubcommand::Status => status::run_status(),
        NucleusSubcommand::Run { cell_path } => run::run_cell(&cell_path),
        NucleusSubcommand::Promote => promote::run_promote(),
        NucleusSubcommand::Verdict {
            trial_name,
            outcome,
            reason,
        } => verdict::run_verdict(&trial_name, &outcome, reason.as_deref()),
        NucleusSubcommand::Next => next::run_next(),
    }
}
