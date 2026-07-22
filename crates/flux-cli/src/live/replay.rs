//! Fill replay for crash recovery.
//!
//! On startup, the `FillReplayer` reads the JSONL fill log, compares the total
//! record count against the fill count stored in the last checkpoint, and
//! replays any fills that occurred after that checkpoint through the position
//! tracker. This reconciles stale state files with the authoritative fill log.

use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

use crate::live::fill_logger::FillRecord;
use crate::live::position::LivePositionTracker;
use flux_runtime::Signal;

/// Errors that can occur during fill replay.
#[derive(Debug)]
pub enum ReplayError {
    /// An I/O error occurred while reading the fill log.
    IoError(io::Error),
    /// A line in the fill log could not be parsed as a valid FillRecord.
    ParseError(String),
}

impl From<io::Error> for ReplayError {
    fn from(err: io::Error) -> Self {
        ReplayError::IoError(err)
    }
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplayError::IoError(e) => write!(f, "IO error reading fill log: {}", e),
            ReplayError::ParseError(msg) => write!(f, "Parse error in fill log: {}", msg),
        }
    }
}

impl std::error::Error for ReplayError {}

/// Reads the fill log and reconciles against restored state.
pub struct FillReplayer;

impl FillReplayer {
    /// Read the fill log, compare against the state's fill_count,
    /// and return the fills that need to be replayed.
    ///
    /// Behavior:
    /// - If the file is missing or empty, logs a warning and returns an empty vec.
    /// - If total records == state_fill_count, logs "no replay needed" and returns empty vec.
    /// - If total records > state_fill_count, returns fills after the checkpoint.
    /// - If total records < state_fill_count, logs a warning and returns empty vec.
    pub fn compute_replay(
        fill_log_path: &Path,
        state_fill_count: u64,
    ) -> Result<Vec<FillRecord>, ReplayError> {
        // Handle missing file gracefully
        if !fill_log_path.exists() {
            eprintln!(
                "[replay] warning: fill log not found at '{}', proceeding without replay",
                fill_log_path.display()
            );
            return Ok(Vec::new());
        }

        let file = File::open(fill_log_path)?;
        let reader = BufReader::new(file);
        let mut all_fills: Vec<FillRecord> = Vec::new();

        for (line_num, line_result) in reader.lines().enumerate() {
            let line = line_result?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let record: FillRecord = serde_json::from_str(trimmed).map_err(|e| {
                ReplayError::ParseError(format!("line {}: {}", line_num + 1, e))
            })?;
            all_fills.push(record);
        }

        // Handle empty file
        if all_fills.is_empty() {
            eprintln!(
                "[replay] warning: fill log at '{}' is empty, proceeding without replay",
                fill_log_path.display()
            );
            return Ok(Vec::new());
        }

        let total = all_fills.len() as u64;

        if total == state_fill_count {
            eprintln!("[replay] no replay needed (fill log has {} records, state fill_count = {})", total, state_fill_count);
            return Ok(Vec::new());
        }

        if total < state_fill_count {
            eprintln!(
                "[replay] warning: fill log has {} records but state fill_count is {} (log may be truncated), proceeding with restored state",
                total, state_fill_count
            );
            return Ok(Vec::new());
        }

        // total > state_fill_count: return the fills after the checkpoint
        let replay_start = state_fill_count as usize;
        let fills_to_replay = all_fills[replay_start..].to_vec();
        eprintln!(
            "[replay] replaying {} fills (log has {}, state checkpoint at {})",
            fills_to_replay.len(),
            total,
            state_fill_count
        );

        Ok(fills_to_replay)
    }

    /// Apply a sequence of fill records to the position tracker.
    ///
    /// For each fill, creates a Signal based on the side:
    /// - "buy" → `Signal::open(symbol, qty)`
    /// - "sell" → `Signal::close_qty(symbol, qty)`
    ///
    /// Each fill is logged to stderr with a "[replay]" prefix.
    pub fn replay_fills(
        fills: &[FillRecord],
        tracker: &mut LivePositionTracker,
    ) {
        for fill in fills {
            eprintln!(
                "[replay] seq={} {} {} qty={} price={} strategy={} bar={}",
                fill.seq, fill.side, fill.symbol, fill.qty, fill.price, fill.strategy, fill.bar_index
            );

            let signal = match fill.side.as_str() {
                "buy" => Signal::open(fill.symbol.clone(), fill.qty),
                "sell" => Signal::close_qty(fill.symbol.clone(), fill.qty),
                other => {
                    eprintln!("[replay] warning: unknown side '{}' for seq={}, skipping", other, fill.seq);
                    continue;
                }
            };

            tracker.process_signal(
                &signal,
                fill.price,
                fill.bar_index as usize,
                &fill.strategy,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_fill(seq: u64, side: &str, symbol: &str, qty: f64, price: f64) -> FillRecord {
        FillRecord {
            seq,
            timestamp: format!("2024-06-15T14:{:02}:00.000Z", seq),
            symbol: symbol.to_string(),
            side: side.to_string(),
            qty,
            price,
            strategy: "TestStrategy".to_string(),
            bar_index: seq,
        }
    }

    fn write_fills_to_file(path: &Path, fills: &[FillRecord]) {
        let lines: Vec<String> = fills
            .iter()
            .map(|f| serde_json::to_string(f).unwrap())
            .collect();
        fs::write(path, lines.join("\n") + "\n").unwrap();
    }

    #[test]
    fn compute_replay_missing_file_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("nonexistent.jsonl");

        let result = FillReplayer::compute_replay(&path, 5).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn compute_replay_empty_file_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("fills.jsonl");
        fs::write(&path, "").unwrap();

        let result = FillReplayer::compute_replay(&path, 0).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn compute_replay_matching_count_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("fills.jsonl");

        let fills = vec![
            make_fill(1, "buy", "AAPL", 100.0, 150.0),
            make_fill(2, "sell", "AAPL", 100.0, 155.0),
        ];
        write_fills_to_file(&path, &fills);

        let result = FillReplayer::compute_replay(&path, 2).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn compute_replay_log_ahead_returns_delta() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("fills.jsonl");

        let fills = vec![
            make_fill(1, "buy", "AAPL", 100.0, 150.0),
            make_fill(2, "sell", "AAPL", 100.0, 155.0),
            make_fill(3, "buy", "MSFT", 50.0, 300.0),
        ];
        write_fills_to_file(&path, &fills);

        // State only knows about 1 fill, so we should replay fills 2 and 3
        let result = FillReplayer::compute_replay(&path, 1).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].seq, 2);
        assert_eq!(result[1].seq, 3);
    }

    #[test]
    fn compute_replay_log_behind_state_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("fills.jsonl");

        let fills = vec![
            make_fill(1, "buy", "AAPL", 100.0, 150.0),
        ];
        write_fills_to_file(&path, &fills);

        // State says 5 fills, but log only has 1 — log is truncated
        let result = FillReplayer::compute_replay(&path, 5).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn replay_fills_processes_buy_and_sell() {
        let mut tracker = LivePositionTracker::new(100_000.0);

        let fills = vec![
            make_fill(1, "buy", "AAPL", 100.0, 150.0),
            make_fill(2, "sell", "AAPL", 100.0, 155.0),
        ];

        FillReplayer::replay_fills(&fills, &mut tracker);

        // After buy + sell, should have 2 fills in attribution
        assert_eq!(tracker.fill_attribution.len(), 2);
        assert_eq!(tracker.fill_attribution[0], "TestStrategy");
        assert_eq!(tracker.fill_attribution[1], "TestStrategy");
    }

    #[test]
    fn replay_fills_skips_unknown_side() {
        let mut tracker = LivePositionTracker::new(100_000.0);

        let fills = vec![
            make_fill(1, "buy", "AAPL", 100.0, 150.0),
            make_fill(2, "unknown", "AAPL", 100.0, 155.0),
        ];

        FillReplayer::replay_fills(&fills, &mut tracker);

        // Only the buy should produce a fill
        assert_eq!(tracker.fill_attribution.len(), 1);
    }

    #[test]
    fn compute_replay_state_zero_replays_all() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("fills.jsonl");

        let fills = vec![
            make_fill(1, "buy", "AAPL", 100.0, 150.0),
            make_fill(2, "sell", "AAPL", 100.0, 155.0),
            make_fill(3, "buy", "MSFT", 50.0, 300.0),
        ];
        write_fills_to_file(&path, &fills);

        // State has 0 fills (fresh state), so replay all
        let result = FillReplayer::compute_replay(&path, 0).unwrap();
        assert_eq!(result.len(), 3);
    }
}
