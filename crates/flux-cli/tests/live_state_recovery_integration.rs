//! Integration tests for the live state recovery system.
//!
//! These tests exercise the full crash-recovery flow at the component level:
//! FillLogger, CheckpointScheduler, save_state/load_state, FillReplayer, and
//! restore_state working together in a realistic sequence.
//!
//! **Validates: Requirements 3.1, 3.2, 4.1, 4.3, 5.6, 7.1, 7.5**

use tempfile::TempDir;

use flux_cli::live::checkpoint::CheckpointScheduler;
use flux_cli::live::fill_logger::{FillLogger, FillRecord};
use flux_cli::live::position::LivePositionTracker;
use flux_cli::live::replay::FillReplayer;
use flux_cli::live::state::{
    save_state, load_state, HarnessState, PositionState, SerializedPosition, STATE_VERSION,
};

use std::time::Duration;

// =============================================================================
// Test 1: Full crash-recovery cycle
// =============================================================================

/// Validates: Requirements 3.1, 3.2, 4.1, 4.3, 7.1, 7.5
///
/// Simulates a complete session lifecycle:
/// 1. First session: open fill logger, append fills, checkpoint after 2 fills
/// 2. A 3rd fill occurs after the checkpoint (simulating crash before next checkpoint)
/// 3. Restart: load state, compute replay, restore positions, replay delta fills
/// 4. Verify: final positions match what they should be after all 3 fills
#[test]
fn test_full_crash_recovery_cycle() {
    // Phase 1: "First session" — simulate harness operation
    let dir = TempDir::new().unwrap();
    let state_path = dir.path().join("harness_state.json");
    let fill_log_path = dir.path().join("harness_state.jsonl");

    // Open fill logger, write some fills
    let mut logger = FillLogger::open(&fill_log_path).unwrap();

    let fill1 = FillRecord {
        seq: 0,
        timestamp: "2024-06-15T14:30:00.000Z".to_string(),
        symbol: "AAPL".to_string(),
        side: "buy".to_string(),
        qty: 100.0,
        price: 150.0,
        strategy: "TestStrat".to_string(),
        bar_index: 1,
    };
    let fill2 = FillRecord {
        seq: 0,
        timestamp: "2024-06-15T14:31:00.000Z".to_string(),
        symbol: "MSFT".to_string(),
        side: "buy".to_string(),
        qty: 50.0,
        price: 300.0,
        strategy: "TestStrat".to_string(),
        bar_index: 2,
    };
    let fill3 = FillRecord {
        seq: 0,
        timestamp: "2024-06-15T14:32:00.000Z".to_string(),
        symbol: "AAPL".to_string(),
        side: "sell".to_string(),
        qty: 100.0,
        price: 155.0,
        strategy: "TestStrat".to_string(),
        bar_index: 3,
    };

    // Append fills 1 and 2 (assigned seq 1, 2 by logger)
    let seq1 = logger.append(&fill1).unwrap();
    let seq2 = logger.append(&fill2).unwrap();
    assert_eq!(seq1, 1);
    assert_eq!(seq2, 2);

    // "Checkpoint" after 2 fills — save state with fill_count=2
    // This represents the positions AFTER processing fills 1 and 2:
    //   AAPL: +100 shares @ 150.0
    //   MSFT: +50 shares @ 300.0
    let state = HarnessState {
        version: STATE_VERSION,
        positions: PositionState {
            initial_capital: 100_000.0,
            positions: vec![
                SerializedPosition {
                    symbol: "AAPL".to_string(),
                    qty: 100.0,
                    avg_entry_price: 150.0,
                    realized_pnl: 0.0,
                },
                SerializedPosition {
                    symbol: "MSFT".to_string(),
                    qty: 50.0,
                    avg_entry_price: 300.0,
                    realized_pnl: 0.0,
                },
            ],
            total_realized_pnl: 0.0,
            last_prices: vec![
                ("AAPL".to_string(), 152.0),
                ("MSFT".to_string(), 305.0),
            ],
        },
        strategy_states: vec![],
        fill_count: 2,
        checkpoint_timestamp: "2024-06-15T14:31:30.000Z".to_string(),
        bars_processed: 10,
    };
    save_state(&state, &state_path).unwrap();

    // Fill 3 happens AFTER the checkpoint (simulating a crash before next checkpoint)
    let seq3 = logger.append(&fill3).unwrap();
    assert_eq!(seq3, 3);
    drop(logger);

    // Phase 2: "Simulate crash" — everything is dropped

    // Phase 3: "Restart" — load state and replay
    let loaded = load_state(&state_path).unwrap().unwrap();
    assert_eq!(loaded.fill_count, 2);
    assert_eq!(loaded.bars_processed, 10);

    // Compute replay from fill log — should identify 1 fill after checkpoint
    let fills_to_replay = FillReplayer::compute_replay(&fill_log_path, loaded.fill_count).unwrap();
    assert_eq!(fills_to_replay.len(), 1, "should replay 1 fill after checkpoint");
    assert_eq!(fills_to_replay[0].seq, 3);
    assert_eq!(fills_to_replay[0].side, "sell");
    assert_eq!(fills_to_replay[0].symbol, "AAPL");

    // Restore positions to a fresh tracker
    let mut tracker = LivePositionTracker::new(100_000.0);
    tracker.inner.restore_from_state(
        loaded
            .positions
            .positions
            .iter()
            .map(|p| {
                (
                    p.symbol.clone(),
                    p.qty,
                    p.avg_entry_price,
                    p.realized_pnl,
                )
            })
            .collect(),
        loaded.positions.total_realized_pnl,
        loaded.positions.last_prices.clone(),
    );

    // Verify restored state before replay
    let aapl_before = tracker.inner.position("AAPL").unwrap();
    assert_eq!(aapl_before.qty, 100.0);
    let msft_before = tracker.inner.position("MSFT").unwrap();
    assert_eq!(msft_before.qty, 50.0);

    // Replay the delta fill (sell 100 AAPL @ 155)
    FillReplayer::replay_fills(&fills_to_replay, &mut tracker);

    // Verify: AAPL position should be closed (buy 100 + sell 100 = 0)
    let aapl_after = tracker.inner.position("AAPL");
    assert!(
        aapl_after.is_none() || aapl_after.unwrap().qty == 0.0,
        "AAPL should be closed after replay, got qty={:?}",
        aapl_after.map(|p| p.qty),
    );

    // MSFT should still be open at 50 qty (untouched by replay)
    let msft_after = tracker.inner.position("MSFT").unwrap();
    assert_eq!(msft_after.qty, 50.0);
    assert_eq!(msft_after.avg_entry_price, 300.0);
}

// =============================================================================
// Test 2: Graceful shutdown writes final state with correct fill_count and
//          bars_processed
// =============================================================================

/// Validates: Requirements 3.2, 5.6, 7.5
///
/// Simulates a session where:
/// 1. Multiple fills are logged
/// 2. A checkpoint scheduler tracks bars
/// 3. On graceful shutdown, state is saved with correct fill_count and bars_processed
#[test]
fn test_graceful_shutdown_includes_fill_count_and_bars_processed() {
    let dir = TempDir::new().unwrap();
    let state_path = dir.path().join("harness_state.json");
    let fill_log_path = dir.path().join("harness_state.jsonl");

    // Open fill logger and write 5 fills
    let mut logger = FillLogger::open(&fill_log_path).unwrap();
    for i in 1..=5u64 {
        let record = FillRecord {
            seq: 0,
            timestamp: format!("2024-06-15T14:{:02}:00.000Z", 30 + i),
            symbol: if i % 2 == 0 { "MSFT" } else { "AAPL" }.to_string(),
            side: "buy".to_string(),
            qty: 10.0 * i as f64,
            price: 150.0 + i as f64,
            strategy: "TestStrat".to_string(),
            bar_index: i,
        };
        logger.append(&record).unwrap();
    }

    // The fill_count should be next_seq - 1 (fills logged so far)
    let fill_count = logger.next_seq() - 1;
    assert_eq!(fill_count, 5);

    // Simulate bar processing with a checkpoint scheduler
    let mut scheduler = CheckpointScheduler::new(50, Duration::from_secs(300));
    for _ in 0..23 {
        scheduler.on_bar();
    }
    let bars_processed = scheduler.total_bars();
    assert_eq!(bars_processed, 23);

    // Build "shutdown" state — simulates what LiveHarness::build_harness_state does
    let final_state = HarnessState {
        version: STATE_VERSION,
        positions: PositionState {
            initial_capital: 100_000.0,
            positions: vec![
                SerializedPosition {
                    symbol: "AAPL".to_string(),
                    qty: 90.0,
                    avg_entry_price: 152.0,
                    realized_pnl: 0.0,
                },
                SerializedPosition {
                    symbol: "MSFT".to_string(),
                    qty: 60.0,
                    avg_entry_price: 153.5,
                    realized_pnl: 0.0,
                },
            ],
            total_realized_pnl: 0.0,
            last_prices: vec![
                ("AAPL".to_string(), 156.0),
                ("MSFT".to_string(), 155.0),
            ],
        },
        strategy_states: vec![],
        fill_count,
        checkpoint_timestamp: "2024-06-15T14:55:00.000Z".to_string(),
        bars_processed,
    };

    // Save state (simulates graceful shutdown)
    save_state(&final_state, &state_path).unwrap();

    // Verify: reload and check fill_count and bars_processed are correct
    let loaded = load_state(&state_path).unwrap().unwrap();
    assert_eq!(loaded.fill_count, 5, "fill_count should match total fills logged");
    assert_eq!(loaded.bars_processed, 23, "bars_processed should match scheduler total");
    assert_eq!(loaded.version, STATE_VERSION);
    assert_eq!(loaded.positions.positions.len(), 2);

    // Verify that on next restart, no replay is needed (fill_count matches log)
    let fills_to_replay =
        FillReplayer::compute_replay(&fill_log_path, loaded.fill_count).unwrap();
    assert!(
        fills_to_replay.is_empty(),
        "no replay should be needed after graceful shutdown"
    );
}

// =============================================================================
// Test 3: Checkpoint scheduler triggers save during session
// =============================================================================

/// Validates: Requirements 5.6, 7.1
///
/// Verifies that the checkpoint scheduler correctly triggers based on bar count,
/// and that after a triggered checkpoint + additional fills, the replay logic
/// correctly identifies the delta.
#[test]
fn test_checkpoint_triggers_and_replay_identifies_delta() {
    let dir = TempDir::new().unwrap();
    let state_path = dir.path().join("harness_state.json");
    let fill_log_path = dir.path().join("harness_state.jsonl");

    // Use a small bar_interval (3) for testing
    let mut scheduler = CheckpointScheduler::new(3, Duration::from_secs(600));
    let mut logger = FillLogger::open(&fill_log_path).unwrap();

    // Simulate 3 bars with a fill on each
    for i in 1..=3u64 {
        let record = FillRecord {
            seq: 0,
            timestamp: format!("2024-06-15T14:{:02}:00.000Z", 30 + i),
            symbol: "AAPL".to_string(),
            side: "buy".to_string(),
            qty: 10.0,
            price: 150.0 + i as f64,
            strategy: "TestStrat".to_string(),
            bar_index: i,
        };
        logger.append(&record).unwrap();
        let triggered = scheduler.on_bar();

        // Should trigger on bar 3
        if i == 3 {
            assert!(triggered, "scheduler should trigger on bar 3");
        } else {
            assert!(!triggered, "scheduler should not trigger on bar {}", i);
        }
    }

    // Checkpoint now — save state with fill_count=3
    let checkpoint_state = HarnessState {
        version: STATE_VERSION,
        positions: PositionState {
            initial_capital: 100_000.0,
            positions: vec![SerializedPosition {
                symbol: "AAPL".to_string(),
                qty: 30.0,
                avg_entry_price: 152.0,
                realized_pnl: 0.0,
            }],
            total_realized_pnl: 0.0,
            last_prices: vec![("AAPL".to_string(), 153.0)],
        },
        strategy_states: vec![],
        fill_count: 3,
        checkpoint_timestamp: "2024-06-15T14:33:00.000Z".to_string(),
        bars_processed: scheduler.total_bars(),
    };
    save_state(&checkpoint_state, &state_path).unwrap();
    scheduler.mark_checkpointed();

    // More fills happen after checkpoint (bars 4, 5)
    for i in 4..=5u64 {
        let record = FillRecord {
            seq: 0,
            timestamp: format!("2024-06-15T14:{:02}:00.000Z", 30 + i),
            symbol: "AAPL".to_string(),
            side: "buy".to_string(),
            qty: 10.0,
            price: 150.0 + i as f64,
            strategy: "TestStrat".to_string(),
            bar_index: i,
        };
        logger.append(&record).unwrap();
        scheduler.on_bar();
    }
    drop(logger);

    // Simulate crash and restart
    let loaded = load_state(&state_path).unwrap().unwrap();
    assert_eq!(loaded.fill_count, 3);
    assert_eq!(loaded.bars_processed, 3);

    // Replay should identify 2 fills after checkpoint
    let fills_to_replay = FillReplayer::compute_replay(&fill_log_path, loaded.fill_count).unwrap();
    assert_eq!(fills_to_replay.len(), 2, "should replay 2 fills after checkpoint");
    assert_eq!(fills_to_replay[0].seq, 4);
    assert_eq!(fills_to_replay[1].seq, 5);
}
