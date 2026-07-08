//! Replay connector for testing the live harness with historical CSV data.
//!
//! Reads bars from a CSV file and emits them at a configurable rate.
//! When `playback_rate` is 0.0, bars are emitted immediately (useful for
//! integration tests). When `playback_rate` is 1.0, bars are emitted at
//! real-time pacing using `tokio::time::sleep`.

use std::path::PathBuf;

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::csv_loader::load_csv;

use super::connector::{Connector, ConnectorError, ConnectorState, LiveBar};

/// A connector that replays bars from a CSV file at a configurable rate.
///
/// Useful for testing the live harness end-to-end without a real data source.
/// The connector reads all bars from the CSV file on `connect()` and spawns
/// a tokio task that sends them over the channel at the configured playback rate.
pub struct ReplayConnector {
    /// Human-readable identifier for this connector instance.
    id: String,
    /// Path to the CSV file containing bar data.
    file_path: PathBuf,
    /// Playback speed multiplier.
    /// - 0.0 = as-fast-as-possible (no sleep between bars)
    /// - 1.0 = real-time pacing (1 second between bars)
    /// - 2.0 = double speed (0.5 seconds between bars)
    playback_rate: f64,
    /// Current connection state.
    state: ConnectorState,
    /// Handle to the spawned replay task (if connected).
    task_handle: Option<JoinHandle<()>>,
}

impl ReplayConnector {
    /// Create a new replay connector.
    ///
    /// # Arguments
    /// - `id` — Human-readable identifier for observability
    /// - `file_path` — Path to the CSV file with bar data
    /// - `playback_rate` — Speed multiplier (0.0 = instant, 1.0 = real-time)
    pub fn new(id: impl Into<String>, file_path: PathBuf, playback_rate: f64) -> Self {
        Self {
            id: id.into(),
            file_path,
            playback_rate,
            state: ConnectorState::Disconnected,
            task_handle: None,
        }
    }
}

#[async_trait]
impl Connector for ReplayConnector {
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
        self.state = ConnectorState::Connecting;

        // Load bars from the CSV file synchronously (file I/O).
        let bars = load_csv(&self.file_path).map_err(|e| {
            ConnectorError::ConnectionFailed(format!(
                "failed to load CSV '{}': {}",
                self.file_path.display(),
                e
            ))
        })?;

        let connector_id = self.id.clone();
        let playback_rate = self.playback_rate;

        // Spawn a tokio task that iterates through bars and sends them.
        let handle = tokio::spawn(async move {
            for bar in bars {
                let live_bar = LiveBar {
                    bar,
                    connector_id: connector_id.clone(),
                    received_at: chrono::Utc::now(),
                };

                // If the receiver has dropped, stop sending.
                if tx.send(live_bar).await.is_err() {
                    break;
                }

                // Apply playback pacing if rate > 0.
                if playback_rate > 0.0 {
                    let sleep_duration = std::time::Duration::from_secs_f64(1.0 / playback_rate);
                    tokio::time::sleep(sleep_duration).await;
                }
            }
        });

        self.task_handle = Some(handle);
        self.state = ConnectorState::Connected;

        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), ConnectorError> {
        // Abort the replay task if it's still running.
        if let Some(handle) = self.task_handle.take() {
            handle.abort();
        }

        self.state = ConnectorState::Disconnected;
        Ok(())
    }

    async fn subscribe(&mut self, _symbols: &[String]) -> Result<(), ConnectorError> {
        // For replay, subscribe is a no-op — all bars from the CSV are emitted
        // regardless of symbol subscription. Filtering can be done downstream.
        Ok(())
    }
}
