use std::io::Write;

use chrono::NaiveDateTime;

use super::types::Interval;
use super::OhlcvRecord;

/// Serialize OHLCV records to the standard Flux CSV format.
///
/// Records must already be sorted and grouped by the caller.
/// Writes the header line followed by one line per record.
pub fn write_csv<W: Write>(
    writer: &mut W,
    records: &[OhlcvRecord],
    interval: Interval,
) -> std::io::Result<()> {
    writeln!(writer, "timestamp,symbol,open,high,low,close,volume")?;
    for record in records {
        let ts = format_timestamp(&record.timestamp, interval);
        writeln!(
            writer,
            "{},{},{},{},{},{},{}",
            ts, record.symbol, record.open, record.high, record.low, record.close, record.volume
        )?;
    }
    Ok(())
}

/// Format timestamp based on interval granularity.
///
/// - Daily, weekly, and monthly intervals use `YYYY-MM-DD`
/// - Intraday intervals (1m, 5m, 15m, 1h) use `YYYY-MM-DDTHH:MM:SS`
pub fn format_timestamp(dt: &NaiveDateTime, interval: Interval) -> String {
    if interval.is_intraday() {
        dt.format("%Y-%m-%dT%H:%M:%S").to_string()
    } else {
        dt.format("%Y-%m-%d").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn make_record(
        date: NaiveDate,
        hour: u32,
        min: u32,
        symbol: &str,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        volume: f64,
    ) -> OhlcvRecord {
        OhlcvRecord {
            timestamp: date.and_hms_opt(hour, min, 0).unwrap(),
            symbol: symbol.to_string(),
            open,
            high,
            low,
            close,
            volume,
        }
    }

    #[test]
    fn format_timestamp_daily_produces_date_only() {
        let dt = NaiveDate::from_ymd_opt(2024, 3, 15)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        assert_eq!(format_timestamp(&dt, Interval::Day1), "2024-03-15");
    }

    #[test]
    fn format_timestamp_weekly_produces_date_only() {
        let dt = NaiveDate::from_ymd_opt(2024, 7, 22)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        assert_eq!(format_timestamp(&dt, Interval::Week1), "2024-07-22");
    }

    #[test]
    fn format_timestamp_monthly_produces_date_only() {
        let dt = NaiveDate::from_ymd_opt(2024, 12, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        assert_eq!(format_timestamp(&dt, Interval::Month1), "2024-12-01");
    }

    #[test]
    fn format_timestamp_intraday_1m_produces_datetime() {
        let dt = NaiveDate::from_ymd_opt(2024, 3, 15)
            .unwrap()
            .and_hms_opt(14, 30, 45)
            .unwrap();
        assert_eq!(
            format_timestamp(&dt, Interval::Min1),
            "2024-03-15T14:30:45"
        );
    }

    #[test]
    fn format_timestamp_intraday_5m_produces_datetime() {
        let dt = NaiveDate::from_ymd_opt(2024, 1, 2)
            .unwrap()
            .and_hms_opt(9, 35, 0)
            .unwrap();
        assert_eq!(
            format_timestamp(&dt, Interval::Min5),
            "2024-01-02T09:35:00"
        );
    }

    #[test]
    fn format_timestamp_intraday_15m_produces_datetime() {
        let dt = NaiveDate::from_ymd_opt(2024, 6, 10)
            .unwrap()
            .and_hms_opt(16, 0, 0)
            .unwrap();
        assert_eq!(
            format_timestamp(&dt, Interval::Min15),
            "2024-06-10T16:00:00"
        );
    }

    #[test]
    fn format_timestamp_intraday_1h_produces_datetime() {
        let dt = NaiveDate::from_ymd_opt(2024, 11, 5)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap();
        assert_eq!(
            format_timestamp(&dt, Interval::Hour1),
            "2024-11-05T10:00:00"
        );
    }

    #[test]
    fn write_csv_header_is_correct() {
        let mut buf = Vec::new();
        write_csv(&mut buf, &[], Interval::Day1).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert_eq!(output, "timestamp,symbol,open,high,low,close,volume\n");
    }

    #[test]
    fn write_csv_daily_records() {
        let records = vec![
            make_record(
                NaiveDate::from_ymd_opt(2024, 1, 2).unwrap(),
                0,
                0,
                "AAPL",
                185.5,
                186.75,
                185.1,
                186.2,
                1200000.0,
            ),
            make_record(
                NaiveDate::from_ymd_opt(2024, 1, 3).unwrap(),
                0,
                0,
                "AAPL",
                186.0,
                187.5,
                185.8,
                187.1,
                1100000.0,
            ),
        ];
        let mut buf = Vec::new();
        write_csv(&mut buf, &records, Interval::Day1).unwrap();
        let output = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines[0], "timestamp,symbol,open,high,low,close,volume");
        assert_eq!(lines[1], "2024-01-02,AAPL,185.5,186.75,185.1,186.2,1200000");
        assert_eq!(lines[2], "2024-01-03,AAPL,186,187.5,185.8,187.1,1100000");
    }

    #[test]
    fn write_csv_intraday_records() {
        let records = vec![make_record(
            NaiveDate::from_ymd_opt(2024, 3, 15).unwrap(),
            14,
            30,
            "MSFT",
            420.25,
            421.0,
            419.5,
            420.75,
            50000.0,
        )];
        let mut buf = Vec::new();
        write_csv(&mut buf, &records, Interval::Min5).unwrap();
        let output = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(
            lines[1],
            "2024-03-15T14:30:00,MSFT,420.25,421,419.5,420.75,50000"
        );
    }

    #[test]
    fn write_csv_preserves_full_decimal_precision() {
        let records = vec![make_record(
            NaiveDate::from_ymd_opt(2024, 1, 2).unwrap(),
            0,
            0,
            "TEST",
            123.456789012345,
            234.567890123456,
            100.111111111111,
            200.222222222222,
            9999999.99,
        )];
        let mut buf = Vec::new();
        write_csv(&mut buf, &records, Interval::Day1).unwrap();
        let output = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = output.lines().collect();
        let fields: Vec<&str> = lines[1].split(',').collect();
        // Verify numeric values are preserved with full precision (no rounding)
        assert_eq!(fields[2], "123.456789012345");
        assert_eq!(fields[3], "234.567890123456");
        assert_eq!(fields[4], "100.111111111111");
        assert_eq!(fields[5], "200.222222222222");
        assert_eq!(fields[6], "9999999.99");
    }
}
