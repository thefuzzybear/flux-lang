//! Account runtime boot orchestration.
//!
//! Contains helpers for wiring AccountConfig into a running LiveHarness.
//! The main entry point is `boot_account_runtime()`, which orchestrates the
//! complete startup sequence for account-directory mode (`flux live dir/`).

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::interpreter::Interpreter;
use crate::live::account_config::AccountConfig;
use crate::live::aggregator::{RiskConstraints, SignalAggregator};
use crate::live::broker::ibkr::IbkrAdapter;
use crate::live::broker::mock::MockBrokerAdapter;
use crate::live::broker::{resolve_execution_policy, BrokerAdapter, DeduplicationGuard, ExecutionPolicy};
use crate::live::connector::{Connector, ReconnectPolicy};
use crate::live::futures_roll::{parse_generic, FuturesRollManager, GenericSymbol};
use crate::live::harness::LiveHarness;
use crate::live::loader::StrategyModule;
use crate::live::market_calendar::MarketCalendar;
use crate::live::position::LivePositionTracker;
use crate::live::product_registry::ProductRegistry;
use crate::live::risk_limits::{RiskLimits, RiskLimitsConfig};
use crate::live::storage::StorageBackend;

/// Boot an AccountRuntime from a validated AccountConfig.
///
/// Orchestrates the complete startup sequence:
/// 1. Load market calendar (optional — warning if absent)
/// 2. Build ProductRegistry from config.products
/// 3. Build RiskLimits from config.risk + registry + calendar
/// 4. Load and compile strategy modules (partial failure tolerated)
/// 5. Resolve ExecutionPolicy for each strategy
/// 6. Build SignalAggregator with allocation constraints
/// 7. Connect broker adapter with exponential backoff retry
/// 8. Reconcile DeduplicationGuard against broker open orders
/// 9. Build LiveHarness with all components
/// 10. Load and restore checkpoint from storage (if available)
/// 11. Print startup summary and enter event loop
///
/// # Errors
/// - All strategies fail to compile → exit code 1
/// - Broker connection fails after 5-minute retry → exit code 1
/// - Market calendar parse error (file exists but invalid) → exit code 1
pub async fn boot_account_runtime(
    config: AccountConfig,
    account_dir: &Path,
    storage: Option<Arc<dyn StorageBackend>>,
) -> Result<(), Box<dyn std::error::Error>> {
    // 1. Load market calendar (optional — warning if absent)
    let calendar_path = account_dir.join("market_calendar.toml");
    let calendar = if calendar_path.exists() {
        match MarketCalendar::from_file(&calendar_path) {
            Ok(cal) => {
                eprintln!("[boot] loaded market calendar from {}", calendar_path.display());
                Some(cal)
            }
            Err(e) => {
                return Err(AccountRuntimeError::CalendarParseError(
                    format!("{}: {}", calendar_path.display(), e),
                )
                .into());
            }
        }
    } else {
        eprintln!(
            "[boot] warning: no market_calendar.toml found — proceeding without session awareness"
        );
        None
    };

    // 2. Build ProductRegistry from config.products
    let registry = ProductRegistry::from_entries(&config.products);

    // 3. Build RiskLimits from config.risk + registry + calendar
    let risk_limits = {
        let risk_config = RiskLimitsConfig {
            max_daily_loss: config.risk.max_daily_loss,
            max_weekly_loss: config.risk.max_weekly_loss,
            max_position_per_product: config.risk.max_position_per_product as u32,
            max_total_notional: config.risk.max_total_notional,
            max_drawdown_pct: config.risk.max_drawdown_pct,
            correlation_warning_threshold: config.risk.correlation_warning_threshold as usize,
            initial_equity: config.risk.initial_equity,
        };
        if let Some(ref cal) = calendar {
            Some(
                RiskLimits::new(risk_config, registry.clone(), cal.clone())
                    .map_err(|e| format!("risk limits config error: {}", e))?,
            )
        } else {
            // RiskLimits requires a calendar — use a minimal empty one
            // Without a real calendar we still need risk enforcement, so parse
            // a minimal TOML with no sessions/holidays
            let minimal_toml = r#"
[[session]]
exchange = "DEFAULT"
open = "00:00"
close = "23:59"
timezone = "US/Eastern"
"#;
            let default_cal = MarketCalendar::from_toml(minimal_toml)
                .map_err(|e| format!("internal calendar construction error: {}", e))?;
            Some(
                RiskLimits::new(risk_config, registry.clone(), default_cal)
                    .map_err(|e| format!("risk limits config error: {}", e))?,
            )
        }
    };

    // 4. Load and compile strategy modules (partial failure tolerated)
    let strategies = load_strategies_from_config(&config, account_dir)
        .map_err(AccountRuntimeError::AllStrategiesFailed)?;

    eprintln!("[boot] loaded {} strategies", strategies.len());

    // 5. Resolve ExecutionPolicy for each strategy
    let execution_policies = build_execution_policies(&config);

    // 6. Build SignalAggregator with allocation constraints
    let constraints = RiskConstraints {
        max_position_size: None,
        max_exposure: Some(config.risk.max_total_notional),
        max_positions: Some(config.risk.max_position_per_product as usize),
    };
    let aggregator = SignalAggregator::new(constraints);

    // 7. Connect broker adapter (mock for replay, real for live)
    let broker_arc: Arc<dyn BrokerAdapter> = if config.data.source == "replay" {
        eprintln!("[boot] replay mode — using mock broker (no live execution)");
        Arc::new(MockBrokerAdapter::new())
    } else {
        connect_broker_with_retry(&config).await?
    };

    // 8. Reconcile DeduplicationGuard against broker open orders
    let mut dedup = DeduplicationGuard::new();
    if config.data.source != "replay" {
        match dedup.reconcile(broker_arc.as_ref()).await {
            Ok(open_ids) => {
                if !open_ids.is_empty() {
                    eprintln!(
                        "[boot] reconciled {} open orders into dedup guard",
                        open_ids.len()
                    );
                }
            }
            Err(e) => {
                eprintln!(
                    "[boot] warning: dedup reconciliation failed: {} — proceeding with empty guard",
                    e
                );
            }
        }
    }

    // 9. Build FuturesRollManager if generic symbols are present in data.symbols
    let futures_roll_manager = {
        // Detect generic symbols (contain '=') in the data block
        let generic_symbols: Vec<GenericSymbol> = config
            .data
            .symbols
            .iter()
            .filter_map(|s| parse_generic(s).ok())
            .collect();

        if generic_symbols.is_empty() {
            None
        } else {
            let cal = calendar.clone().unwrap_or_else(|| {
                // Fall back to a minimal calendar for the roll manager
                let minimal_toml = r#"
[[session]]
exchange = "CME"
open = "09:30"
close = "16:00"
timezone = "US/Eastern"
"#;
                MarketCalendar::from_toml(minimal_toml).expect("minimal calendar should parse")
            });
            let mut frm = FuturesRollManager::new(
                Arc::new(registry.clone()),
                Arc::new(cal),
            );

            // For replay mode, determine the start date from the CSV so we
            // initialize L1/L2 relative to the data's time frame, not today.
            let init_date = if config.data.source == "replay" {
                if let Some(ref replay_file) = config.data.replay_file {
                    let replay_path = account_dir.join(replay_file);
                    peek_first_date(&replay_path).unwrap_or_else(|| chrono::Utc::now().date_naive())
                } else {
                    chrono::Utc::now().date_naive()
                }
            } else {
                chrono::Utc::now().date_naive()
            };

            for sym in &generic_symbols {
                let strategy_name = config
                    .strategies
                    .first()
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| "default".to_string());

                // Use the init_date to determine L1/L2 contracts
                let contracts = crate::live::futures_roll::QuarterlyCycle::nearest_contracts(
                    &sym.root, init_date, 2,
                );
                if contracts.len() >= 2 {
                    frm.register_subscription_with_contracts(
                        sym.clone(),
                        strategy_name,
                        contracts[0].clone(),
                        contracts[1].clone(),
                    );
                } else {
                    frm.register_subscription(sym.clone(), strategy_name);
                }
            }
            let concrete_subs = frm.required_subscriptions();
            if !concrete_subs.is_empty() {
                eprintln!(
                    "[boot] futures roll manager active — subscribing to: {}",
                    concrete_subs.join(", ")
                );
            }

            // For replay mode with per-contract data: pre-scan the CSV to find all
            // roll points and compute backward adjustment factors. This seeds the
            // adjuster so that from bar 1, the =F series is fully backward-adjusted.
            if config.data.source == "replay" {
                if let Some(ref replay_file) = config.data.replay_file {
                    let replay_path = account_dir.join(replay_file);
                    let bars = crate::csv_loader::load_csv(&replay_path).ok();
                    if let Some(bars) = bars {
                        // Check if the data has per-contract symbols (ESH4, etc.)
                        // vs already-continuous symbols (ES=F)
                        let has_concrete = bars.iter().any(|b| {
                            crate::live::futures_roll::parse_concrete(&b.symbol).is_ok()
                        });

                        if has_concrete {
                            eprintln!("[boot] pre-scanning replay data for backward adjustment...");
                            // Run a dry-run of the roll manager to find all roll ratios
                            let today = chrono::Utc::now().date_naive();
                            let mut prescan_frm = FuturesRollManager::new(
                                frm.product_registry.clone(),
                                frm.calendar.clone(),
                            );
                            // Register same subscriptions on prescan copy
                            for sym in &generic_symbols {
                                let contracts = crate::live::futures_roll::QuarterlyCycle::nearest_contracts(
                                    &sym.root, init_date, 2,
                                );
                                if contracts.len() >= 2 {
                                    prescan_frm.register_subscription_with_contracts(
                                        sym.clone(),
                                        "prescan".to_string(),
                                        contracts[0].clone(),
                                        contracts[1].clone(),
                                    );
                                }
                            }
                            // Run all bars through the prescan to accumulate ratios
                            for bar in &bars {
                                let live_bar = crate::live::connector::LiveBar {
                                    bar: bar.clone(),
                                    connector_id: "prescan".to_string(),
                                    received_at: chrono::Utc::now(),
                                };
                                prescan_frm.process_daily_bar(&live_bar, today);
                            }
                            // Extract the cumulative factors and seed the real FRM
                            let prescan_state = prescan_frm.snapshot_state();
                            for adj in &prescan_state.adjusters {
                                if let Some(real_adj) = frm.adjusters.get_mut(&adj.product_root) {
                                    // The prescan computed the forward cumulative factor
                                    // (product of all ratios). Seed the real adjuster with this
                                    // as the initial factor and enable backward mode so that
                                    // each subsequent roll DIVIDES instead of multiplies.
                                    real_adj.cumulative_factor = adj.cumulative_factor;
                                    real_adj.adjustments = adj.adjustments.clone();
                                    real_adj.backward_mode = true;
                                    eprintln!(
                                        "[boot] {} backward adjustment factor: {:.6} ({} rolls)",
                                        adj.product_root,
                                        adj.cumulative_factor,
                                        adj.adjustments.len(),
                                    );
                                }
                            }
                            // Also restore roll history
                            frm.roll_history = prescan_state.roll_history;
                        }
                    }
                }
            }

            Some(frm)
        }
    };

    // 10. Build LiveHarness with all components
    // Set up position tracker with contract multipliers for generic symbols.
    // Maps "ES=F" → 50.0, "NQ=F" → 20.0, etc. from the product registry.
    let tracker = {
        let mut t = LivePositionTracker::new(config.risk.initial_equity);
        for product in &config.products {
            let multiplier = product.multiplier;
            // Set multiplier for the root symbol (ES, NQ, etc.)
            t.inner.set_multiplier(&product.name, multiplier);
            // Also set for generic symbol variants (ES=F, ES=1, ES=2, etc.)
            t.inner.set_multiplier(&format!("{}=F", product.name), multiplier);
            for n in 1..=4u8 {
                t.inner.set_multiplier(&format!("{}={}", product.name, n), multiplier);
            }
        }
        t
    };

    let mut harness = LiveHarness::new(
        strategies,
        aggregator,
        tracker,
        None, // state_file — using storage backend instead
        ReconnectPolicy::default(),
        Duration::from_secs(30), // heartbeat interval
        None,                    // fill_logger
        None,                    // checkpoint_scheduler
        if config.data.source == "replay" { None } else { risk_limits }, // skip risk limits in replay for backtest parity
        storage.clone(),
        calendar,
        None, // notifications — not yet wired; requires AlertConfig parsing in account.flux
        Some(broker_arc),
        execution_policies,
        dedup,
        futures_roll_manager,
    );

    // 11. Load and restore checkpoint from storage (if available)
    if let Some(ref store) = storage {
        match store.load_latest_checkpoint().await {
            Ok(Some(state)) => {
                eprintln!("[boot] restoring state from checkpoint");
                harness.restore_state(&state);
            }
            Ok(None) => {
                eprintln!("[boot] starting fresh — no checkpoint found");
            }
            Err(e) => {
                eprintln!(
                    "[boot] warning: checkpoint load failed: {} — starting fresh",
                    e
                );
            }
        }
    }

    // 12. Print startup summary and enter event loop
    harness.print_startup_summary();

    // Create channel for bars
    let (bar_tx, bar_rx) = mpsc::channel(256);

    // Wire data source based on config.data.source
    let connector_count = if config.data.source == "replay" {
        // Replay mode: read daily bars from CSV file
        if let Some(ref replay_file) = config.data.replay_file {
            let replay_path = account_dir.join(replay_file);
            if !replay_path.exists() {
                return Err(format!(
                    "replay_file not found: {} (resolved to {})",
                    replay_file,
                    replay_path.display()
                ).into());
            }
            eprintln!("[boot] replay mode — loading bars from {}", replay_path.display());

            let mut connector = crate::live::replay_connector::ReplayConnector::new(
                "databento-replay",
                replay_path,
                0.0, // instant playback for backtesting
            );

            let tx = bar_tx.clone();
            tokio::spawn(async move {
                if let Err(e) = connector.connect(&[], tx).await {
                    eprintln!("[replay] connector error: {}", e);
                }
            });
            drop(bar_tx);
            1
        } else {
            eprintln!("[boot] warning: source=replay but no replay_file specified");
            drop(bar_tx);
            0
        }
    } else {
        // Live mode: broker adapter feeds bars via its own mechanism
        drop(bar_tx);
        0
    };

    harness.run(bar_rx, connector_count).await?;
    Ok(())
}

/// Load strategy modules from AccountConfig entries, resolving paths
/// relative to the account directory.
///
/// Tolerates partial failures: if some strategies fail to compile,
/// the others are still loaded. Returns Err only when ALL fail.
pub fn load_strategies_from_config(
    config: &AccountConfig,
    account_dir: &Path,
) -> Result<Vec<StrategyModule>, Vec<(String, String)>> {
    let mut modules = Vec::new();
    let mut errors: Vec<(String, String)> = Vec::new();

    for strategy_entry in &config.strategies {
        let strategy_path = account_dir.join(&strategy_entry.path);

        // Check the path exists
        if !strategy_path.exists() {
            let abs_path = account_dir
                .join(&strategy_entry.path)
                .canonicalize()
                .unwrap_or_else(|_| account_dir.join(&strategy_entry.path));
            let err_msg = format!(
                "strategy file not found: {}",
                abs_path.display()
            );
            eprintln!("[boot] error: {}: {}", strategy_entry.name, err_msg);
            errors.push((strategy_entry.name.clone(), err_msg));
            continue;
        }

        // Read source
        let source = match std::fs::read_to_string(&strategy_path) {
            Ok(s) => s,
            Err(e) => {
                let err_msg = format!("failed to read file: {}", e);
                eprintln!("[boot] error: {}: {}", strategy_entry.name, err_msg);
                errors.push((strategy_entry.name.clone(), err_msg));
                continue;
            }
        };

        // Compile: lex → parse → resolve modules → typecheck → interpreter
        match compile_strategy_from_source(&source, &strategy_path) {
            Ok(module) => modules.push(module),
            Err(err_msg) => {
                eprintln!("[boot] error: {}: {}", strategy_entry.name, err_msg);
                errors.push((strategy_entry.name.clone(), err_msg));
            }
        }
    }

    // If ALL strategies failed, return error listing all failures
    if modules.is_empty() && !errors.is_empty() {
        return Err(errors);
    }

    // If no strategies were declared at all, that's also an error
    if modules.is_empty() && errors.is_empty() {
        return Err(vec![(
            "(none)".to_string(),
            "no strategies declared in manifest".to_string(),
        )]);
    }

    Ok(modules)
}

/// Compile a single strategy source file through the full pipeline.
///
/// Runs: lex → parse → module resolution → typecheck → interpreter construction.
/// Module imports are resolved relative to the strategy file's parent directory,
/// so `from lib::portfolio import { ... }` resolves to `./lib/portfolio.flux`.
///
/// # Errors
/// Returns a human-readable error string if any compilation stage fails
/// (lexer, parser, module resolution, or type checker).
fn compile_strategy_from_source(
    source: &str,
    strategy_path: &Path,
) -> Result<StrategyModule, String> {
    // Lex
    let tokens = flux_compiler::lexer::lex_with_spans(source)
        .map_err(|e| format!("lexer error: {}", e))?;

    // Parse
    let ast = flux_compiler::parser::parse(tokens)
        .map_err(|e| format!("parse error: {}", e))?;

    // Resolve modules relative to strategy file's parent directory
    let strategy_dir = strategy_path.parent().unwrap_or_else(|| Path::new("."));
    let ast = crate::module_resolver::resolve_modules(ast, strategy_dir)
        .map_err(|e| format!("module resolution error: {}", e))?;

    // Typecheck
    let typed_program = flux_compiler::typeck::check(ast)
        .map_err(|e| format!("type error: {}", e))?;

    // Extract strategy name
    let name = typed_program.strategy.name.clone();

    // Extract subscribed symbols from data_block or connector_block
    let subscribed_symbols = extract_symbols_from_program(&typed_program);

    // Create interpreter
    let interpreter = Interpreter::new(&typed_program);

    Ok(StrategyModule {
        name,
        source_path: strategy_path.to_path_buf(),
        interpreter,
        subscribed_symbols,
    })
}

/// Extract the list of subscribed symbols from a compiled program.
///
/// Checks `connector_block.symbols` first (live mode), then falls back
/// to `data_block.symbols` (backtest/replay mode). Returns an empty vec
/// if neither block declares symbols.
fn extract_symbols_from_program(
    program: &flux_compiler::typeck::typed_ast::TypedProgram,
) -> Vec<String> {
    if let Some(ref cb) = program.connector_block {
        if let Some(ref symbols) = cb.symbols {
            return symbols.clone();
        }
    }
    if let Some(ref db) = program.data_block {
        if let Some(ref symbols) = db.symbols {
            return symbols.clone();
        }
    }
    Vec::new()
}

/// Build execution policy map from AccountConfig.
///
/// Priority per strategy:
///   1. strategy.execution (if present) + strategy.execution_offset_ticks
///   2. config.execution_default (account-level, if present)
///   3. ExecutionPolicy::Market (hardcoded fallback)
pub fn build_execution_policies(config: &AccountConfig) -> HashMap<String, ExecutionPolicy> {
    let mut policies = HashMap::new();
    for strategy in &config.strategies {
        let policy = resolve_execution_policy(
            strategy.execution.as_deref(),
            strategy.execution_offset_ticks,
            config.execution_default.as_deref(),
        );
        policies.insert(strategy.name.clone(), policy);
    }
    policies
}

/// Connect to the broker with exponential backoff.
///
/// Schedule: 1s, 2s, 4s, 8s, 16s, 32s, 60s, 60s, ...
/// Timeout: exits with error after 5 minutes of continuous failure.
///
/// Gateway endpoint selection:
/// - mode = "paper" → uses configured port (from config.gateway.port)
/// - mode = "live"  → overrides to port 4001
///
/// Broker type dispatch:
/// - broker = "ibkr" → IbkrAdapter::connect(host, port, client_id)
/// - broker = "mock" → MockBrokerAdapter::new()
pub async fn connect_broker_with_retry(
    config: &AccountConfig,
) -> Result<Arc<dyn BrokerAdapter>, AccountRuntimeError> {
    // Determine port: live mode overrides to 4001, paper uses config port
    let port = if config.account.mode == "live" {
        4001u16
    } else {
        config.gateway.port as u16
    };

    let host = &config.gateway.host;

    // Mock broker doesn't need retry — it always succeeds
    if config.account.broker == "mock" {
        let adapter = MockBrokerAdapter::new();
        eprintln!("[broker] using mock broker adapter");
        return Ok(Arc::new(adapter));
    }

    // IbkrAdapter with exponential backoff retry
    let start = std::time::Instant::now();
    let max_duration = std::time::Duration::from_secs(300); // 5 minutes
    let mut attempt = 0u32;

    loop {
        match IbkrAdapter::connect(host, port, 1).await {
            Ok(adapter) => {
                eprintln!("[broker] connected to {}:{}", host, port);
                return Ok(Arc::new(adapter));
            }
            Err(e) => {
                if start.elapsed() >= max_duration {
                    return Err(AccountRuntimeError::BrokerConnectionFailed(format!(
                        "failed to connect to {}:{} after 5 minutes: {}",
                        host, port, e
                    )));
                }

                // Exponential backoff: 1s, 2s, 4s, 8s, 16s, 32s, 60s (cap)
                let delay_secs = std::cmp::min(2u64.pow(attempt), 60);
                eprintln!(
                    "[broker] connection attempt {} failed: {} — retrying in {}s",
                    attempt + 1,
                    e,
                    delay_secs
                );
                tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;
                attempt += 1;
            }
        }
    }
}

/// Errors that can occur when booting an account runtime.
#[derive(Debug, thiserror::Error)]
pub enum AccountRuntimeError {
    /// The broker adapter failed to connect after exhausting the 5-minute
    /// exponential backoff retry window. Usually indicates the gateway
    /// (TWS/IB Gateway) is not running or the port is misconfigured.
    #[error("broker connection failed after retries: {0}")]
    BrokerConnectionFailed(String),

    /// Every strategy declared in the account manifest failed to compile.
    /// Contains a vec of `(strategy_name, error_message)` pairs. Individual
    /// failures are tolerated — this only fires when zero strategies load.
    #[error("all strategies failed to compile:\n{}", .0.iter().map(|(n, e)| format!("  - {}: {}", n, e)).collect::<Vec<_>>().join("\n"))]
    AllStrategiesFailed(Vec<(String, String)>),

    /// The `market_calendar.toml` file exists but could not be parsed.
    /// A missing file is a warning (not an error); an unparseable file
    /// is fatal because it likely indicates a configuration mistake.
    #[error("market calendar parse error: {0}")]
    CalendarParseError(String),

    /// The storage backend (e.g. Postgres) failed to initialize during boot.
    /// Occurs when the connection string is invalid or the database is unreachable.
    #[error("storage initialization failed: {0}")]
    StorageInitFailed(String),

    /// An error propagated from the LiveHarness event loop after boot
    /// completed successfully. Wraps `LiveError` variants (connector
    /// disconnect, unrecoverable broker fault, etc.).
    #[error("live harness error: {0}")]
    HarnessError(#[from] crate::live::harness::LiveError),
}

/// Peek at the first data row of a CSV file to extract the start date.
///
/// Used in replay mode to initialize the futures roll manager's L1/L2
/// contracts relative to the historical data's time frame rather than
/// the current date.
///
/// Expects the standard CSV format with a `YYYY-MM-DD` date in the first
/// column and a header row.
///
/// # Returns
/// `Some(date)` if the file exists, has at least one data row, and the
/// first column parses as a valid date. `None` otherwise.
fn peek_first_date(csv_path: &Path) -> Option<chrono::NaiveDate> {
    use std::io::BufRead;
    let file = std::fs::File::open(csv_path).ok()?;
    let reader = std::io::BufReader::new(file);
    let mut lines = reader.lines();

    // Skip header
    let _ = lines.next()?;

    // Read first data line
    let first_line = lines.next()?.ok()?;
    let date_str = first_line.split(',').next()?;

    // Parse YYYY-MM-DD
    chrono::NaiveDate::parse_from_str(date_str.trim(), "%Y-%m-%d").ok()
}
