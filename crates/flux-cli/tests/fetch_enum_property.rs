// Feature: flux-data-fetcher, Property 7: Invalid enum inputs produce helpful error messages
//!
//! **Validates: Requirements 6.2, 6.3**
//!
//! For any string not in the set of valid Period values, parsing it as a Period
//! returns an error whose message contains all valid period option strings.
//! The same property holds for Interval parsing.

use proptest::prelude::*;
use flux_cli::data::types::{Period, Interval};
use std::str::FromStr;

// =============================================================================
// Property 7: Invalid enum inputs produce helpful error messages
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// **Validates: Requirements 6.2**
    ///
    /// For any string that is not a valid Period value, parsing it as a Period
    /// returns an error whose message contains every valid period option.
    #[test]
    fn invalid_period_lists_all_options(s in "[a-z0-9]{1,10}") {
        let valid = Period::all_values();
        if !valid.contains(&s.as_str()) {
            let err = Period::from_str(&s).unwrap_err();
            for &v in valid {
                prop_assert!(err.contains(v), "error missing valid option '{}' in: {}", v, err);
            }
        }
    }

    /// **Validates: Requirements 6.3**
    ///
    /// For any string that is not a valid Interval value, parsing it as an Interval
    /// returns an error whose message contains every valid interval option.
    #[test]
    fn invalid_interval_lists_all_options(s in "[a-z0-9]{1,10}") {
        let valid = Interval::all_values();
        if !valid.contains(&s.as_str()) {
            let err = Interval::from_str(&s).unwrap_err();
            for &v in valid {
                prop_assert!(err.contains(v), "error missing valid option '{}' in: {}", v, err);
            }
        }
    }
}
