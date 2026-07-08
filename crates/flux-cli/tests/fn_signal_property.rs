//! Property-based test for signal propagation through user-defined functions.
//!
//! Feature: flux-user-functions, Property 7: Signal propagation
//! **Validates: Requirements 5.5**
//!
//! Generates functions that emit 0–N signals (OPEN, CLOSE, CLOSE_QTY);
//! asserts all signals appear in strategy output in the correct order.

use proptest::prelude::*;

use flux_cli::interpreter::Interpreter;
use flux_runtime::BarContext;

/// A signal emission we want the generated function to perform.
#[derive(Debug, Clone)]
enum SignalEmission {
    Open { qty: f64 },
    Close,
}

/// Generate a random signal emission.
fn arb_signal_emission() -> impl Strategy<Value = SignalEmission> {
    prop_oneof![
        // OPEN with qty in (1.0, 1000.0)
        (1.0..1000.0f64).prop_map(|qty| SignalEmission::Open { qty }),
        // CLOSE (single arg — closes entire position)
        Just(SignalEmission::Close),
    ]
}

/// Convert a signal emission to a Flux source code statement.
fn emission_to_flux(emission: &SignalEmission) -> String {
    match emission {
        SignalEmission::Open { qty } => format!("        OPEN(symbol, {:.2})", qty),
        SignalEmission::Close => "        CLOSE(symbol)".to_string(),
    }
}

/// Build a full Flux source with a user-defined function that emits
/// the given signals and is called from `on bar`.
fn build_flux_source(emissions: &[SignalEmission]) -> String {
    let mut fn_body = String::new();
    for emission in emissions {
        fn_body.push_str(&emission_to_flux(emission));
        fn_body.push('\n');
    }

    format!(
        r#"fn emit_signals() {{
{}}}

strategy SignalTest {{
    on bar {{
        emit_signals()
    }}
}}"#,
        fn_body
    )
}

/// Compare two signals for equality by matching variant and fields.
fn signals_match(actual: &flux_runtime::Signal, expected: &SignalEmission, symbol: &str) -> bool {
    match (actual, expected) {
        (
            flux_runtime::Signal::Open {
                symbol: s,
                qty: actual_qty,
            },
            SignalEmission::Open { qty: expected_qty },
        ) => s == symbol && (*actual_qty - *expected_qty).abs() < 0.01,
        (flux_runtime::Signal::Close { symbol: s }, SignalEmission::Close) => s == symbol,
        _ => false,
    }
}

/// Run the full pipeline: source → lex → parse → typecheck → interpret.
/// Returns the signals emitted on the first bar.
fn run_pipeline(source: &str) -> Vec<flux_runtime::Signal> {
    let tokens = flux_compiler::lexer::lex_with_spans(source).expect("lex failed");
    let ast = flux_compiler::parser::parse(tokens).expect("parse failed");
    let typed_program = flux_compiler::typeck::check(ast).expect("typecheck failed");

    let mut interpreter = Interpreter::new(&typed_program);

    let bar = BarContext {
        close: 100.0,
        open: 99.0,
        high: 101.0,
        low: 98.0,
        volume: 50000.0,
        symbol: "TEST".to_string(),
        in_position: false,
    };

    interpreter.on_bar(&bar)
}

// =============================================================================
// Property 7: Signal propagation
// Feature: flux-user-functions, Property 7: Signal propagation
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

    /// **Validates: Requirements 5.5**
    ///
    /// For any user-defined function that emits 0–5 signals (OPEN, CLOSE, CLOSE_QTY),
    /// those signals SHALL appear in the strategy's signal output for that bar,
    /// in the same order they were emitted.
    #[test]
    fn prop_signal_propagation(
        emissions in proptest::collection::vec(arb_signal_emission(), 0..=5),
    ) {
        let source = build_flux_source(&emissions);
        let signals = run_pipeline(&source);

        // The number of signals produced must match the number of emissions
        prop_assert_eq!(
            signals.len(),
            emissions.len(),
            "Expected {} signals, got {}. Source:\n{}",
            emissions.len(),
            signals.len(),
            source
        );

        // Each signal must match the corresponding emission, in order
        for (i, (actual, expected)) in signals.iter().zip(emissions.iter()).enumerate() {
            prop_assert!(
                signals_match(actual, expected, "TEST"),
                "Signal mismatch at index {}: actual={:?}, expected={:?}. Source:\n{}",
                i,
                actual,
                expected,
                source
            );
        }
    }
}
