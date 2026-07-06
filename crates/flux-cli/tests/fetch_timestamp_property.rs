// Feature: flux-data-fetcher, Property 4: Timestamp format correctness per interval type
// **Validates: Requirements 4.2**

use chrono::NaiveDate;
use flux_cli::data::csv_writer::format_timestamp;
use flux_cli::data::types::Interval;
use proptest::prelude::*;
use regex::Regex;

fn arb_datetime() -> impl Strategy<Value = chrono::NaiveDateTime> {
    (2000i32..2100, 1u32..=12, 1u32..=28, 0u32..24, 0u32..60, 0u32..60).prop_map(
        |(y, m, d, h, min, s)| {
            NaiveDate::from_ymd_opt(y, m, d)
                .unwrap()
                .and_hms_opt(h, min, s)
                .unwrap()
        },
    )
}

fn arb_interval() -> impl Strategy<Value = Interval> {
    prop_oneof![
        Just(Interval::Min1),
        Just(Interval::Min5),
        Just(Interval::Min15),
        Just(Interval::Hour1),
        Just(Interval::Day1),
        Just(Interval::Week1),
        Just(Interval::Month1),
    ]
}

proptest! {
    #[test]
    fn timestamp_format_matches_interval_type(dt in arb_datetime(), interval in arb_interval()) {
        let formatted = format_timestamp(&dt, interval);
        if interval.is_intraday() {
            let re = Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}$").unwrap();
            prop_assert!(re.is_match(&formatted), "intraday format mismatch: {}", formatted);
        } else {
            let re = Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap();
            prop_assert!(re.is_match(&formatted), "daily format mismatch: {}", formatted);
        }
    }
}
