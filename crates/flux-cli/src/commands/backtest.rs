use std::collections::HashMap;
use std::path::Path;

use flux_compiler::lexer::span::Span;
use flux_compiler::typeck::typed_ast::{TypedExpr, TypedExprKind, TypedProgram};
use flux_compiler::typeck::types::FluxType;
use flux_runtime::{BarContext, PositionTracker, FillSide, Signal};

use crate::csv_loader;
use crate::diagnostics;
use crate::error::{CliError, CompileErrorWithSpan};
use crate::interpreter::{Interpreter, Value};
use crate::module_resolver;

/// Parse a multiplier specification string into a HashMap.
/// Format: "SYMBOL:VALUE,SYMBOL:VALUE,..." (e.g., "ES=F:50,NQ=F:20,RTY=F:50,YM=F:5")
/// Invalid entries are silently skipped.
pub fn parse_multipliers(spec: &str) -> HashMap<String, f64> {
    let mut map = HashMap::new();
    for pair in spec.split(',') {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        if let Some((sym, val_str)) = pair.rsplit_once(':') {
            if let Ok(val) = val_str.parse::<f64>() {
                if val > 0.0 {
                    map.insert(sym.to_string(), val);
                }
            }
        }
    }
    map
}

/// Parameters for constructing an Order from a Signal.
///
/// This is a Rust-side bridge struct that captures the essential fields
/// needed to build an interpreter-level Order Value for engine submission.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderParams {
    /// The symbol for the order.
    pub symbol: String,
    /// True for Buy, false for Sell.
    pub side_is_buy: bool,
    /// The quantity to trade.
    pub qty: f64,
    /// Sequential order ID assigned to this order.
    pub order_id: i64,
}

/// Convert a Signal into OrderParams suitable for engine submission.
///
/// For `Signal::Close`, the caller must supply the current position quantity
/// via `position_qty`. If the position is zero (or negative), no order is
/// needed and `None` is returned.
///
/// Each successful translation consumes one order ID from `next_order_id`.
pub fn signal_to_order_params(
    signal: &Signal,
    next_order_id: &mut i64,
    position_qty: f64,
) -> Option<OrderParams> {
    match signal {
        Signal::Open { symbol, qty } => {
            let id = *next_order_id;
            *next_order_id += 1;
            Some(OrderParams {
                symbol: symbol.clone(),
                side_is_buy: true,
                qty: *qty,
                order_id: id,
            })
        }
        Signal::Short { symbol, qty } => {
            let id = *next_order_id;
            *next_order_id += 1;
            Some(OrderParams {
                symbol: symbol.clone(),
                side_is_buy: false,
                qty: *qty,
                order_id: id,
            })
        }
        Signal::Close { symbol } => {
            if position_qty == 0.0 {
                return None; // No position to close
            }
            let id = *next_order_id;
            *next_order_id += 1;
            // If position is long (qty > 0), sell to close. If short (qty < 0), buy to close.
            Some(OrderParams {
                symbol: symbol.clone(),
                side_is_buy: position_qty < 0.0,
                qty: position_qty.abs(),
                order_id: id,
            })
        }
        Signal::CloseQty { symbol, qty } => {
            let id = *next_order_id;
            *next_order_id += 1;
            Some(OrderParams {
                symbol: symbol.clone(),
                side_is_buy: false,
                qty: *qty,
                order_id: id,
            })
        }
    }
}

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
        Signal::Short { symbol, qty } => {
            format!("{} Short {} {}", bar_index, symbol, qty)
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
/// Short: {short_count}
/// Close: {close_count}
/// CloseQty: {close_qty_count}
/// ```
pub fn format_summary(results: &[(usize, Signal)]) -> String {
    let total = results.len();
    let mut open_count = 0usize;
    let mut short_count = 0usize;
    let mut close_count = 0usize;
    let mut close_qty_count = 0usize;

    for (_idx, signal) in results {
        match signal {
            Signal::Open { .. } => open_count += 1,
            Signal::Short { .. } => short_count += 1,
            Signal::Close { .. } => close_count += 1,
            Signal::CloseQty { .. } => close_qty_count += 1,
        }
    }

    format!(
        "--- Summary ---\nTotal signals: {}\nOpen: {}\nShort: {}\nClose: {}\nCloseQty: {}",
        total, open_count, short_count, close_count, close_qty_count
    )
}

pub fn run_backtest_cmd(file: &Path, data_paths: &[&Path], initial_capital: f64, fidelity: u8, depth: Option<u32>, spread: Option<f64>, liquidity: Option<f64>, l2_data: Option<&Path>, multipliers: &HashMap<String, f64>) -> Result<(), CliError> {
    // === Fidelity Level Routing ===
    //
    // Backward compatibility guarantee (Requirements 4.7, 11.7):
    // When fidelity == 0 (the default), the entire backtest runs through the
    // existing PositionTracker code path. This produces output IDENTICAL to the
    // pre-fidelity `flux backtest` behavior — same fill prices, quantities,
    // sides, ordering, and output format (Signals, Fills, Portfolio Summary,
    // Summary sections).
    //
    // The PositionTracker is the reference implementation. Higher fidelity
    // levels (1, 2) will use Flux stdlib engine modules when available.
    //
    // Fidelity 0: PositionTracker (fill at close, zero slippage)
    // Fidelity 1: SyntheticEngine (synthetic book from OHLCV) — future
    // Fidelity 2: ReplayEngine (L2 data replay) — future

    // Validate fidelity level
    if fidelity > 2 {
        eprintln!("error: --fidelity must be 0, 1, or 2 (got {})", fidelity);
        return Err(CliError::Usage(format!("invalid fidelity level: {}", fidelity)));
    }

    // Validate fidelity 2 requires --l2-data
    if fidelity == 2 && l2_data.is_none() {
        eprintln!("error: --fidelity 2 requires --l2-data <path>");
        return Err(CliError::Usage("fidelity level 2 requires L2 market data (--l2-data)".to_string()));
    }

    // Validate synthetic book params only valid for fidelity 1
    if fidelity != 1 && (depth.is_some() || spread.is_some() || liquidity.is_some()) {
        eprintln!("error: --depth, --spread, and --liquidity are only valid for --fidelity 1");
        return Err(CliError::Usage("synthetic book parameters (--depth, --spread, --liquidity) are only valid for fidelity level 1".to_string()));
    }

    // Validate depth range
    if let Some(d) = depth {
        if d < 1 || d > 20 {
            eprintln!("error: --depth must be between 1 and 20 (got {})", d);
            return Err(CliError::Usage(format!("--depth must be between 1 and 20 (got {})", d)));
        }
    }

    // Validate spread range
    if let Some(s) = spread {
        if s < 0.01 || s > 10.0 {
            eprintln!("error: --spread must be between 0.01 and 10.0 (got {})", s);
            return Err(CliError::Usage(format!("--spread must be between 0.01 and 10.0 (got {})", s)));
        }
    }

    // Validate liquidity range
    if let Some(l) = liquidity {
        if l < 100.0 || l > 10_000_000.0 {
            eprintln!("error: --liquidity must be between 100 and 10000000 (got {})", l);
            return Err(CliError::Usage(format!("--liquidity must be between 100 and 10000000 (got {})", l)));
        }
    }

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

    // When fidelity > 0, inject engine module imports into the AST so the
    // interpreter has access to engine structs, enums, and impl methods.
    let mut ast = ast;
    if fidelity > 0 {
        use flux_compiler::parser::ast::Import;

        // Always need engine::types for Order, Fill, etc.
        ast.imports.push(Import {
            module_path: "engine::types".to_string(),
            names: vec![
                "Order".to_string(), "Fill".to_string(), "PositionState".to_string(),
                "OrderSide".to_string(), "OrderType".to_string(), "TimeInForce".to_string(),
                "FillResult".to_string(), "BacktestEngine".to_string(),
            ],
            span: Span::new(0, 0),
        });
        ast.imports.push(Import {
            module_path: "engine::book".to_string(),
            names: vec!["OrderBook".to_string(), "PriceLevel".to_string()],
            span: Span::new(0, 0),
        });
        ast.imports.push(Import {
            module_path: "engine::metrics".to_string(),
            names: vec![
                "Metrics".to_string(),
                "compute_metrics".to_string(),
                "compute_sharpe".to_string(),
                "compute_max_drawdown".to_string(),
                "compute_trade_pnls".to_string(),
            ],
            span: Span::new(0, 0),
        });

        if fidelity == 1 {
            ast.imports.push(Import {
                module_path: "engine::synthetic".to_string(),
                names: vec![
                    "SyntheticEngine".to_string(),
                    "SyntheticConfig".to_string(),
                    "generate_price_path".to_string(),
                    "build_synthetic_book".to_string(),
                    "update_synth_position".to_string(),
                ],
                span: Span::new(0, 0),
            });
        } else if fidelity == 2 {
            ast.imports.push(Import {
                module_path: "engine::replay".to_string(),
                names: vec![
                    "ReplayEngine".to_string(),
                    "L2Event".to_string(),
                    "L2Action".to_string(),
                    "QueuedOrder".to_string(),
                    "get_queue_ahead".to_string(),
                    "process_l2_event".to_string(),
                    "advance_queues".to_string(),
                    "check_queue_fills".to_string(),
                    "update_replay_position".to_string(),
                    "trim_book".to_string(),
                ],
                span: Span::new(0, 0),
            });
        }

        // Also import fast engine (always useful as reference/fallback)
        ast.imports.push(Import {
            module_path: "engine::fast".to_string(),
            names: vec!["FastEngine".to_string(), "update_position".to_string()],
            span: Span::new(0, 0),
        });
    }

    // Module resolution
    let ast = module_resolver::resolve_modules(ast, file.parent().unwrap_or(Path::new(".")))?;

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

    if fidelity > 0 {
        // Engine-based backtest (fidelity 1 or 2)
        run_engine_backtest(&typed_program, &bars, initial_capital, fidelity, depth, spread, liquidity, l2_data, file)?;
    } else {
        // Detect multi-symbol: if 2+ distinct symbols, use timestamp-interleaved mode
        let distinct_symbols: std::collections::HashSet<&str> = bars.iter()
            .map(|b| b.symbol.as_str())
            .collect();

        if distinct_symbols.len() >= 2 {
            // Multi-symbol: re-load with timestamps for grouping
            let mut all_bars: Vec<BarContext> = Vec::new();
            let mut all_timestamps: Vec<String> = Vec::new();
            for data_path in data_paths {
                let loaded = csv_loader::load_csv_with_timestamps(data_path)
                    .map_err(CliError::Csv)?;
                all_bars.extend(loaded.bars);
                all_timestamps.extend(loaded.timestamps);
            }
            run_fidelity_zero_backtest_interleaved(&typed_program, &all_bars, &all_timestamps, initial_capital, multipliers)?;
        } else {
            // Single-symbol: existing sequential path (unchanged)
            run_fidelity_zero_backtest(&typed_program, &bars, initial_capital, multipliers)?;
        }
    }

    Ok(())
}

/// Run the fidelity 0 (fast) backtest using the existing PositionTracker.
/// This preserves backward compatibility with the original `flux backtest` output format.
fn run_fidelity_zero_backtest(
    typed_program: &TypedProgram,
    bars: &[BarContext],
    initial_capital: f64,
    multipliers: &HashMap<String, f64>,
) -> Result<(), CliError> {
    // Create interpreter and position tracker
    let mut interpreter = Interpreter::new(typed_program);
    let mut tracker = PositionTracker::new_with_multipliers(initial_capital, multipliers.clone());

    let mut results: Vec<(usize, Signal)> = Vec::new();
    for (i, bar) in bars.iter().enumerate() {
        // Set in_position per-symbol from tracker state (multi-symbol aware).
        // This ensures each symbol gets its own in_position flag.
        interpreter.in_position = tracker.position(&bar.symbol)
            .map(|p| p.qty != 0.0)
            .unwrap_or(false);

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

/// Run fidelity 0 backtest with timestamp-interleaved multi-symbol processing.
///
/// Groups bars by timestamp and processes all symbols for each timestamp before
/// advancing. Uses a two-pass approach per group:
///   Pass 1: Run on_bar for all symbols (updates indicators, fires exits, marks pending entries)
///   Pass 2: Re-run on_bar for symbols with pending entries (fills at same-day close)
///
/// This ensures cross-symbol ranking decisions see all products' updated scores
/// before making entry decisions, matching vectorized research logic.
fn run_fidelity_zero_backtest_interleaved(
    typed_program: &TypedProgram,
    bars: &[BarContext],
    timestamps: &[String],
    initial_capital: f64,
    multipliers: &HashMap<String, f64>,
) -> Result<(), CliError> {
    let mut interpreter = Interpreter::new(typed_program);
    let mut tracker = PositionTracker::new_with_multipliers(initial_capital, multipliers.clone());
    let mut results: Vec<(usize, Signal)> = Vec::new();

    // Group bars by timestamp for interleaved processing
    let groups = group_bars_by_timestamp(bars, timestamps)
        .map_err(|e| CliError::Runtime(e))?;

    let mut global_bar_index: usize = 0;

    for group in &groups {
        // Update interpreter with all close prices for this timestamp group
        interpreter.update_prices(&group.closes);

        let group_start_index = global_bar_index;

        // ─── Pass 1: Process all bars in this group ───
        // This runs indicators, updates scores, fires exits, and sets pending entries.
        // OPEN signals are collected but we also track which symbols might need
        // a second pass (if they had no position and were processed before the
        // rotation boundary set their pending flag).
        for bar in &group.bars {
            // Set per-symbol in_position from tracker
            interpreter.in_position = tracker.position(&bar.symbol)
                .map(|p| p.qty != 0.0)
                .unwrap_or(false);

            // Execute on_bar handler
            let signals = interpreter.on_bar(bar);

            // Process signals through position tracker
            tracker.process_signals(&signals, bar.close, global_bar_index);

            // Collect signals
            for signal in signals {
                results.push((global_bar_index, signal));
            }

            // Mark-to-market this specific symbol
            tracker.mark_to_market(bar.close, &bar.symbol);

            global_bar_index += 1;
        }

        // ─── Pass 2: Re-process symbols that now have pending entries ───
        // After pass 1, the rotation boundary has fired and pending_entry_map
        // is populated. Symbols processed *before* the boundary in pass 1
        // missed their entry. Re-run their on_bar to pick up the pending flag.
        // Only re-run for symbols not already in position and not already signalled.
        for (i, bar) in group.bars.iter().enumerate() {
            let bar_index = group_start_index + i;

            // Only re-process if symbol has no position (entry candidate)
            let already_in = tracker.position(&bar.symbol)
                .map(|p| p.qty != 0.0)
                .unwrap_or(false);

            if already_in {
                continue;
            }

            // Set in_position for this second pass
            interpreter.in_position = false;

            // Re-run on_bar — this will check pending_entry_map and fire OPEN
            let signals = interpreter.on_bar(bar);

            // Only process OPEN signals from pass 2 (ignore duplicates of other types)
            let open_signals: Vec<Signal> = signals.into_iter().filter(|s| {
                matches!(s, Signal::Open { .. })
            }).collect();

            if !open_signals.is_empty() {
                tracker.process_signals(&open_signals, bar.close, bar_index);
                for signal in open_signals {
                    results.push((bar_index, signal));
                }
                tracker.mark_to_market(bar.close, &bar.symbol);
            }
        }
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

/// Helper: construct a TypedExpr node for use in programmatic interpreter calls.
fn make_typed_expr(kind: TypedExprKind, resolved_type: FluxType) -> TypedExpr {
    TypedExpr {
        kind,
        resolved_type,
        span: Span::new(0, 0),
    }
}

/// Construct a Value::Struct representing an Order for the engine interpreter.
fn make_order_value(params: &OrderParams) -> Value {
    let mut fields = HashMap::new();
    fields.insert("id".to_string(), Value::Int(params.order_id));
    fields.insert("symbol".to_string(), Value::Str(params.symbol.clone()));
    fields.insert(
        "side".to_string(),
        if params.side_is_buy {
            Value::Enum {
                enum_name: "OrderSide".to_string(),
                variant_name: "Buy".to_string(),
                fields: vec![],
            }
        } else {
            Value::Enum {
                enum_name: "OrderSide".to_string(),
                variant_name: "Sell".to_string(),
                fields: vec![],
            }
        },
    );
    fields.insert(
        "order_type".to_string(),
        Value::Enum {
            enum_name: "OrderType".to_string(),
            variant_name: "Market".to_string(),
            fields: vec![],
        },
    );
    fields.insert("qty".to_string(), Value::Float(params.qty));
    fields.insert(
        "tif".to_string(),
        Value::Enum {
            enum_name: "TimeInForce".to_string(),
            variant_name: "GTC".to_string(),
            fields: vec![],
        },
    );
    Value::Struct {
        type_name: "Order".to_string(),
        fields,
    }
}

/// Construct a Value::Struct representing a Bar for the engine interpreter.
fn make_bar_value(bar: &BarContext, timestamp: f64) -> Value {
    let mut fields = HashMap::new();
    fields.insert("symbol".to_string(), Value::Str(bar.symbol.clone()));
    fields.insert("open".to_string(), Value::Float(bar.open));
    fields.insert("high".to_string(), Value::Float(bar.high));
    fields.insert("low".to_string(), Value::Float(bar.low));
    fields.insert("close".to_string(), Value::Float(bar.close));
    fields.insert("volume".to_string(), Value::Float(bar.volume));
    fields.insert("timestamp".to_string(), Value::Float(timestamp));
    Value::Struct {
        type_name: "Bar".to_string(),
        fields,
    }
}

/// Call a method on the engine value by constructing a TypedExpr::MethodCall
/// and evaluating it. The engine value must be stored in locals under `engine_var`.
fn call_engine_method(
    interpreter: &mut Interpreter,
    locals: &mut HashMap<String, Value>,
    engine_var: &str,
    method: &str,
    args: Vec<Value>,
) -> Result<Value, String> {
    // Build argument expressions as identifiers pointing to temporary locals
    let mut arg_exprs = Vec::new();
    let mut temp_names = Vec::new();
    for (i, arg) in args.into_iter().enumerate() {
        let temp_name = format!("__arg_{}_{}", method, i);
        locals.insert(temp_name.clone(), arg);
        arg_exprs.push(make_typed_expr(
            TypedExprKind::Ident(temp_name.clone()),
            FluxType::Null,
        ));
        temp_names.push(temp_name);
    }

    let method_call = make_typed_expr(
        TypedExprKind::MethodCall {
            receiver: Box::new(make_typed_expr(
                TypedExprKind::Ident(engine_var.to_string()),
                FluxType::Null,
            )),
            method: method.to_string(),
            args: arg_exprs,
        },
        FluxType::Null,
    );

    let result = interpreter.eval_expr(&method_call, locals)?;

    // Clean up temp argument locals
    for name in temp_names {
        locals.remove(&name);
    }

    Ok(result)
}

/// Call a free function through the interpreter by constructing a TypedExpr::FunctionCall.
fn call_function(
    interpreter: &mut Interpreter,
    locals: &mut HashMap<String, Value>,
    func_name: &str,
    args: Vec<Value>,
) -> Result<Value, String> {
    let mut arg_exprs = Vec::new();
    let mut temp_names = Vec::new();
    for (i, arg) in args.into_iter().enumerate() {
        let temp_name = format!("__farg_{}_{}", func_name, i);
        locals.insert(temp_name.clone(), arg);
        arg_exprs.push(make_typed_expr(
            TypedExprKind::Ident(temp_name.clone()),
            FluxType::Null,
        ));
        temp_names.push(temp_name);
    }

    let fn_call = make_typed_expr(
        TypedExprKind::FunctionCall {
            function: Box::new(make_typed_expr(
                TypedExprKind::Ident(func_name.to_string()),
                FluxType::Null,
            )),
            args: arg_exprs,
        },
        FluxType::Null,
    );

    let result = interpreter.eval_expr(&fn_call, locals)?;

    // Clean up temp argument locals
    for name in temp_names {
        locals.remove(&name);
    }

    Ok(result)
}

/// Extract an f64 from a Value (Int or Float).
fn value_to_f64(val: &Value) -> f64 {
    match val {
        Value::Float(f) => *f,
        Value::Int(i) => *i as f64,
        _ => 0.0,
    }
}

/// Extract a string from a Value.
fn value_to_string(val: &Value) -> String {
    match val {
        Value::Str(s) => s.clone(),
        _ => String::new(),
    }
}

/// Extract an i64 from a Value.
fn value_to_i64(val: &Value) -> i64 {
    match val {
        Value::Int(i) => *i,
        Value::Float(f) => *f as i64,
        _ => 0,
    }
}

/// A fill record extracted from the engine's Value::Struct representation.
#[derive(Debug, Clone)]
struct EngineFill {
    pub order_id: i64,
    pub symbol: String,
    pub side_is_buy: bool,
    pub price: f64,
    pub qty: f64,
    pub timestamp: f64,
    pub slippage: f64,
}

/// Unpack a Fill Value::Struct into an EngineFill.
fn unpack_fill(fill_value: &Value) -> Option<EngineFill> {
    if let Value::Struct { type_name, fields } = fill_value {
        if type_name != "Fill" {
            return None;
        }
        let order_id = value_to_i64(fields.get("order_id")?);
        let symbol = value_to_string(fields.get("symbol")?);
        let side_is_buy = match fields.get("side")? {
            Value::Enum { variant_name, .. } => variant_name == "Buy",
            _ => true,
        };
        let price = value_to_f64(fields.get("price")?);
        let qty = value_to_f64(fields.get("qty")?);
        let timestamp = value_to_f64(fields.get("timestamp")?);
        let slippage = value_to_f64(fields.get("slippage")?);
        Some(EngineFill {
            order_id,
            symbol,
            side_is_buy,
            price,
            qty,
            timestamp,
            slippage,
        })
    } else {
        None
    }
}

/// Get position qty for a specific symbol.
fn get_symbol_position_qty(positions: &HashMap<String, f64>, symbol: &str) -> f64 {
    positions.get(symbol).copied().unwrap_or(0.0)
}

/// Run engine-based backtest for fidelity levels 1 and 2.
///
/// Uses the Flux interpreter to instantiate and call methods on engine modules.
/// The engine Value is passed through the interpreter's method dispatch, achieving
/// full integration between Rust-side signal translation and Flux-side matching logic.
#[allow(clippy::too_many_arguments)]
fn run_engine_backtest(
    typed_program: &TypedProgram,
    bars: &[BarContext],
    initial_capital: f64,
    fidelity: u8,
    depth: Option<u32>,
    spread: Option<f64>,
    liquidity: Option<f64>,
    _l2_data: Option<&Path>,
    _strategy_file: &Path,
) -> Result<(), CliError> {
    // Create strategy interpreter (includes engine modules via injected imports)
    let mut interpreter = Interpreter::new(typed_program);
    let mut locals: HashMap<String, Value> = HashMap::new();

    // Instantiate the engine based on fidelity level
    let engine_value = match fidelity {
        1 => {
            // Build SyntheticConfig struct
            let depth_val = depth.unwrap_or(5) as i64;
            let spread_val = spread.unwrap_or(0.1);
            let liquidity_val = liquidity.unwrap_or(10000.0);

            let mut config_fields = HashMap::new();
            config_fields.insert("depth".to_string(), Value::Int(depth_val));
            config_fields.insert("spread_pct".to_string(), Value::Float(spread_val));
            config_fields.insert("liquidity_per_side".to_string(), Value::Float(liquidity_val));
            let config = Value::Struct {
                type_name: "SyntheticConfig".to_string(),
                fields: config_fields,
            };

            // Call SyntheticEngine.new(config) via static method dispatch
            locals.insert("__config".to_string(), config);
            let new_call = make_typed_expr(
                TypedExprKind::MethodCall {
                    receiver: Box::new(make_typed_expr(
                        TypedExprKind::Ident("SyntheticEngine".to_string()),
                        FluxType::Struct("SyntheticEngine".to_string()),
                    )),
                    method: "new".to_string(),
                    args: vec![make_typed_expr(
                        TypedExprKind::Ident("__config".to_string()),
                        FluxType::Struct("SyntheticConfig".to_string()),
                    )],
                },
                FluxType::Struct("SyntheticEngine".to_string()),
            );
            let engine = interpreter.eval_expr(&new_call, &mut locals).map_err(|e| {
                CliError::Runtime(format!("failed to create SyntheticEngine: {}", e))
            })?;
            locals.remove("__config");
            engine
        }
        2 => {
            // Call ReplayEngine.new() via static method dispatch
            let new_call = make_typed_expr(
                TypedExprKind::MethodCall {
                    receiver: Box::new(make_typed_expr(
                        TypedExprKind::Ident("ReplayEngine".to_string()),
                        FluxType::Struct("ReplayEngine".to_string()),
                    )),
                    method: "new".to_string(),
                    args: vec![],
                },
                FluxType::Struct("ReplayEngine".to_string()),
            );
            interpreter.eval_expr(&new_call, &mut locals).map_err(|e| {
                CliError::Runtime(format!("failed to create ReplayEngine: {}", e))
            })?
        }
        _ => unreachable!("fidelity validated earlier"),
    };

    // Store engine in locals for method dispatch
    locals.insert("__engine".to_string(), engine_value);

    // Track positions (symbol → qty) on the Rust side for signal_to_order_params
    let mut position_qtys: HashMap<String, f64> = HashMap::new();
    let mut next_order_id: i64 = 0;
    let mut all_fills: Vec<EngineFill> = Vec::new();
    let mut equity_curve: Vec<f64> = vec![initial_capital];
    let mut current_equity = initial_capital;
    let mut results: Vec<(usize, Signal)> = Vec::new();

    // Main backtest loop
    for (i, bar) in bars.iter().enumerate() {
        // Set in_position from position state (any non-zero position, long or short)
        interpreter.in_position = position_qtys.values().any(|&q| q != 0.0);

        // Run strategy to get signals
        let signals = interpreter.on_bar(bar);

        // Collect signals for display
        for signal in &signals {
            results.push((i, signal.clone()));
        }

        // Translate signals to orders and submit them to the engine
        for signal in &signals {
            let sym = match signal {
                Signal::Open { symbol, .. } => symbol,
                Signal::Short { symbol, .. } => symbol,
                Signal::Close { symbol } => symbol,
                Signal::CloseQty { symbol, .. } => symbol,
            };
            let pos_qty = get_symbol_position_qty(&position_qtys, sym);

            if let Some(order_params) = signal_to_order_params(signal, &mut next_order_id, pos_qty) {
                let order_value = make_order_value(&order_params);

                // Call engine.submit_order(order)
                let new_engine = call_engine_method(
                    &mut interpreter,
                    &mut locals,
                    "__engine",
                    "submit_order",
                    vec![order_value],
                ).map_err(|e| {
                    CliError::Runtime(format!("engine.submit_order failed: {}", e))
                })?;
                locals.insert("__engine".to_string(), new_engine);
            }
        }

        // Call engine.process_bar(bar) with the current bar as a Value::Struct
        let bar_value = make_bar_value(bar, i as f64);
        let new_engine = call_engine_method(
            &mut interpreter,
            &mut locals,
            "__engine",
            "process_bar",
            vec![bar_value],
        ).map_err(|e| {
            CliError::Runtime(format!("engine.process_bar failed: {}", e))
        })?;
        locals.insert("__engine".to_string(), new_engine);

        // Call engine.get_fills() to collect fills from this bar
        let fills_value = call_engine_method(
            &mut interpreter,
            &mut locals,
            "__engine",
            "get_fills",
            vec![],
        ).map_err(|e| {
            CliError::Runtime(format!("engine.get_fills failed: {}", e))
        })?;

        // Extract fills from Value::List
        if let Value::List(fill_values) = &fills_value {
            for fill_val in fill_values {
                if let Some(fill) = unpack_fill(fill_val) {
                    // Update position tracking on Rust side
                    if fill.side_is_buy {
                        *position_qtys.entry(fill.symbol.clone()).or_insert(0.0) += fill.qty;
                    } else {
                        let pos = position_qtys.entry(fill.symbol.clone()).or_insert(0.0);
                        *pos -= fill.qty;
                        if *pos <= 0.0 {
                            position_qtys.remove(&fill.symbol);
                        }
                    }

                    all_fills.push(fill);
                }
            }
        }

        // Update equity curve: initial_capital + realized P&L from all fills so far
        let realized_pnl = compute_realized_pnl_from_fills(&all_fills);
        current_equity = initial_capital + realized_pnl;
        equity_curve.push(current_equity);
    }

    // --- Output Results ---

    // Print signals
    println!("--- Signals ---");
    for (idx, sig) in &results {
        println!("  {}", format_signal(*idx, sig));
    }

    // Print fills from engine
    if !all_fills.is_empty() {
        println!("\n--- Fills ---");
        for fill in &all_fills {
            let side_str = if fill.side_is_buy { "BUY" } else { "SELL" };
            println!(
                "  Bar {:>4} | {:>4} | {} {:>10.2} @ {:>10.2} (slippage: {:.4})",
                fill.timestamp as usize, side_str, fill.symbol, fill.qty, fill.price, fill.slippage
            );
        }
    }

    // Call compute_metrics through the interpreter for final metrics
    let fills_as_values: Vec<Value> = all_fills.iter().map(|f| {
        let mut fields = HashMap::new();
        fields.insert("order_id".to_string(), Value::Int(f.order_id));
        fields.insert("symbol".to_string(), Value::Str(f.symbol.clone()));
        fields.insert("side".to_string(), if f.side_is_buy {
            Value::Enum { enum_name: "OrderSide".to_string(), variant_name: "Buy".to_string(), fields: vec![] }
        } else {
            Value::Enum { enum_name: "OrderSide".to_string(), variant_name: "Sell".to_string(), fields: vec![] }
        });
        fields.insert("price".to_string(), Value::Float(f.price));
        fields.insert("qty".to_string(), Value::Float(f.qty));
        fields.insert("timestamp".to_string(), Value::Float(f.timestamp));
        fields.insert("slippage".to_string(), Value::Float(f.slippage));
        Value::Struct { type_name: "Fill".to_string(), fields }
    }).collect();

    let equity_values: Vec<Value> = equity_curve.iter().map(|e| Value::Float(*e)).collect();

    let metrics_result = call_function(
        &mut interpreter,
        &mut locals,
        "compute_metrics",
        vec![Value::List(fills_as_values), Value::List(equity_values)],
    );

    // Print metrics summary
    println!("\n--- Engine Backtest Summary (Fidelity {}) ---", fidelity);
    println!("  Initial Capital:   {:>12.2}", initial_capital);
    println!("  Final Equity:      {:>12.2}", current_equity);

    let realized_pnl = compute_realized_pnl_from_fills(&all_fills);
    let total_return = if initial_capital > 0.0 {
        ((current_equity - initial_capital) / initial_capital) * 100.0
    } else {
        0.0
    };
    println!("  Realized P&L:      {:>12.2}", realized_pnl);
    println!("  Total Return:      {:>11.2}%", total_return);
    println!("  Total Fills:       {:>12}", all_fills.len());

    // Print metrics from compute_metrics if available
    if let Ok(Value::Struct { fields, .. }) = &metrics_result {
        if let Some(Value::Float(sharpe)) = fields.get("sharpe_ratio") {
            println!("  Sharpe Ratio:      {:>12.4}", sharpe);
        }
        if let Some(Value::Float(max_dd)) = fields.get("max_drawdown_pct") {
            println!("  Max Drawdown:      {:>11.2}%", max_dd * 100.0);
        }
        if let Some(Value::Float(win_rate)) = fields.get("win_rate") {
            println!("  Win Rate:          {:>11.2}%", win_rate * 100.0);
        }
        if let Some(Value::Float(pf)) = fields.get("profit_factor") {
            println!("  Profit Factor:     {:>12.4}", pf);
        }
        if let Some(Value::Float(avg_pnl)) = fields.get("avg_trade_pnl") {
            println!("  Avg Trade P&L:     {:>12.2}", avg_pnl);
        }
        if let Some(total_trades) = fields.get("total_trades") {
            println!("  Total Trades:      {:>12}", value_to_i64(total_trades));
        }
    } else if let Err(e) = &metrics_result {
        eprintln!("  (metrics computation unavailable: {})", e);
    }

    // Print signal summary
    println!("\n{}", format_summary(&results));

    Ok(())
}

/// Compute realized P&L from fills by pairing buys with sells per symbol.
///
/// Uses FIFO matching: each sell fill's P&L is computed against a volume-weighted
/// average entry price from preceding buy fills for the same symbol.
fn compute_realized_pnl_from_fills(fills: &[EngineFill]) -> f64 {
    let mut positions: HashMap<String, (f64, f64)> = HashMap::new(); // symbol -> (qty, avg_price)
    let mut realized = 0.0;

    for fill in fills {
        if fill.side_is_buy {
            let entry = positions.entry(fill.symbol.clone()).or_insert((0.0, 0.0));
            let new_qty = entry.0 + fill.qty;
            let new_avg = if new_qty > 0.0 {
                (entry.1 * entry.0 + fill.price * fill.qty) / new_qty
            } else {
                fill.price
            };
            *entry = (new_qty, new_avg);
        } else {
            if let Some(entry) = positions.get_mut(&fill.symbol) {
                let pnl = (fill.price - entry.1) * fill.qty;
                realized += pnl;
                entry.0 -= fill.qty;
                if entry.0 <= 0.0 {
                    positions.remove(&fill.symbol);
                }
            }
        }
    }

    realized
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
            "--- Summary ---\nTotal signals: 0\nOpen: 0\nShort: 0\nClose: 0\nCloseQty: 0"
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
            "--- Summary ---\nTotal signals: 5\nOpen: 2\nShort: 0\nClose: 2\nCloseQty: 1"
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
            "--- Summary ---\nTotal signals: 3\nOpen: 3\nShort: 0\nClose: 0\nCloseQty: 0"
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

    // --- signal_to_order_params unit tests ---

    #[test]
    fn test_signal_to_order_params_open() {
        let signal = Signal::Open { symbol: "AAPL".to_string(), qty: 100.0 };
        let mut next_id = 0;
        let result = signal_to_order_params(&signal, &mut next_id, 0.0);
        assert!(result.is_some());
        let params = result.unwrap();
        assert_eq!(params.symbol, "AAPL");
        assert!(params.side_is_buy);
        assert_eq!(params.qty, 100.0);
        assert_eq!(params.order_id, 0);
        assert_eq!(next_id, 1);
    }

    #[test]
    fn test_signal_to_order_params_close() {
        let signal = Signal::Close { symbol: "AAPL".to_string() };
        let mut next_id = 5;
        let result = signal_to_order_params(&signal, &mut next_id, 50.0);
        assert!(result.is_some());
        let params = result.unwrap();
        assert_eq!(params.symbol, "AAPL");
        assert!(!params.side_is_buy);
        assert_eq!(params.qty, 50.0); // full position qty
        assert_eq!(params.order_id, 5);
        assert_eq!(next_id, 6);
    }

    #[test]
    fn test_signal_to_order_params_close_no_position() {
        let signal = Signal::Close { symbol: "AAPL".to_string() };
        let mut next_id = 0;
        let result = signal_to_order_params(&signal, &mut next_id, 0.0);
        assert!(result.is_none());
        assert_eq!(next_id, 0); // ID not consumed
    }

    #[test]
    fn test_signal_to_order_params_close_negative_position() {
        // Closing a short position: should buy to cover
        let signal = Signal::Close { symbol: "AAPL".to_string() };
        let mut next_id = 3;
        let result = signal_to_order_params(&signal, &mut next_id, -10.0);
        assert!(result.is_some());
        let params = result.unwrap();
        assert_eq!(params.symbol, "AAPL");
        assert!(params.side_is_buy); // Buy to cover
        assert_eq!(params.qty, 10.0); // abs(-10)
        assert_eq!(next_id, 4);
    }

    #[test]
    fn test_signal_to_order_params_close_qty() {
        let signal = Signal::CloseQty { symbol: "MSFT".to_string(), qty: 25.0 };
        let mut next_id = 10;
        let result = signal_to_order_params(&signal, &mut next_id, 100.0);
        assert!(result.is_some());
        let params = result.unwrap();
        assert_eq!(params.symbol, "MSFT");
        assert!(!params.side_is_buy);
        assert_eq!(params.qty, 25.0);
        assert_eq!(params.order_id, 10);
        assert_eq!(next_id, 11);
    }

    #[test]
    fn test_signal_to_order_params_sequential_ids() {
        let mut next_id = 0;
        let s1 = Signal::Open { symbol: "A".to_string(), qty: 10.0 };
        let s2 = Signal::Open { symbol: "B".to_string(), qty: 20.0 };
        let s3 = Signal::CloseQty { symbol: "A".to_string(), qty: 5.0 };
        let r1 = signal_to_order_params(&s1, &mut next_id, 0.0).unwrap();
        let r2 = signal_to_order_params(&s2, &mut next_id, 0.0).unwrap();
        let r3 = signal_to_order_params(&s3, &mut next_id, 10.0).unwrap();
        assert_eq!(r1.order_id, 0);
        assert_eq!(r2.order_id, 1);
        assert_eq!(r3.order_id, 2);
        assert_eq!(next_id, 3);
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
            ("[A-Z]{1,5}", qty.clone()).prop_map(|(s, q)| Signal::Short {
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
                Signal::Short { symbol, qty } => {
                    prop_assert!(
                        output.contains("Short"),
                        "Output {:?} does not contain 'Short'",
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
