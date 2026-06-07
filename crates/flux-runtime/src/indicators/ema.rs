use super::state::{EmaState, IndicatorState, INDICATOR_STATE};
use std::panic::Location;

/// Compute the Exponential Moving Average.
///
/// Each call site maintains independent state. The first call returns `value` itself.
/// Subsequent calls compute: EMA = value * k + prev_ema * (1 - k), where k = 2.0 / (period + 1).
///
/// # Panics
/// Panics if `period < 1`.
#[track_caller]
pub fn ema(value: f64, period: i64) -> f64 {
    assert!(period >= 1, "ema: period must be at least 1");
    let caller = Location::caller();
    let key = format!("{}:{}:{}", caller.file(), caller.line(), caller.column());

    INDICATOR_STATE.with(|state| {
        let mut registry = state.borrow_mut();
        let entry = registry
            .entry(key)
            .or_insert_with(|| IndicatorState::Ema(EmaState::new(period as usize)));

        match entry {
            IndicatorState::Ema(ema_state) => ema_state.next(value),
            _ => panic!("indicator state type mismatch"),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::state::reset_indicator_state;
    use proptest::prelude::*;

    #[test]
    fn ema_first_call_returns_value() {
        reset_indicator_state();
        let result = ema(42.0, 3);
        assert_eq!(result, 42.0);
    }

    #[test]
    fn ema_subsequent_calls_apply_formula() {
        reset_indicator_state();
        // k = 2.0 / (3 + 1) = 0.5
        // Feed values through a single call site using a loop
        let values = [10.0, 20.0, 30.0];
        // first => 10.0, second => 20*0.5 + 10*0.5 = 15.0, third => 30*0.5 + 15*0.5 = 22.5
        let expected = [10.0, 15.0, 22.5];
        let mut results = Vec::new();
        for &v in &values {
            results.push(ema(v, 3)); // single call site
        }
        assert_eq!(results, expected);
    }

    #[test]
    fn ema_period_one_always_returns_current_value() {
        reset_indicator_state();
        // k = 2.0 / (1 + 1) = 1.0, so EMA = value * 1.0 + prev * 0.0 = value
        let values = [5.0, 10.0, 3.0, 99.0];
        for &v in &values {
            let result = ema(v, 1); // single call site
            assert_eq!(result, v, "period=1 should always return the current value");
        }
    }

    #[test]
    fn ema_smoothing_factor_correctness() {
        reset_indicator_state();
        // period=9 => k = 2.0 / (9 + 1) = 0.2
        // first => 100.0, second => 200*0.2 + 100*0.8 = 40 + 80 = 120
        let values = [100.0, 200.0];
        let expected = [100.0, 120.0];
        let mut results = Vec::new();
        for &v in &values {
            results.push(ema(v, 9)); // single call site
        }
        for (result, exp) in results.iter().zip(expected.iter()) {
            assert!(
                (result - exp).abs() < 1e-10,
                "expected {}, got {}",
                exp,
                result
            );
        }
    }

    #[test]
    #[should_panic(expected = "ema: period must be at least 1")]
    fn ema_panics_on_period_zero() {
        reset_indicator_state();
        ema(10.0, 0);
    }

    #[test]
    #[should_panic(expected = "ema: period must be at least 1")]
    fn ema_panics_on_negative_period() {
        reset_indicator_state();
        ema(10.0, -5);
    }

    // Feature: flux-runtime, Property 3: EMA Correctness
    // **Validates: Requirements 5.2, 5.3, 5.4**
    proptest! {
        #[test]
        fn prop_ema_correctness(
            values in proptest::collection::vec(0.01..1000.0f64, 1..50),
            period in 1..20i64,
        ) {
            reset_indicator_state();
            let k = 2.0 / (period as f64 + 1.0);
            let mut expected_ema = values[0];

            for (i, &val) in values.iter().enumerate() {
                let result = ema(val, period);
                if i == 0 {
                    expected_ema = val;
                } else {
                    expected_ema = val * k + expected_ema * (1.0 - k);
                }
                prop_assert!((result - expected_ema).abs() < 1e-10,
                    "EMA mismatch at step {}: got {}, expected {}", i, result, expected_ema);
            }
        }
    }
}
