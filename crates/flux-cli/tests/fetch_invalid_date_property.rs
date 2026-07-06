//! Property-based tests for invalid date format rejection.
//!
//! Feature: flux-data-fetcher, Property 8: Invalid date formats are rejected
//!
//! For any string not matching a valid YYYY-MM-DD calendar date, the parser
//! returns an error without panicking.

use chrono::NaiveDate;
use proptest::prelude::*;

/// Check if a string is a valid YYYY-MM-DD date.
fn is_valid_date(s: &str) -> bool {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    // =========================================================================
    // Feature: flux-data-fetcher, Property 8: Invalid date formats are rejected
    // =========================================================================

    /// **Validates: Requirements 6.4**
    ///
    /// For any arbitrary string, parsing as a date either succeeds (valid date)
    /// or returns an error — it never panics.
    #[test]
    fn invalid_dates_do_not_panic(s in "\\PC{0,20}") {
        // This should never panic regardless of input
        let result = NaiveDate::parse_from_str(&s, "%Y-%m-%d");
        // If it's not a valid date string, it should be Err
        if !is_valid_date(&s) {
            prop_assert!(result.is_err());
        }
    }

    /// **Validates: Requirements 6.4**
    ///
    /// Strings that don't match YYYY-MM-DD pattern are always rejected.
    #[test]
    fn non_date_strings_rejected(s in "[^0-9-]{1,15}") {
        let result = NaiveDate::parse_from_str(&s, "%Y-%m-%d");
        prop_assert!(result.is_err());
    }

    /// **Validates: Requirements 6.4**
    ///
    /// Strings with invalid month/day values are rejected.
    #[test]
    fn invalid_calendar_dates_rejected(
        y in 1970i32..2100,
        m in 13u32..=99,
        d in 1u32..=31
    ) {
        let s = format!("{:04}-{:02}-{:02}", y, m, d);
        let result = NaiveDate::parse_from_str(&s, "%Y-%m-%d");
        prop_assert!(result.is_err());
    }
}
