// Feature: flux-data-fetcher, Property 5: Multi-symbol output ordering and grouping
//
// **Validates: Requirements 4.4, 4.5, 7.3**

use proptest::prelude::*;
use proptest::collection::vec;
use flux_cli::data::{merge_records, OhlcvRecord};
use chrono::NaiveDate;
use std::collections::HashSet;

fn arb_record() -> impl Strategy<Value = OhlcvRecord> {
    (
        (2020i32..2025, 1u32..=12, 1u32..=28),
        "[A-Z]{1,5}",
        1.0f64..1000.0,
        1.0f64..1000.0,
        1.0f64..1000.0,
        1.0f64..1000.0,
        1000.0f64..1000000.0,
    )
        .prop_map(|((y, m, d), sym, o, h, l, c, v)| {
            OhlcvRecord {
                timestamp: NaiveDate::from_ymd_opt(y, m, d)
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap(),
                symbol: sym,
                open: o,
                high: h,
                low: l,
                close: c,
                volume: v,
            }
        })
}

proptest! {
    #[test]
    fn merged_records_are_in_nondecreasing_timestamp_order(records in vec(arb_record(), 0..50)) {
        let merged = merge_records(records);
        for window in merged.windows(2) {
            prop_assert!(window[0].timestamp <= window[1].timestamp);
        }
    }

    #[test]
    fn same_timestamp_records_are_contiguous(records in vec(arb_record(), 0..50)) {
        let merged = merge_records(records);
        // Check that all occurrences of each timestamp form a contiguous block
        let mut seen_timestamps: HashSet<chrono::NaiveDateTime> = HashSet::new();
        let mut prev_ts: Option<chrono::NaiveDateTime> = None;
        for record in &merged {
            if Some(record.timestamp) != prev_ts {
                // New timestamp — must not have seen it before
                prop_assert!(
                    !seen_timestamps.contains(&record.timestamp),
                    "timestamp {:?} appeared non-contiguously",
                    record.timestamp
                );
                if let Some(pt) = prev_ts {
                    seen_timestamps.insert(pt);
                }
            }
            prev_ts = Some(record.timestamp);
        }
    }
}
