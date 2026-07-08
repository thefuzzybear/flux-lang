//! The `flux run` command: compile, fetch data, and backtest in one step.
//!
//! This module provides the data configuration resolution logic,
//! the OhlcvRecord → BarContext conversion bridge, and the orchestrator
//! function that ties compile → fetch → backtest into a single command.

use std::path::Path;

use flux_compiler::typeck::TypedDataBlock;
use flux_runtime::{BarContext, FillSide, PositionTracker, Signal};

use crate::commands::backtest::{format_signal, format_summary, group_bars_by_timestamp};
use crate::data::registry::{build_registry, get_provider};
use crate::data::types::{Interval, Period, TimeRange};
use crate::data::{self, FetchRequest, OhlcvRecord};
use crate::diagnostics;
use crate::error::{CliError, CompileErrorWithSpan};
use crate::interpreter::Interpreter;
use crate::module_resolver;

/// Resolved data configuration after merging DataBlock values with CLI overrides.
///
/// All fields are fully resolved — either from the data block, CLI flags, or defaults.
#[derive(Debug, Clone)]
pub struct DataConfig {
    /// Ticker symbols to fetch (required — must come from block or CLI)
    pub symbols: Vec<String>,
    /// Time period (default: "1y")
    pub period: String,
    /// Bar interval (default: "1d")
    pub interval: String,
    /// Data provider name (default: "yahoo")
    pub source: String,
}

/// Merge a TypedDataBlock with CLI overrides to produce a fully-resolved DataConfig.
///
/// Priority order: CLI override > DataBlock value > default
///
/// Returns an error if no symbols are available from either source.
pub fn resolve_data_config(
    data_block: Option<&TypedDataBlock>,
    cli_symbols: Option<&str>,
    cli_period: Option<&str>,
    cli_interval: Option<&str>,
    cli_source: Option<&str>,
) -> Result<DataConfig, String> {
    // Symbols: CLI override > data block (required from at least one source)
    let symbols = if let Some(cli_syms) = cli_symbols {
        cli_syms
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
    } else if let Some(block) = data_block {
        block.symbols.clone().unwrap_or_default()
    } else {
        Vec::new()
    };

    if symbols.is_empty() {
        return Err(
            "error: no symbols specified. Add a data block with symbols \
             or pass --symbols on the command line"
                .to_string(),
        );
    }

    // Period: CLI > block > default "1y"
    let period = cli_period
        .map(|s| s.to_string())
        .or_else(|| data_block.and_then(|b| b.period.clone()))
        .unwrap_or_else(|| "1y".to_string());

    // Interval: CLI > block > default "1d"
    let interval = cli_interval
        .map(|s| s.to_string())
        .or_else(|| data_block.and_then(|b| b.interval.clone()))
        .unwrap_or_else(|| "1d".to_string());

    // Source: CLI > block > default "yahoo"
    let source = cli_source
        .map(|s| s.to_string())
        .or_else(|| data_block.and_then(|b| b.source.clone()))
        .unwrap_or_else(|| "yahoo".to_string());

    Ok(DataConfig {
        symbols,
        period,
        interval,
        source,
    })
}

/// Convert fetched OHLCV records into BarContext records for the interpreter.
///
/// - `in_position` is initialized to `false` (set dynamically during the backtest loop)
/// - The timestamp is not carried into BarContext (it's used for grouping only)
///
/// Returns both the converted bars and the timestamp strings (for grouping).
pub fn ohlcv_to_bars(records: &[OhlcvRecord]) -> (Vec<BarContext>, Vec<String>) {
    let mut bars = Vec::with_capacity(records.len());
    let mut timestamps = Vec::with_capacity(records.len());

    for record in records {
        bars.push(BarContext {
            close: record.close,
            open: record.open,
            high: record.high,
            low: record.low,
            volume: record.volume,
            symbol: record.symbol.clone(),
            in_position: false,
        });
        timestamps.push(record.timestamp.format("%Y-%m-%d").to_string());
    }

    (bars, timestamps)
}

/// Extract byte offset from a compiler error message string.
///
/// Recognized patterns:
/// - "at byte N: ..." (parser, typeck)
/// - "Lexer error at byte N: ..." (lexer)
///
/// Returns (offset, cleaned_message). If no pattern matches, returns (0, original_message).
fn extract_offset_and_message(error_msg: &str) -> (usize, String) {
    if let Some(pos) = error_msg.find("at byte ") {
        let after_prefix = &error_msg[pos + "at byte ".len()..];
        if let Some(colon_pos) = after_prefix.find(':') {
            let num_str = &after_prefix[..colon_pos];
            if let Ok(offset) = num_str.trim().parse::<usize>() {
                let message = after_prefix[colon_pos + 1..].trim().to_string();
                return (offset, message);
            }
        }
    }
    (0, error_msg.to_string())
}

/// Convert a `CompileError` into a list of `CompileErrorWithSpan`.
fn compile_error_to_spans(error: &flux_compiler::CompileError) -> Vec<CompileErrorWithSpan> {
    let msg = match error {
        flux_compiler::CompileError::Lexer(s) => s.clone(),
        flux_compiler::CompileError::Parser(s) => s.clone(),
        flux_compiler::CompileError::Type(s) => s.clone(),
        _ => error.to_string(),
    };

    msg.lines()
        .map(|line| {
            let (offset, message) = extract_offset_and_message(line);
            CompileErrorWithSpan { offset, message }
        })
        .collect()
}

/// Run the `flux run` command end-to-end: compile, fetch data, and backtest.
///
/// This orchestrator function:
/// 1. Reads and compiles the source file (lex → parse → typecheck)
/// 2. Resolves data configuration from the data block + CLI overrides
/// 3. Fetches market data per-symbol via the data provider registry
/// 4. Converts records to bars and runs the interpreter + position tracker
/// 5. Outputs results in the same format as `flux backtest`
///
/// Compilation errors are displayed before any network calls are made.
pub fn run_run_cmd(
    file: &Path,
    cli_symbols: Option<&str>,
    cli_period: Option<&str>,
    cli_interval: Option<&str>,
    cli_source: Option<&str>,
    initial_capital: f64,
) -> Result<(), CliError> {
    let source = std::fs::read_to_string(file).map_err(CliError::Io)?;
    let file_display = file.display().to_string();

    // --- Phase 1: Compile (all errors shown before any network call) ---
    eprintln!("  Compiling {}...", file_display);

    // Lex
    let tokens = match flux_compiler::lexer::lex_with_spans(&source) {
        Ok(tokens) => tokens,
        Err(err) => {
            let errors = compile_error_to_spans(&err);
            let formatted = diagnostics::format_errors(&file_display, &source, &errors);
            eprintln!("{}", formatted);
            return Err(CliError::Compile(errors));
        }
    };

    // Parse
    let ast = match flux_compiler::parser::parse(tokens) {
        Ok(ast) => ast,
        Err(err) => {
            let errors = compile_error_to_spans(&err);
            let formatted = diagnostics::format_errors(&file_display, &source, &errors);
            eprintln!("{}", formatted);
            return Err(CliError::Compile(errors));
        }
    };

    // Module resolution
    let ast = module_resolver::resolve_modules(ast, file.parent().unwrap_or(Path::new(".")))?;

    // Typecheck
    let typed_program = match flux_compiler::typeck::check(ast) {
        Ok(typed) => typed,
        Err(err) => {
            let errors = compile_error_to_spans(&err);
            let formatted = diagnostics::format_errors(&file_display, &source, &errors);
            eprintln!("{}", formatted);
            return Err(CliError::Compile(errors));
        }
    };

    // --- Phase 2: Resolve data configuration ---
    let data_config = resolve_data_config(
        typed_program.data_block.as_ref(),
        cli_symbols,
        cli_period,
        cli_interval,
        cli_source,
    )
    .map_err(CliError::Usage)?;

    // --- Phase 3: Fetch data ---
    eprintln!(
        "  Fetching {} for [{}] ({})...",
        data_config.period,
        data_config.symbols.join(", "),
        data_config.source
    );

    let interval: Interval = data_config
        .interval
        .parse()
        .map_err(|e: String| CliError::Usage(e))?;
    let period: Period = data_config
        .period
        .parse()
        .map_err(|e: String| CliError::Usage(e))?;
    let time_range = TimeRange::Period(period);

    let registry = build_registry();
    let provider =
        get_provider(&registry, &data_config.source).map_err(|e| CliError::Usage(e))?;

    let mut all_records: Vec<OhlcvRecord> = Vec::new();
    let mut failures: Vec<(String, String)> = Vec::new();

    for sym in &data_config.symbols {
        let request = FetchRequest {
            symbol: sym.clone(),
            time_range: time_range.clone(),
            interval,
        };
        match provider.fetch(&request) {
            Ok(records) => all_records.extend(records),
            Err(e) => {
                eprintln!("  warning: failed to fetch {}: {}", sym, e);
                failures.push((sym.clone(), e.to_string()));
            }
        }
    }

    // Total failure — no data at all
    if all_records.is_empty() {
        return Err(CliError::Runtime(
            "all symbols failed to fetch — no data available".to_string(),
        ));
    }

    // --- Phase 4: Merge, convert, and run backtest ---
    let merged = data::merge_records(all_records);
    let (bars, timestamps) = ohlcv_to_bars(&merged);

    eprintln!("  Fetched {} bars. Running backtest...", bars.len());

    let groups = group_bars_by_timestamp(&bars, &timestamps)
        .map_err(|e| CliError::Runtime(e))?;

    let mut interpreter = Interpreter::new(&typed_program);
    let mut tracker = PositionTracker::new(initial_capital);

    let mut results: Vec<(usize, Signal)> = Vec::new();
    let mut bar_index = 0;

    for group in &groups {
        for bar in &group.bars {
            // Set in_position from tracker state (multi-symbol aware)
            interpreter.in_position = tracker.open_position_count() > 0;

            let signals = interpreter.on_bar(bar);

            // Feed signals through position tracker
            tracker.process_signals(&signals, bar.close, bar_index);

            // Collect raw signal pairs
            for signal in signals {
                results.push((bar_index, signal));
            }

            // Mark all open positions to market at bar close
            tracker.mark_to_market(bar.close, &bar.symbol);
            bar_index += 1;
        }
    }

    // --- Phase 5: Output (same format as flux backtest) ---
    println!("--- Signals ---");
    for (idx, sig) in &results {
        println!("  {}", format_signal(*idx, sig));
    }

    // Print fills
    let fills = tracker.fills();
    if !fills.is_empty() {
        println!("\n--- Fills ---");
        for fill in fills {
            let side_str = match fill.side {
                FillSide::Open => "BUY",
                FillSide::Close => "SELL",
            };
            println!(
                "  Bar {:>4} | {:>4} | {} {:>10.2} @ {:>10.2}",
                fill.bar_index, side_str, fill.symbol, fill.qty, fill.price
            );
        }
    }

    // Print portfolio summary
    let portfolio = tracker.portfolio_state();
    println!("\n--- Portfolio Summary ---");
    println!("  Initial Capital:   {:>12.2}", portfolio.initial_capital);
    println!("  Final Equity:      {:>12.2}", portfolio.equity);
    println!("  Realized P&L:      {:>12.2}", portfolio.realized_pnl);
    println!("  Unrealized P&L:    {:>12.2}", portfolio.unrealized_pnl);
    println!(
        "  Total Return:      {:>11.2}%",
        if portfolio.initial_capital > 0.0 {
            ((portfolio.equity - portfolio.initial_capital) / portfolio.initial_capital) * 100.0
        } else {
            0.0
        }
    );
    println!("  Open Positions:    {:>12}", portfolio.open_position_count);
    println!("  Gross Exposure:    {:>12.2}", portfolio.gross_exposure);
    println!("  Net Exposure:      {:>12.2}", portfolio.net_exposure);
    println!("  Total Fills:       {:>12}", fills.len());

    // Print signal summary
    println!("\n{}", format_summary(&results));

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use flux_compiler::lexer::Span;

    #[test]
    fn resolve_data_config_cli_overrides_block() {
        let block = TypedDataBlock {
            symbols: Some(vec!["AAPL".to_string()]),
            period: Some("6mo".to_string()),
            interval: Some("1h".to_string()),
            source: Some("yahoo".to_string()),
            span: Span::new(0, 0),
        };

        let config = resolve_data_config(
            Some(&block),
            Some("MSFT,GOOG"),
            Some("1y"),
            Some("1d"),
            None,
        )
        .unwrap();

        assert_eq!(config.symbols, vec!["MSFT", "GOOG"]);
        assert_eq!(config.period, "1y");
        assert_eq!(config.interval, "1d");
        assert_eq!(config.source, "yahoo"); // from block, no CLI override
    }

    #[test]
    fn resolve_data_config_block_values_used_when_no_cli() {
        let block = TypedDataBlock {
            symbols: Some(vec!["TSLA".to_string()]),
            period: Some("3mo".to_string()),
            interval: Some("1wk".to_string()),
            source: Some("yahoo".to_string()),
            span: Span::new(0, 0),
        };

        let config = resolve_data_config(Some(&block), None, None, None, None).unwrap();

        assert_eq!(config.symbols, vec!["TSLA"]);
        assert_eq!(config.period, "3mo");
        assert_eq!(config.interval, "1wk");
        assert_eq!(config.source, "yahoo");
    }

    #[test]
    fn resolve_data_config_defaults_when_no_block_no_cli() {
        let block = TypedDataBlock {
            symbols: Some(vec!["SPY".to_string()]),
            period: None,
            interval: None,
            source: None,
            span: Span::new(0, 0),
        };

        let config = resolve_data_config(Some(&block), None, None, None, None).unwrap();

        assert_eq!(config.symbols, vec!["SPY"]);
        assert_eq!(config.period, "1y");
        assert_eq!(config.interval, "1d");
        assert_eq!(config.source, "yahoo");
    }

    #[test]
    fn resolve_data_config_error_when_no_symbols() {
        let result = resolve_data_config(None, None, None, None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no symbols specified"));
    }

    #[test]
    fn resolve_data_config_error_when_block_has_no_symbols_and_no_cli() {
        let block = TypedDataBlock {
            symbols: None,
            period: Some("1y".to_string()),
            interval: None,
            source: None,
            span: Span::new(0, 0),
        };

        let result = resolve_data_config(Some(&block), None, None, None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no symbols specified"));
    }

    #[test]
    fn resolve_data_config_cli_symbols_empty_after_split_is_error() {
        let result = resolve_data_config(None, Some("  , , "), None, None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no symbols specified"));
    }

    #[test]
    fn ohlcv_to_bars_converts_fields_correctly() {
        let timestamp = NaiveDate::from_ymd_opt(2024, 3, 15)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();

        let records = vec![OhlcvRecord {
            timestamp,
            symbol: "AAPL".to_string(),
            open: 170.0,
            high: 175.0,
            low: 169.0,
            close: 173.5,
            volume: 5_000_000.0,
        }];

        let (bars, timestamps) = ohlcv_to_bars(&records);

        assert_eq!(bars.len(), 1);
        assert_eq!(bars[0].open, 170.0);
        assert_eq!(bars[0].high, 175.0);
        assert_eq!(bars[0].low, 169.0);
        assert_eq!(bars[0].close, 173.5);
        assert_eq!(bars[0].volume, 5_000_000.0);
        assert_eq!(bars[0].symbol, "AAPL");
        assert!(!bars[0].in_position);

        assert_eq!(timestamps.len(), 1);
        assert_eq!(timestamps[0], "2024-03-15");
    }

    #[test]
    fn ohlcv_to_bars_empty_input() {
        let (bars, timestamps) = ohlcv_to_bars(&[]);
        assert!(bars.is_empty());
        assert!(timestamps.is_empty());
    }

    #[test]
    fn ohlcv_to_bars_multiple_records() {
        let records = vec![
            OhlcvRecord {
                timestamp: NaiveDate::from_ymd_opt(2024, 1, 1)
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap(),
                symbol: "AAPL".to_string(),
                open: 100.0,
                high: 105.0,
                low: 95.0,
                close: 102.0,
                volume: 1000.0,
            },
            OhlcvRecord {
                timestamp: NaiveDate::from_ymd_opt(2024, 1, 2)
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap(),
                symbol: "MSFT".to_string(),
                open: 200.0,
                high: 210.0,
                low: 195.0,
                close: 205.0,
                volume: 2000.0,
            },
        ];

        let (bars, timestamps) = ohlcv_to_bars(&records);

        assert_eq!(bars.len(), 2);
        assert_eq!(bars[0].symbol, "AAPL");
        assert_eq!(bars[1].symbol, "MSFT");
        assert_eq!(timestamps[0], "2024-01-01");
        assert_eq!(timestamps[1], "2024-01-02");
    }
}
