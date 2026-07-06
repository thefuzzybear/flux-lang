// Feature: flux-data-fetcher, Property 6: HTTP error messages contain status code
// **Validates: Requirements 5.2**

use proptest::prelude::*;

use flux_cli::data::error::FetchError;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// For any HTTP status code in 400–599, formatting FetchError::HttpError
    /// produces a string that contains the numeric status code as a substring.
    #[test]
    fn http_error_contains_status_code(status in 400u16..=599u16, message in ".*") {
        let error = FetchError::HttpError { status, message };
        let formatted = format!("{}", error);
        prop_assert!(
            formatted.contains(&status.to_string()),
            "Formatted error {:?} does not contain status code {}",
            formatted,
            status
        );
    }
}
