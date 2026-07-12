//! Property-based tests for Portfolio Construction.
//!
//! This file contains property tests validating universal correctness properties
//! defined in the design document for the portfolio-construction spec.

use std::collections::HashMap;

use proptest::prelude::*;

use flux_compiler::lexer::Span;
use flux_compiler::typeck::typed_ast::*;
use flux_compiler::typeck::types::{FluxType, FnParams};

use flux_cli::commands::backtest::group_bars_by_timestamp;
use flux_cli::interpreter::{Interpreter, Value};
use flux_runtime::{BarContext, PositionTracker, Signal};

// =============================================================================
// Helpers
// =============================================================================

/// Build a minimal TypedProgram with an empty on_bar handler (no strategy logic).
/// The interpreter just needs a valid program to initialize — we'll call eval_expr directly.
fn build_empty_strategy() -> TypedProgram {
    TypedProgram {
        imports: vec![],
            structs: vec![],
            enums: vec![],
        functions: vec![],
        impl_blocks: vec![],
            traits: vec![],
        data_block: None,
        connector_block: None,
        strategy: TypedStrategy {
            name: "Empty".to_string(),
            body: vec![TypedStrategyItem::EventHandler(TypedEventHandler {
                event_name: "bar".to_string(),
                body: vec![],
                span: Span::new(0, 0),
            })],
            span: Span::new(0, 0),
        },
        span: Span::new(0, 0),
    }
}

/// Construct a TypedExpr representing a VecFloat literal from a Vec<f64>.
fn make_vecfloat_literal(values: &[f64]) -> TypedExpr {
    let items: Vec<TypedExpr> = values
        .iter()
        .enumerate()
        .map(|(i, &v)| TypedExpr {
            kind: TypedExprKind::FloatLiteral(v),
            resolved_type: FluxType::Float,
            span: Span::new(i * 10, i * 10 + 5),
        })
        .collect();

    TypedExpr {
        kind: TypedExprKind::ListLiteral(items),
        resolved_type: FluxType::VecFloat,
        span: Span::new(0, 1000),
    }
}

// =============================================================================
// Property 4: VecFloat Indexing Correctness
// Feature: portfolio-construction, Property 4: VecFloat Indexing Correctness
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 2.1, 2.2, 2.3, 2.4**
    ///
    /// For any VecFloat value of length N and any integer index i:
    /// - If 0 <= i < N: indexing returns the element at position i as Value::Float
    /// - If i < 0 or i >= N: indexing returns an error containing "out of bounds"
    #[test]
    fn prop_vecfloat_indexing(
        values in proptest::collection::vec(
            prop::num::f64::ANY.prop_filter("filter NaN and Inf", |v| v.is_finite()),
            1..=50usize
        ),
        // Generate an index that can be valid, negative, or out-of-bounds
        raw_index in -10i64..60i64,
    ) {
        let program = build_empty_strategy();
        let mut interp = Interpreter::new(&program);
        let mut locals: HashMap<String, Value> = HashMap::new();

        let len = values.len() as i64;

        // Construct the IndexAccess expression: vecfloat_literal[raw_index]
        let object_expr = make_vecfloat_literal(&values);
        let index_expr = TypedExpr {
            kind: TypedExprKind::IntLiteral(raw_index),
            resolved_type: FluxType::Int,
            span: Span::new(2000, 2010),
        };

        let index_access_expr = TypedExpr {
            kind: TypedExprKind::IndexAccess {
                object: Box::new(object_expr),
                index: Box::new(index_expr),
            },
            resolved_type: FluxType::Float,
            span: Span::new(0, 3000),
        };

        // Evaluate through the interpreter
        let result = interp.eval_expr(&index_access_expr, &mut locals);

        if raw_index >= 0 && raw_index < len {
            // Valid index: should return the element at that position
            let val = result.expect("valid index should not error");
            match val {
                Value::Float(f) => {
                    prop_assert_eq!(
                        f, values[raw_index as usize],
                        "Element at index {} mismatch: got {}, expected {}",
                        raw_index, f, values[raw_index as usize]
                    );
                }
                other => {
                    prop_assert!(false, "Expected Value::Float, got {:?}", other);
                }
            }
        } else {
            // Invalid index (negative or out-of-bounds): should return an error
            let err = result.expect_err(
                &format!("index {} with len {} should produce an error", raw_index, len)
            );
            prop_assert!(
                err.contains("out of bounds"),
                "Error message should contain 'out of bounds', got: {}",
                err
            );
        }
    }
}

// =============================================================================
// Property 8: ret() Computes Correct Simple Return
// Feature: portfolio-construction, Property 8: ret() Computes Correct Simple Return
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 4.1, 4.2, 4.4, 4.6, 4.7**
    ///
    /// For any symbol with prev_close and current_close set, ret(symbol) SHALL return
    /// (current_close / prev_close) - 1.0.
    /// For a missing symbol, ret() SHALL return 0.0.
    /// For prev_close == 0.0, ret() SHALL return 0.0.
    /// Calling ret(symbol) multiple times within the same bar SHALL return the same value.
    #[test]
    fn prop_ret_computes_correct_simple_return(
        sym in "[a-zA-Z]{1,4}",
        prev_close in 0.01f64..10000.0f64,
        current_close in 0.01f64..10000.0f64,
    ) {
        let program = build_empty_strategy();
        let mut interp = Interpreter::new(&program);
        let mut locals: HashMap<String, Value> = HashMap::new();

        // Set up price state: prev_closes and current_closes for the symbol
        interp.prev_closes.insert(sym.clone(), prev_close);
        interp.current_closes.insert(sym.clone(), current_close);

        // Construct ret(sym) function call expression
        let func_ident = TypedExpr {
            kind: TypedExprKind::Ident("ret".to_string()),
            resolved_type: FluxType::Fn {
                params: FnParams::Fixed(vec![FluxType::String]),
                ret: Box::new(FluxType::Float),
            },
            span: Span::new(0, 3),
        };

        let arg_expr = TypedExpr {
            kind: TypedExprKind::StringLiteral(sym.clone()),
            resolved_type: FluxType::String,
            span: Span::new(4, 4 + sym.len()),
        };

        let call_expr = TypedExpr {
            kind: TypedExprKind::FunctionCall {
                function: Box::new(func_ident),
                args: vec![arg_expr],
            },
            resolved_type: FluxType::Float,
            span: Span::new(0, 100),
        };

        // Evaluate ret(sym)
        let result = interp.eval_expr(&call_expr, &mut locals)
            .expect("ret() should not error for valid symbol with prices set");

        let expected = (current_close / prev_close) - 1.0;
        match result {
            Value::Float(f) => {
                prop_assert!(
                    (f - expected).abs() < 1e-10,
                    "ret({}) = {} but expected ({}/ {} ) - 1.0 = {}",
                    sym, f, current_close, prev_close, expected
                );
            }
            other => {
                prop_assert!(false, "Expected Value::Float, got {:?}", other);
            }
        }

        // Verify idempotency: calling ret again yields same value
        let func_ident2 = TypedExpr {
            kind: TypedExprKind::Ident("ret".to_string()),
            resolved_type: FluxType::Fn {
                params: FnParams::Fixed(vec![FluxType::String]),
                ret: Box::new(FluxType::Float),
            },
            span: Span::new(0, 3),
        };
        let arg_expr2 = TypedExpr {
            kind: TypedExprKind::StringLiteral(sym.clone()),
            resolved_type: FluxType::String,
            span: Span::new(4, 4 + sym.len()),
        };
        let call_expr2 = TypedExpr {
            kind: TypedExprKind::FunctionCall {
                function: Box::new(func_ident2),
                args: vec![arg_expr2],
            },
            resolved_type: FluxType::Float,
            span: Span::new(200, 300),
        };
        let result2 = interp.eval_expr(&call_expr2, &mut locals)
            .expect("second ret() call should not error");
        match result2 {
            Value::Float(f2) => {
                prop_assert!(
                    (f2 - expected).abs() < 1e-10,
                    "ret({}) second call = {} but expected {}",
                    sym, f2, expected
                );
            }
            other => {
                prop_assert!(false, "Expected Value::Float on second call, got {:?}", other);
            }
        }

        // Edge case: missing symbol → 0.0
        let missing_sym = format!("{}_MISSING", sym);
        let func_ident3 = TypedExpr {
            kind: TypedExprKind::Ident("ret".to_string()),
            resolved_type: FluxType::Fn {
                params: FnParams::Fixed(vec![FluxType::String]),
                ret: Box::new(FluxType::Float),
            },
            span: Span::new(0, 3),
        };
        let arg_expr3 = TypedExpr {
            kind: TypedExprKind::StringLiteral(missing_sym.clone()),
            resolved_type: FluxType::String,
            span: Span::new(4, 20),
        };
        let call_expr3 = TypedExpr {
            kind: TypedExprKind::FunctionCall {
                function: Box::new(func_ident3),
                args: vec![arg_expr3],
            },
            resolved_type: FluxType::Float,
            span: Span::new(300, 400),
        };
        let result3 = interp.eval_expr(&call_expr3, &mut locals)
            .expect("ret() with missing symbol should not error");
        match result3 {
            Value::Float(f) => {
                prop_assert_eq!(f, 0.0, "ret() with missing symbol should return 0.0, got {}", f);
            }
            other => {
                prop_assert!(false, "Expected Value::Float(0.0) for missing symbol, got {:?}", other);
            }
        }

        // Edge case: prev_close == 0.0 → 0.0
        let zero_sym = format!("{}_ZERO", sym);
        interp.prev_closes.insert(zero_sym.clone(), 0.0);
        interp.current_closes.insert(zero_sym.clone(), current_close);
        let func_ident4 = TypedExpr {
            kind: TypedExprKind::Ident("ret".to_string()),
            resolved_type: FluxType::Fn {
                params: FnParams::Fixed(vec![FluxType::String]),
                ret: Box::new(FluxType::Float),
            },
            span: Span::new(0, 3),
        };
        let arg_expr4 = TypedExpr {
            kind: TypedExprKind::StringLiteral(zero_sym.clone()),
            resolved_type: FluxType::String,
            span: Span::new(4, 20),
        };
        let call_expr4 = TypedExpr {
            kind: TypedExprKind::FunctionCall {
                function: Box::new(func_ident4),
                args: vec![arg_expr4],
            },
            resolved_type: FluxType::Float,
            span: Span::new(400, 500),
        };
        let result4 = interp.eval_expr(&call_expr4, &mut locals)
            .expect("ret() with prev_close=0 should not error");
        match result4 {
            Value::Float(f) => {
                prop_assert_eq!(f, 0.0, "ret() with prev_close=0.0 should return 0.0, got {}", f);
            }
            other => {
                prop_assert!(false, "Expected Value::Float(0.0) for prev_close=0, got {:?}", other);
            }
        }
    }
}

// =============================================================================
// Property 2: VecFloat Evaluation Preserves Elements
// Feature: portfolio-construction, Property 2: VecFloat Evaluation Preserves Elements
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 1.4, 1.7**
    ///
    /// For any sequence of numeric values of length N (where 1 ≤ N ≤ 100),
    /// evaluating them as a VecFloat literal SHALL produce a Value::VecFloat
    /// containing exactly N elements in the same order with exact values preserved.
    #[test]
    fn prop_vecfloat_evaluation_preserves_elements(
        values in proptest::collection::vec(
            prop::num::f64::ANY.prop_filter("filter NaN and Inf", |v| v.is_finite()),
            1..=100
        ),
    ) {
        let program = build_empty_strategy();
        let mut interp = Interpreter::new(&program);
        let mut locals: HashMap<String, Value> = HashMap::new();

        // Build the VecFloat literal expression from the generated values
        let expr = make_vecfloat_literal(&values);

        // Evaluate the expression through the interpreter
        let result = interp.eval_expr(&expr, &mut locals)
            .expect("VecFloat literal evaluation should not error");

        // Assert the result is Value::VecFloat with the same elements
        match result {
            Value::VecFloat(ref result_vec) => {
                // Same length
                prop_assert_eq!(
                    result_vec.len(),
                    values.len(),
                    "VecFloat length mismatch: got {}, expected {}",
                    result_vec.len(),
                    values.len()
                );

                // Same elements in same order (exact equality — no rounding for literals)
                for (i, (&actual, &expected)) in result_vec.iter().zip(values.iter()).enumerate() {
                    prop_assert_eq!(
                        actual, expected,
                        "Element {} mismatch: got {}, expected {}",
                        i, actual, expected
                    );
                }
            }
            other => {
                prop_assert!(
                    false,
                    "Expected Value::VecFloat, got {:?}",
                    other
                );
            }
        }
    }
}

// =============================================================================
// Property 9: Signal Dispatch Preserves Symbol and Quantity
// Feature: portfolio-construction, Property 9: Signal Dispatch Preserves Symbol and Quantity
// =============================================================================

/// Helper to construct a TypedExpr for a function call (e.g., OPEN, CLOSE, CLOSE_QTY).
fn make_fn_call(name: &str, args: Vec<TypedExpr>, ty: FluxType) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::FunctionCall {
            function: Box::new(TypedExpr {
                kind: TypedExprKind::Ident(name.to_string()),
                resolved_type: FluxType::Fn {
                    params: FnParams::Fixed(vec![]),
                    ret: Box::new(ty.clone()),
                },
                span: Span::new(0, name.len()),
            }),
            args,
        },
        resolved_type: ty,
        span: Span::new(0, 100),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 5.1, 5.2, 5.3**
    ///
    /// For any non-empty symbol string S and any positive quantity Q:
    /// - OPEN(S, Q) emits Signal::Open { symbol: S, qty: Q }
    /// - CLOSE(S) emits Signal::Close { symbol: S }
    /// - CLOSE_QTY(S, Q) emits Signal::CloseQty { symbol: S, qty: Q }
    #[test]
    fn prop_signal_dispatch_preserves_symbol_and_quantity(
        symbol in "[a-zA-Z]{1,8}",
        qty in 0.01f64..10000.0f64,
    ) {
        let program = build_empty_strategy();
        let mut interp = Interpreter::new(&program);
        let mut locals: HashMap<String, Value> = HashMap::new();

        // --- Test OPEN(symbol, qty) ---
        let open_expr = make_fn_call(
            "OPEN",
            vec![
                TypedExpr {
                    kind: TypedExprKind::StringLiteral(symbol.clone()),
                    resolved_type: FluxType::String,
                    span: Span::new(10, 20),
                },
                TypedExpr {
                    kind: TypedExprKind::FloatLiteral(qty),
                    resolved_type: FluxType::Float,
                    span: Span::new(22, 30),
                },
            ],
            FluxType::Signal,
        );
        let open_result = interp.eval_expr(&open_expr, &mut locals)
            .expect("OPEN should not error for valid symbol and qty");
        match open_result {
            Value::Signal(Signal::Open { symbol: s, qty: q }) => {
                prop_assert_eq!(&s, &symbol, "OPEN signal symbol mismatch");
                prop_assert_eq!(q, qty, "OPEN signal qty mismatch");
            }
            other => {
                prop_assert!(false, "Expected Value::Signal(Signal::Open{{..}}), got {:?}", other);
            }
        }

        // --- Test CLOSE(symbol) ---
        let close_expr = make_fn_call(
            "CLOSE",
            vec![
                TypedExpr {
                    kind: TypedExprKind::StringLiteral(symbol.clone()),
                    resolved_type: FluxType::String,
                    span: Span::new(10, 20),
                },
            ],
            FluxType::Signal,
        );
        let close_result = interp.eval_expr(&close_expr, &mut locals)
            .expect("CLOSE should not error for valid symbol");
        match close_result {
            Value::Signal(Signal::Close { symbol: s }) => {
                prop_assert_eq!(&s, &symbol, "CLOSE signal symbol mismatch");
            }
            other => {
                prop_assert!(false, "Expected Value::Signal(Signal::Close{{..}}), got {:?}", other);
            }
        }

        // --- Test CLOSE_QTY(symbol, qty) ---
        let close_qty_expr = make_fn_call(
            "CLOSE_QTY",
            vec![
                TypedExpr {
                    kind: TypedExprKind::StringLiteral(symbol.clone()),
                    resolved_type: FluxType::String,
                    span: Span::new(10, 20),
                },
                TypedExpr {
                    kind: TypedExprKind::FloatLiteral(qty),
                    resolved_type: FluxType::Float,
                    span: Span::new(22, 30),
                },
            ],
            FluxType::Signal,
        );
        let close_qty_result = interp.eval_expr(&close_qty_expr, &mut locals)
            .expect("CLOSE_QTY should not error for valid symbol and qty");
        match close_qty_result {
            Value::Signal(Signal::CloseQty { symbol: s, qty: q }) => {
                prop_assert_eq!(&s, &symbol, "CLOSE_QTY signal symbol mismatch");
                prop_assert_eq!(q, qty, "CLOSE_QTY signal qty mismatch");
            }
            other => {
                prop_assert!(false, "Expected Value::Signal(Signal::CloseQty{{..}}), got {:?}", other);
            }
        }
    }
}

// =============================================================================
// Property 7: Missing Symbols Not Forward-Filled
// Feature: portfolio-construction, Property 7: Missing Symbols Not Forward-Filled
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 3.3**
    ///
    /// For any sequence of bar groups where a symbol appears in group G1 but not
    /// in group G2 (G2 after G1), the accessible bar data in G2 SHALL NOT contain
    /// that symbol's data. This verifies no forward-filling behavior.
    #[test]
    fn prop_missing_symbols_not_forward_filled(
        // Generate a "missing" symbol that appears only in the first timestamp group
        missing_sym in "[A-Z]{1,4}",
        // Generate a "present" symbol that appears in both groups
        present_sym in "[A-Z]{1,4}".prop_filter("must differ from missing", |s| s.len() > 0),
        // Prices for the missing symbol in group 1
        missing_close in 1.0f64..1000.0f64,
        missing_open in 1.0f64..1000.0f64,
        missing_high in 1.0f64..1000.0f64,
        missing_low in 1.0f64..1000.0f64,
        missing_volume in 100.0f64..1_000_000.0f64,
        // Prices for the present symbol in group 1
        present_close1 in 1.0f64..1000.0f64,
        present_open1 in 1.0f64..1000.0f64,
        present_high1 in 1.0f64..1000.0f64,
        present_low1 in 1.0f64..1000.0f64,
        present_volume1 in 100.0f64..1_000_000.0f64,
        // Prices for the present symbol in group 2
        present_close2 in 1.0f64..1000.0f64,
        present_open2 in 1.0f64..1000.0f64,
        present_high2 in 1.0f64..1000.0f64,
        present_low2 in 1.0f64..1000.0f64,
        present_volume2 in 100.0f64..1_000_000.0f64,
    ) {
        // Ensure symbols are actually different
        prop_assume!(missing_sym != present_sym);

        // Build bars: group 1 has both symbols, group 2 has only the present symbol
        let bars = vec![
            // Group 1: missing_sym
            BarContext {
                close: missing_close,
                open: missing_open,
                high: missing_high,
                low: missing_low,
                volume: missing_volume,
                symbol: missing_sym.clone(),
                in_position: false,
            },
            // Group 1: present_sym
            BarContext {
                close: present_close1,
                open: present_open1,
                high: present_high1,
                low: present_low1,
                volume: present_volume1,
                symbol: present_sym.clone(),
                in_position: false,
            },
            // Group 2: only present_sym (missing_sym is absent)
            BarContext {
                close: present_close2,
                open: present_open2,
                high: present_high2,
                low: present_low2,
                volume: present_volume2,
                symbol: present_sym.clone(),
                in_position: false,
            },
        ];

        let timestamps = vec![
            "2024-01-01".to_string(),
            "2024-01-01".to_string(),
            "2024-01-02".to_string(),
        ];

        let groups = group_bars_by_timestamp(&bars, &timestamps)
            .expect("grouping should succeed");

        // Should produce exactly 2 groups
        prop_assert_eq!(groups.len(), 2, "Expected 2 groups, got {}", groups.len());

        // Group 1 should contain both symbols
        prop_assert!(
            groups[0].closes.contains_key(&missing_sym),
            "Group 1 should contain the missing symbol's close data"
        );
        prop_assert!(
            groups[0].closes.contains_key(&present_sym),
            "Group 1 should contain the present symbol's close data"
        );

        // Group 2 should NOT contain the missing symbol
        prop_assert!(
            !groups[1].closes.contains_key(&missing_sym),
            "Group 2 closes should NOT contain the missing symbol '{}' (no forward-fill)",
            missing_sym
        );

        // Group 2 bars should NOT contain a bar with the missing symbol
        let has_missing_bar = groups[1].bars.iter().any(|b| b.symbol == missing_sym);
        prop_assert!(
            !has_missing_bar,
            "Group 2 bars should NOT contain a bar with the missing symbol '{}' (no forward-fill)",
            missing_sym
        );

        // Sanity: Group 2 should contain the present symbol
        prop_assert!(
            groups[1].closes.contains_key(&present_sym),
            "Group 2 should contain the present symbol's close data"
        );
    }
}

// =============================================================================
// Property 6: Bar Grouping Correctness
// Feature: portfolio-construction, Property 6: Bar Grouping Correctness
// =============================================================================

/// Strategy to generate multi-symbol bar data grouped by timestamp.
///
/// Generates 2-5 distinct timestamps and 1-4 distinct symbols. For each timestamp,
/// a subset of symbols is selected (at least one), and bars are generated for those
/// symbols. Bars for the same timestamp are placed consecutively (as they would be
/// in a CSV file).
fn arb_multi_symbol_bar_data() -> impl Strategy<Value = (Vec<BarContext>, Vec<String>)> {
    // Generate distinct timestamps (2-5)
    let timestamps_strat = proptest::collection::vec("[0-9]{4}-[0-9]{2}-[0-9]{2}", 2..=5)
        .prop_map(|ts_vec| {
            // Ensure distinct timestamps
            let mut seen = std::collections::HashSet::new();
            ts_vec
                .into_iter()
                .filter(|t| seen.insert(t.clone()))
                .collect::<Vec<_>>()
        })
        .prop_filter("need at least 2 distinct timestamps", |ts| ts.len() >= 2);

    // Generate distinct symbols (1-4)
    let symbols_strat = proptest::collection::vec("[A-Z]{2,4}", 1..=4).prop_map(|sym_vec| {
        let mut seen = std::collections::HashSet::new();
        sym_vec
            .into_iter()
            .filter(|s| seen.insert(s.clone()))
            .collect::<Vec<_>>()
    });

    (timestamps_strat, symbols_strat).prop_flat_map(|(timestamps, symbols)| {
        // For each timestamp, generate bars for a non-empty subset of symbols
        let n_timestamps = timestamps.len();
        let n_symbols = symbols.len();

        // Generate which symbols appear in each timestamp group
        // Each group gets a bitmask ensuring at least one symbol is present
        let bitmasks = proptest::collection::vec(1u32..(1u32 << n_symbols), n_timestamps);

        // Generate prices for each bar
        let max_bars = n_timestamps * n_symbols;
        let closes = proptest::collection::vec(1.0f64..1000.0, max_bars);
        let opens = proptest::collection::vec(1.0f64..1000.0, max_bars);
        let highs = proptest::collection::vec(1.0f64..1000.0, max_bars);
        let lows = proptest::collection::vec(1.0f64..1000.0, max_bars);
        let volumes = proptest::collection::vec(100.0f64..1_000_000.0, max_bars);

        (
            Just(timestamps),
            Just(symbols),
            bitmasks,
            closes,
            opens,
            highs,
            lows,
            volumes,
        )
            .prop_map(
                |(timestamps, symbols, bitmasks, closes, opens, highs, lows, volumes)| {
                    let mut bars: Vec<BarContext> = Vec::new();
                    let mut ts_vec: Vec<String> = Vec::new();
                    let mut price_idx = 0;

                    for (ts_idx, ts) in timestamps.iter().enumerate() {
                        let mask = bitmasks[ts_idx];
                        for (sym_idx, sym) in symbols.iter().enumerate() {
                            if mask & (1 << sym_idx) != 0 {
                                let close = closes[price_idx % closes.len()];
                                let open = opens[price_idx % opens.len()];
                                let high = highs[price_idx % highs.len()];
                                let low = lows[price_idx % lows.len()];
                                let volume = volumes[price_idx % volumes.len()];
                                price_idx += 1;

                                bars.push(BarContext {
                                    close,
                                    open,
                                    high,
                                    low,
                                    volume,
                                    symbol: sym.clone(),
                                    in_position: false,
                                });
                                ts_vec.push(ts.clone());
                            }
                        }
                    }

                    (bars, ts_vec)
                },
            )
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 3.1, 3.2, 3.4, 3.5**
    ///
    /// For any multi-symbol bar data where rows are ordered by timestamp (with
    /// consecutive same-timestamp rows), the bar grouping function SHALL produce
    /// groups where:
    /// (a) each group contains exactly the bars sharing that timestamp
    /// (b) groups are in first-occurrence order
    /// (c) single-symbol data produces one bar per group
    /// (d) bar order within each group matches CSV row order
    #[test]
    fn prop_bar_grouping(
        (bars, timestamps) in arb_multi_symbol_bar_data()
    ) {
        let result = group_bars_by_timestamp(&bars, &timestamps)
            .expect("group_bars_by_timestamp should not error for valid input");

        // Collect distinct timestamps in first-occurrence order from input
        let mut expected_ts_order: Vec<String> = Vec::new();
        let mut seen_ts = std::collections::HashSet::new();
        for ts in &timestamps {
            if seen_ts.insert(ts.clone()) {
                expected_ts_order.push(ts.clone());
            }
        }

        // (a) + assertion: number of groups == number of distinct timestamps
        prop_assert_eq!(
            result.len(),
            expected_ts_order.len(),
            "Number of groups ({}) should equal number of distinct timestamps ({})",
            result.len(),
            expected_ts_order.len()
        );

        // (b) Groups are in first-occurrence order of timestamps
        for (i, group) in result.iter().enumerate() {
            prop_assert_eq!(
                &group.timestamp,
                &expected_ts_order[i],
                "Group {} has timestamp '{}' but expected '{}'",
                i,
                group.timestamp,
                expected_ts_order[i]
            );
        }

        // (a) Each group contains exactly the bars sharing that timestamp
        for group in &result {
            // All bars in this group have the correct timestamp
            let expected_bars: Vec<&BarContext> = bars
                .iter()
                .zip(timestamps.iter())
                .filter(|(_, ts)| **ts == group.timestamp)
                .map(|(bar, _)| bar)
                .collect();

            prop_assert_eq!(
                group.bars.len(),
                expected_bars.len(),
                "Group '{}' has {} bars but expected {}",
                group.timestamp,
                group.bars.len(),
                expected_bars.len()
            );

            // (d) Bar order within each group matches CSV row order
            for (j, (actual, expected)) in
                group.bars.iter().zip(expected_bars.iter()).enumerate()
            {
                prop_assert_eq!(
                    &actual.symbol,
                    &expected.symbol,
                    "Group '{}' bar {} has symbol '{}' but expected '{}'",
                    group.timestamp,
                    j,
                    actual.symbol,
                    expected.symbol
                );
                prop_assert_eq!(
                    actual.close,
                    expected.close,
                    "Group '{}' bar {} has close {} but expected {}",
                    group.timestamp,
                    j,
                    actual.close,
                    expected.close
                );
            }
        }

        // (c) Single-symbol data produces one bar per group
        // Check: if all bars share the same symbol, every group has exactly 1 bar
        let all_symbols: std::collections::HashSet<&str> =
            bars.iter().map(|b| b.symbol.as_str()).collect();
        if all_symbols.len() == 1 {
            for group in &result {
                prop_assert_eq!(
                    group.bars.len(),
                    1,
                    "Single-symbol data: group '{}' should have exactly 1 bar, got {}",
                    group.timestamp,
                    group.bars.len()
                );
            }
        }
    }
}

// =============================================================================
// Property 10: in_position Reflects Any Open Position
// Feature: portfolio-construction, Property 10: in_position Reflects Any Open Position
// =============================================================================

/// Strategy to generate a sequence of OPEN/CLOSE signals for multiple symbols.
///
/// Generates 2-5 distinct symbols and a sequence of 2-10 OPEN/CLOSE signals
/// targeting those symbols. Each signal is either an OPEN with a positive qty
/// or a CLOSE for one of the generated symbols.
fn arb_open_close_signal_sequence() -> impl Strategy<Value = (Vec<String>, Vec<Signal>)> {
    // Generate 2-5 distinct symbols
    let symbols_strat = proptest::collection::vec("[A-Z]{2,4}", 2..=5)
        .prop_map(|sym_vec| {
            let mut seen = std::collections::HashSet::new();
            sym_vec
                .into_iter()
                .filter(|s| seen.insert(s.clone()))
                .collect::<Vec<_>>()
        })
        .prop_filter("need at least 2 distinct symbols", |syms| syms.len() >= 2);

    symbols_strat.prop_flat_map(|symbols| {
        let n_symbols = symbols.len();
        // Generate 2-10 signals, each picking a random symbol index and signal type
        let signals_strat = proptest::collection::vec(
            (0..n_symbols, prop::bool::ANY, 1.0f64..1000.0f64),
            2..=10,
        );

        (Just(symbols), signals_strat)
    })
    .prop_map(|(symbols, raw_signals)| {
        let signals: Vec<Signal> = raw_signals
            .into_iter()
            .map(|(sym_idx, is_open, qty)| {
                let symbol = symbols[sym_idx].clone();
                if is_open {
                    Signal::open(symbol, qty)
                } else {
                    Signal::close(symbol)
                }
            })
            .collect();
        (symbols, signals)
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 6.1, 6.2, 6.5**
    ///
    /// For any sequence of OPEN/CLOSE signals processed by the PositionTracker,
    /// `open_position_count() > 0` SHALL be true if and only if at least one symbol
    /// has a position with quantity > 0.
    #[test]
    fn prop_in_position(
        (symbols, signals) in arb_open_close_signal_sequence(),
        fill_price in 1.0f64..10000.0f64,
    ) {
        let mut tracker = PositionTracker::new(100_000.0);

        // Track expected qty per symbol manually
        let mut expected_qty: HashMap<String, f64> = HashMap::new();
        for sym in &symbols {
            expected_qty.insert(sym.clone(), 0.0);
        }

        for (bar_index, signal) in signals.iter().enumerate() {
            // Process the signal through the tracker
            tracker.process_signals(&[signal.clone()], fill_price, bar_index);

            // Update our expected state
            match signal {
                Signal::Open { symbol, qty } => {
                    *expected_qty.entry(symbol.clone()).or_insert(0.0) += qty;
                }
                Signal::Close { symbol } => {
                    // Close removes the entire position
                    expected_qty.insert(symbol.clone(), 0.0);
                }
                Signal::CloseQty { symbol, qty } => {
                    let current = expected_qty.get(symbol).copied().unwrap_or(0.0);
                    let actual_close = qty.min(current);
                    expected_qty.insert(symbol.clone(), current - actual_close);
                }
            }

            // Check: open_position_count() > 0 should match whether any symbol has qty > 0
            let expected_in_position = expected_qty.values().any(|&q| q > 0.0);
            let actual_in_position = tracker.open_position_count() > 0;

            prop_assert_eq!(
                actual_in_position,
                expected_in_position,
                "After signal {:?} at bar {}: open_position_count() > 0 is {} but expected {}. \
                 Tracker count: {}, expected_qty: {:?}",
                signal,
                bar_index,
                actual_in_position,
                expected_in_position,
                tracker.open_position_count(),
                expected_qty,
            );
        }
    }
}

// =============================================================================
// Property 12: Distinct Open Signals Yield Correct Position Count
// Feature: portfolio-construction, Property 12: Distinct Open Signals Yield Correct Position Count
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 5.5, 8.5**
    ///
    /// For any N distinct symbol strings, emitting OPEN(symbol_i, qty_i) for each
    /// without any CLOSE signals SHALL result in open_position_count == N.
    #[test]
    fn prop_distinct_opens(
        symbols in proptest::collection::vec("[A-Z]{1,6}", 1..=10usize),
        qtys in proptest::collection::vec(0.01f64..10000.0f64, 10usize),
        capital in 1000.0f64..1_000_000.0f64,
        fill_price in 1.0f64..5000.0f64,
    ) {
        // Deduplicate symbols using a HashSet to ensure N distinct symbols
        let mut seen = std::collections::HashSet::new();
        let distinct_symbols: Vec<String> = symbols
            .into_iter()
            .filter(|s| seen.insert(s.clone()))
            .collect();

        // Need at least 1 distinct symbol
        prop_assume!(!distinct_symbols.is_empty());

        let n = distinct_symbols.len();

        // Create OPEN signals for each distinct symbol with a positive qty
        let signals: Vec<Signal> = distinct_symbols
            .iter()
            .enumerate()
            .map(|(i, sym)| Signal::Open {
                symbol: sym.clone(),
                qty: qtys[i % qtys.len()],
            })
            .collect();

        // Process all signals through a fresh PositionTracker
        let mut tracker = PositionTracker::new(capital);
        tracker.process_signals(&signals, fill_price, 0);

        // Assert: open_position_count == N (number of distinct symbols)
        prop_assert_eq!(
            tracker.open_position_count(),
            n,
            "After opening {} distinct symbols, open_position_count should be {}, got {}",
            n,
            n,
            tracker.open_position_count()
        );
    }
}

// =============================================================================
// Property 11: Multi-Asset Fill Prices Use Per-Symbol Close
// Feature: portfolio-construction, Property 11: Multi-Asset Fill Prices Use Per-Symbol Close
// =============================================================================

/// Strategy to generate 2-5 distinct symbols with different close prices.
fn arb_multi_asset_symbols_and_prices(
) -> impl Strategy<Value = Vec<(String, f64, f64)>> {
    // Generate 2-5 (symbol, close_price, qty) tuples with distinct symbols
    proptest::collection::vec(
        (
            "[A-Z]{1,5}".prop_map(|s| s.to_string()),
            1.0f64..10000.0f64,  // close price
            1.0f64..1000.0f64,   // quantity
        ),
        2..=5,
    )
    .prop_map(|entries| {
        // Deduplicate symbols — keep first occurrence
        let mut seen = std::collections::HashSet::<String>::new();
        entries
            .into_iter()
            .filter(|(sym, _, _)| seen.insert(sym.clone()))
            .collect::<Vec<_>>()
    })
    .prop_filter("need at least 2 distinct symbols", |entries| entries.len() >= 2)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Validates: Requirements 5.4, 8.3**
    ///
    /// For any bar group containing symbols {S1, S2, ..., Sn} with close prices
    /// {P1, P2, ..., Pn}, when a signal targets symbol Si, the fill price SHALL be Pi
    /// (the close price from that symbol's bar in the group).
    #[test]
    fn prop_multi_asset_fill_prices_use_per_symbol_close(
        entries in arb_multi_asset_symbols_and_prices()
    ) {
        let mut tracker = PositionTracker::new(1_000_000.0);

        // Build the closes map (simulating a bar group's per-symbol close prices)
        let closes: HashMap<String, f64> = entries
            .iter()
            .map(|(sym, price, _)| (sym.clone(), *price))
            .collect();

        // For each symbol, create an OPEN signal and process it using
        // that symbol's close price from the group (the per-symbol fill logic)
        for (sym, close_price, qty) in &entries {
            let signal = Signal::open(sym.clone(), *qty);
            let fill_price = closes.get(sym).copied().unwrap();

            // Process signal with the per-symbol fill price
            let fill = tracker.process_signal(&signal, fill_price, 0);

            // Verify a fill was generated
            prop_assert!(
                fill.is_some(),
                "Expected a fill for symbol '{}', got None",
                sym
            );

            let fill = fill.unwrap();

            // Verify the fill price matches the symbol's close price from the group
            prop_assert_eq!(
                fill.price,
                *close_price,
                "Fill price for '{}' should be {} (symbol's close), got {}",
                sym,
                close_price,
                fill.price
            );
        }

        // Additionally verify that each position's avg_entry_price equals
        // the symbol's close price (since each symbol was opened only once)
        for (sym, close_price, _) in &entries {
            let position = tracker.position(sym);
            prop_assert!(
                position.is_some(),
                "Expected position for symbol '{}' after OPEN signal",
                sym
            );
            let position = position.unwrap();
            prop_assert_eq!(
                position.avg_entry_price,
                *close_price,
                "Position avg_entry_price for '{}' should be {} (symbol's close), got {}",
                sym,
                close_price,
                position.avg_entry_price
            );
        }

        // Verify via fills() that all recorded fills have the correct per-symbol price
        let all_fills = tracker.fills();
        prop_assert_eq!(
            all_fills.len(),
            entries.len(),
            "Expected {} fills (one per symbol), got {}",
            entries.len(),
            all_fills.len()
        );

        for fill in all_fills {
            let expected_price = closes.get(&fill.symbol).copied().unwrap();
            prop_assert_eq!(
                fill.price,
                expected_price,
                "Fill log: fill for '{}' has price {} but expected {} (per-symbol close)",
                fill.symbol,
                fill.price,
                expected_price
            );
        }
    }
}

// =============================================================================
// End-to-End Integration Test: Portfolio Strategy
// Feature: portfolio-construction, Task 8.2
// =============================================================================

/// End-to-end integration test for a portfolio strategy that uses:
/// - `[ret("AAPL"), ret("MSFT"), ret("GOOG")]` to construct return vectors
/// - `cov_matrix(returns, period)` to estimate covariance
/// - `min_variance_weights(cov, constraints)` to compute optimal weights
/// - Multi-asset `OPEN` signals to open positions in each symbol
///
/// **Validates: Requirements 7.1, 7.2, 7.3, 7.4, 7.5, 8.1, 8.2, 8.4, 8.5**
#[test]
fn test_end_to_end_portfolio_strategy() {
    use std::path::Path;

    // --- Step 1: Define a Flux strategy source that exercises the full portfolio pipeline ---
    // Use constraints [0.1, 0.9] to ensure all weights are positive (avoiding qty=0 panics)
    let strategy_source = r#"
strategy MinVariance {
    params {
        period = 5
    }
    state {
        bar_count = 0
    }
    on bar {
        bar_count = bar_count + 1
        if bar_count > 4 and not in_position {
            returns = [ret("AAPL"), ret("MSFT"), ret("GOOG")]
            cov = cov_matrix(returns, period)
            weights = min_variance_weights(cov, [0.1, 0.9])
            OPEN("AAPL", weights[0] * 100.0)
            OPEN("MSFT", weights[1] * 100.0)
            OPEN("GOOG", weights[2] * 100.0)
        }
    }
}
"#;

    // --- Step 2: Compile through lex → parse → typecheck ---
    let tokens = flux_compiler::lexer::lex_with_spans(strategy_source)
        .expect("Lexing should succeed for valid portfolio strategy");

    let ast = flux_compiler::parser::parse(tokens)
        .expect("Parsing should succeed for valid portfolio strategy");

    let typed_program = flux_compiler::typeck::check(ast)
        .expect("Type checking should succeed for valid portfolio strategy");

    // --- Step 3: Create interpreter and position tracker ---
    let mut interpreter = Interpreter::new(&typed_program);
    let mut tracker = PositionTracker::new(100_000.0);

    // --- Step 4: Load multi-asset CSV data ---
    let csv_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("multi_asset_data.csv");

    let loaded = flux_cli::csv_loader::load_csv_with_timestamps(&csv_path)
        .expect("Loading multi-asset CSV should succeed");

    // --- Step 5: Group bars by timestamp ---
    let bar_groups = group_bars_by_timestamp(&loaded.bars, &loaded.timestamps)
        .expect("Bar grouping should succeed");

    // The CSV has 12 timestamps with 3 symbols each
    assert_eq!(bar_groups.len(), 12, "Expected 12 bar groups (12 timestamps)");

    // --- Step 6: Iterate through bar groups, simulating the backtest loop ---
    let mut all_signals: Vec<(usize, Signal)> = Vec::new();

    for (group_idx, group) in bar_groups.iter().enumerate() {
        // Update interpreter price state (for ret() computation)
        interpreter.update_prices(&group.closes);

        // Set in_position from tracker state (multi-symbol aware)
        interpreter.in_position = tracker.open_position_count() > 0;

        // Execute on_bar with the first bar as primary bar context
        let primary_bar = &group.bars[0];
        let signals = interpreter.on_bar(primary_bar);

        // Process signals with per-symbol fill prices from the group
        for signal in &signals {
            let fill_price = match signal {
                Signal::Open { symbol, .. } => {
                    group.closes.get(symbol).copied().unwrap_or(primary_bar.close)
                }
                Signal::Close { symbol } => {
                    group.closes.get(symbol).copied().unwrap_or(primary_bar.close)
                }
                Signal::CloseQty { symbol, .. } => {
                    group.closes.get(symbol).copied().unwrap_or(primary_bar.close)
                }
            };
            tracker.process_signal(signal, fill_price, group_idx);
        }

        // Mark all positions to market using per-symbol close prices
        tracker.mark_all_to_market(&group.closes);

        // Collect signals
        for signal in signals {
            all_signals.push((group_idx, signal));
        }
    }

    // --- Step 7: Verify results ---

    // 7a: Signals were generated (the strategy should fire after bar_count > 3)
    assert!(
        !all_signals.is_empty(),
        "Strategy should have generated at least one signal"
    );

    // 7b: At least one fill was generated
    let fills = tracker.fills();
    assert!(
        !fills.is_empty(),
        "At least one fill should have been generated"
    );

    // 7c: Fills for each symbol (AAPL, MSFT, GOOG) should exist
    let filled_symbols: std::collections::HashSet<&str> =
        fills.iter().map(|f| f.symbol.as_str()).collect();
    assert!(
        filled_symbols.contains("AAPL"),
        "Expected a fill for AAPL, fills: {:?}",
        fills
    );
    assert!(
        filled_symbols.contains("MSFT"),
        "Expected a fill for MSFT, fills: {:?}",
        fills
    );
    assert!(
        filled_symbols.contains("GOOG"),
        "Expected a fill for GOOG, fills: {:?}",
        fills
    );

    // 7d: open_position_count should reflect 3 open positions (one per symbol)
    //     (The strategy opens positions once and never closes them)
    assert_eq!(
        tracker.open_position_count(),
        3,
        "Expected 3 open positions (one per symbol), got {}",
        tracker.open_position_count()
    );

    // 7e: Aggregate portfolio metrics should be valid
    let portfolio = tracker.portfolio_state();
    assert!(
        portfolio.equity > 0.0,
        "Equity should be positive, got {}",
        portfolio.equity
    );
    assert!(
        portfolio.gross_exposure > 0.0,
        "Gross exposure should be positive with open positions, got {}",
        portfolio.gross_exposure
    );
    // Equity = initial_capital + realized_pnl + unrealized_pnl
    let computed_equity = tracker.initial_capital() + tracker.realized_pnl() + tracker.unrealized_pnl();
    assert!(
        (portfolio.equity - computed_equity).abs() < 1e-6,
        "Equity decomposition: portfolio_state.equity ({}) != initial + realized + unrealized ({})",
        portfolio.equity,
        computed_equity
    );

    // 7f: Fill prices should match per-symbol closes from the bar group where signals fired
    // (The strategy fires at bar_count > 3, which is group index 3 — 0-indexed)
    // Find the first signal group index
    let first_signal_group = all_signals[0].0;
    let signal_group_closes = &bar_groups[first_signal_group].closes;
    for fill in fills {
        if fill.bar_index == first_signal_group {
            let expected_price = signal_group_closes
                .get(&fill.symbol)
                .expect("Fill symbol should exist in bar group closes");
            assert_eq!(
                fill.price, *expected_price,
                "Fill for {} at bar {} should use per-symbol close price ({}) but got {}",
                fill.symbol, fill.bar_index, expected_price, fill.price
            );
        }
    }
}

/// Single-symbol backward compatibility test.
///
/// Verifies that loading the existing `sample_data.csv` (single-symbol AAPL)
/// with a simple strategy still works correctly — single-symbol CSVs produce
/// one bar per group and the backtest behaves identically to the old path.
///
/// **Validates: Requirements 8.2 (backward compatibility)**
#[test]
fn test_end_to_end_single_symbol_backward_compat() {
    use std::path::Path;

    // A simple strategy that opens a position when close > open
    let strategy_source = r#"
strategy SimpleMA {
    state {
        bar_count = 0
    }
    on bar {
        bar_count = bar_count + 1
        if bar_count > 2 and not in_position {
            OPEN(symbol, 100.0)
        }
    }
}
"#;

    // Compile
    let tokens = flux_compiler::lexer::lex_with_spans(strategy_source)
        .expect("Lexing should succeed");
    let ast = flux_compiler::parser::parse(tokens)
        .expect("Parsing should succeed");
    let typed_program = flux_compiler::typeck::check(ast)
        .expect("Type checking should succeed");

    // Create interpreter and tracker
    let mut interpreter = Interpreter::new(&typed_program);
    let mut tracker = PositionTracker::new(100_000.0);

    // Load single-symbol CSV
    let csv_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("sample_data.csv");

    let loaded = flux_cli::csv_loader::load_csv_with_timestamps(&csv_path)
        .expect("Loading sample CSV should succeed");

    // Group bars by timestamp — single-symbol should produce one bar per group
    let bar_groups = group_bars_by_timestamp(&loaded.bars, &loaded.timestamps)
        .expect("Bar grouping should succeed");

    assert_eq!(
        bar_groups.len(),
        loaded.bars.len(),
        "Single-symbol CSV: each timestamp should produce exactly one group"
    );

    // Verify each group has exactly 1 bar
    for (i, group) in bar_groups.iter().enumerate() {
        assert_eq!(
            group.bars.len(),
            1,
            "Single-symbol: group {} should have 1 bar, got {}",
            i,
            group.bars.len()
        );
    }

    // Run through the backtest loop
    let mut all_signals: Vec<Signal> = Vec::new();
    for (group_idx, group) in bar_groups.iter().enumerate() {
        interpreter.update_prices(&group.closes);
        interpreter.in_position = tracker.open_position_count() > 0;

        let primary_bar = &group.bars[0];
        let signals = interpreter.on_bar(primary_bar);

        for signal in &signals {
            let fill_price = group.closes
                .get(match signal {
                    Signal::Open { symbol, .. } => symbol,
                    Signal::Close { symbol } => symbol,
                    Signal::CloseQty { symbol, .. } => symbol,
                })
                .copied()
                .unwrap_or(primary_bar.close);
            tracker.process_signal(signal, fill_price, group_idx);
        }

        tracker.mark_all_to_market(&group.closes);
        all_signals.extend(signals);
    }

    // Verify: signals were generated
    assert!(
        !all_signals.is_empty(),
        "Simple strategy should generate at least one signal"
    );

    // Verify: fills generated for AAPL
    let fills = tracker.fills();
    assert!(!fills.is_empty(), "Should have at least one fill");
    assert!(
        fills.iter().all(|f| f.symbol == "AAPL"),
        "All fills should be for AAPL in single-symbol mode"
    );

    // Verify: position opened
    assert_eq!(
        tracker.open_position_count(),
        1,
        "Should have exactly 1 open position for AAPL"
    );

    // Verify: portfolio metrics
    let portfolio = tracker.portfolio_state();
    assert!(portfolio.equity > 0.0, "Equity should be positive");
}
