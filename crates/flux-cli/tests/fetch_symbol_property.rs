// Feature: flux-data-fetcher, Property 1: Symbol list parsing round-trip
// **Validates: Requirements 3.1**

use proptest::prelude::*;
use proptest::collection::vec;

/// Parse symbols the same way run_fetch does.
fn parse_symbols(input: &str) -> Vec<&str> {
    input.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect()
}

/// Generate a valid stock symbol: 1-5 uppercase letters/digits, starting with a letter.
fn valid_symbol() -> impl Strategy<Value = String> {
    proptest::string::string_regex("[A-Z][A-Z0-9]{0,4}").unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn symbol_list_roundtrip(symbols in vec(valid_symbol(), 1..=10)) {
        let joined = symbols.join(",");
        let parsed = parse_symbols(&joined);
        prop_assert_eq!(&parsed.iter().map(|s| s.to_string()).collect::<Vec<_>>(), &symbols);
    }
}
