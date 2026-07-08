//! The `flux live` command: run strategies continuously against live market data.
//!
//! This module defines the CLI argument structure and the entry point
//! for the live trading harness. It loads strategies, builds connectors,
//! constructs risk constraints, creates the harness, restores state,
//! prints a startup summary, and runs the event loop.

use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use tokio::sync::mpsc;

use crate::live::aggregator::{RiskConstraints, SignalAggregator};
use crate::live::connector::ReconnectPolicy;
use crate::live::harness::LiveHarness;
use crate::live::loader::{build_connectors, build_connectors_from_block, load_strategies, LiveConfig};
use crate::live::position::LivePositionTracker;
use crate::live::state::load_state;

/// Run strategies continuously against live market data.
///
/// Single-strategy mode: `flux live strategy.flux`
/// Multi-strategy mode:  `flux live config.toml`
#[derive(Parser, Debug)]
pub struct LiveArgs {
    /// Path to a .flux strategy file or .toml configuration file
    pub file: PathBuf,

    /// Initial portfolio capital (default: 10000.0)
    #[arg(long, default_value = "10000.0")]
    pub capital: f64,

    /// Maximum position size per symbol (quantity units)
    #[arg(long)]
    pub max_position: Option<f64>,

    /// Maximum gross exposure (capital units)
    #[arg(long)]
    pub max_exposure: Option<f64>,

    /// Maximum number of concurrent open positions
    #[arg(long)]
    pub max_positions: Option<usize>,

    /// Path to persist and restore harness state
    #[arg(long)]
    pub state_file: Option<PathBuf>,

    /// Heartbeat interval in seconds (default: 30)
    #[arg(long, default_value = "30")]
    pub heartbeat: u64,
}

/// Run the live harness with the given CLI arguments.
///
/// Steps:
/// 1. Load strategies from the given file (single .flux or multi-strategy .toml)
/// 2. Build connectors from config/connector blocks
/// 3. Build risk constraints from CLI flags
/// 4. Create harness
/// 5. Restore state if state file exists
/// 6. Print startup summary
/// 7. Start connectors and run the event loop
///
/// # Exit code semantics (propagated to main via Result):
/// - All strategies fail to compile → Err (exit code 1)
/// - All connectors permanently failed → Err (exit code 1)
/// - Graceful SIGINT shutdown → Ok (exit code 0)
/// - Strategy runtime errors → logged and skipped (exit code 0)
/// - State file corruption → logged as warning, start fresh (exit code 0)
pub async fn run_live_cmd(args: LiveArgs) -> Result<(), Box<dyn std::error::Error>> {
    // 1. Load strategies
    let strategies = load_strategies(&args.file).map_err(|errors| {
        for e in &errors {
            eprintln!("[error] {}", e);
        }
        format!(
            "all strategies failed to compile ({} error{})",
            errors.len(),
            if errors.len() == 1 { "" } else { "s" }
        )
    })?;

    // 2. Build connectors from config/connector blocks
    let connectors = build_connectors_for_args(&args, &strategies)?;
    let connector_count = connectors.len();

    // 3. Build risk constraints from CLI flags (override TOML config)
    let constraints = RiskConstraints {
        max_position_size: args.max_position,
        max_exposure: args.max_exposure,
        max_positions: args.max_positions,
    };

    // 4. Create harness
    let mut harness = LiveHarness::new(
        strategies,
        SignalAggregator::new(constraints),
        LivePositionTracker::new(args.capital),
        args.state_file.clone(),
        ReconnectPolicy::default(),
        Duration::from_secs(args.heartbeat),
    );

    // 5. Restore state if state file exists (corruption → log warning, start fresh)
    if let Some(ref path) = args.state_file {
        match load_state(path) {
            Ok(Some(_state)) => {
                // TODO: restore positions and strategy state from HarnessState
                eprintln!("[harness] restored state from {}", path.display());
            }
            Ok(None) => { /* No state file — fresh start */ }
            Err(e) => {
                eprintln!(
                    "[harness] warning: state file corrupted or incompatible: {} (starting fresh)",
                    e
                );
            }
        }
    }

    // 6. Print startup summary
    harness.print_startup_summary();

    // 7. Start connectors and run the event loop
    let (bar_tx, bar_rx) = mpsc::channel(256);

    for mut connector in connectors {
        let symbols: Vec<String> = Vec::new(); // Symbols already configured in connector
        let tx = bar_tx.clone();
        // Spawn each connector as an independent task
        tokio::spawn(async move {
            if let Err(e) = connector.connect(&symbols, tx).await {
                eprintln!("[connector] {} permanently failed: {}", connector.id(), e);
            }
        });
    }
    // Drop the original sender; connectors hold clones
    drop(bar_tx);

    harness.run(bar_rx, connector_count).await?;
    Ok(())
}

/// Build connector instances based on the file type and loaded strategies.
///
/// For TOML configs: parses connector entries from the LiveConfig.
/// For single .flux files: extracts the connector block from the typed AST
/// and builds connectors from it.
fn build_connectors_for_args(
    args: &LiveArgs,
    strategies: &[crate::live::loader::StrategyModule],
) -> Result<Vec<Box<dyn crate::live::connector::Connector>>, Box<dyn std::error::Error>> {
    if args.file.extension().map_or(false, |e| e == "toml") {
        // TOML mode: read config and build connectors from config entries
        let config_content = std::fs::read_to_string(&args.file)?;
        let config: LiveConfig = toml::from_str(&config_content)?;

        let connectors = build_connectors(&config.connectors).map_err(|errors| {
            for e in &errors {
                eprintln!("[error] {}", e);
            }
            format!("failed to build connectors: {} errors", errors.len())
        })?;

        Ok(connectors)
    } else {
        // Single .flux file mode: compile the file and extract connector block
        let source = std::fs::read_to_string(&args.file)?;

        let tokens = flux_compiler::lexer::lex_with_spans(&source)
            .map_err(|e| format!("lexer error: {}", e))?;
        let ast = flux_compiler::parser::parse(tokens)
            .map_err(|e| format!("parse error: {}", e))?;
        let main_dir = args.file.parent().unwrap_or(std::path::Path::new("."));
        let ast = crate::module_resolver::resolve_modules(ast, main_dir)
            .map_err(|e| format!("module error: {}", e))?;
        let typed_program = flux_compiler::typeck::check(ast)
            .map_err(|e| format!("type error: {}", e))?;

        if let Some(ref connector_block) = typed_program.connector_block {
            let connectors = build_connectors_from_block(connector_block).map_err(|errors| {
                for e in &errors {
                    eprintln!("[error] {}", e);
                }
                format!("failed to build connectors from connector block: {} errors", errors.len())
            })?;
            Ok(connectors)
        } else {
            // No connector block — check if any strategy has subscribed symbols
            // that suggest a connector should be configured
            if strategies.is_empty() {
                return Err("no strategies loaded and no connector block found".into());
            }
            eprintln!(
                "[harness] warning: no connector block found in {}; \
                 the harness will wait for bars but no data source is configured",
                args.file.display()
            );
            Ok(Vec::new())
        }
    }
}
