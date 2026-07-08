//! Integration tests for the live harness reconnection logic.
//!
//! Verifies that `reconnect_loop()` implements exponential backoff correctly
//! and resumes bar delivery after successful reconnection.
//!
//! **Validates: Requirements 6.1, 6.3, 6.4**

use async_trait::async_trait;
use flux_cli::live::connector::{
    Connector, ConnectorError, ConnectorState, LiveBar, ReconnectPolicy,
};
use flux_cli::live::reconnect::reconnect_loop;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

/// A mock connector that fails the first `fail_count` connect() calls,
/// then succeeds on subsequent calls by sending bars over the channel.
struct MockFailingConnector {
    id: String,
    state: ConnectorState,
    /// Number of times connect() should fail before succeeding.
    fail_count: u32,
    /// Tracks the total number of connect() calls made.
    connect_attempts: Arc<AtomicU32>,
    /// Number of bars to send on successful connection.
    bars_to_send: u32,
}

impl MockFailingConnector {
    fn new(fail_count: u32, bars_to_send: u32) -> Self {
        Self {
            id: "mock-failing".to_string(),
            state: ConnectorState::Disconnected,
            fail_count,
            connect_attempts: Arc::new(AtomicU32::new(0)),
            bars_to_send,
        }
    }

    fn attempts(&self) -> u32 {
        self.connect_attempts.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Connector for MockFailingConnector {
    fn id(&self) -> &str {
        &self.id
    }

    fn state(&self) -> ConnectorState {
        self.state
    }

    async fn connect(
        &mut self,
        _symbols: &[String],
        tx: mpsc::Sender<LiveBar>,
    ) -> Result<(), ConnectorError> {
        let attempt = self.connect_attempts.fetch_add(1, Ordering::SeqCst);

        if attempt < self.fail_count {
            self.state = ConnectorState::Disconnected;
            return Err(ConnectorError::ConnectionFailed(format!(
                "simulated failure (attempt {})",
                attempt + 1
            )));
        }

        // Success: send bars over the channel
        self.state = ConnectorState::Connected;
        for i in 0..self.bars_to_send {
            let bar = flux_runtime::BarContext {
                open: 100.0 + i as f64,
                high: 101.0 + i as f64,
                low: 99.0 + i as f64,
                close: 100.5 + i as f64,
                volume: 1000.0,
                symbol: "TEST".to_string(),
                in_position: false,
            };
            let live_bar = LiveBar {
                bar,
                connector_id: self.id.clone(),
                received_at: chrono::Utc::now(),
            };
            // Ignore send errors (receiver may be dropped in test)
            let _ = tx.send(live_bar).await;
        }

        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), ConnectorError> {
        self.state = ConnectorState::Disconnected;
        Ok(())
    }

    async fn subscribe(&mut self, _symbols: &[String]) -> Result<(), ConnectorError> {
        Ok(())
    }
}

/// A mock connector that always fails to connect — used to test
/// the max-retry-exceeded case.
struct AlwaysFailingConnector {
    id: String,
    state: ConnectorState,
    connect_attempts: Arc<AtomicU32>,
}

impl AlwaysFailingConnector {
    fn new() -> Self {
        Self {
            id: "always-failing".to_string(),
            state: ConnectorState::Disconnected,
            connect_attempts: Arc::new(AtomicU32::new(0)),
        }
    }

    fn attempts(&self) -> u32 {
        self.connect_attempts.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Connector for AlwaysFailingConnector {
    fn id(&self) -> &str {
        &self.id
    }

    fn state(&self) -> ConnectorState {
        self.state
    }

    async fn connect(
        &mut self,
        _symbols: &[String],
        _tx: mpsc::Sender<LiveBar>,
    ) -> Result<(), ConnectorError> {
        self.connect_attempts.fetch_add(1, Ordering::SeqCst);
        self.state = ConnectorState::Disconnected;
        Err(ConnectorError::ConnectionFailed(
            "permanent failure".to_string(),
        ))
    }

    async fn disconnect(&mut self) -> Result<(), ConnectorError> {
        self.state = ConnectorState::Disconnected;
        Ok(())
    }

    async fn subscribe(&mut self, _symbols: &[String]) -> Result<(), ConnectorError> {
        Ok(())
    }
}

// =============================================================================
// Tests
// =============================================================================

/// Validates: Requirement 6.1, 6.3
/// reconnect_loop succeeds after the connector recovers from transient failures.
/// Verifies that bar delivery resumes after successful reconnection.
#[tokio::test]
async fn reconnect_loop_succeeds_after_transient_failures() {
    let fail_count = 3;
    let bars_to_send = 5;
    let mut connector = MockFailingConnector::new(fail_count, bars_to_send);

    let (tx, mut rx) = mpsc::channel::<LiveBar>(32);
    let symbols = vec!["TEST".to_string()];

    // Use a fast policy so the test completes quickly
    let policy = ReconnectPolicy {
        initial_backoff_ms: 1,
        max_backoff_ms: 10,
        max_attempts: 5,
        multiplier: 2.0,
    };

    let result = reconnect_loop(&mut connector, &symbols, tx, &policy).await;

    // Should succeed after fail_count + 1 total connect() calls
    assert!(result.is_ok(), "reconnect_loop should succeed: {:?}", result);

    // Verify the connector was called the expected number of times
    // (fail_count failures + 1 success = fail_count + 1 total)
    assert_eq!(
        connector.attempts(),
        fail_count + 1,
        "expected {} connect attempts (3 failures + 1 success)",
        fail_count + 1
    );

    // Verify that bars were delivered after reconnection
    let mut received_bars = Vec::new();
    while let Ok(bar) = rx.try_recv() {
        received_bars.push(bar);
    }
    assert_eq!(
        received_bars.len(),
        bars_to_send as usize,
        "expected {} bars delivered after reconnection",
        bars_to_send
    );

    // Verify bar content is correct
    for (i, live_bar) in received_bars.iter().enumerate() {
        assert_eq!(live_bar.bar.symbol, "TEST");
        assert_eq!(live_bar.connector_id, "mock-failing");
        assert!((live_bar.bar.close - (100.5 + i as f64)).abs() < f64::EPSILON);
    }
}

/// Validates: Requirement 6.1, 6.4
/// reconnect_loop returns ConnectionFailed when max retries are exhausted.
#[tokio::test]
async fn reconnect_loop_fails_when_max_retries_exceeded() {
    let max_attempts = 4;
    let mut connector = AlwaysFailingConnector::new();

    let (tx, _rx) = mpsc::channel::<LiveBar>(32);
    let symbols = vec!["TEST".to_string()];

    let policy = ReconnectPolicy {
        initial_backoff_ms: 1,
        max_backoff_ms: 10,
        max_attempts,
        multiplier: 2.0,
    };

    let result = reconnect_loop(&mut connector, &symbols, tx, &policy).await;

    // Should fail with ConnectionFailed
    assert!(result.is_err(), "reconnect_loop should fail");
    let err = result.unwrap_err();
    match &err {
        ConnectorError::ConnectionFailed(msg) => {
            assert!(
                msg.contains("max reconnection attempts"),
                "error should mention max attempts, got: {}",
                msg
            );
            assert!(
                msg.contains(&max_attempts.to_string()),
                "error should contain attempt count {}, got: {}",
                max_attempts,
                msg
            );
        }
        _ => panic!("expected ConnectionFailed, got: {:?}", err),
    }

    // Verify all attempts were made
    assert_eq!(
        connector.attempts(),
        max_attempts,
        "expected exactly {} connect attempts",
        max_attempts
    );
}

/// Validates: Requirement 6.1
/// reconnect_loop succeeds immediately if the connector connects on the first attempt.
#[tokio::test]
async fn reconnect_loop_succeeds_on_first_attempt() {
    let mut connector = MockFailingConnector::new(0, 3); // 0 failures = immediate success

    let (tx, mut rx) = mpsc::channel::<LiveBar>(32);
    let symbols = vec!["AAPL".to_string()];

    let policy = ReconnectPolicy {
        initial_backoff_ms: 1,
        max_backoff_ms: 10,
        max_attempts: 5,
        multiplier: 2.0,
    };

    let result = reconnect_loop(&mut connector, &symbols, tx, &policy).await;

    assert!(result.is_ok(), "reconnect_loop should succeed on first attempt");
    assert_eq!(connector.attempts(), 1, "expected exactly 1 connect attempt");

    // Verify bars were delivered
    let mut received_bars = Vec::new();
    while let Ok(bar) = rx.try_recv() {
        received_bars.push(bar);
    }
    assert_eq!(received_bars.len(), 3);
}

/// Validates: Requirement 6.1, 6.3
/// reconnect_loop succeeds on the very last allowed attempt.
#[tokio::test]
async fn reconnect_loop_succeeds_on_last_attempt() {
    let max_attempts = 5;
    // Fail max_attempts - 1 times, succeed on the last
    let mut connector = MockFailingConnector::new(max_attempts - 1, 2);

    let (tx, mut rx) = mpsc::channel::<LiveBar>(32);
    let symbols = vec!["MSFT".to_string()];

    let policy = ReconnectPolicy {
        initial_backoff_ms: 1,
        max_backoff_ms: 10,
        max_attempts,
        multiplier: 2.0,
    };

    let result = reconnect_loop(&mut connector, &symbols, tx, &policy).await;

    assert!(
        result.is_ok(),
        "should succeed on the last allowed attempt"
    );
    assert_eq!(
        connector.attempts(),
        max_attempts,
        "expected {} connect attempts (all used)",
        max_attempts
    );

    // Verify bars delivered
    let mut received_bars = Vec::new();
    while let Ok(bar) = rx.try_recv() {
        received_bars.push(bar);
    }
    assert_eq!(received_bars.len(), 2);
}

/// Validates: Requirement 6.4
/// While one connector is reconnecting, the channel remains available for other use.
/// This test verifies that the mpsc channel doesn't block during reconnection attempts.
#[tokio::test]
async fn reconnect_does_not_block_channel_during_backoff() {
    let mut connector = MockFailingConnector::new(2, 3);

    let (tx, mut rx) = mpsc::channel::<LiveBar>(32);
    let symbols = vec!["TEST".to_string()];

    let policy = ReconnectPolicy {
        initial_backoff_ms: 1,
        max_backoff_ms: 5,
        max_attempts: 5,
        multiplier: 1.5,
    };

    // Spawn reconnect_loop in a separate task
    let tx_clone = tx.clone();
    let reconnect_handle = tokio::spawn(async move {
        reconnect_loop(&mut connector, &symbols, tx_clone, &policy).await
    });

    // Meanwhile, send a bar from a "healthy connector" on the same channel
    let healthy_bar = LiveBar {
        bar: flux_runtime::BarContext {
            open: 200.0,
            high: 201.0,
            low: 199.0,
            close: 200.5,
            volume: 500.0,
            symbol: "HEALTHY".to_string(),
            in_position: false,
        },
        connector_id: "healthy-connector".to_string(),
        received_at: chrono::Utc::now(),
    };
    tx.send(healthy_bar).await.expect("channel should not be blocked");

    // Wait for reconnect_loop to complete
    let result = reconnect_handle.await.expect("task should not panic");
    assert!(result.is_ok(), "reconnect_loop should eventually succeed");

    // Collect all bars from the channel
    drop(tx); // Close the sender so rx.recv() eventually returns None
    let mut bars = Vec::new();
    while let Some(bar) = rx.recv().await {
        bars.push(bar);
    }

    // Should have the healthy bar + 3 bars from the reconnected mock connector
    assert_eq!(bars.len(), 4, "expected 1 healthy + 3 reconnected bars");

    // The healthy bar should be present
    assert!(
        bars.iter().any(|b| b.bar.symbol == "HEALTHY"),
        "should contain bar from healthy connector"
    );

    // The reconnected bars should be present
    let test_bars: Vec<_> = bars.iter().filter(|b| b.bar.symbol == "TEST").collect();
    assert_eq!(test_bars.len(), 3, "should have 3 bars from reconnected connector");
}
