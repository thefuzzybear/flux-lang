use super::state::{IndicatorState, SmaState, INDICATOR_STATE};
use std::panic::Location;

/// Compute the Simple Moving Average over a rolling window.
///
/// Each call site maintains independent state. The first call returns `value` itself.
/// Subsequent calls accumulate values and return the mean of the last `period` values.
///
/// # Panics
/// Panics if `period < 1`.
#[track_caller]
pub fn sma(value: f64, period: i64) -> f64 {
    assert!(period >= 1, "sma: period must be at least 1");
    let caller = Location::caller();
    let key = format!("{}:{}:{}", caller.file(), caller.line(), caller.column());

    INDICATOR_STATE.with(|state| {
        let mut registry = state.borrow_mut();
        let entry = registry
            .entry(key)
            .or_insert_with(|| IndicatorState::Sma(SmaState::new(period as usize)));

        match entry {
            IndicatorState::Sma(sma_state) => sma_state.next(value),
            _ => panic!("indicator state type mismatch"),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::state::reset_indicator_state;
    use proptest::prelude::*;

    /// Compute expected SMA at step i given values[0..=i] and period.
    /// This is the reference oracle for verifying indicator output.
    fn expected_sma(values: &[f64], period: usize) -> Vec<f64> {
        let mut results = Vec::with_capacity(values.len());
        for i in 0..values.len() {
            let window_start = if (i + 1) > period { i + 1 - period } else { 0 };
            let window = &values[window_start..=i];
            results.push(window.iter().sum::<f64>() / window.len() as f64);
        }
        results
    }

    // Feature: flux-runtime, Property 4: Indicator Call-Site Isolation
    proptest! {
        /// **Validates: Requirements 4.5, 5.5, 6.1, 6.2**
        ///
        /// For any two independent call sites invoking sma, state at each call site
        /// is fully independent — feeding different sequences to different call sites
        /// produces the same results as if each existed alone.
        #[test]
        fn prop_indicator_call_site_isolation(
            values_a in proptest::collection::vec(0.01..1000.0f64, 1..30),
            values_b in proptest::collection::vec(0.01..1000.0f64, 1..30),
            period in 1..10i64,
        ) {
            reset_indicator_state();

            let min_len = values_a.len().min(values_b.len());

            // Feed interleaved values to two different call sites (different lines)
            let mut results_a = Vec::new();
            let mut results_b = Vec::new();
            for i in 0..min_len {
                results_a.push(sma(values_a[i], period)); // call site A (this line)
                results_b.push(sma(values_b[i], period)); // call site B (this line)
            }

            // Compute expected results independently using reference oracle
            let expected_a = expected_sma(&values_a[..min_len], period as usize);
            let expected_b = expected_sma(&values_b[..min_len], period as usize);

            // Interleaved results should match independently-computed expected values
            for i in 0..min_len {
                prop_assert!((results_a[i] - expected_a[i]).abs() < 1e-10,
                    "Call site A isolation broken at step {}: got {}, expected {}",
                    i, results_a[i], expected_a[i]);
                prop_assert!((results_b[i] - expected_b[i]).abs() < 1e-10,
                    "Call site B isolation broken at step {}: got {}, expected {}",
                    i, results_b[i], expected_b[i]);
            }
        }
    }

    #[test]
    fn sma_period_one_returns_value() {
        reset_indicator_state();
        // Each call on the same line shares state via #[track_caller]
        let values = [42.0, 99.0, 7.0];
        for &v in &values {
            let result = sma(v, 1); // single call site (same line)
            assert_eq!(result, v, "period=1 should always return the current value");
        }
    }

    #[test]
    fn sma_accumulating_phase_returns_partial_mean() {
        reset_indicator_state();
        // Feed values through a single call site using a loop
        let values = [10.0, 20.0, 30.0];
        let expected = [10.0, 15.0, 20.0]; // partial means for period=3
        let mut results = Vec::new();
        for &v in &values {
            results.push(sma(v, 3)); // single call site
        }
        assert_eq!(results, expected);
    }

    #[test]
    fn sma_full_window_returns_correct_mean() {
        reset_indicator_state();
        // Feed 5 values through period=3, check all outputs
        let values = [10.0, 20.0, 30.0, 40.0, 50.0];
        // Filling: mean([10])=10, mean([10,20])=15, mean([10,20,30])=20
        // Full:   mean([20,30,40])=30, mean([30,40,50])=40
        let expected = [10.0, 15.0, 20.0, 30.0, 40.0];
        let mut results = Vec::new();
        for &v in &values {
            results.push(sma(v, 3)); // single call site
        }
        assert_eq!(results, expected);
    }

    #[test]
    #[should_panic(expected = "sma: period must be at least 1")]
    fn sma_panics_on_period_less_than_one() {
        reset_indicator_state();
        sma(10.0, 0);
    }

    #[test]
    #[should_panic(expected = "sma: period must be at least 1")]
    fn sma_panics_on_negative_period() {
        reset_indicator_state();
        sma(10.0, -5);
    }

    // Feature: flux-runtime, Property 2: SMA Correctness
    // **Validates: Requirements 4.2, 4.3, 4.4**
    proptest! {
        #[test]
        fn prop_sma_correctness(
            values in proptest::collection::vec(0.01..1000.0f64, 1..50),
            period in 1..20i64,
        ) {
            reset_indicator_state();
            for (i, &val) in values.iter().enumerate() {
                let result = sma(val, period);
                let window_start = if (i + 1) > period as usize { i + 1 - period as usize } else { 0 };
                let window = &values[window_start..=i];
                let expected = window.iter().sum::<f64>() / window.len() as f64;
                prop_assert!((result - expected).abs() < 1e-10,
                    "SMA mismatch at step {}: got {}, expected {}", i, result, expected);
            }
        }
    }
}
