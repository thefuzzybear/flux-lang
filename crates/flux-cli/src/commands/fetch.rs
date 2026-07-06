use std::path::PathBuf;

use chrono::NaiveDate;

use crate::data::csv_writer::write_csv;
use crate::data::registry::{build_registry, get_provider};
use crate::data::types::{Interval, Period, TimeRange};
use crate::data::{self, FetchRequest, OhlcvRecord};

/// Run the fetch command — orchestrates validation, fetching, merge, and output.
///
/// This function validates all inputs before making any network calls,
/// then fetches data per symbol, merges the results, and writes output.
pub fn run_fetch(
    symbols: &str,
    source: &str,
    period: Option<&str>,
    interval: &str,
    from: Option<&str>,
    to: Option<&str>,
    output: Option<&PathBuf>,
) -> Result<(), String> {
    // 1. Parse symbol list (comma-separated)
    let symbol_list: Vec<&str> = symbols
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if symbol_list.is_empty() {
        return Err("no symbols provided".to_string());
    }

    // 2. Validate interval
    let interval: Interval = interval
        .parse()
        .map_err(|e: String| format!("error: {}", e))?;

    // 3. Validate mutual exclusion of period and from/to
    if period.is_some() && (from.is_some() || to.is_some()) {
        return Err("error: --period and --from/--to are mutually exclusive".to_string());
    }

    // 4. Build time range
    let time_range = build_time_range(period, from, to)?;

    // 5. Look up provider
    let registry = build_registry();
    let provider = get_provider(&registry, source).map_err(|e| format!("error: {}", e))?;

    // 6. Fetch per symbol
    let mut all_records: Vec<OhlcvRecord> = Vec::new();
    let mut failures: Vec<(String, String)> = Vec::new();

    for sym in &symbol_list {
        let request = FetchRequest {
            symbol: sym.to_string(),
            time_range: time_range.clone(),
            interval,
        };
        match provider.fetch(&request) {
            Ok(records) => {
                if records.is_empty() {
                    eprintln!(
                        "warning: no data found for {} in the given time range",
                        sym
                    );
                }
                all_records.extend(records);
            }
            Err(e) => {
                eprintln!("warning: failed to fetch {}: {}", sym, e);
                failures.push((sym.to_string(), e.to_string()));
            }
        }
    }

    // 7. If all failed, return error
    if all_records.is_empty() && !failures.is_empty() {
        return Err("error: all symbols failed to fetch".to_string());
    }

    // 8. Merge and sort
    let merged = data::merge_records(all_records);

    // 9. Write output
    write_output(&merged, output, interval)?;

    Ok(())
}

/// Write merged records to the specified output destination.
///
/// When `--output` is specified, creates parent directories if needed,
/// overwrites existing files, and prints a summary to stderr.
/// When `--output` is not specified, writes to stdout.
fn write_output(
    records: &[OhlcvRecord],
    output: Option<&PathBuf>,
    interval: Interval,
) -> Result<(), String> {
    match output {
        Some(path) => {
            // Create parent directories if needed
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        format!(
                            "error: failed to create directory '{}': {}",
                            parent.display(),
                            e
                        )
                    })?;
                }
            }
            // Write to file (overwrites existing)
            let file = std::fs::File::create(path).map_err(|e| {
                format!("error: failed to write '{}': {}", path.display(), e)
            })?;
            let mut writer = std::io::BufWriter::new(file);
            write_csv(&mut writer, records, interval)
                .map_err(|e| format!("error: write failed: {}", e))?;
            // Print summary to stderr
            eprintln!("wrote {} records to {}", records.len(), path.display());
        }
        None => {
            // Write to stdout
            let stdout = std::io::stdout();
            let mut writer = stdout.lock();
            write_csv(&mut writer, records, interval)
                .map_err(|e| format!("error: write failed: {}", e))?;
        }
    }
    Ok(())
}

/// Build the time range from CLI arguments.
///
/// Handles three cases:
/// - Period only: parse as relative period
/// - From+To: parse as absolute date range, validate from < to
/// - Neither: default to 1 year period
///
/// Also validates partial from/to (one without the other).
fn build_time_range(
    period: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<TimeRange, String> {
    match (period, from, to) {
        (Some(p), None, None) => {
            let period: Period = p.parse().map_err(|e: String| format!("error: {}", e))?;
            Ok(TimeRange::Period(period))
        }
        (None, Some(f), Some(t)) => {
            let from_date = parse_date(f)?;
            let to_date = parse_date(t)?;
            if from_date >= to_date {
                return Err(
                    "error: --from date must be before --to date".to_string()
                );
            }
            Ok(TimeRange::DateRange {
                from: from_date,
                to: to_date,
            })
        }
        (None, Some(_), None) => Err("error: --from requires --to".to_string()),
        (None, None, Some(_)) => Err("error: --to requires --from".to_string()),
        (None, None, None) => {
            // Default to 1y period
            Ok(TimeRange::Period(Period::Year1))
        }
        _ => Err("error: --period and --from/--to are mutually exclusive".to_string()),
    }
}

/// Parse a date string in YYYY-MM-DD format.
fn parse_date(s: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| format!("error: invalid date '{}'. Expected format: YYYY-MM-DD", s))
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- parse_date tests ---

    #[test]
    fn parse_date_valid() {
        let date = parse_date("2024-01-15").unwrap();
        assert_eq!(date, NaiveDate::from_ymd_opt(2024, 1, 15).unwrap());
    }

    #[test]
    fn parse_date_invalid_format() {
        let err = parse_date("01-15-2024").unwrap_err();
        assert!(err.contains("invalid date '01-15-2024'"));
        assert!(err.contains("YYYY-MM-DD"));
    }

    #[test]
    fn parse_date_invalid_nonsense() {
        let err = parse_date("not-a-date").unwrap_err();
        assert!(err.contains("invalid date"));
    }

    #[test]
    fn parse_date_invalid_day() {
        let err = parse_date("2024-02-30").unwrap_err();
        assert!(err.contains("invalid date '2024-02-30'"));
    }

    // --- build_time_range tests ---

    #[test]
    fn build_time_range_period_only() {
        let result = build_time_range(Some("1y"), None, None).unwrap();
        assert_eq!(result, TimeRange::Period(Period::Year1));
    }

    #[test]
    fn build_time_range_date_range() {
        let result = build_time_range(None, Some("2024-01-01"), Some("2024-06-30")).unwrap();
        let from = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2024, 6, 30).unwrap();
        assert_eq!(result, TimeRange::DateRange { from, to });
    }

    #[test]
    fn build_time_range_defaults_to_1y() {
        let result = build_time_range(None, None, None).unwrap();
        assert_eq!(result, TimeRange::Period(Period::Year1));
    }

    #[test]
    fn build_time_range_mutual_exclusion() {
        let err = build_time_range(Some("1y"), Some("2024-01-01"), None).unwrap_err();
        assert!(err.contains("mutually exclusive"));
    }

    #[test]
    fn build_time_range_from_without_to() {
        let err = build_time_range(None, Some("2024-01-01"), None).unwrap_err();
        assert!(err.contains("--from requires --to"));
    }

    #[test]
    fn build_time_range_to_without_from() {
        let err = build_time_range(None, None, Some("2024-06-30")).unwrap_err();
        assert!(err.contains("--to requires --from"));
    }

    #[test]
    fn build_time_range_from_after_to() {
        let err =
            build_time_range(None, Some("2024-06-30"), Some("2024-01-01")).unwrap_err();
        assert!(err.contains("--from date must be before --to date"));
    }

    #[test]
    fn build_time_range_from_equals_to() {
        let err =
            build_time_range(None, Some("2024-01-01"), Some("2024-01-01")).unwrap_err();
        assert!(err.contains("--from date must be before --to date"));
    }

    #[test]
    fn build_time_range_invalid_period() {
        let err = build_time_range(Some("invalid"), None, None).unwrap_err();
        assert!(err.contains("invalid period"));
    }

    #[test]
    fn build_time_range_invalid_from_date() {
        let err = build_time_range(None, Some("bad-date"), Some("2024-06-30")).unwrap_err();
        assert!(err.contains("invalid date 'bad-date'"));
    }

    #[test]
    fn build_time_range_invalid_to_date() {
        let err = build_time_range(None, Some("2024-01-01"), Some("bad-date")).unwrap_err();
        assert!(err.contains("invalid date 'bad-date'"));
    }

    // --- run_fetch validation tests (no network needed) ---

    #[test]
    fn run_fetch_empty_symbols() {
        let result = run_fetch("", "yahoo", None, "1d", None, None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no symbols provided"));
    }

    #[test]
    fn run_fetch_whitespace_only_symbols() {
        let result = run_fetch("  ,  , ", "yahoo", None, "1d", None, None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no symbols provided"));
    }

    #[test]
    fn run_fetch_invalid_interval() {
        let result = run_fetch("AAPL", "yahoo", None, "2h", None, None, None);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("invalid interval '2h'"));
    }

    #[test]
    fn run_fetch_mutual_exclusion() {
        let result = run_fetch(
            "AAPL",
            "yahoo",
            Some("1y"),
            "1d",
            Some("2024-01-01"),
            None,
            None,
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("mutually exclusive"));
    }

    #[test]
    fn run_fetch_unknown_source() {
        let result = run_fetch("AAPL", "unknown_provider", None, "1d", None, None, None);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("unknown provider 'unknown_provider'"));
        assert!(err.contains("yahoo"));
    }

    #[test]
    fn run_fetch_invalid_date_format() {
        let result = run_fetch(
            "AAPL",
            "yahoo",
            None,
            "1d",
            Some("01/15/2024"),
            Some("2024-06-30"),
            None,
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("invalid date '01/15/2024'"));
    }

    #[test]
    fn run_fetch_from_after_to() {
        let result = run_fetch(
            "AAPL",
            "yahoo",
            None,
            "1d",
            Some("2024-06-30"),
            Some("2024-01-01"),
            None,
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("--from date must be before --to date"));
    }
}
