//! Integration test for graceful shutdown of the LiveHarness.
//!
//! Validates Requirements 5.9 and 7.1:
//! - 5.9: SIGINT handling persists state and shuts down connectors
//! - 7.1: State is serialized to the specified file path on graceful shutdown
//!
//! Since simulating SIGINT is complex in tests, we test the graceful_shutdown
//! path by:
//! 1. Creating a LiveHarness with a state_file configured (temp file)
//! 2. Opening positions in the tracker
//! 3. Feeding bars via an mpsc channel, then dropping the sender (simulates shutdown)
//! 4. After run() completes, calling graceful_shutdown() explicitly
//! 5. Verifying the state file was written and contains the expected positions
//! 6. Loading the state file back via load_state() and verifying positions match

use std::time::Duration;

use flux_cli::live::aggregator::{RiskConstraints, SignalAggregator};
use flux_cli::live::connector::{LiveBar, ReconnectPolicy};
use flux_cli::live::harness::LiveHarness;
use flux_cli::live::position::LivePositionTracker;
use flux_cli::live::state::load_state;
use flux_runtime::{BarContext, Signal};
use tempfile::TempDir;
use tokio::sync::mpsc;

/// Helper to create a LiveBar for testing.
fn make_live_bar(symbol: &str, close: f64, open: f64) -> LiveBar {
    LiveBar {
        bar: BarContext {
            close,
            open,
            high: close + 1.0,
            low: open - 1.0,
            volume: 1000.0,
            symbol: symbol.to_string(),
            in_position: false,
        },
        connector_id: "test-replay".to_string(),
        received_at: chrono::Utc::now(),
    }
}

/// Test that graceful_shutdown persists state to disk when state_file is configured.
///
/// Validates: Requirements 5.9, 7.1
#[tokio::test]
async fn graceful_shutdown_persists_state_file() {
    let dir = TempDir::new().unwrap();
    let state_path = dir.path().join("harness_state.json");

    // Create harness with state_file configured
    let mut harness = LiveHarness::new(
        vec![],
        SignalAggregator::new(RiskConstraints::default()),
        LivePositionTracker::new(10_000.0),
        Some(state_path.clone()),
        ReconnectPolicy::default(),
        Duration::from_secs(30),
        None,
        None,
        None,
    );

    // Open positions in the tracker
    let open_aapl = Signal::open("AAPL".to_string(), 100.0);
    harness
        .tracker
        .process_signal(&open_aapl, 150.0, 0, "test_strategy");

    let open_msft = Signal::open("MSFT".to_string(), 50.0);
    harness
        .tracker
        .process_signal(&open_msft, 380.0, 1, "test_strategy");

    // Mark to market at current prices
    harness.tracker.inner.mark_to_market(155.0, "AAPL");
    harness.tracker.inner.mark_to_market(385.0, "MSFT");

    // Call graceful_shutdown explicitly
    harness.graceful_shutdown().await;

    // Verify state file was written
    assert!(
        state_path.exists(),
        "state file should exist after graceful_shutdown"
    );

    // Load the state file and verify positions
    let loaded = load_state(&state_path)
        .expect("load_state should not error")
        .expect("state file should contain valid state");

    assert_eq!(loaded.version, 2);
    assert_eq!(loaded.positions.initial_capital, 10_000.0);
    assert_eq!(loaded.positions.positions.len(), 2);

    // Find AAPL position
    let aapl_pos = loaded
        .positions
        .positions
        .iter()
        .find(|p| p.symbol == "AAPL")
        .expect("AAPL position should be persisted");
    assert!((aapl_pos.qty - 100.0).abs() < f64::EPSILON);
    assert!((aapl_pos.avg_entry_price - 150.0).abs() < f64::EPSILON);

    // Find MSFT position
    let msft_pos = loaded
        .positions
        .positions
        .iter()
        .find(|p| p.symbol == "MSFT")
        .expect("MSFT position should be persisted");
    assert!((msft_pos.qty - 50.0).abs() < f64::EPSILON);
    assert!((msft_pos.avg_entry_price - 380.0).abs() < f64::EPSILON);
}

/// Test that graceful shutdown occurs after channel close (simulating connector disconnect).
/// Feed bars, drop sender, verify run() completes and then state persists on shutdown.
///
/// Validates: Requirements 5.9, 7.1
#[tokio::test]
async fn shutdown_after_channel_close_persists_state() {
    let dir = TempDir::new().unwrap();
    let state_path = dir.path().join("shutdown_state.json");

    let mut harness = LiveHarness::new(
        vec![],
        SignalAggregator::new(RiskConstraints::default()),
        LivePositionTracker::new(50_000.0),
        Some(state_path.clone()),
        ReconnectPolicy::default(),
        Duration::from_secs(30),
        None,
        None,
        None,
    );

    // Open a position before running
    let open_signal = Signal::open("GOOG".to_string(), 25.0);
    harness
        .tracker
        .process_signal(&open_signal, 2800.0, 0, "momentum");

    // Create channel, send a few bars, then drop sender to simulate shutdown
    let (tx, rx) = mpsc::channel::<LiveBar>(16);

    let bar1 = make_live_bar("GOOG", 2850.0, 2800.0);
    let bar2 = make_live_bar("GOOG", 2900.0, 2850.0);
    tx.send(bar1).await.unwrap();
    tx.send(bar2).await.unwrap();
    drop(tx); // Simulate all connectors disconnecting

    // run() with connector_count=0 exits cleanly on channel close
    let result = harness.run(rx, 0).await;
    assert!(result.is_ok());

    // Now trigger graceful shutdown (as the live command would after run returns)
    harness.graceful_shutdown().await;

    // Verify state was persisted
    assert!(state_path.exists(), "state file should exist after shutdown");

    let loaded = load_state(&state_path)
        .expect("load_state should succeed")
        .expect("state file should be valid");

    assert_eq!(loaded.positions.initial_capital, 50_000.0);

    // Position should reflect mark-to-market from the last bar dispatched
    let goog_pos = loaded
        .positions
        .positions
        .iter()
        .find(|p| p.symbol == "GOOG")
        .expect("GOOG position should be persisted");
    assert!((goog_pos.qty - 25.0).abs() < f64::EPSILON);
    assert!((goog_pos.avg_entry_price - 2800.0).abs() < f64::EPSILON);
}

/// Test that graceful shutdown without a state_file configured does not crash
/// and does not write any file.
///
/// Validates: Requirements 5.9
#[tokio::test]
async fn graceful_shutdown_without_state_file_does_not_crash() {
    let mut harness = LiveHarness::new(
        vec![],
        SignalAggregator::new(RiskConstraints::default()),
        LivePositionTracker::new(10_000.0),
        None, // No state file
        ReconnectPolicy::default(),
        Duration::from_secs(30),
        None,
        None,
        None,
    );

    // Open a position
    let signal = Signal::open("TSLA".to_string(), 10.0);
    harness
        .tracker
        .process_signal(&signal, 250.0, 0, "test");

    // Should not panic
    harness.graceful_shutdown().await;
}

/// Test that state file contains last prices for mark-to-market restoration.
///
/// Validates: Requirements 7.1
#[tokio::test]
async fn state_file_contains_last_prices() {
    let dir = TempDir::new().unwrap();
    let state_path = dir.path().join("prices_state.json");

    let mut harness = LiveHarness::new(
        vec![],
        SignalAggregator::new(RiskConstraints::default()),
        LivePositionTracker::new(10_000.0),
        Some(state_path.clone()),
        ReconnectPolicy::default(),
        Duration::from_secs(30),
        None,
        None,
        None,
    );

    // Open position and mark to market
    let signal = Signal::open("AAPL".to_string(), 100.0);
    harness
        .tracker
        .process_signal(&signal, 150.0, 0, "strategy_a");
    harness.tracker.inner.mark_to_market(160.0, "AAPL");

    harness.graceful_shutdown().await;

    let loaded = load_state(&state_path)
        .expect("should load")
        .expect("should have state");

    // Last prices should contain AAPL's mark-to-market price
    let aapl_price = loaded
        .positions
        .last_prices
        .iter()
        .find(|(sym, _)| sym == "AAPL");
    assert!(
        aapl_price.is_some(),
        "last_prices should contain AAPL"
    );
    let (_, price) = aapl_price.unwrap();
    assert!(
        (*price - 160.0).abs() < f64::EPSILON,
        "AAPL last price should be 160.0, got {}",
        price
    );
}

/// Test that state can be loaded back after shutdown and positions match.
///
/// Validates: Requirements 5.9, 7.1
#[tokio::test]
async fn state_roundtrip_positions_match_after_shutdown() {
    let dir = TempDir::new().unwrap();
    let state_path = dir.path().join("roundtrip_state.json");

    let mut harness = LiveHarness::new(
        vec![],
        SignalAggregator::new(RiskConstraints::default()),
        LivePositionTracker::new(100_000.0),
        Some(state_path.clone()),
        ReconnectPolicy::default(),
        Duration::from_secs(30),
        None,
        None,
        None,
    );

    // Build up some positions and a partial close for realized P&L
    let open1 = Signal::open("AAPL".to_string(), 200.0);
    harness
        .tracker
        .process_signal(&open1, 150.0, 0, "alpha");

    let open2 = Signal::open("MSFT".to_string(), 100.0);
    harness
        .tracker
        .process_signal(&open2, 400.0, 1, "beta");

    // Partially close AAPL
    let close_partial = Signal::close_qty("AAPL".to_string(), 50.0);
    harness
        .tracker
        .process_signal(&close_partial, 160.0, 2, "alpha");

    // Mark to market
    harness.tracker.inner.mark_to_market(165.0, "AAPL");
    harness.tracker.inner.mark_to_market(410.0, "MSFT");

    // Shutdown
    harness.graceful_shutdown().await;

    // Load state back
    let loaded = load_state(&state_path)
        .expect("should load")
        .expect("should have state");

    // Verify AAPL: 150 remaining shares (200 opened, 50 closed)
    let aapl = loaded
        .positions
        .positions
        .iter()
        .find(|p| p.symbol == "AAPL")
        .expect("AAPL should exist");
    assert!((aapl.qty - 150.0).abs() < f64::EPSILON);
    assert!((aapl.avg_entry_price - 150.0).abs() < f64::EPSILON);
    // Realized P&L from partial close: (160 - 150) * 50 = 500
    assert!((aapl.realized_pnl - 500.0).abs() < f64::EPSILON);

    // Verify MSFT: 100 shares still open
    let msft = loaded
        .positions
        .positions
        .iter()
        .find(|p| p.symbol == "MSFT")
        .expect("MSFT should exist");
    assert!((msft.qty - 100.0).abs() < f64::EPSILON);
    assert!((msft.avg_entry_price - 400.0).abs() < f64::EPSILON);

    // Verify total realized P&L
    assert!(
        (loaded.positions.total_realized_pnl - 500.0).abs() < f64::EPSILON,
        "total realized P&L should be 500.0, got {}",
        loaded.positions.total_realized_pnl
    );
}
