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
use crate::live::connector::ReconnectPolicy;
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
        .map_err(|errs| AccountRuntimeError::AllStrategiesFailed(errs))?;

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

    // 7. Connect broker adapter with exponential backoff retry
    let broker_arc: Arc<dyn BrokerAdapter> = connect_broker_with_retry(&config).await?;

    // 8. Reconcile DeduplicationGuard against broker open orders
    let mut dedup = DeduplicationGuard::new();
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

    // 9. Build LiveHarness with all components
    let mut harness = LiveHarness::new(
        strategies,
        aggregator,
        LivePositionTracker::new(config.risk.initial_equity),
        None, // state_file — using storage backend instead
        ReconnectPolicy::default(),
        Duration::from_secs(30), // heartbeat interval
        None,                    // fill_logger
        None,                    // checkpoint_scheduler
        risk_limits,
        storage.clone(),
        calendar,
        None, // notifications — TODO: wire from AlertConfig when support is added
        Some(broker_arc),
        execution_policies,
        dedup,
    );

    // 10. Load and restore checkpoint from storage (if available)
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

    // 11. Print startup summary and enter event loop
    harness.print_startup_summary();

    // Create channel for bars — in account mode, the broker adapter/connectors
    // will feed bars through their own mechanism. For now we create the channel
    // and let the harness manage the event loop. The broker's data feed spawns
    // tasks that send bars into bar_tx.
    let (bar_tx, bar_rx) = mpsc::channel(256);

    // Drop sender — broker adapter feeds bars internally or via a connector.
    // In the future, IBKR historical data bar streaming would use bar_tx.
    drop(bar_tx);

    harness.run(bar_rx, 0).await?;
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
/// Resolves module imports relative to the strategy file's parent directory.
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

/// Extract symbols from a typed program's connector_block or data_block.
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
    #[error("broker connection failed after retries: {0}")]
    BrokerConnectionFailed(String),

    #[error("all strategies failed to compile:\n{}", .0.iter().map(|(n, e)| format!("  - {}: {}", n, e)).collect::<Vec<_>>().join("\n"))]
    AllStrategiesFailed(Vec<(String, String)>),

    #[error("market calendar parse error: {0}")]
    CalendarParseError(String),

    #[error("storage initialization failed: {0}")]
    StorageInitFailed(String),

    #[error("live harness error: {0}")]
    HarnessError(#[from] crate::live::harness::LiveError),
}
