//! Integration test: Backward compatibility with both `data` and `connector` blocks.
//!
//! Verifies that a `.flux` file containing both `data {}` and `connector {}` blocks:
//! 1. Compiles successfully through lex → parse → typecheck (no errors)
//! 2. When loaded via `load_strategies()` for live mode, extracts symbols from the `connector` block
//! 3. When loaded via the backtest command path, uses the `data` block
//!
//! This ensures backward compatibility: the same strategy file can be used for both
//! `flux backtest` (using data block) and `flux live` (using connector block).
//!
//! **Validates: Requirements 8.7, 8.8**

use std::io::Write;
use std::path::PathBuf;

use tempfile::NamedTempFile;

use flux_cli::live::loader::load_strategies;

/// Create a temporary `.flux` file with the given source content.
fn write_temp_flux(source: &str) -> (NamedTempFile, PathBuf) {
    let mut file = tempfile::Builder::new()
        .suffix(".flux")
        .tempfile()
        .expect("failed to create temp file");
    file.write_all(source.as_bytes())
        .expect("failed to write temp file");
    let path = file.path().to_path_buf();
    (file, path)
}

/// Strategy source with both `data` and `connector` blocks.
/// The data block has symbols = ["SPY"] (for backtest).
/// The connector block has symbols = ["AAPL", "MSFT"] (for live).
const DUAL_BLOCK_STRATEGY: &str = r#"
data {
    symbols = ["SPY"]
    period = "6mo"
    interval = "1d"
    source = "yahoo"
}

connector {
    type = "websocket"
    url = "wss://stream.example.com/v1"
    symbols = ["AAPL", "MSFT"]
    interval = "1m"
}

strategy DualMode {
    on bar {
        if close > open and not in_position {
            OPEN(symbol, 100.0)
        }
        if close < open and in_position {
            CLOSE(symbol)
        }
    }
}
"#;

/// Validates: Requirements 8.7, 8.8
///
/// A strategy with both `data` and `connector` blocks compiles successfully
/// through the full pipeline (lex → parse → typecheck).
#[test]
fn dual_block_strategy_compiles_successfully() {
    let source = DUAL_BLOCK_STRATEGY;

    // Lex
    let tokens = flux_compiler::lexer::lex_with_spans(source).expect("lex failed");

    // Parse
    let ast = flux_compiler::parser::parse(tokens).expect("parse failed");

    // Verify both blocks are present in the AST
    assert!(
        ast.data_block.is_some(),
        "expected data_block to be present in AST"
    );
    assert!(
        ast.connector_block.is_some(),
        "expected connector_block to be present in AST"
    );

    // Typecheck
    let typed_program = flux_compiler::typeck::check(ast).expect("typecheck failed");

    // Verify both typed blocks are present
    assert!(
        typed_program.data_block.is_some(),
        "expected typed data_block to be present"
    );
    assert!(
        typed_program.connector_block.is_some(),
        "expected typed connector_block to be present"
    );

    // Verify strategy name
    assert_eq!(typed_program.strategy.name, "DualMode");
}

/// Validates: Requirements 8.7, 8.8
///
/// When loaded via `load_strategies()` (the live mode path), a strategy with
/// both `data` and `connector` blocks extracts symbols from the `connector`
/// block, NOT the `data` block.
#[test]
fn live_mode_uses_connector_block_symbols() {
    let (_file, path) = write_temp_flux(DUAL_BLOCK_STRATEGY);

    let result = load_strategies(&path);
    assert!(result.is_ok(), "load_strategies failed: {:?}", result.err());

    let modules = result.unwrap();
    assert_eq!(modules.len(), 1, "expected exactly one strategy module");

    let module = &modules[0];
    assert_eq!(module.name, "DualMode");

    // The live loader should prefer connector block symbols over data block symbols
    assert_eq!(
        module.subscribed_symbols,
        vec!["AAPL".to_string(), "MSFT".to_string()],
        "live mode should use connector block symbols ['AAPL', 'MSFT'], not data block symbols ['SPY']"
    );

    // Specifically verify it did NOT use the data block symbols
    assert!(
        !module.subscribed_symbols.contains(&"SPY".to_string()),
        "live mode must NOT use data block symbols"
    );
}

/// Validates: Requirements 8.7, 8.8
///
/// When a strategy has ONLY a `data` block (no connector block), the live
/// loader falls back to data block symbols. This tests the fallback behavior.
#[test]
fn live_mode_falls_back_to_data_block_when_no_connector() {
    let source = r#"
data {
    symbols = ["SPY", "QQQ"]
    period = "6mo"
    interval = "1d"
    source = "yahoo"
}

strategy DataOnly {
    on bar {
        if close > open and not in_position {
            OPEN(symbol, 100.0)
        }
    }
}
"#;
    let (_file, path) = write_temp_flux(source);

    let result = load_strategies(&path);
    assert!(result.is_ok(), "load_strategies failed: {:?}", result.err());

    let modules = result.unwrap();
    assert_eq!(modules.len(), 1);
    assert_eq!(modules[0].name, "DataOnly");

    // Falls back to data block symbols when no connector block exists
    assert_eq!(
        modules[0].subscribed_symbols,
        vec!["SPY".to_string(), "QQQ".to_string()],
        "should fall back to data block symbols when no connector block"
    );
}

/// Validates: Requirements 8.7, 8.8
///
/// Verify that the backtest path (lex → parse → typecheck → interpreter) works
/// correctly with a dual-block strategy. The backtest path creates an Interpreter
/// that runs the strategy logic — it should compile and run without errors
/// regardless of the connector block presence.
#[test]
fn backtest_path_works_with_dual_block_strategy() {
    use flux_cli::interpreter::Interpreter;
    use flux_runtime::BarContext;

    let source = DUAL_BLOCK_STRATEGY;

    // Compile through the full pipeline (same as `flux backtest` does)
    let tokens = flux_compiler::lexer::lex_with_spans(source).expect("lex failed");
    let ast = flux_compiler::parser::parse(tokens).expect("parse failed");
    let typed_program = flux_compiler::typeck::check(ast).expect("typecheck failed");

    // Create an interpreter (same as backtest mode)
    let mut interpreter = Interpreter::new(&typed_program);

    // Simulate a bar (as backtest would do with data from the data block's source)
    let bar = BarContext {
        close: 155.0,
        open: 150.0,
        high: 156.0,
        low: 149.0,
        volume: 1_000_000.0,
        symbol: "SPY".to_string(),
        in_position: false,
    };

    // The interpreter should produce signals without errors
    let signals = interpreter.on_bar(&bar);

    // With close > open and not in_position, we expect an OPEN signal
    assert_eq!(signals.len(), 1, "expected one signal from the strategy");
}

/// Validates: Requirements 8.7, 8.8
///
/// Connector block with different symbols than data block: verify the loader
/// correctly differentiates which symbols come from which block when loading
/// for live mode.
#[test]
fn connector_block_symbols_take_priority_over_data_block() {
    let source = r#"
data {
    symbols = ["SPY", "QQQ", "IWM"]
    period = "1y"
    interval = "1d"
    source = "yahoo"
}

connector {
    type = "poll"
    url = "https://api.example.com/bars"
    symbols = ["AAPL", "MSFT", "GOOGL", "AMZN"]
    interval = "5m"
}

strategy MultiAsset {
    on bar {
        if close > open and not in_position {
            OPEN(symbol, 50.0)
        }
        if close < open and in_position {
            CLOSE(symbol)
        }
    }
}
"#;
    let (_file, path) = write_temp_flux(source);

    let result = load_strategies(&path);
    assert!(result.is_ok(), "load_strategies failed: {:?}", result.err());

    let modules = result.unwrap();
    assert_eq!(modules.len(), 1);
    assert_eq!(modules[0].name, "MultiAsset");

    // Connector block symbols should be used for live mode
    let expected_symbols: Vec<String> = vec![
        "AAPL".to_string(),
        "MSFT".to_string(),
        "GOOGL".to_string(),
        "AMZN".to_string(),
    ];
    assert_eq!(
        modules[0].subscribed_symbols, expected_symbols,
        "live mode should use connector block symbols, got: {:?}",
        modules[0].subscribed_symbols
    );

    // None of the data block symbols should appear
    for data_sym in &["SPY", "QQQ", "IWM"] {
        assert!(
            !modules[0].subscribed_symbols.contains(&data_sym.to_string()),
            "live mode must NOT include data block symbol '{}'",
            data_sym
        );
    }
}

/// Validates: Requirements 8.7, 8.8
///
/// Verify that the data block contents are still accessible after compilation
/// for the backtest path, even when a connector block is present. The typed
/// program should preserve both blocks independently.
#[test]
fn typed_program_preserves_both_blocks() {
    let source = DUAL_BLOCK_STRATEGY;

    let tokens = flux_compiler::lexer::lex_with_spans(source).expect("lex failed");
    let ast = flux_compiler::parser::parse(tokens).expect("parse failed");
    let typed_program = flux_compiler::typeck::check(ast).expect("typecheck failed");

    // Data block should have SPY
    let data_block = typed_program.data_block.as_ref().expect("data_block missing");
    let data_symbols = data_block.symbols.as_ref().expect("data_block symbols missing");
    assert_eq!(data_symbols, &vec!["SPY".to_string()]);

    // Connector block should have AAPL, MSFT
    let connector_block = typed_program
        .connector_block
        .as_ref()
        .expect("connector_block missing");
    let connector_symbols = connector_block
        .symbols
        .as_ref()
        .expect("connector_block symbols missing");
    assert_eq!(
        connector_symbols,
        &vec!["AAPL".to_string(), "MSFT".to_string()]
    );

    // They should be different
    assert_ne!(
        data_symbols, connector_symbols,
        "data and connector blocks should have different symbols"
    );
}
