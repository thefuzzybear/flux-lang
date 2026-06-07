use std::path::Path;

use flux_runtime::BarContext;

use crate::error::CsvError;

/// Loads bar data from a CSV file.
///
/// The CSV must contain a header row with (at minimum) the columns:
/// `timestamp`, `symbol`, `open`, `high`, `low`, `close`, `volume`.
/// Column matching is case-insensitive. Extra columns are ignored.
///
/// Returns a `Vec<BarContext>` with `in_position` set to `false` for all bars.
pub fn load_csv(path: &Path) -> Result<Vec<BarContext>, CsvError> {
    let content = std::fs::read_to_string(path).map_err(CsvError::FileAccess)?;

    let mut lines = content.lines();

    // Parse header row
    let header_line = match lines.next() {
        Some(line) => line,
        None => return Err(CsvError::EmptyFile),
    };

    let headers: Vec<String> = header_line
        .split(',')
        .map(|h| h.trim().to_lowercase())
        .collect();

    // Find indices of required columns
    let mut missing: Vec<String> = Vec::new();

    let timestamp_idx = find_column_index(&headers, "timestamp", &mut missing);
    let symbol_idx = find_column_index(&headers, "symbol", &mut missing);
    let open_idx = find_column_index(&headers, "open", &mut missing);
    let high_idx = find_column_index(&headers, "high", &mut missing);
    let low_idx = find_column_index(&headers, "low", &mut missing);
    let close_idx = find_column_index(&headers, "close", &mut missing);
    let volume_idx = find_column_index(&headers, "volume", &mut missing);

    if !missing.is_empty() {
        return Err(CsvError::MissingColumns(missing));
    }

    // Safe to unwrap since we verified no columns are missing
    let _timestamp_idx = timestamp_idx.unwrap();
    let symbol_idx = symbol_idx.unwrap();
    let open_idx = open_idx.unwrap();
    let high_idx = high_idx.unwrap();
    let low_idx = low_idx.unwrap();
    let close_idx = close_idx.unwrap();
    let volume_idx = volume_idx.unwrap();

    // Parse data rows
    let mut bars: Vec<BarContext> = Vec::new();

    for (line_number, line) in lines.enumerate() {
        // Skip empty lines
        if line.trim().is_empty() {
            continue;
        }

        let row = line_number + 1; // 1-based row number (after header)
        let fields: Vec<&str> = line.split(',').map(|f| f.trim()).collect();

        let symbol = fields
            .get(symbol_idx)
            .unwrap_or(&"")
            .to_string();

        let open = parse_f64(&fields, open_idx, row, "open")?;
        let high = parse_f64(&fields, high_idx, row, "high")?;
        let low = parse_f64(&fields, low_idx, row, "low")?;
        let close = parse_f64(&fields, close_idx, row, "close")?;
        let volume = parse_f64(&fields, volume_idx, row, "volume")?;

        bars.push(BarContext {
            close,
            open,
            high,
            low,
            volume,
            symbol,
            in_position: false,
        });
    }

    if bars.is_empty() {
        return Err(CsvError::EmptyFile);
    }

    Ok(bars)
}

/// Finds the index of a required column in the headers list (case-insensitive).
/// If not found, pushes the column name onto the `missing` vec and returns `None`.
fn find_column_index(
    headers: &[String],
    column: &str,
    missing: &mut Vec<String>,
) -> Option<usize> {
    match headers.iter().position(|h| h == column) {
        Some(idx) => Some(idx),
        None => {
            missing.push(column.to_string());
            None
        }
    }
}

/// Parses a field at a given index as f64.
/// Returns `CsvError::InvalidValue` on failure.
fn parse_f64(
    fields: &[&str],
    idx: usize,
    row: usize,
    column: &str,
) -> Result<f64, CsvError> {
    let value = fields.get(idx).unwrap_or(&"");
    value.parse::<f64>().map_err(|_| CsvError::InvalidValue {
        row,
        column: column.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use proptest::prelude::*;

    /// Helper to write CSV content to a temp file and return its path.
    fn write_temp_csv(name: &str, content: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("flux_csv_tests");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn test_load_valid_csv() {
        let csv = "timestamp,symbol,open,high,low,close,volume\n\
                   2024-01-01,AAPL,150.0,155.0,149.0,153.0,1000000\n\
                   2024-01-02,AAPL,153.0,157.0,152.0,156.0,1200000\n";
        let path = write_temp_csv("valid.csv", csv);

        let bars = load_csv(&path).unwrap();
        assert_eq!(bars.len(), 2);
        assert_eq!(bars[0].symbol, "AAPL");
        assert_eq!(bars[0].open, 150.0);
        assert_eq!(bars[0].high, 155.0);
        assert_eq!(bars[0].low, 149.0);
        assert_eq!(bars[0].close, 153.0);
        assert_eq!(bars[0].volume, 1000000.0);
        assert!(!bars[0].in_position);
    }

    #[test]
    fn test_case_insensitive_headers() {
        let csv = "Timestamp,SYMBOL,Open,HIGH,Low,CLOSE,Volume\n\
                   2024-01-01,MSFT,100.0,105.0,99.0,103.0,500000\n";
        let path = write_temp_csv("case_insensitive.csv", csv);

        let bars = load_csv(&path).unwrap();
        assert_eq!(bars.len(), 1);
        assert_eq!(bars[0].symbol, "MSFT");
        assert_eq!(bars[0].open, 100.0);
    }

    #[test]
    fn test_missing_columns() {
        let csv = "timestamp,symbol,open\n\
                   2024-01-01,AAPL,150.0\n";
        let path = write_temp_csv("missing_cols.csv", csv);

        let err = load_csv(&path).unwrap_err();
        match err {
            CsvError::MissingColumns(cols) => {
                assert!(cols.contains(&"high".to_string()));
                assert!(cols.contains(&"low".to_string()));
                assert!(cols.contains(&"close".to_string()));
                assert!(cols.contains(&"volume".to_string()));
                assert_eq!(cols.len(), 4);
            }
            _ => panic!("Expected MissingColumns error"),
        }
    }

    #[test]
    fn test_invalid_value() {
        let csv = "timestamp,symbol,open,high,low,close,volume\n\
                   2024-01-01,AAPL,not_a_number,155.0,149.0,153.0,1000000\n";
        let path = write_temp_csv("invalid_value.csv", csv);

        let err = load_csv(&path).unwrap_err();
        match err {
            CsvError::InvalidValue { row, column } => {
                assert_eq!(row, 1);
                assert_eq!(column, "open");
            }
            _ => panic!("Expected InvalidValue error"),
        }
    }

    #[test]
    fn test_empty_file() {
        let csv = "";
        let path = write_temp_csv("empty.csv", csv);

        let err = load_csv(&path).unwrap_err();
        assert!(matches!(err, CsvError::EmptyFile));
    }

    #[test]
    fn test_header_only_no_data() {
        let csv = "timestamp,symbol,open,high,low,close,volume\n";
        let path = write_temp_csv("header_only.csv", csv);

        let err = load_csv(&path).unwrap_err();
        assert!(matches!(err, CsvError::EmptyFile));
    }

    #[test]
    fn test_extra_columns_ignored() {
        let csv = "timestamp,symbol,open,high,low,close,volume,extra1,extra2\n\
                   2024-01-01,AAPL,150.0,155.0,149.0,153.0,1000000,foo,bar\n";
        let path = write_temp_csv("extra_cols.csv", csv);

        let bars = load_csv(&path).unwrap();
        assert_eq!(bars.len(), 1);
        assert_eq!(bars[0].open, 150.0);
    }

    #[test]
    fn test_file_not_found() {
        let err = load_csv(Path::new("/nonexistent/path/data.csv")).unwrap_err();
        assert!(matches!(err, CsvError::FileAccess(_)));
    }

    #[test]
    fn test_columns_in_different_order() {
        let csv = "volume,close,low,high,open,symbol,timestamp\n\
                   1000000,153.0,149.0,155.0,150.0,AAPL,2024-01-01\n";
        let path = write_temp_csv("reordered.csv", csv);

        let bars = load_csv(&path).unwrap();
        assert_eq!(bars.len(), 1);
        assert_eq!(bars[0].open, 150.0);
        assert_eq!(bars[0].close, 153.0);
        assert_eq!(bars[0].volume, 1000000.0);
    }

    // **Validates: Requirements 5.1, 5.3, 5.7**
    //
    // Property 4: CSV parsing round-trip
    // For any valid sequence of bar data records (with finite positive numeric
    // values for OHLCV, non-empty symbol strings, and arbitrary timestamp strings),
    // serializing them to CSV format with the required header and then parsing
    // with the CSV loader SHALL produce an equivalent Vec<BarContext> preserving
    // field values, row order, and symbol strings.
    proptest! {
        #[test]
        fn prop_csv_round_trip(
            rows in proptest::collection::vec(
                (
                    "[A-Z]{1,5}",           // symbol
                    "[0-9]{4}-[0-9]{2}-[0-9]{2}", // timestamp
                    0.01..1_000_000.0f64,   // open
                    0.01..1_000_000.0f64,   // high
                    0.01..1_000_000.0f64,   // low
                    0.01..1_000_000.0f64,   // close
                    0.01..1_000_000.0f64,   // volume
                ),
                1..=20,
            )
        ) {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);

            // Serialize to CSV
            let mut csv_content = String::from("timestamp,symbol,open,high,low,close,volume\n");
            for (symbol, timestamp, open, high, low, close, volume) in &rows {
                csv_content.push_str(&format!(
                    "{},{},{:.10},{:.10},{:.10},{:.10},{:.10}\n",
                    timestamp, symbol, open, high, low, close, volume
                ));
            }

            // Write to temp file with unique name
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let file_name = format!("prop_csv_rt_{}.csv", id);
            let path = write_temp_csv(&file_name, &csv_content);

            // Parse back
            let result = load_csv(&path);
            let _ = fs::remove_file(&path);

            let bars = result.unwrap();

            // Verify same number of bars
            prop_assert_eq!(bars.len(), rows.len());

            // Verify each bar matches input
            for (i, (symbol, _timestamp, open, high, low, close, volume)) in rows.iter().enumerate() {
                prop_assert_eq!(&bars[i].symbol, symbol, "symbol mismatch at row {}", i);
                let eps = 1e-6;
                prop_assert!(
                    (bars[i].open - open).abs() < eps,
                    "open mismatch at row {}: got {} expected {}",
                    i, bars[i].open, open
                );
                prop_assert!(
                    (bars[i].high - high).abs() < eps,
                    "high mismatch at row {}: got {} expected {}",
                    i, bars[i].high, high
                );
                prop_assert!(
                    (bars[i].low - low).abs() < eps,
                    "low mismatch at row {}: got {} expected {}",
                    i, bars[i].low, low
                );
                prop_assert!(
                    (bars[i].close - close).abs() < eps,
                    "close mismatch at row {}: got {} expected {}",
                    i, bars[i].close, close
                );
                prop_assert!(
                    (bars[i].volume - volume).abs() < eps,
                    "volume mismatch at row {}: got {} expected {}",
                    i, bars[i].volume, volume
                );
                prop_assert_eq!(bars[i].in_position, false, "in_position should be false at row {}", i);
            }
        }
    }

    // ========================================================================
    // Property 7: Missing column detection accuracy
    // ========================================================================

    // **Validates: Requirements 5.5**
    //
    // Property 7: Missing column detection accuracy
    // For any non-empty subset of the required columns removed from the CSV
    // header, the CSV loader SHALL return an error whose missing-columns list
    // is exactly that subset (same elements, regardless of order).
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_missing_column_detection(
            remove_mask in prop::array::uniform7(any::<bool>())
        ) {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);

            let all_columns = ["timestamp", "symbol", "open", "high", "low", "close", "volume"];

            // Determine which columns to remove (at least one must be removed)
            let columns_to_remove: Vec<&str> = all_columns
                .iter()
                .zip(remove_mask.iter())
                .filter(|(_, &remove)| remove)
                .map(|(&col, _)| col)
                .collect();

            // Skip if no columns are removed (property requires non-empty subset)
            prop_assume!(!columns_to_remove.is_empty());

            // Build header with remaining columns only
            let remaining_columns: Vec<&str> = all_columns
                .iter()
                .zip(remove_mask.iter())
                .filter(|(_, &remove)| !remove)
                .map(|(&col, _)| col)
                .collect();

            let header = remaining_columns.join(",");

            // Build a data row matching the remaining columns
            // Use placeholder values (won't matter since we fail at header check)
            let data_values: Vec<&str> = remaining_columns
                .iter()
                .map(|&col| match col {
                    "timestamp" => "2024-01-01",
                    "symbol" => "AAPL",
                    _ => "100.0",
                })
                .collect();
            let data_row = data_values.join(",");

            let csv_content = format!("{}\n{}\n", header, data_row);

            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = write_temp_csv(&format!("prop7_missing_{}.csv", id), &csv_content);
            let result = load_csv(&path);
            let _ = fs::remove_file(&path);

            // Must be a MissingColumns error
            match result {
                Err(CsvError::MissingColumns(mut reported_missing)) => {
                    // Sort both lists and compare
                    reported_missing.sort();
                    let mut expected_missing: Vec<String> = columns_to_remove
                        .iter()
                        .map(|&s| s.to_string())
                        .collect();
                    expected_missing.sort();

                    prop_assert_eq!(
                        reported_missing,
                        expected_missing,
                        "Missing columns mismatch"
                    );
                }
                Err(other) => {
                    prop_assert!(
                        false,
                        "Expected MissingColumns error, got: {:?}",
                        other
                    );
                }
                Ok(_) => {
                    prop_assert!(
                        false,
                        "Expected MissingColumns error, but parsing succeeded"
                    );
                }
            }
        }
    }

    // ========================================================================
    // Property 6: Invalid numeric value detection accuracy
    // ========================================================================

    // **Validates: Requirements 5.4**
    //
    // Property 6: Invalid numeric value detection accuracy
    // For any valid CSV content with a single numeric cell replaced by a
    // non-numeric string at row r and column c, the CSV loader SHALL return
    // an error identifying exactly row r and column name c.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_invalid_numeric_detection(
            rows in proptest::collection::vec(
                (
                    "[A-Z]{1,4}",           // symbol
                    "[0-9]{4}-[0-9]{2}-[0-9]{2}", // timestamp
                    0.01..1_000_000.0f64,   // open
                    0.01..1_000_000.0f64,   // high
                    0.01..1_000_000.0f64,   // low
                    0.01..1_000_000.0f64,   // close
                    0.01..1_000_000.0f64,   // volume
                ),
                1..=10,
            ),
            corrupt_row_idx in 0..10usize,
            corrupt_col_idx in 0..5usize,
            corrupt_value in "[a-z]{1,5}",
        ) {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);

            // Ensure corrupt_row_idx is within bounds
            let corrupt_row_idx = corrupt_row_idx % rows.len();

            let numeric_columns = ["open", "high", "low", "close", "volume"];
            let corrupt_col_name = numeric_columns[corrupt_col_idx];

            // Build CSV content
            let header = "timestamp,symbol,open,high,low,close,volume";
            let mut csv_lines: Vec<String> = Vec::with_capacity(rows.len() + 1);
            csv_lines.push(header.to_string());

            for (i, (symbol, timestamp, open, high, low, close, volume)) in rows.iter().enumerate() {
                let values: [String; 5] = [
                    format!("{:.10}", open),
                    format!("{:.10}", high),
                    format!("{:.10}", low),
                    format!("{:.10}", close),
                    format!("{:.10}", volume),
                ];

                // Replace the target cell with the corrupt value
                let mut row_values = values.clone();
                if i == corrupt_row_idx {
                    row_values[corrupt_col_idx] = corrupt_value.clone();
                }

                csv_lines.push(format!(
                    "{},{},{},{},{},{},{}",
                    timestamp, symbol,
                    row_values[0], row_values[1], row_values[2],
                    row_values[3], row_values[4]
                ));
            }

            let csv_content = csv_lines.join("\n") + "\n";

            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let file_name = format!("prop6_invalid_{}.csv", id);
            let path = write_temp_csv(&file_name, &csv_content);

            let result = load_csv(&path);
            let _ = fs::remove_file(&path);

            // The loader processes columns in order: open, high, low, close, volume.
            // Since we corrupt only one cell, the error should point to that exact column.
            // However, the row reported depends on whether the corrupt row is the first
            // row with any error. Since all other rows are valid, the first error will
            // be in the corrupt row at the corrupt column.
            //
            // The expected row is 1-based (corrupt_row_idx + 1).
            // The expected column depends on column parse order within that row.
            // Since only one cell is corrupted, it must be the one that fails.
            let expected_row = corrupt_row_idx + 1;
            let expected_col = corrupt_col_name;

            match result {
                Err(CsvError::InvalidValue { row, column }) => {
                    prop_assert_eq!(
                        row, expected_row,
                        "Expected error at row {}, got row {} (corrupt_col: {})",
                        expected_row, row, expected_col
                    );
                    prop_assert_eq!(
                        column.as_str(), expected_col,
                        "Expected error in column '{}', got '{}' (row: {})",
                        expected_col, column, expected_row
                    );
                }
                Err(other) => {
                    prop_assert!(
                        false,
                        "Expected InvalidValue error, got: {:?}",
                        other
                    );
                }
                Ok(_) => {
                    prop_assert!(
                        false,
                        "Expected InvalidValue error but parsing succeeded"
                    );
                }
            }
        }
    }

    // ========================================================================
    // Property 5: Case-insensitive header matching
    // ========================================================================

    /// Apply a random case transformation to a header string based on a boolean vector.
    /// For each alphabetic character, if the corresponding bool is true, convert to uppercase;
    /// otherwise keep it lowercase.
    fn apply_case_transform(header: &str, case_bits: &[bool]) -> String {
        let mut result = String::with_capacity(header.len());
        let mut bit_idx = 0;
        for ch in header.chars() {
            if ch.is_alphabetic() {
                if bit_idx < case_bits.len() && case_bits[bit_idx] {
                    result.extend(ch.to_uppercase());
                } else {
                    result.extend(ch.to_lowercase());
                }
                bit_idx += 1;
            } else {
                result.push(ch);
            }
        }
        result
    }

    // **Validates: Requirements 5.2**
    //
    // Property 5: Case-insensitive header matching
    // For any valid CSV content, applying an arbitrary case transformation
    // (upper, lower, mixed) to each character in the header row SHALL still
    // result in successful parsing that produces the same Vec<BarContext> as
    // the original.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_case_insensitive_header_matching(
            case_bits in prop::collection::vec(any::<bool>(), 38)
        ) {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);

            // The standard header: "timestamp,symbol,open,high,low,close,volume"
            // has 38 alphabetic characters
            let original_header = "timestamp,symbol,open,high,low,close,volume";
            let data_row = "2024-01-01,AAPL,150.0,155.0,149.0,153.0,1000000";

            // Create CSV with original (lowercase) header
            let original_csv = format!("{}\n{}\n", original_header, data_row);
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let original_path = write_temp_csv(
                &format!("prop5_orig_{}.csv", id),
                &original_csv,
            );
            let original_bars = load_csv(&original_path).unwrap();
            let _ = fs::remove_file(&original_path);

            // Apply random case transformation to header
            let transformed_header = apply_case_transform(original_header, &case_bits);
            let transformed_csv = format!("{}\n{}\n", transformed_header, data_row);
            let transformed_path = write_temp_csv(
                &format!("prop5_trans_{}.csv", id),
                &transformed_csv,
            );
            let transformed_result = load_csv(&transformed_path);
            let _ = fs::remove_file(&transformed_path);

            // Parsing with transformed header must succeed
            let transformed_bars = transformed_result.unwrap();

            // Both should produce the same results
            prop_assert_eq!(
                original_bars.len(),
                transformed_bars.len(),
                "Row count mismatch: original={}, transformed={} (header: '{}')",
                original_bars.len(),
                transformed_bars.len(),
                transformed_header
            );

            for (i, (orig, trans)) in original_bars.iter().zip(transformed_bars.iter()).enumerate() {
                prop_assert_eq!(&orig.symbol, &trans.symbol, "Row {}: symbol mismatch", i);
                prop_assert_eq!(orig.open, trans.open, "Row {}: open mismatch", i);
                prop_assert_eq!(orig.high, trans.high, "Row {}: high mismatch", i);
                prop_assert_eq!(orig.low, trans.low, "Row {}: low mismatch", i);
                prop_assert_eq!(orig.close, trans.close, "Row {}: close mismatch", i);
                prop_assert_eq!(orig.volume, trans.volume, "Row {}: volume mismatch", i);
                prop_assert_eq!(orig.in_position, trans.in_position, "Row {}: in_position mismatch", i);
            }
        }
    }

    // ========================================================================
    // Property 8: Extra columns are ignored
    // ========================================================================

    // **Validates: Requirements 5.9**
    //
    // Property 8: Extra columns are ignored
    // For any valid CSV content with additional arbitrary columns appended
    // (with arbitrary header names and cell values), parsing SHALL produce
    // the same Vec<BarContext> as parsing the CSV without the extra columns.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_extra_columns_ignored(
            rows in proptest::collection::vec(
                (
                    "[A-Z]{1,5}",           // symbol
                    "[0-9]{4}-[0-9]{2}-[0-9]{2}", // timestamp
                    0.01..1_000_000.0f64,   // open
                    0.01..1_000_000.0f64,   // high
                    0.01..1_000_000.0f64,   // low
                    0.01..1_000_000.0f64,   // close
                    0.01..1_000_000.0f64,   // volume
                ),
                1..=10,
            ),
            extra_col_names in proptest::collection::vec("[a-z]{3,8}", 1..=5),
            extra_col_values_seed in proptest::collection::vec(
                proptest::collection::vec("[a-zA-Z0-9]{1,10}", 1..=5),
                1..=10,
            ),
        ) {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);

            // Filter out extra column names that collide with required columns
            let required_names = ["timestamp", "symbol", "open", "high", "low", "close", "volume"];
            let extra_names: Vec<&String> = extra_col_names
                .iter()
                .filter(|name| !required_names.contains(&name.as_str()))
                .collect();

            // If all extra names collided with required names, skip this test case
            prop_assume!(!extra_names.is_empty());

            let num_extra = extra_names.len();

            // Build the base CSV (without extra columns)
            let base_header = "timestamp,symbol,open,high,low,close,volume";
            let mut base_csv = format!("{}\n", base_header);
            for (symbol, timestamp, open, high, low, close, volume) in &rows {
                base_csv.push_str(&format!(
                    "{},{},{:.10},{:.10},{:.10},{:.10},{:.10}\n",
                    timestamp, symbol, open, high, low, close, volume
                ));
            }

            // Build the extended CSV (with extra columns appended)
            let extra_header_part = extra_names.iter().map(|n| n.as_str()).collect::<Vec<_>>().join(",");
            let extended_header = format!("{},{}", base_header, extra_header_part);
            let mut extended_csv = format!("{}\n", extended_header);
            for (i, (symbol, timestamp, open, high, low, close, volume)) in rows.iter().enumerate() {
                // Get extra values for this row, padding with "x" if needed
                let row_extras: Vec<&str> = if i < extra_col_values_seed.len() {
                    extra_col_values_seed[i]
                        .iter()
                        .take(num_extra)
                        .map(|s| s.as_str())
                        .collect()
                } else {
                    vec!["x"; num_extra]
                };
                // Pad if we don't have enough extra values
                let mut padded_extras = row_extras;
                while padded_extras.len() < num_extra {
                    padded_extras.push("x");
                }

                let extra_values_part = padded_extras.join(",");
                extended_csv.push_str(&format!(
                    "{},{},{:.10},{:.10},{:.10},{:.10},{:.10},{}\n",
                    timestamp, symbol, open, high, low, close, volume, extra_values_part
                ));
            }

            // Write both CSVs to temp files
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let base_path = write_temp_csv(&format!("prop8_base_{}.csv", id), &base_csv);
            let extended_path = write_temp_csv(&format!("prop8_ext_{}.csv", id), &extended_csv);

            // Parse both
            let base_result = load_csv(&base_path);
            let extended_result = load_csv(&extended_path);
            let _ = fs::remove_file(&base_path);
            let _ = fs::remove_file(&extended_path);

            let base_bars = base_result.unwrap();
            let extended_bars = extended_result.unwrap();

            // Verify they produce the same Vec<BarContext>
            prop_assert_eq!(
                base_bars.len(),
                extended_bars.len(),
                "Row count mismatch: base={}, extended={}",
                base_bars.len(),
                extended_bars.len()
            );

            for (i, (base, ext)) in base_bars.iter().zip(extended_bars.iter()).enumerate() {
                prop_assert_eq!(&base.symbol, &ext.symbol, "Row {}: symbol mismatch", i);
                prop_assert_eq!(base.open, ext.open, "Row {}: open mismatch", i);
                prop_assert_eq!(base.high, ext.high, "Row {}: high mismatch", i);
                prop_assert_eq!(base.low, ext.low, "Row {}: low mismatch", i);
                prop_assert_eq!(base.close, ext.close, "Row {}: close mismatch", i);
                prop_assert_eq!(base.volume, ext.volume, "Row {}: volume mismatch", i);
                prop_assert_eq!(base.in_position, ext.in_position, "Row {}: in_position mismatch", i);
            }
        }
    }
}
