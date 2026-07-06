//! Data fetching subsystem for the Flux CLI.
//!
//! This module provides the `DataFetcher` trait, OHLCV record types,
//! and pluggable provider implementations for downloading historical
//! market data.

pub mod csv_writer;
pub mod error;
pub mod registry;
pub mod types;
pub mod yahoo;

use chrono::NaiveDateTime;
use error::FetchError;
use types::{Interval, TimeRange};

/// A single OHLCV data point.
#[derive(Debug, Clone, PartialEq)]
pub struct OhlcvRecord {
    /// Timestamp of this bar.
    pub timestamp: NaiveDateTime,
    /// Stock ticker symbol.
    pub symbol: String,
    /// Opening price.
    pub open: f64,
    /// High price.
    pub high: f64,
    /// Low price.
    pub low: f64,
    /// Closing price.
    pub close: f64,
    /// Trading volume.
    pub volume: f64,
}

/// Parameters for a single-symbol data fetch.
#[derive(Debug, Clone)]
pub struct FetchRequest {
    /// Stock ticker symbol (e.g., "AAPL", "MSFT").
    pub symbol: String,
    /// Time range specification.
    pub time_range: TimeRange,
    /// Bar interval/granularity.
    pub interval: Interval,
}

/// Trait for pluggable market data providers.
///
/// Implementors fetch historical OHLCV data from a specific source.
/// The trait is object-safe to support dynamic dispatch via the registry.
pub trait DataFetcher {
    /// Human-readable provider name (e.g., "yahoo", "alpaca").
    fn name(&self) -> &str;

    /// Fetch historical data for a single symbol.
    ///
    /// Returns records in chronological order (earliest first).
    /// May return an empty Vec if no data exists for the given range.
    fn fetch(&self, request: &FetchRequest) -> Result<Vec<OhlcvRecord>, FetchError>;
}

/// Merge and sort records from multiple symbols by timestamp.
///
/// Records are sorted primarily by timestamp (chronological),
/// secondarily by symbol (alphabetical) for deterministic ordering
/// within the same timestamp.
pub fn merge_records(mut records: Vec<OhlcvRecord>) -> Vec<OhlcvRecord> {
    records.sort_by(|a, b| {
        a.timestamp
            .cmp(&b.timestamp)
            .then_with(|| a.symbol.cmp(&b.symbol))
    });
    records
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn make_record(date_str: &str, symbol: &str) -> OhlcvRecord {
        let date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d").unwrap();
        OhlcvRecord {
            timestamp: date.and_hms_opt(0, 0, 0).unwrap(),
            symbol: symbol.to_string(),
            open: 100.0,
            high: 105.0,
            low: 95.0,
            close: 102.0,
            volume: 1000.0,
        }
    }

    #[test]
    fn merge_records_empty_input() {
        let result = merge_records(vec![]);
        assert!(result.is_empty());
    }

    #[test]
    fn merge_records_single_symbol_sorted_by_timestamp() {
        let records = vec![
            make_record("2024-01-03", "AAPL"),
            make_record("2024-01-01", "AAPL"),
            make_record("2024-01-02", "AAPL"),
        ];
        let merged = merge_records(records);
        assert_eq!(merged[0].timestamp.date(), NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
        assert_eq!(merged[1].timestamp.date(), NaiveDate::from_ymd_opt(2024, 1, 2).unwrap());
        assert_eq!(merged[2].timestamp.date(), NaiveDate::from_ymd_opt(2024, 1, 3).unwrap());
    }

    #[test]
    fn merge_records_same_timestamp_sorted_alphabetically() {
        let records = vec![
            make_record("2024-01-01", "MSFT"),
            make_record("2024-01-01", "AAPL"),
            make_record("2024-01-01", "GOOG"),
        ];
        let merged = merge_records(records);
        assert_eq!(merged[0].symbol, "AAPL");
        assert_eq!(merged[1].symbol, "GOOG");
        assert_eq!(merged[2].symbol, "MSFT");
    }

    #[test]
    fn merge_records_multi_symbol_chronological_then_alphabetical() {
        let records = vec![
            make_record("2024-01-02", "MSFT"),
            make_record("2024-01-01", "MSFT"),
            make_record("2024-01-02", "AAPL"),
            make_record("2024-01-01", "AAPL"),
        ];
        let merged = merge_records(records);

        // First timestamp block: 2024-01-01
        assert_eq!(merged[0].timestamp.date(), NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
        assert_eq!(merged[0].symbol, "AAPL");
        assert_eq!(merged[1].timestamp.date(), NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
        assert_eq!(merged[1].symbol, "MSFT");

        // Second timestamp block: 2024-01-02
        assert_eq!(merged[2].timestamp.date(), NaiveDate::from_ymd_opt(2024, 1, 2).unwrap());
        assert_eq!(merged[2].symbol, "AAPL");
        assert_eq!(merged[3].timestamp.date(), NaiveDate::from_ymd_opt(2024, 1, 2).unwrap());
        assert_eq!(merged[3].symbol, "MSFT");
    }
}
