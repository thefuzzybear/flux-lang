//! Append-only fill logger writing JSONL to disk.
//!
//! Each fill produced by the position tracker is appended as a single JSON line
//! to a `.jsonl` file. The logger maintains a monotonically increasing sequence
//! number and flushes after every write to ensure durability.

use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

/// A single fill record for the JSONL log.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct FillRecord {
    /// Monotonically increasing sequence number.
    pub seq: u64,
    /// ISO 8601 timestamp when the fill occurred.
    pub timestamp: String,
    /// Trading symbol (e.g. "AAPL").
    pub symbol: String,
    /// "buy" or "sell".
    pub side: String,
    /// Quantity filled.
    pub qty: f64,
    /// Fill price.
    pub price: f64,
    /// Strategy that generated the signal.
    pub strategy: String,
    /// Bar index at which the fill occurred.
    pub bar_index: u64,
}

/// Append-only fill logger writing JSONL to disk.
pub struct FillLogger {
    /// Open file handle in append mode, buffered.
    writer: BufWriter<File>,
    /// Path to the .jsonl file.
    path: PathBuf,
    /// Next sequence number to assign.
    next_seq: u64,
}

impl FillLogger {
    /// Create or open the fill log file in append mode.
    ///
    /// If the file exists, reads it line-by-line to determine the highest `seq`
    /// and sets `next_seq = max_seq + 1`. If the file is empty or missing,
    /// starts at `seq = 1`.
    pub fn open(path: &Path) -> Result<Self, io::Error> {
        let next_seq = if path.exists() {
            Self::read_max_seq(path)? + 1
        } else {
            1
        };

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;

        Ok(Self {
            writer: BufWriter::new(file),
            path: path.to_path_buf(),
            next_seq,
        })
    }

    /// Append a fill record and flush to OS.
    ///
    /// The record's `seq` field is overwritten with `self.next_seq` before
    /// serialization. Returns the assigned sequence number.
    pub fn append(&mut self, record: &FillRecord) -> Result<u64, io::Error> {
        let seq = self.next_seq;

        let mut output = record.clone();
        output.seq = seq;

        let line = serde_json::to_string(&output)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        writeln!(self.writer, "{}", line)?;
        self.writer.flush()?;

        self.next_seq += 1;
        Ok(seq)
    }

    /// Returns the path to the fill log file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the next sequence number that will be assigned.
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// Read an existing fill log and find the maximum `seq` value.
    /// Returns 0 if the file is empty or contains no valid records.
    fn read_max_seq(path: &Path) -> Result<u64, io::Error> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let mut max_seq: u64 = 0;

        for line in reader.lines() {
            let line = line?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(record) = serde_json::from_str::<FillRecord>(trimmed) {
                if record.seq > max_seq {
                    max_seq = record.seq;
                }
            }
            // Skip malformed lines gracefully
        }

        Ok(max_seq)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn sample_record() -> FillRecord {
        FillRecord {
            seq: 0,
            timestamp: "2024-06-15T14:30:00.123Z".to_string(),
            symbol: "AAPL".to_string(),
            side: "buy".to_string(),
            qty: 100.0,
            price: 185.50,
            strategy: "MeanReversion".to_string(),
            bar_index: 42,
        }
    }

    #[test]
    fn test_creates_file_on_first_write() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("fills.jsonl");

        let mut logger = FillLogger::open(&path).unwrap();
        assert!(path.exists());

        let seq = logger.append(&sample_record()).unwrap();
        assert_eq!(seq, 1);

        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content.lines().count(), 1);

        let parsed: FillRecord = serde_json::from_str(content.lines().next().unwrap()).unwrap();
        assert_eq!(parsed.seq, 1);
        assert_eq!(parsed.symbol, "AAPL");
    }

    #[test]
    fn test_appends_to_existing_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("fills.jsonl");

        // Write two records with first logger
        {
            let mut logger = FillLogger::open(&path).unwrap();
            logger.append(&sample_record()).unwrap();
            logger.append(&sample_record()).unwrap();
        }

        // Open a new logger — should resume at seq 3
        {
            let mut logger = FillLogger::open(&path).unwrap();
            assert_eq!(logger.next_seq(), 3);
            let seq = logger.append(&sample_record()).unwrap();
            assert_eq!(seq, 3);
        }

        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content.lines().count(), 3);
    }

    #[test]
    fn test_monotonic_sequence() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("fills.jsonl");

        let mut logger = FillLogger::open(&path).unwrap();
        let s1 = logger.append(&sample_record()).unwrap();
        let s2 = logger.append(&sample_record()).unwrap();
        let s3 = logger.append(&sample_record()).unwrap();

        assert_eq!(s1, 1);
        assert_eq!(s2, 2);
        assert_eq!(s3, 3);
    }

    #[test]
    fn test_empty_file_starts_at_seq_1() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("fills.jsonl");

        // Create an empty file
        fs::write(&path, "").unwrap();

        let logger = FillLogger::open(&path).unwrap();
        assert_eq!(logger.next_seq(), 1);
    }

    #[test]
    fn test_single_line_per_record() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("fills.jsonl");

        let mut logger = FillLogger::open(&path).unwrap();
        logger.append(&sample_record()).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        // Each record must be exactly one line (no embedded newlines)
        for line in content.lines() {
            let _: FillRecord = serde_json::from_str(line).unwrap();
        }
    }

    #[test]
    fn test_missing_file_starts_fresh() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("nonexistent.jsonl");

        let logger = FillLogger::open(&path).unwrap();
        assert_eq!(logger.next_seq(), 1);
    }

    #[test]
    fn test_flush_guarantees_readable_after_append() {
        // After each append(), a separate reader must be able to read the content
        // immediately — this validates that flush() actually pushes data to the OS.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("fills.jsonl");

        let mut logger = FillLogger::open(&path).unwrap();

        // Append first record
        logger.append(&sample_record()).unwrap();

        // Open the file independently (simulates another process reading)
        let content = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 1);
        let r1: FillRecord = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(r1.seq, 1);

        // Append second record
        logger.append(&sample_record()).unwrap();

        // Read again from scratch — both records must be present
        let content = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        let r2: FillRecord = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(r2.seq, 2);

        // Append third record
        logger.append(&sample_record()).unwrap();

        // All three readable immediately
        let content = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 3);
        let r3: FillRecord = serde_json::from_str(lines[2]).unwrap();
        assert_eq!(r3.seq, 3);
    }

    #[test]
    fn test_appending_preserves_previous_records_byte_for_byte() {
        // When a second logger session appends, the first N records must be
        // byte-for-byte identical to what the first session wrote.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("fills.jsonl");

        // First session: write 3 records with distinct data
        let records_session1 = vec![
            FillRecord {
                seq: 0,
                timestamp: "2024-06-15T14:30:00.000Z".to_string(),
                symbol: "AAPL".to_string(),
                side: "buy".to_string(),
                qty: 100.0,
                price: 185.50,
                strategy: "Alpha".to_string(),
                bar_index: 1,
            },
            FillRecord {
                seq: 0,
                timestamp: "2024-06-15T14:31:00.000Z".to_string(),
                symbol: "MSFT".to_string(),
                side: "sell".to_string(),
                qty: 50.0,
                price: 380.25,
                strategy: "Beta".to_string(),
                bar_index: 2,
            },
            FillRecord {
                seq: 0,
                timestamp: "2024-06-15T14:32:00.000Z".to_string(),
                symbol: "GOOG".to_string(),
                side: "buy".to_string(),
                qty: 25.0,
                price: 2800.00,
                strategy: "Gamma".to_string(),
                bar_index: 3,
            },
        ];

        {
            let mut logger = FillLogger::open(&path).unwrap();
            for r in &records_session1 {
                logger.append(r).unwrap();
            }
        }

        // Capture the raw bytes after first session
        let content_after_session1 = fs::read_to_string(&path).unwrap();
        let lines_session1: Vec<&str> = content_after_session1.lines().collect();
        assert_eq!(lines_session1.len(), 3);

        // Second session: open logger (resumes at seq 4), append 2 more records
        {
            let mut logger = FillLogger::open(&path).unwrap();
            assert_eq!(logger.next_seq(), 4);
            logger.append(&FillRecord {
                seq: 0,
                timestamp: "2024-06-15T14:33:00.000Z".to_string(),
                symbol: "TSLA".to_string(),
                side: "buy".to_string(),
                qty: 10.0,
                price: 250.00,
                strategy: "Delta".to_string(),
                bar_index: 4,
            }).unwrap();
            logger.append(&FillRecord {
                seq: 0,
                timestamp: "2024-06-15T14:34:00.000Z".to_string(),
                symbol: "NVDA".to_string(),
                side: "sell".to_string(),
                qty: 30.0,
                price: 900.00,
                strategy: "Epsilon".to_string(),
                bar_index: 5,
            }).unwrap();
        }

        // Read back — the first 3 lines must be byte-for-byte identical
        let content_after_session2 = fs::read_to_string(&path).unwrap();
        let lines_session2: Vec<&str> = content_after_session2.lines().collect();
        assert_eq!(lines_session2.len(), 5);

        // Byte-for-byte preservation of prior records
        for i in 0..3 {
            assert_eq!(
                lines_session1[i], lines_session2[i],
                "Line {} was modified by second session",
                i
            );
        }

        // Verify new records have correct seq
        let r4: FillRecord = serde_json::from_str(lines_session2[3]).unwrap();
        let r5: FillRecord = serde_json::from_str(lines_session2[4]).unwrap();
        assert_eq!(r4.seq, 4);
        assert_eq!(r4.symbol, "TSLA");
        assert_eq!(r5.seq, 5);
        assert_eq!(r5.symbol, "NVDA");
    }
}
