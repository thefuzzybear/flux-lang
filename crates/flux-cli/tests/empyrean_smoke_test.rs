//! Smoke test: feed real daily data through Aether via the live harness.
//!
//! Uses empyrean/swing/aether/strategy/aether_daily.csv (real ES/NQ/RTY/YM daily bars).
//! Adds per-bar logging to see exactly where the strategy is in its decision pipeline.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use chrono::Utc;
use tokio::sync::mpsc;

use flux_cli::live::account_config::{
    AccountConfig, AccountSection, DataSection, DatabaseSection, GatewaySection, ProductEntry,
    RiskSection, StrategyEntry,
};
use flux_cli::live::account_runtime::{build_execution_policies, load_strategies_from_config};
use flux_cli::live::aggregator::{RiskConstraints, SignalAggregator};
use flux_cli::live::broker::DeduplicationGuard;
use flux_cli::live::connector::{LiveBar, ReconnectPolicy};
use flux_cli::live::harness::LiveHarness;
use flux_cli::live::position::LivePositionTracker;
use flux_runtime::BarContext;

fn aether_config() -> AccountConfig {
    AccountConfig {
        account: AccountSection {
            name: "swing".into(),
            broker: "mock".into(),
            account_id: "SMOKE".into(),
            mode: "paper".into(),
        },
        gateway: GatewaySection { host: "127.0.0.1".into(), port: 4002 },
        data: DataSection {
            source: "mock".into(),
            symbols: vec!["ES=F".into(), "NQ=F".into(), "RTY=F".into(), "YM=F".into()],
            interval: "1d".into(),
            replay_file: None,
        },
        database: DatabaseSection { url: "".into(), schema: "".into() },
        risk: RiskSection {
            max_daily_loss: -15000.0,
            max_weekly_loss: -30000.0,
            max_position_per_product: 10,
            max_total_notional: 3000000.0,
            max_drawdown_pct: 0.08,
            correlation_warning_threshold: 4,
            initial_equity: 500000.0,
        },
        products: vec![
            ProductEntry { name: "ES=F".into(), multiplier: 50.0, tick_size: 0.25, margin: 15840.0 },
            ProductEntry { name: "NQ=F".into(), multiplier: 20.0, tick_size: 0.25, margin: 21120.0 },
            ProductEntry { name: "RTY=F".into(), multiplier: 50.0, tick_size: 0.10, margin: 7920.0 },
            ProductEntry { name: "YM=F".into(), multiplier: 5.0, tick_size: 1.0, margin: 10560.0 },
        ],
        strategies: vec![StrategyEntry {
            name: "aether".into(),
            path: "aether/strategy/strategy.flux".into(),
            allocation: 1.0,
            priority: 1,
            execution: Some("market".into()),
            execution_offset_ticks: None,
        }],
        execution_default: None,
    }
}

/// Load bars from the aether_daily.csv file.
/// Returns bars in file order (interleaved by date: ES, NQ, RTY, YM per day).
fn load_daily_csv(path: &str, max_bars: usize) -> Vec<LiveBar> {
    use std::io::{BufRead, BufReader};
    use std::fs::File;

    let file = File::open(path).expect("failed to open aether_daily.csv");
    let reader = BufReader::new(file);
    let mut bars = Vec::new();

    for (i, line) in reader.lines().enumerate() {
        if i == 0 { continue; } // skip header
        if bars.len() >= max_bars { break; }

        let line = line.unwrap_or_default();
        let fields: Vec<&str> = line.split(',').collect();
        if fields.len() < 7 { continue; }

        // timestamp,symbol,open,high,low,close,volume
        let symbol = fields[1].trim().to_string();
        let open: f64 = fields[2].parse().unwrap_or(0.0);
        let high: f64 = fields[3].parse().unwrap_or(0.0);
        let low: f64 = fields[4].parse().unwrap_or(0.0);
        let close: f64 = fields[5].parse().unwrap_or(0.0);
        let volume: f64 = fields[6].parse().unwrap_or(0.0);

        if close == 0.0 { continue; }

        bars.push(LiveBar {
            bar: BarContext {
                open, high, low, close, volume,
                symbol,
                in_position: false,
            },
            connector_id: "csv_replay".to_string(),
            received_at: Utc::now(),
        });
    }
    bars
}

#[tokio::test]
async fn smoke_test_aether_real_daily() {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap().parent().unwrap().to_path_buf();
    let account_dir = workspace_root.join("empyrean/swing");
    let csv_path = workspace_root.join("empyrean/swing/aether/strategy/aether_daily.csv");

    if !csv_path.exists() {
        eprintln!("SKIP: aether_daily.csv not found at {}", csv_path.display());
        return;
    }

    eprintln!("\n======================================================================");
    eprintln!("  AETHER SMOKE TEST — Real Daily Data");
    eprintln!("======================================================================\n");

    // Load first 400 bars (100 trading days × 4 symbols)
    // This gives 100 days — plenty of warmup for 20-bar indicators
    let bars = load_daily_csv(csv_path.to_str().unwrap(), 400);
    eprintln!("[data] loaded {} bars from {}", bars.len(), csv_path.file_name().unwrap().to_str().unwrap());
    eprintln!("[data] first bar: {} {} close={}", bars[0].bar.symbol, "2018-07-18", bars[0].bar.close);
    eprintln!("[data] last bar:  {} close={}", bars.last().unwrap().bar.symbol, bars.last().unwrap().bar.close);

    // Count per symbol
    let mut sym_counts: HashMap<String, usize> = HashMap::new();
    for b in &bars {
        *sym_counts.entry(b.bar.symbol.clone()).or_insert(0) += 1;
    }
    eprintln!("[data] per-symbol: {:?}", sym_counts);

    let config = aether_config();
    let strategies = load_strategies_from_config(&config, &account_dir)
        .expect("Aether should compile");
    eprintln!("[boot] ✓ loaded strategy: {}", strategies[0].name);
    eprintln!("[boot] subscribed_symbols: {:?}", strategies[0].subscribed_symbols);

    let policies = build_execution_policies(&config);
    eprintln!("[boot] execution_policies: {:?}", policies);

    let mut harness = LiveHarness::new(
        strategies,
        SignalAggregator::new(RiskConstraints {
            max_position_size: None,
            max_exposure: Some(3000000.0),
            max_positions: Some(10),
        }),
        LivePositionTracker::new(500000.0),
        None,
        ReconnectPolicy::default(),
        Duration::from_secs(999_999),
        None, None, None, None, None, None,
        None, // no broker
        policies,
        DeduplicationGuard::new(),
        None, // futures_roll_manager
    );

    harness.print_startup_summary();

    // Inject bars — the harness will log [SIGNAL] for any signals emitted
    let (tx, rx) = mpsc::channel::<LiveBar>(4096);
    let bar_count = bars.len();

    // Log every 40th bar so we can see progress (every 10 trading days)
    for (i, bar) in bars.iter().enumerate() {
        if i < 8 || i % 40 == 0 || i == bar_count - 1 {
            eprintln!(
                "[bar {:>3}] {} | C={:.2} V={:.0}",
                i, bar.bar.symbol, bar.bar.close, bar.bar.volume
            );
        }
        tx.send(bar.clone()).await.unwrap();
    }
    drop(tx);

    eprintln!("\n[harness] processing {} bars...\n", bar_count);

    let result = harness.run(rx, 0).await;
    assert!(result.is_ok(), "harness error: {:?}", result.err());

    // Print final state
    let _equity = 500000.0;
    eprintln!("\n======================================================================");
    eprintln!("  RESULT: Processed {} bars", bar_count);
    eprintln!("  If no [SIGNAL] lines appeared above, the strategy didn't trigger.");
    eprintln!("  This means eligibility conditions (vol percentile + momentum) weren't met.");
    eprintln!("======================================================================");
}

/// Direct interpreter test — bypasses harness, logs internal state.
/// Feeds ALL symbols so Aether has full portfolio context.
#[test]
fn debug_aether_interpreter_state() {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap().parent().unwrap().to_path_buf();
    let account_dir = workspace_root.join("empyrean/swing");
    let csv_path = workspace_root.join("empyrean/swing/aether/strategy/aether_daily.csv");

    if !csv_path.exists() {
        eprintln!("SKIP: aether_daily.csv not found");
        return;
    }

    let config = aether_config();
    let mut strategies = load_strategies_from_config(&config, &account_dir)
        .expect("Aether should compile");
    let strategy = &mut strategies[0];

    eprintln!("\n[debug] Running Aether interpreter on all symbols...");

    let bars = load_daily_csv(csv_path.to_str().unwrap(), 400);
    eprintln!("[debug] {} bars loaded (100 days × 4 symbols)\n", bars.len());

    let mut signal_count = 0;
    for (i, live_bar) in bars.iter().enumerate() {
        let bar = &live_bar.bar;
        let signals = strategy.interpreter.on_bar(bar);

        if !signals.is_empty() {
            signal_count += signals.len();
            eprintln!(
                "[bar {:>3}] *** SIGNAL *** {} close={:.2} | {:?}",
                i, bar.symbol, bar.close, signals
            );
        }

        // Log state every 40 bars (~10 trading days) for ES=F bars only
        if bar.symbol == "ES=F" && i >= 80 && (i / 4) % 10 == 0 && i % 4 == 0 {
            let state = &strategy.interpreter.state;
            let bar_count_val = state.get("bar_count").map(|v| format!("{:?}", v)).unwrap_or("?".into());
            let vol_map = state.get("vol_pctile_map").map(|v| format!("{:?}", v)).unwrap_or("?".into());
            let mom_map = state.get("mom_score_map").map(|v| format!("{:?}", v)).unwrap_or("?".into());
            let elig_map = state.get("eligible_map").map(|v| format!("{:?}", v)).unwrap_or("?".into());

            eprintln!(
                "\n[bar {:>3}] ES=F close={:.2} | bar_count={}",
                i, bar.close, bar_count_val
            );
            eprintln!("  vol_pctile: {}", vol_map);
            eprintln!("  mom_score:  {}", mom_map);
            eprintln!("  eligible:   {}\n", elig_map);
        }
    }

    eprintln!("[debug] Finished: {} signals across {} bars", signal_count, bars.len());
    if signal_count == 0 {
        eprintln!("[debug] No signals. vol_pctile=1.0 means vol is too HIGH for entry.");
        eprintln!("[debug] Aether only enters when vol_pctile < 0.5 (low vol regime).");
        eprintln!("[debug] This is 2018 Q3-Q4 data — post-volmageddon = elevated vol. Expected behavior.");
    }
}
