// Feature: flux-data-fetcher, Property 3: CSV serialization round-trip
//
// **Validates: Requirements 4.1, 4.3**
//
// Property 3: CSV serialization round-trip (precision preservation)
// For any valid OHLCV records, serializing to CSV and parsing with
// `csv_loader::load_csv()` preserves numeric values within epsilon (1e-10),
// symbols, and record order.

use std::fs;
use std::io::BufWriter;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::NaiveDate;
use proptest::prelude::*;

use flux_cli::csv_loader::load_csv;
use flux_cli::data::csv_writer::write_csv;
use flux_cli::data::types::Interval;
use flux_cli::data::OhlcvRecord;

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Write content to a uniquely-named temp file and return its path.
fn temp_csv_path(prefix: &str) -> PathBuf {
    let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join("flux_csv_roundtrip_tests");
    fs::create_dir_all(&dir).unwrap();
    dir.join(format!("{}_{}.csv", prefix, id))
}

/// Strategy for generating valid positive f64 values (finite, positive, not subnormal).
/// Uses a range that avoids precision issues at extremes.
fn positive_f64() -> impl Strategy<Value = f64> {
    0.01f64..1_000_000.0f64
}

/// Strategy for generating valid stock symbols (uppercase letters + digits, 1-5 chars).
fn valid_symbol() -> impl Strategy<Value = String> {
    proptest::string::string_regex("[A-Z][A-Z0-9]{0,4}")
        .unwrap()
}

/// Strategy for generating valid timestamps as NaiveDate within 2020-01-01 to 2024-12-31.
/// We use daily interval, so timestamps are at midnight.
fn valid_date() -> impl Strategy<Value = NaiveDate> {
    // Days offset from 2020-01-01 (range covers ~5 years = ~1826 days)
    (0u32..1826u32).prop_map(|days_offset| {
        let base = NaiveDate::from_ymd_opt(2020, 1, 1).unwrap();
        base + chrono::Duration::days(days_offset as i64)
    })
}

/// Strategy for generating a single valid OhlcvRecord.
fn ohlcv_record_strategy() -> impl Strategy<Value = OhlcvRecord> {
    (
        valid_date(),
        valid_symbol(),
        positive_f64(),
        positive_f64(),
        positive_f64(),
        positive_f64(),
        positive_f64(),
    )
        .prop_map(|(date, symbol, open, high, low, close, volume)| {
            OhlcvRecord {
                timestamp: date.and_hms_opt(0, 0, 0).unwrap(),
                symbol,
                open,
                high,
                low,
                close,
                volume,
            }
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn prop_csv_serialization_roundtrip(
        records in proptest::collection::vec(ohlcv_record_strategy(), 1..=20)
    ) {
        // Write records to CSV using the data module's csv_writer
        let path = temp_csv_path("roundtrip");
        {
            let file = fs::File::create(&path).unwrap();
            let mut writer = BufWriter::new(file);
            write_csv(&mut writer, &records, Interval::Day1).unwrap();
        }

        // Read back using csv_loader::load_csv
        let result = load_csv(&path);
        let _ = fs::remove_file(&path);

        let bars = result.unwrap();

        // Same number of records
        prop_assert_eq!(
            bars.len(),
            records.len(),
            "Record count mismatch: wrote {} records, read back {}",
            records.len(),
            bars.len()
        );

        // Compare each record field by field
        let epsilon = 1e-10;
        for (i, (original, loaded)) in records.iter().zip(bars.iter()).enumerate() {
            // Symbol preserved exactly
            prop_assert_eq!(
                &loaded.symbol,
                &original.symbol,
                "Symbol mismatch at record {}: expected '{}', got '{}'",
                i,
                original.symbol,
                loaded.symbol
            );

            // Numeric values within epsilon
            prop_assert!(
                (loaded.open - original.open).abs() < epsilon,
                "Open mismatch at record {}: expected {}, got {} (diff: {})",
                i,
                original.open,
                loaded.open,
                (loaded.open - original.open).abs()
            );
            prop_assert!(
                (loaded.high - original.high).abs() < epsilon,
                "High mismatch at record {}: expected {}, got {} (diff: {})",
                i,
                original.high,
                loaded.high,
                (loaded.high - original.high).abs()
            );
            prop_assert!(
                (loaded.low - original.low).abs() < epsilon,
                "Low mismatch at record {}: expected {}, got {} (diff: {})",
                i,
                original.low,
                loaded.low,
                (loaded.low - original.low).abs()
            );
            prop_assert!(
                (loaded.close - original.close).abs() < epsilon,
                "Close mismatch at record {}: expected {}, got {} (diff: {})",
                i,
                original.close,
                loaded.close,
                (loaded.close - original.close).abs()
            );
            prop_assert!(
                (loaded.volume - original.volume).abs() < epsilon,
                "Volume mismatch at record {}: expected {}, got {} (diff: {})",
                i,
                original.volume,
                loaded.volume,
                (loaded.volume - original.volume).abs()
            );
        }
    }
}
