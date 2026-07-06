// Feature: flux-data-fetcher, Property 2: Date string parsing round-trip
//!
//! For any valid date (year 1970–2100), formatting as YYYY-MM-DD then parsing
//! produces the original date.

use chrono::NaiveDate;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// **Validates: Requirements 3.5**
    ///
    /// For any valid date (year 1970–2100, month 1–12, day 1–28),
    /// formatting as YYYY-MM-DD and then parsing with NaiveDate::parse_from_str
    /// produces the original date value.
    #[test]
    fn date_roundtrip(y in 1970i32..=2100, m in 1u32..=12, d in 1u32..=28) {
        // Use day ≤28 to avoid invalid dates
        let date = NaiveDate::from_ymd_opt(y, m, d).unwrap();
        let formatted = date.format("%Y-%m-%d").to_string();
        let parsed = NaiveDate::parse_from_str(&formatted, "%Y-%m-%d").unwrap();
        prop_assert_eq!(date, parsed);
    }
}
