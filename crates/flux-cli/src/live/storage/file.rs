//! File-based storage backend implementation.
//!
//! Provides a backwards-compatible `StorageBackend` using JSONL append logs
//! for fills, signals, orders, risk events, and equity snapshots, plus an
//! atomic `state.json` checkpoint file. Suitable for local development and
//! testing without requiring a PostgreSQL instance.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use super::{
    EquitySnapshot, FillInfo, FillRecord, OrderRecord, PositionRecord, RiskEventRecord,
    SignalRecord, StorageBackend, StorageResult,
};
use crate::live::state::HarnessState;

/// File-based storage backend errors.
#[derive(Debug, thiserror::Error)]
pub enum FileStorageError {
    #[error("{operation} I/O error: {source}")]
    Io {
        operation: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("deserialization error: {source}")]
    Deserialize {
        #[source]
        source: serde_json::Error,
    },
}

/// A file-based implementation of `StorageBackend`.
///
/// Appends JSONL records to `*.jsonl` files and uses atomic write (tmp+rename)
/// for checkpoint state. Positions are tracked in-memory and persisted during
/// checkpoints.
pub struct FileBackend {
    dir: PathBuf,
    /// In-memory position map, persisted on save_checkpoint.
    positions: Mutex<HashMap<String, (f64, f64)>>, // symbol -> (qty, avg_entry)
}

/// JSON structure for order status update lines in `orders.jsonl`.
#[derive(Debug, Serialize, Deserialize)]
struct OrderStatusUpdate {
    order_id: String,
    status: String,
    timestamp: chrono::DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fill_price: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fill_qty: Option<f64>,
}

impl FileBackend {
    /// Create or open the file backend at the given directory.
    ///
    /// Creates the directory (and parents) if it doesn't exist.
    pub fn new(dir: PathBuf) -> StorageResult<Self> {
        fs::create_dir_all(&dir).map_err(|source| {
            Box::new(FileStorageError::Io {
                operation: "create_dir_all",
                source,
            }) as Box<dyn std::error::Error + Send + Sync>
        })?;

        Ok(Self {
            dir,
            positions: Mutex::new(HashMap::new()),
        })
    }

    /// Append a serialized JSON line to the given file, then flush.
    fn append_jsonl<T: Serialize>(&self, filename: &str, record: &T) -> StorageResult<()> {
        let path = self.dir.join(filename);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|source| {
                Box::new(FileStorageError::Io {
                    operation: "open_append",
                    source,
                }) as Box<dyn std::error::Error + Send + Sync>
            })?;

        let line = serde_json::to_string(record).map_err(|source| {
            Box::new(FileStorageError::Deserialize { source })
                as Box<dyn std::error::Error + Send + Sync>
        })?;

        writeln!(file, "{}", line).map_err(|source| {
            Box::new(FileStorageError::Io {
                operation: "write_jsonl",
                source,
            }) as Box<dyn std::error::Error + Send + Sync>
        })?;

        file.flush().map_err(|source| {
            Box::new(FileStorageError::Io {
                operation: "flush",
                source,
            }) as Box<dyn std::error::Error + Send + Sync>
        })?;

        Ok(())
    }
}

#[async_trait]
impl StorageBackend for FileBackend {
    async fn record_fill(&self, fill: &FillRecord) -> StorageResult<()> {
        self.append_jsonl("fills.jsonl", fill)
    }

    async fn record_signal(&self, signal: &SignalRecord) -> StorageResult<()> {
        self.append_jsonl("signals.jsonl", signal)
    }

    async fn record_order(&self, order: &OrderRecord) -> StorageResult<()> {
        self.append_jsonl("orders.jsonl", order)
    }

    async fn record_risk_event(&self, event: &RiskEventRecord) -> StorageResult<()> {
        self.append_jsonl("risk_events.jsonl", event)
    }

    async fn snapshot_equity(&self, snapshot: &EquitySnapshot) -> StorageResult<()> {
        self.append_jsonl("equity.jsonl", snapshot)
    }

    async fn upsert_position(&self, symbol: &str, qty: f64, avg_entry: f64) -> StorageResult<()> {
        let mut positions = self.positions.lock().unwrap();
        positions.insert(symbol.to_string(), (qty, avg_entry));
        Ok(())
    }

    async fn save_checkpoint(&self, state: &HarnessState) -> StorageResult<()> {
        // Clone the state so we can update positions before serializing.
        let mut state_to_save = state.clone();

        // Update the positions in state with the current in-memory position map.
        let positions = self.positions.lock().unwrap();
        state_to_save.positions.positions = positions
            .iter()
            .map(
                |(symbol, (qty, avg_entry))| crate::live::state::SerializedPosition {
                    symbol: symbol.clone(),
                    qty: *qty,
                    avg_entry_price: *avg_entry,
                    realized_pnl: 0.0,
                },
            )
            .collect();
        drop(positions);

        let json = serde_json::to_string_pretty(&state_to_save).map_err(|source| {
            Box::new(FileStorageError::Deserialize { source })
                as Box<dyn std::error::Error + Send + Sync>
        })?;

        let state_path = self.dir.join("state.json");
        let tmp_path = self.dir.join("state.json.tmp");

        fs::write(&tmp_path, &json).map_err(|source| {
            Box::new(FileStorageError::Io {
                operation: "write_checkpoint_tmp",
                source,
            }) as Box<dyn std::error::Error + Send + Sync>
        })?;

        fs::rename(&tmp_path, &state_path).map_err(|source| {
            Box::new(FileStorageError::Io {
                operation: "rename_checkpoint",
                source,
            }) as Box<dyn std::error::Error + Send + Sync>
        })?;

        Ok(())
    }

    async fn load_latest_checkpoint(&self) -> StorageResult<Option<HarnessState>> {
        let state_path = self.dir.join("state.json");

        if !state_path.exists() {
            return Ok(None);
        }

        let json = fs::read_to_string(&state_path).map_err(|source| {
            Box::new(FileStorageError::Io {
                operation: "read_checkpoint",
                source,
            }) as Box<dyn std::error::Error + Send + Sync>
        })?;

        let state: HarnessState = serde_json::from_str(&json).map_err(|source| {
            Box::new(FileStorageError::Deserialize { source })
                as Box<dyn std::error::Error + Send + Sync>
        })?;

        Ok(Some(state))
    }

    async fn load_positions(&self) -> StorageResult<Vec<PositionRecord>> {
        let checkpoint = self.load_latest_checkpoint().await?;

        match checkpoint {
            None => Ok(Vec::new()),
            Some(state) => {
                let positions = state
                    .positions
                    .positions
                    .iter()
                    .map(|sp| PositionRecord {
                        symbol: sp.symbol.clone(),
                        qty: sp.qty,
                        avg_entry: sp.avg_entry_price,
                        realized_pnl: sp.realized_pnl,
                        updated_at: state
                            .checkpoint_timestamp
                            .parse::<chrono::DateTime<Utc>>()
                            .unwrap_or_else(|_| Utc::now()),
                    })
                    .collect();
                Ok(positions)
            }
        }
    }

    async fn update_order_status(
        &self,
        order_id: &str,
        status: &str,
        fill_info: Option<&FillInfo>,
    ) -> StorageResult<()> {
        let update = OrderStatusUpdate {
            order_id: order_id.to_string(),
            status: status.to_string(),
            timestamp: Utc::now(),
            fill_price: fill_info.map(|fi| fi.fill_price),
            fill_qty: fill_info.map(|fi| fi.fill_qty),
        };

        self.append_jsonl("orders.jsonl", &update)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use tempfile::TempDir;

    #[tokio::test]
    async fn new_creates_directory() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("nested").join("storage");
        let _backend = FileBackend::new(dir.clone()).unwrap();
        assert!(dir.exists());
    }

    #[tokio::test]
    async fn record_fill_appends_to_jsonl() {
        let tmp = TempDir::new().unwrap();
        let backend = FileBackend::new(tmp.path().to_path_buf()).unwrap();

        let fill = FillRecord {
            timestamp: Utc::now(),
            strategy: "TestStrategy".to_string(),
            symbol: "AAPL".to_string(),
            side: "buy".to_string(),
            qty: 100.0,
            price: 150.25,
            order_id: Some("ord-123".to_string()),
            latency_ms: Some(5),
            bar_index: 42,
        };

        backend.record_fill(&fill).await.unwrap();

        let content = fs::read_to_string(tmp.path().join("fills.jsonl")).unwrap();
        let lines: Vec<&str> = content.trim().lines().collect();
        assert_eq!(lines.len(), 1);

        let deserialized: FillRecord = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(deserialized, fill);
    }

    #[tokio::test]
    async fn load_latest_checkpoint_returns_none_when_missing() {
        let tmp = TempDir::new().unwrap();
        let backend = FileBackend::new(tmp.path().to_path_buf()).unwrap();

        let result = backend.load_latest_checkpoint().await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn save_and_load_checkpoint_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let backend = FileBackend::new(tmp.path().to_path_buf()).unwrap();

        // Upsert some positions
        backend.upsert_position("AAPL", 100.0, 150.0).await.unwrap();
        backend.upsert_position("MSFT", 50.0, 380.0).await.unwrap();

        let state = HarnessState {
            version: crate::live::state::STATE_VERSION,
            positions: crate::live::state::PositionState {
                initial_capital: 10_000.0,
                positions: vec![],
                total_realized_pnl: 0.0,
                last_prices: vec![],
            },
            strategy_states: vec![],
            fill_count: 5,
            checkpoint_timestamp: "2024-06-15T14:30:00.000Z".to_string(),
            bars_processed: 100,
        };

        backend.save_checkpoint(&state).await.unwrap();

        let loaded = backend.load_latest_checkpoint().await.unwrap().unwrap();
        // The positions should be updated from in-memory map
        assert_eq!(loaded.positions.positions.len(), 2);
        assert_eq!(loaded.fill_count, 5);
        assert_eq!(loaded.bars_processed, 100);
    }

    #[tokio::test]
    async fn load_positions_extracts_from_checkpoint() {
        let tmp = TempDir::new().unwrap();
        let backend = FileBackend::new(tmp.path().to_path_buf()).unwrap();

        backend.upsert_position("AAPL", 100.0, 150.0).await.unwrap();

        let state = HarnessState {
            version: crate::live::state::STATE_VERSION,
            positions: crate::live::state::PositionState {
                initial_capital: 10_000.0,
                positions: vec![],
                total_realized_pnl: 0.0,
                last_prices: vec![],
            },
            strategy_states: vec![],
            fill_count: 0,
            checkpoint_timestamp: "2024-06-15T14:30:00.000Z".to_string(),
            bars_processed: 0,
        };

        backend.save_checkpoint(&state).await.unwrap();

        let positions = backend.load_positions().await.unwrap();
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].symbol, "AAPL");
        assert_eq!(positions[0].qty, 100.0);
        assert_eq!(positions[0].avg_entry, 150.0);
    }

    #[tokio::test]
    async fn upsert_position_overwrites_previous() {
        let tmp = TempDir::new().unwrap();
        let backend = FileBackend::new(tmp.path().to_path_buf()).unwrap();

        backend.upsert_position("AAPL", 100.0, 150.0).await.unwrap();
        backend.upsert_position("AAPL", 200.0, 155.0).await.unwrap();

        let positions = backend.positions.lock().unwrap();
        assert_eq!(positions.get("AAPL"), Some(&(200.0, 155.0)));
        assert_eq!(positions.len(), 1);
    }

    #[tokio::test]
    async fn update_order_status_appends_to_orders_jsonl() {
        let tmp = TempDir::new().unwrap();
        let backend = FileBackend::new(tmp.path().to_path_buf()).unwrap();

        let fill_info = FillInfo {
            fill_price: 150.5,
            fill_qty: 100.0,
        };

        backend
            .update_order_status("ord-001", "filled", Some(&fill_info))
            .await
            .unwrap();

        let content = fs::read_to_string(tmp.path().join("orders.jsonl")).unwrap();
        let lines: Vec<&str> = content.trim().lines().collect();
        assert_eq!(lines.len(), 1);

        let update: OrderStatusUpdate = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(update.order_id, "ord-001");
        assert_eq!(update.status, "filled");
        assert_eq!(update.fill_price, Some(150.5));
        assert_eq!(update.fill_qty, Some(100.0));
    }

    #[tokio::test]
    async fn update_order_status_without_fill_info() {
        let tmp = TempDir::new().unwrap();
        let backend = FileBackend::new(tmp.path().to_path_buf()).unwrap();

        backend
            .update_order_status("ord-002", "acknowledged", None)
            .await
            .unwrap();

        let content = fs::read_to_string(tmp.path().join("orders.jsonl")).unwrap();
        let lines: Vec<&str> = content.trim().lines().collect();
        assert_eq!(lines.len(), 1);

        let update: OrderStatusUpdate = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(update.order_id, "ord-002");
        assert_eq!(update.status, "acknowledged");
        assert_eq!(update.fill_price, None);
        assert_eq!(update.fill_qty, None);
    }

    #[tokio::test]
    async fn checkpoint_atomic_no_tmp_remains() {
        let tmp = TempDir::new().unwrap();
        let backend = FileBackend::new(tmp.path().to_path_buf()).unwrap();

        let state = HarnessState {
            version: crate::live::state::STATE_VERSION,
            positions: crate::live::state::PositionState {
                initial_capital: 10_000.0,
                positions: vec![],
                total_realized_pnl: 0.0,
                last_prices: vec![],
            },
            strategy_states: vec![],
            fill_count: 0,
            checkpoint_timestamp: "2024-01-01T00:00:00Z".to_string(),
            bars_processed: 0,
        };

        backend.save_checkpoint(&state).await.unwrap();

        assert!(tmp.path().join("state.json").exists());
        assert!(!tmp.path().join("state.json.tmp").exists());
    }
}
