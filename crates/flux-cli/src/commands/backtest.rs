use std::collections::HashMap;
use std::path::Path;

use flux_runtime::{BarContext, PositionTracker, FillSide, Signal};

use crate::csv_loader;
use crate::diagnostics;
use crate::error::{CliError, CompileErrorWithSpan};
use crate::interpreter::Interpreter;

/// A group of bars sharing the same timestamp.
///
/// When a multi-asset CSV is loaded, consecutive rows with the same timestamp
/// are grouped together for joint processing in the backtest loop.
pub struct BarGroup {
    /// The timestamp string shared by all bars in this group.
    pub timestamp: String,
    /// Bars in this group, in CSV row order.
    pub bars: Vec<BarContext>,
    /// Close prices indexed by symbol for quick lookup.
    pub closes: HashMap<String, f64>,
}

/// Group bars by consecutive same-timestamp sequences.
///
/// Bars sharing the same timestamp string are grouped together.
/// Groups are produced in first-occurrence order, and bar order
/// within each group matches CSV row order.
///
/// # Arguments
/// - `bars` — Slice of bar records in CSV row order.
/// - `timestamps` — Parallel slice of timestamp strings (one per bar).
///
/// # Errors
/// Returns an error if any group contains more than 100 distinct symbols.
pub fn group_bars_by_timestamp(
    bars: &[BarContext],
    timestamps: &[String],
) -> Result<Vec<BarGroup>, String> {
    let mut groups: Vec<BarGroup> = Vec::new();
    let mut i = 0;

    while i < bars.len() {
        let ts = &timestamps[i];
        let mut group_bars: Vec<BarContext> = Vec::new();
        let mut closes: HashMap<String, f64> = HashMap::new();

        // Collect consecutive bars with the same timestamp
        while i < bars.len() && &timestamps[i] == ts {
            closes.insert(bars[i].symbol.clone(), bars[i].close);
            group_bars.push(bars[i].clone());
            i += 1;
        }

        // Validate max 100 symbols per group
        if closes.len() > 100 {
            return Err(format!(
                "timestamp group '{}' exceeds maximum of 100 symbols (found {})",
                ts,
                closes.len()
            ));
        }

        groups.push(BarGroup {
            timestamp: ts.clone(),
            bars: group_bars,
            closes,
        });
    }

    Ok(groups)
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

/// Format a single signal for display output.
///
/// Format:
/// - Open: `"{bar_index} Open {symbol} {qty}"`
/// - Close: `"{bar_index} Close {symbol}"`
/// - CloseQty: `"{bar_index} CloseQty {symbol} {qty}"`
pub fn format_signal(bar_index: usize, signal: &Signal) -> String {
    match signal {
        Signal::Open { symbol, qty } => {
            format!("{} Open {} {}", bar_index, symbol, qty)
        }
        Signal::Close { symbol } => {
            format!("{} Close {}", bar_index, symbol)
        }
        Signal::CloseQty { symbol, qty } => {
            format!("{} CloseQty {} {}", bar_index, symbol, qty)
        }
    }
}

/// Format a summary of backtest results.
///
/// Output format:
/// ```text
/// --- Summary ---
/// Total signals: {total}
/// Open: {open_count}
/// Close: {close_count}
/// CloseQty: {close_qty_count}
/// ```
pub fn format_summary(results: &[(usize, Signal)]) -> String {
    let total = results.len();
    let mut open_count = 0usize;
    let mut close_count = 0usize;
    let mut close_qty_count = 0usize;

    for (_idx, signal) in results {
        match signal {
            Signal::Open { .. } => open_count += 1,
            Signal::Close { .. } => close_count += 1,
            Signal::CloseQty { .. } => close_qty_count += 1,
        }
    }

    format!(
        "--- Summary ---\nTotal signals: {}\nOpen: {}\nClose: {}\nCloseQty: {}",
        total, open_count, close_count, close_qty_count
    )
}

pub fn run_backtest_cmd(file: &Path, data_paths: &[&Path], initial_capital: f64) -> Result<(), CliError> {
    let source = std::fs::read_to_string(file).map_err(CliError::Io)?;
    let file_display = file.display().to_string();

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

    // Type check
    let typed_program = match flux_compiler::typeck::check(ast) {
        Ok(typed_program) => typed_program,
        Err(err) => {
            let errors = compile_error_to_spans(&err);
            let formatted = diagnostics::format_errors(&file_display, &source, &errors);
            eprintln!("{}", formatted);
            return Err(CliError::Compile(errors));
        }
    };

    // Load CSV bar data from all provided data files
    let mut bars: Vec<BarContext> = Vec::new();
    for data_path in data_paths {
        let file_bars = csv_loader::load_csv(data_path).map_err(CliError::Csv)?;
        bars.extend(file_bars);
    }

    // Create interpreter and position tracker
    let mut interpreter = Interpreter::new(&typed_program);
    let mut tracker = PositionTracker::new(initial_capital);

    let mut results: Vec<(usize, Signal)> = Vec::new();
    for (i, bar) in bars.iter().enumerate() {
        // Set in_position from tracker state (multi-symbol aware)
        interpreter.in_position = tracker.open_position_count() > 0;

        let signals = interpreter.on_bar(bar);

        // Feed signals through position tracker
        tracker.process_signals(&signals, bar.close, i);

        // Collect raw signal pairs
        for signal in signals {
            results.push((i, signal));
        }

        // Mark all open positions to market at bar close
        tracker.mark_to_market(bar.close, &bar.symbol);
    }

    // Print signals
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
    println!("  Total Return:      {:>11.2}%", 
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
    use proptest::prelude::*;

    #[test]
    fn test_format_signal_open() {
        let signal = Signal::open("AAPL".to_string(), 100.0);
        let result = format_signal(5, &signal);
        assert_eq!(result, "5 Open AAPL 100");
    }

    #[test]
    fn test_format_signal_close() {
        let signal = Signal::close("MSFT".to_string());
        let result = format_signal(10, &signal);
        assert_eq!(result, "10 Close MSFT");
    }

    #[test]
    fn test_format_signal_close_qty() {
        let signal = Signal::close_qty("GOOG".to_string(), 50.5);
        let result = format_signal(3, &signal);
        assert_eq!(result, "3 CloseQty GOOG 50.5");
    }

    #[test]
    fn test_format_signal_bar_index_zero() {
        let signal = Signal::open("SPY".to_string(), 1.0);
        let result = format_signal(0, &signal);
        assert_eq!(result, "0 Open SPY 1");
    }

    #[test]
    fn test_format_summary_empty() {
        let results: Vec<(usize, Signal)> = vec![];
        let summary = format_summary(&results);
        assert_eq!(
            summary,
            "--- Summary ---\nTotal signals: 0\nOpen: 0\nClose: 0\nCloseQty: 0"
        );
    }

    #[test]
    fn test_format_summary_mixed_signals() {
        let results = vec![
            (0, Signal::open("AAPL".to_string(), 100.0)),
            (1, Signal::close("AAPL".to_string())),
            (2, Signal::open("MSFT".to_string(), 50.0)),
            (3, Signal::close_qty("MSFT".to_string(), 25.0)),
            (4, Signal::close("MSFT".to_string())),
        ];
        let summary = format_summary(&results);
        assert_eq!(
            summary,
            "--- Summary ---\nTotal signals: 5\nOpen: 2\nClose: 2\nCloseQty: 1"
        );
    }

    #[test]
    fn test_format_summary_only_opens() {
        let results = vec![
            (0, Signal::open("A".to_string(), 1.0)),
            (1, Signal::open("B".to_string(), 2.0)),
            (2, Signal::open("C".to_string(), 3.0)),
        ];
        let summary = format_summary(&results);
        assert_eq!(
            summary,
            "--- Summary ---\nTotal signals: 3\nOpen: 3\nClose: 0\nCloseQty: 0"
        );
    }

    #[test]
    fn test_format_summary_count_invariant() {
        let results = vec![
            (0, Signal::open("X".to_string(), 10.0)),
            (1, Signal::close("X".to_string())),
            (2, Signal::close_qty("X".to_string(), 5.0)),
        ];
        let summary = format_summary(&results);
        // Verify total == open + close + close_qty
        assert!(summary.contains("Total signals: 3"));
        assert!(summary.contains("Open: 1"));
        assert!(summary.contains("Close: 1"));
        assert!(summary.contains("CloseQty: 1"));
    }

    /// Generate an arbitrary Signal value.
    fn arb_signal() -> impl Strategy<Value = Signal> {
        let symbol = "[A-Z]{1,5}";
        let qty = 0.01..10000.0f64;

        prop_oneof![
            (symbol, qty.clone()).prop_map(|(s, q)| Signal::Open {
                symbol: s,
                qty: q
            }),
            "[A-Z]{1,5}".prop_map(|s| Signal::Close { symbol: s }),
            ("[A-Z]{1,5}", qty).prop_map(|(s, q)| Signal::CloseQty {
                symbol: s,
                qty: q
            }),
        ]
    }

    // Feature: flux-cli, Property 2: Signal output formatting completeness
    // **Validates: Requirements 4.2**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_signal_formatting_completeness(
            bar_index in 0usize..10000,
            signal in arb_signal(),
        ) {
            let output = format_signal(bar_index, &signal);

            // Output contains the bar index
            prop_assert!(
                output.contains(&bar_index.to_string()),
                "Output {:?} does not contain bar_index {}",
                output,
                bar_index
            );

            // Output contains the signal type name and symbol
            match &signal {
                Signal::Open { symbol, qty } => {
                    prop_assert!(
                        output.contains("Open"),
                        "Output {:?} does not contain 'Open'",
                        output
                    );
                    prop_assert!(
                        output.contains(symbol),
                        "Output {:?} does not contain symbol {:?}",
                        output,
                        symbol
                    );
                    let qty_str = format!("{}", qty);
                    prop_assert!(
                        output.contains(&qty_str),
                        "Output {:?} does not contain qty {:?}",
                        output,
                        qty_str
                    );
                }
                Signal::Close { symbol } => {
                    prop_assert!(
                        output.contains("Close"),
                        "Output {:?} does not contain 'Close'",
                        output
                    );
                    prop_assert!(
                        output.contains(symbol),
                        "Output {:?} does not contain symbol {:?}",
                        output,
                        symbol
                    );
                }
                Signal::CloseQty { symbol, qty } => {
                    prop_assert!(
                        output.contains("CloseQty"),
                        "Output {:?} does not contain 'CloseQty'",
                        output
                    );
                    prop_assert!(
                        output.contains(symbol),
                        "Output {:?} does not contain symbol {:?}",
                        output,
                        symbol
                    );
                    let qty_str = format!("{}", qty);
                    prop_assert!(
                        output.contains(&qty_str),
                        "Output {:?} does not contain qty {:?}",
                        output,
                        qty_str
                    );
                }
            }
        }
    }
}
