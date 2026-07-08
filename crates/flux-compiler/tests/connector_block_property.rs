//! Property-based tests for connector block parse round-trip.
//!
//! Feature: flux-live-harness, Property 10: Connector block parse round-trip
//!
//! **Validates: Requirements 8.2, 8.3, 8.4, 8.5**
//!
//! For any valid combination of connector block fields (type from
//! {"websocket", "poll", "replay"}, url as a simple string, symbols as a list
//! of 1-5 uppercase strings, interval from {"1m", "5m", "15m", "1h", "1d"}),
//! constructing the source text, parsing it through lex → parse, and reading
//! back field values SHALL produce the original values exactly.

use flux_compiler::lexer::lex_with_spans;
use flux_compiler::parser::{parse, ConnectorBlock};
use proptest::prelude::*;

// ============================================================================
// Generators
// ============================================================================

/// Generate a random valid connector type.
fn arb_connector_type() -> impl Strategy<Value = String> {
    prop_oneof![Just("websocket"), Just("poll"), Just("replay"),]
        .prop_map(|s| s.to_string())
}

/// Generate a random simple URL (no special chars that would break string parsing).
fn arb_url() -> impl Strategy<Value = String> {
    "[a-z]{3,8}://[a-z]{3,10}\\.[a-z]{2,4}/[a-z0-9]{1,8}"
}

/// Generate a random symbol (1-6 uppercase letters).
fn arb_symbol() -> impl Strategy<Value = String> {
    "[A-Z]{1,6}"
}

/// Generate a random list of 1-5 symbols.
fn arb_symbols() -> impl Strategy<Value = Vec<String>> {
    proptest::collection::vec(arb_symbol(), 1..=5)
}

/// Generate a random valid interval.
fn arb_interval() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("1m"),
        Just("5m"),
        Just("15m"),
        Just("1h"),
        Just("1d"),
    ]
    .prop_map(|s| s.to_string())
}

/// Generate a random simple file path (no special chars).
fn arb_file_path() -> impl Strategy<Value = String> {
    "[a-z_]{1,8}/[a-z_]{1,8}\\.[a-z]{2,4}"
}

// ============================================================================
// Source construction helpers
// ============================================================================

/// Build a symbols list literal: `["SYM1", "SYM2"]`
fn format_symbols_list(symbols: &[String]) -> String {
    let items: Vec<String> = symbols.iter().map(|s| format!("\"{}\"", s)).collect();
    format!("[{}]", items.join(", "))
}

/// Build a complete valid .flux source with a connector block and minimal strategy.
fn build_source(
    connector_type: Option<&str>,
    url: Option<&str>,
    symbols: Option<&Vec<String>>,
    interval: Option<&str>,
    file: Option<&str>,
) -> String {
    let mut connector_body = String::new();

    if let Some(ct) = connector_type {
        connector_body.push_str(&format!("    type = \"{}\"\n", ct));
    }
    if let Some(u) = url {
        connector_body.push_str(&format!("    url = \"{}\"\n", u));
    }
    if let Some(syms) = symbols {
        connector_body.push_str(&format!("    symbols = {}\n", format_symbols_list(syms)));
    }
    if let Some(i) = interval {
        connector_body.push_str(&format!("    interval = \"{}\"\n", i));
    }
    if let Some(f) = file {
        connector_body.push_str(&format!("    file = \"{}\"\n", f));
    }

    format!(
        "connector {{\n{}}}\n\nstrategy Test {{\n    on bar {{\n    }}\n}}\n",
        connector_body
    )
}

// ============================================================================
// Assertion helpers
// ============================================================================

fn assert_connector_block_fields(
    block: &ConnectorBlock,
    expected_type: Option<&str>,
    expected_url: Option<&str>,
    expected_symbols: Option<&Vec<String>>,
    expected_interval: Option<&str>,
    expected_file: Option<&str>,
) {
    // Check connector_type
    match (expected_type, &block.connector_type) {
        (Some(expected), Some(field)) => {
            assert_eq!(
                field.value, expected,
                "Type mismatch: expected {:?}, got {:?}",
                expected, field.value
            );
        }
        (None, None) => {}
        (Some(expected), None) => {
            panic!("Expected type {:?} but connector block has None", expected);
        }
        (None, Some(field)) => {
            panic!("Expected no type but connector block has {:?}", field.value);
        }
    }

    // Check url
    match (expected_url, &block.url) {
        (Some(expected), Some(field)) => {
            assert_eq!(
                field.value, expected,
                "URL mismatch: expected {:?}, got {:?}",
                expected, field.value
            );
        }
        (None, None) => {}
        (Some(expected), None) => {
            panic!("Expected url {:?} but connector block has None", expected);
        }
        (None, Some(field)) => {
            panic!("Expected no url but connector block has {:?}", field.value);
        }
    }

    // Check symbols
    match (expected_symbols, &block.symbols) {
        (Some(expected), Some(field)) => {
            assert_eq!(
                &field.value, expected,
                "Symbols mismatch: expected {:?}, got {:?}",
                expected, field.value
            );
        }
        (None, None) => {}
        (Some(expected), None) => {
            panic!(
                "Expected symbols {:?} but connector block has None",
                expected
            );
        }
        (None, Some(field)) => {
            panic!(
                "Expected no symbols but connector block has {:?}",
                field.value
            );
        }
    }

    // Check interval
    match (expected_interval, &block.interval) {
        (Some(expected), Some(field)) => {
            assert_eq!(
                field.value, expected,
                "Interval mismatch: expected {:?}, got {:?}",
                expected, field.value
            );
        }
        (None, None) => {}
        (Some(expected), None) => {
            panic!(
                "Expected interval {:?} but connector block has None",
                expected
            );
        }
        (None, Some(field)) => {
            panic!(
                "Expected no interval but connector block has {:?}",
                field.value
            );
        }
    }

    // Check file
    match (expected_file, &block.file) {
        (Some(expected), Some(field)) => {
            assert_eq!(
                field.value, expected,
                "File mismatch: expected {:?}, got {:?}",
                expected, field.value
            );
        }
        (None, None) => {}
        (Some(expected), None) => {
            panic!("Expected file {:?} but connector block has None", expected);
        }
        (None, Some(field)) => {
            panic!("Expected no file but connector block has {:?}", field.value);
        }
    }
}

// ============================================================================
// Property Tests
// ============================================================================

// Feature: flux-live-harness, Property 10: Connector block parse round-trip
// **Validates: Requirements 8.2, 8.3, 8.4, 8.5**
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Property: For any valid combination of connector block fields (type, url,
    /// symbols, interval), constructing source text, parsing it, and reading back
    /// field values produces the original values.
    #[test]
    fn prop_connector_block_parse_round_trip_all_fields(
        connector_type in arb_connector_type(),
        url in arb_url(),
        symbols in arb_symbols(),
        interval in arb_interval(),
    ) {
        let src = build_source(
            Some(&connector_type),
            Some(&url),
            Some(&symbols),
            Some(&interval),
            None,
        );

        let tokens = lex_with_spans(&src).unwrap_or_else(|e| {
            panic!("Lexing failed for source:\n{}\nError: {}", src, e)
        });
        let program = parse(tokens).unwrap_or_else(|e| {
            panic!("Parsing failed for source:\n{}\nError: {}", src, e)
        });

        let connector_block = program.connector_block.as_ref().unwrap_or_else(|| {
            panic!("Expected connector_block to be Some after parsing:\n{}", src)
        });

        assert_connector_block_fields(
            connector_block,
            Some(&connector_type),
            Some(&url),
            Some(&symbols),
            Some(&interval),
            None,
        );
    }

    /// Property: Connector block with type and file (replay mode) round-trips correctly.
    #[test]
    fn prop_connector_block_parse_round_trip_with_file(
        connector_type in arb_connector_type(),
        symbols in arb_symbols(),
        interval in arb_interval(),
        file_path in arb_file_path(),
    ) {
        let src = build_source(
            Some(&connector_type),
            None,
            Some(&symbols),
            Some(&interval),
            Some(&file_path),
        );

        let tokens = lex_with_spans(&src).unwrap_or_else(|e| {
            panic!("Lexing failed for source:\n{}\nError: {}", src, e)
        });
        let program = parse(tokens).unwrap_or_else(|e| {
            panic!("Parsing failed for source:\n{}\nError: {}", src, e)
        });

        let connector_block = program.connector_block.as_ref().unwrap_or_else(|| {
            panic!("Expected connector_block to be Some after parsing:\n{}", src)
        });

        assert_connector_block_fields(
            connector_block,
            Some(&connector_type),
            None,
            Some(&symbols),
            Some(&interval),
            Some(&file_path),
        );
    }

    /// Property: Connector block with only type and symbols round-trips correctly
    /// (minimal required fields).
    #[test]
    fn prop_connector_block_parse_round_trip_minimal(
        connector_type in arb_connector_type(),
        symbols in arb_symbols(),
    ) {
        let src = build_source(
            Some(&connector_type),
            None,
            Some(&symbols),
            None,
            None,
        );

        let tokens = lex_with_spans(&src).unwrap_or_else(|e| {
            panic!("Lexing failed for source:\n{}\nError: {}", src, e)
        });
        let program = parse(tokens).unwrap_or_else(|e| {
            panic!("Parsing failed for source:\n{}\nError: {}", src, e)
        });

        let connector_block = program.connector_block.as_ref().unwrap_or_else(|| {
            panic!("Expected connector_block to be Some after parsing:\n{}", src)
        });

        assert_connector_block_fields(
            connector_block,
            Some(&connector_type),
            None,
            Some(&symbols),
            None,
            None,
        );
    }
}
