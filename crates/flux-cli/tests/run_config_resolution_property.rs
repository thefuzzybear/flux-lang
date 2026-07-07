// Feature: flux-run-harness, Property 5: Configuration resolution priority (CLI > block > default)
//!
//! **Validates: Requirements 3.7, 4.1, 4.2, 4.4, 4.5**
//!
//! For any combination of data block field values and CLI override values,
//! calling `resolve_data_config` SHALL produce a result where:
//! (a) if a CLI override is provided for a field, the resolved value equals the CLI value
//! (b) if no CLI override but a data block value exists, the resolved value equals the block value
//! (c) if neither is provided, the resolved value equals the default (period="1y", interval="1d", source="yahoo")

use flux_cli::commands::run::resolve_data_config;
use flux_compiler::lexer::Span;
use flux_compiler::typeck::TypedDataBlock;
use proptest::prelude::*;

// =============================================================================
// Strategies for generating optional field values
// =============================================================================

/// Generate an optional non-empty string for a data block field.
fn opt_field_value() -> impl Strategy<Value = Option<String>> {
    prop_oneof![
        Just(None),
        "[a-zA-Z0-9_]{1,10}".prop_map(Some),
    ]
}

/// Generate an optional non-empty string for a CLI override field.
fn opt_cli_value() -> impl Strategy<Value = Option<String>> {
    prop_oneof![
        Just(None),
        "[a-zA-Z0-9_]{1,10}".prop_map(Some),
    ]
}

/// Generate an optional symbols list for the data block.
/// When present, contains 1-3 non-empty symbol strings.
fn opt_block_symbols() -> impl Strategy<Value = Option<Vec<String>>> {
    prop_oneof![
        Just(None),
        prop::collection::vec("[A-Z]{1,5}", 1..=3).prop_map(Some),
    ]
}

/// Generate an optional CLI symbols string (comma-separated).
/// When present, contains at least one non-empty symbol.
fn opt_cli_symbols() -> impl Strategy<Value = Option<String>> {
    prop_oneof![
        Just(None),
        prop::collection::vec("[A-Z]{1,5}", 1..=3)
            .prop_map(|syms| Some(syms.join(","))),
    ]
}

// =============================================================================
// Property 5: Configuration resolution priority (CLI > block > default)
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// **Validates: Requirements 3.7, 4.1, 4.2, 4.4, 4.5**
    ///
    /// For any combination of data block fields and CLI overrides where at least
    /// one symbols source exists, the resolved config follows priority:
    /// CLI override > data block value > default.
    #[test]
    fn config_resolution_priority(
        block_symbols in opt_block_symbols(),
        block_period in opt_field_value(),
        block_interval in opt_field_value(),
        block_source in opt_field_value(),
        cli_symbols in opt_cli_symbols(),
        cli_period in opt_cli_value(),
        cli_interval in opt_cli_value(),
        cli_source in opt_cli_value(),
    ) {
        // Ensure at least one symbols source exists to avoid the "no symbols" error path
        prop_assume!(cli_symbols.is_some() || block_symbols.is_some());

        let data_block = TypedDataBlock {
            symbols: block_symbols.clone(),
            period: block_period.clone(),
            interval: block_interval.clone(),
            source: block_source.clone(),
            span: Span::new(0, 0),
        };

        let result = resolve_data_config(
            Some(&data_block),
            cli_symbols.as_deref(),
            cli_period.as_deref(),
            cli_interval.as_deref(),
            cli_source.as_deref(),
        );

        prop_assert!(result.is_ok(), "resolve_data_config failed: {:?}", result);
        let config = result.unwrap();

        // (a) Symbols: CLI > block
        if let Some(ref cli_syms) = cli_symbols {
            let expected: Vec<String> = cli_syms
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            prop_assert_eq!(&config.symbols, &expected,
                "CLI symbols override not applied");
        } else if let Some(ref blk_syms) = block_symbols {
            prop_assert_eq!(&config.symbols, blk_syms,
                "Block symbols not used when CLI absent");
        }

        // (b) Period: CLI > block > default "1y"
        if let Some(ref cli_val) = cli_period {
            prop_assert_eq!(&config.period, cli_val,
                "CLI period override not applied");
        } else if let Some(ref blk_val) = block_period {
            prop_assert_eq!(&config.period, blk_val,
                "Block period not used when CLI absent");
        } else {
            prop_assert_eq!(&config.period, "1y",
                "Default period should be '1y'");
        }

        // (c) Interval: CLI > block > default "1d"
        if let Some(ref cli_val) = cli_interval {
            prop_assert_eq!(&config.interval, cli_val,
                "CLI interval override not applied");
        } else if let Some(ref blk_val) = block_interval {
            prop_assert_eq!(&config.interval, blk_val,
                "Block interval not used when CLI absent");
        } else {
            prop_assert_eq!(&config.interval, "1d",
                "Default interval should be '1d'");
        }

        // (d) Source: CLI > block > default "yahoo"
        if let Some(ref cli_val) = cli_source {
            prop_assert_eq!(&config.source, cli_val,
                "CLI source override not applied");
        } else if let Some(ref blk_val) = block_source {
            prop_assert_eq!(&config.source, blk_val,
                "Block source not used when CLI absent");
        } else {
            prop_assert_eq!(&config.source, "yahoo",
                "Default source should be 'yahoo'");
        }
    }

    /// **Validates: Requirements 3.7, 4.1, 4.2, 4.4, 4.5**
    ///
    /// When no data block is provided at all (None), CLI values are used for all fields,
    /// and defaults apply for fields not specified via CLI.
    #[test]
    fn config_resolution_no_block(
        cli_symbols in opt_cli_symbols().prop_filter("need symbols", |s| s.is_some()),
        cli_period in opt_cli_value(),
        cli_interval in opt_cli_value(),
        cli_source in opt_cli_value(),
    ) {
        let result = resolve_data_config(
            None,
            cli_symbols.as_deref(),
            cli_period.as_deref(),
            cli_interval.as_deref(),
            cli_source.as_deref(),
        );

        prop_assert!(result.is_ok(), "resolve_data_config failed: {:?}", result);
        let config = result.unwrap();

        // Symbols from CLI
        let expected_symbols: Vec<String> = cli_symbols.as_ref().unwrap()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        prop_assert_eq!(&config.symbols, &expected_symbols);

        // Period: CLI > default "1y"
        if let Some(ref cli_val) = cli_period {
            prop_assert_eq!(&config.period, cli_val);
        } else {
            prop_assert_eq!(&config.period, "1y");
        }

        // Interval: CLI > default "1d"
        if let Some(ref cli_val) = cli_interval {
            prop_assert_eq!(&config.interval, cli_val);
        } else {
            prop_assert_eq!(&config.interval, "1d");
        }

        // Source: CLI > default "yahoo"
        if let Some(ref cli_val) = cli_source {
            prop_assert_eq!(&config.source, cli_val);
        } else {
            prop_assert_eq!(&config.source, "yahoo");
        }
    }
}
