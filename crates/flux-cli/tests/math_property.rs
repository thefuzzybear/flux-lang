//! Property-based tests for Math, Stats & Portfolio Operations.
//!
//! This file contains property tests validating universal correctness properties
//! defined in the design document for the math-stats-portfolio-ops spec.

use std::collections::HashMap;

use proptest::prelude::*;

use flux_cli::interpreter::{IndicatorStateEntry, Value};
use flux_cli::math_builtins::eval_math_builtin;
use flux_cli::portfolio_ops::{eval_matrix_op, eval_portfolio_op};
use flux_cli::stat_indicators::eval_stat_indicator;

// =============================================================================
// Property 5: Stddev² Equals Variance
// Feature: math-stats-portfolio-ops, Property 5: Stddev² Equals Variance
// =============================================================================

proptest! {
    /// **Validates: Requirements 4.1, 4.2**
    ///
    /// For any sequence of Float values and any valid period, after feeding the
    /// same values to both `stddev(v, period)` and `variance(v, period)` at each
    /// step, `stddev(v, period)² ≈ variance(v, period)` holds within tolerance.
    #[test]
    fn prop_stddev_squared_equals_variance(
        values in proptest::collection::vec(-1000.0..1000.0f64, 5..20),
        period in 2usize..10,
    ) {
        let mut stddev_indicators: HashMap<String, IndicatorStateEntry> = HashMap::new();
        let mut variance_indicators: HashMap<String, IndicatorStateEntry> = HashMap::new();

        let mut last_stddev: Option<f64> = None;
        let mut last_variance: Option<f64> = None;

        for val in &values {
            let stddev_result = eval_stat_indicator(
                "stddev",
                &[Value::Float(*val), Value::Int(period as i64)],
                &mut stddev_indicators,
                "stddev_test_0_10",
            ).unwrap().unwrap();

            let variance_result = eval_stat_indicator(
                "variance",
                &[Value::Float(*val), Value::Int(period as i64)],
                &mut variance_indicators,
                "variance_test_0_10",
            ).unwrap().unwrap();

            if let Value::Float(s) = stddev_result {
                last_stddev = Some(s);
            }
            if let Value::Float(v) = variance_result {
                last_variance = Some(v);
            }
        }

        let stddev_val = last_stddev.unwrap();
        let variance_val = last_variance.unwrap();
        let stddev_squared = stddev_val * stddev_val;

        // Relative tolerance for floating-point comparison
        let tolerance = 1e-10 * variance_val.abs().max(1.0);
        prop_assert!(
            (stddev_squared - variance_val).abs() < tolerance,
            "stddev²={} != variance={}, diff={}",
            stddev_squared, variance_val, (stddev_squared - variance_val).abs()
        );
    }
}

// =============================================================================
// Property 6: Correlation Bounded
// Feature: math-stats-portfolio-ops, Property 6: Correlation Bounded
// =============================================================================

proptest! {
    /// **Validates: Requirements 5.1**
    ///
    /// For any two sequences of Float values and any valid period,
    /// `corr(a, b, period)` returns a value in the range [-1.0, 1.0].
    #[test]
    fn prop_correlation_bounded(
        values_a in proptest::collection::vec(-1000.0..1000.0f64, 5..20),
        values_b in proptest::collection::vec(-1000.0..1000.0f64, 5..20),
        period in 2usize..10,
    ) {
        let len = values_a.len().min(values_b.len());
        let mut indicators: HashMap<String, IndicatorStateEntry> = HashMap::new();

        for i in 0..len {
            let result = eval_stat_indicator(
                "corr",
                &[Value::Float(values_a[i]), Value::Float(values_b[i]), Value::Int(period as i64)],
                &mut indicators,
                "corr_test_0_10",
            ).unwrap().unwrap();

            if let Value::Float(corr_val) = result {
                prop_assert!(
                    corr_val >= -1.0 && corr_val <= 1.0,
                    "corr={} out of [-1.0, 1.0] at step {}",
                    corr_val, i
                );
            } else {
                prop_assert!(false, "Expected Float result from corr");
            }
        }
    }
}

// =============================================================================
// Property 7: RSI Bounded
// Feature: math-stats-portfolio-ops, Property 7: RSI Bounded
// =============================================================================

proptest! {
    /// **Validates: Requirements 6.1**
    ///
    /// For any sequence of Float values and any valid period (≥ 1), after at
    /// least 2 values have been fed, `rsi(value, period)` returns a value in
    /// the range [0.0, 100.0].
    #[test]
    fn prop_rsi_bounded(
        values in proptest::collection::vec(-1000.0..1000.0f64, 3..20),
        period in 2usize..10,
    ) {
        let mut indicators: HashMap<String, IndicatorStateEntry> = HashMap::new();

        for (i, val) in values.iter().enumerate() {
            let result = eval_stat_indicator(
                "rsi",
                &[Value::Float(*val), Value::Int(period as i64)],
                &mut indicators,
                "rsi_test_0_10",
            ).unwrap().unwrap();

            if let Value::Float(rsi_val) = result {
                // After at least 2 values, RSI must be in [0.0, 100.0]
                if i >= 1 {
                    prop_assert!(
                        rsi_val >= 0.0 && rsi_val <= 100.0,
                        "rsi={} out of [0.0, 100.0] at step {} (after 2+ values)",
                        rsi_val, i
                    );
                }
            } else {
                prop_assert!(false, "Expected Float result from rsi");
            }
        }
    }
}

// =============================================================================
// Property 8: ATR Non-Negative
// Feature: math-stats-portfolio-ops, Property 8: ATR Non-Negative
// =============================================================================

proptest! {
    /// **Validates: Requirements 7.1, 7.2**
    ///
    /// For any sequence of valid bar data (where high >= low, close between
    /// high and low) and any valid period, `atr(high, low, close, period)`
    /// returns a value >= 0.0.
    #[test]
    fn prop_atr_non_negative(
        // Generate base prices, then derive valid OHLC from them
        bases in proptest::collection::vec(10.0..1000.0f64, 3..20),
        spreads in proptest::collection::vec(0.1..50.0f64, 3..20),
        close_fracs in proptest::collection::vec(0.0..1.0f64, 3..20),
        period in 2usize..10,
    ) {
        let len = bases.len().min(spreads.len()).min(close_fracs.len());
        let mut indicators: HashMap<String, IndicatorStateEntry> = HashMap::new();

        for i in 0..len {
            let low = bases[i];
            let high = low + spreads[i]; // ensures high >= low
            let close = low + close_fracs[i] * spreads[i]; // close between low and high

            let result = eval_stat_indicator(
                "atr",
                &[Value::Float(high), Value::Float(low), Value::Float(close), Value::Int(period as i64)],
                &mut indicators,
                "atr_test_0_10",
            ).unwrap().unwrap();

            if let Value::Float(atr_val) = result {
                prop_assert!(
                    atr_val >= 0.0,
                    "atr={} is negative at step {}",
                    atr_val, i
                );
            } else {
                prop_assert!(false, "Expected Float result from atr");
            }
        }
    }
}

// =============================================================================
// Property 9: Z-Score Relationship
// Feature: math-stats-portfolio-ops, Property 9: Z-Score Relationship
// =============================================================================

proptest! {
    /// **Validates: Requirements 8.1**
    ///
    /// For any sequence of Float values and any valid period, if the rolling
    /// standard deviation is non-zero, then:
    /// `zscore(v, period) * stddev(v, period) + mean(v, period) ≈ v`
    /// holds for the current value `v` within floating-point tolerance.
    #[test]
    fn prop_zscore_relationship(
        values in proptest::collection::vec(-1000.0..1000.0f64, 5..20),
        period in 3usize..10,
    ) {
        let mut zscore_indicators: HashMap<String, IndicatorStateEntry> = HashMap::new();
        let mut stddev_indicators: HashMap<String, IndicatorStateEntry> = HashMap::new();

        // We also need to track the rolling mean manually, using a circular buffer
        let mut buffer: Vec<f64> = vec![0.0; period];
        let mut buf_index = 0;
        let mut buf_count = 0;
        let mut sum = 0.0;

        for val in &values {
            // Feed to zscore
            let zscore_result = eval_stat_indicator(
                "zscore",
                &[Value::Float(*val), Value::Int(period as i64)],
                &mut zscore_indicators,
                "zscore_test_0_10",
            ).unwrap().unwrap();

            // Feed to stddev
            let stddev_result = eval_stat_indicator(
                "stddev",
                &[Value::Float(*val), Value::Int(period as i64)],
                &mut stddev_indicators,
                "stddev_test_0_10",
            ).unwrap().unwrap();

            // Compute rolling mean
            if buf_count < period {
                buffer[buf_index] = *val;
                sum += *val;
                buf_count += 1;
            } else {
                sum -= buffer[buf_index];
                buffer[buf_index] = *val;
                sum += *val;
            }
            buf_index = (buf_index + 1) % period;
            let mean = sum / buf_count as f64;

            if let (Value::Float(z), Value::Float(s)) = (zscore_result, stddev_result) {
                // Only check when stddev > 0 (non-constant series in window)
                if s > 1e-12 {
                    let reconstructed = z * s + mean;
                    let tolerance = 1e-9 * val.abs().max(1.0);
                    prop_assert!(
                        (reconstructed - *val).abs() < tolerance,
                        "zscore*stddev+mean={} != value={}, zscore={}, stddev={}, mean={}, diff={}",
                        reconstructed, val, z, s, mean, (reconstructed - *val).abs()
                    );
                }
            }
        }
    }
}

// =============================================================================
// Property 10: Call-Site State Isolation
// Feature: math-stats-portfolio-ops, Property 10: Call-Site State Isolation
// =============================================================================

proptest! {
    /// **Validates: Requirements 4.4, 5.5, 6.6, 7.5, 8.4, 12.6**
    ///
    /// For any stateful indicator function and two distinct call-site keys,
    /// feeding value sequence A to call-site 1 and value sequence B to call-site 2
    /// produces the same results as if each were the only call site in the program
    /// (outputs are independent of the other's inputs).
    #[test]
    fn prop_call_site_state_isolation(
        values_a in proptest::collection::vec(1.0..500.0f64, 5..15),
        values_b in proptest::collection::vec(1.0..500.0f64, 5..15),
        period in 2usize..8,
    ) {
        // Shared indicator map — both call sites use the same HashMap
        let mut shared_indicators: HashMap<String, IndicatorStateEntry> = HashMap::new();
        // Isolated indicator maps — one per call site
        let mut isolated_a: HashMap<String, IndicatorStateEntry> = HashMap::new();
        let mut isolated_b: HashMap<String, IndicatorStateEntry> = HashMap::new();

        let len = values_a.len().min(values_b.len());

        for i in 0..len {
            // Feed value_a to key "site_a" in shared map
            let shared_result_a = eval_stat_indicator(
                "stddev",
                &[Value::Float(values_a[i]), Value::Int(period as i64)],
                &mut shared_indicators,
                "site_a",
            ).unwrap().unwrap();

            // Feed value_b to key "site_b" in shared map
            let _shared_result_b = eval_stat_indicator(
                "stddev",
                &[Value::Float(values_b[i]), Value::Int(period as i64)],
                &mut shared_indicators,
                "site_b",
            ).unwrap().unwrap();

            // Feed value_a to isolated map A
            let isolated_result_a = eval_stat_indicator(
                "stddev",
                &[Value::Float(values_a[i]), Value::Int(period as i64)],
                &mut isolated_a,
                "site_a",
            ).unwrap().unwrap();

            // Feed value_b to isolated map B
            let _isolated_result_b = eval_stat_indicator(
                "stddev",
                &[Value::Float(values_b[i]), Value::Int(period as i64)],
                &mut isolated_b,
                "site_b",
            ).unwrap().unwrap();

            // The result from call-site A in the shared map should be identical
            // to the result from the isolated map (unaffected by site B's data)
            if let (Value::Float(shared_a), Value::Float(iso_a)) = (shared_result_a, isolated_result_a) {
                prop_assert!(
                    (shared_a - iso_a).abs() < 1e-12,
                    "State isolation violated at step {}: shared_a={}, isolated_a={}, diff={}",
                    i, shared_a, iso_a, (shared_a - iso_a).abs()
                );
            }
        }
    }
}

// =============================================================================
// Tier 3 Property Tests — Matrix & Portfolio Operations
// =============================================================================

// =============================================================================
// Helpers for Tier 3 tests
// =============================================================================

/// Build a MatFloat Value from a flat data vector with given dimensions.
fn mat(data: Vec<f64>, rows: usize, cols: usize) -> Value {
    Value::MatFloat { data, rows, cols }
}

/// Build a VecFloat Value from a vector of f64.
fn vecf(data: Vec<f64>) -> Value {
    Value::VecFloat(data)
}

/// Extract matrix data from a Value::MatFloat.
fn unwrap_mat(v: &Value) -> (&[f64], usize, usize) {
    match v {
        Value::MatFloat { data, rows, cols } => (data.as_slice(), *rows, *cols),
        _ => panic!("expected MatFloat, got {:?}", v),
    }
}

/// Extract float from Value::Float.
fn unwrap_float(v: &Value) -> f64 {
    match v {
        Value::Float(f) => *f,
        _ => panic!("expected Float, got {:?}", v),
    }
}

/// Extract vec from Value::VecFloat.
fn unwrap_vec(v: &Value) -> &[f64] {
    match v {
        Value::VecFloat(v) => v.as_slice(),
        _ => panic!("expected VecFloat, got {:?}", v),
    }
}

// =============================================================================
// Property 11: Transpose Involution
// Feature: math-stats-portfolio-ops, Property 11: Transpose Involution
// =============================================================================

/// Strategy: generate random matrices with 2-5 rows and 2-5 cols.
fn arb_matrix(min_rows: usize, max_rows: usize, min_cols: usize, max_cols: usize)
    -> impl Strategy<Value = (Vec<f64>, usize, usize)>
{
    (min_rows..=max_rows, min_cols..=max_cols).prop_flat_map(|(rows, cols)| {
        let size = rows * cols;
        (
            proptest::collection::vec(-100.0..100.0f64, size..=size),
            Just(rows),
            Just(cols),
        )
    })
}

proptest! {
    /// **Validates: Requirements 11.3**
    ///
    /// For any Mat_Float matrix A, transpose(transpose(A)) == A exactly.
    #[test]
    fn prop_transpose_involution(
        (data, rows, cols) in arb_matrix(2, 5, 2, 5)
    ) {
        let a = mat(data.clone(), rows, cols);

        // First transpose
        let t1 = eval_matrix_op("transpose", &[a.clone()])
            .expect("transpose should not error")
            .expect("transpose should return Some");

        // Second transpose
        let t2 = eval_matrix_op("transpose", &[t1])
            .expect("transpose should not error")
            .expect("transpose should return Some");

        let (result_data, result_rows, result_cols) = unwrap_mat(&t2);

        prop_assert_eq!(result_rows, rows, "rows should match original");
        prop_assert_eq!(result_cols, cols, "cols should match original");

        for i in 0..data.len() {
            prop_assert!(
                (result_data[i] - data[i]).abs() < 1e-15,
                "Element {} differs: got {}, expected {}",
                i, result_data[i], data[i]
            );
        }
    }
}

// =============================================================================
// Property 12: Inverse Round-Trip
// Feature: math-stats-portfolio-ops, Property 12: Inverse Round-Trip
// =============================================================================

/// Strategy: generate well-conditioned square matrices (2-4 size).
/// Uses A = I + small random perturbation to ensure invertibility.
fn arb_well_conditioned_matrix(min_n: usize, max_n: usize)
    -> impl Strategy<Value = (Vec<f64>, usize)>
{
    (min_n..=max_n).prop_flat_map(|n| {
        let size = n * n;
        (
            proptest::collection::vec(-0.3..0.3f64, size..=size),
            Just(n),
        )
    }).prop_map(|(perturbation, n)| {
        // Start with identity and add small perturbation
        let mut data = vec![0.0; n * n];
        for i in 0..n {
            data[i * n + i] = 1.0;
        }
        for i in 0..n * n {
            data[i] += perturbation[i];
        }
        (data, n)
    })
}

proptest! {
    /// **Validates: Requirements 11.4, 11.6**
    ///
    /// For any square invertible matrix A (well-conditioned),
    /// mat_mul(A, inverse(A)) ≈ I within floating-point tolerance.
    #[test]
    fn prop_inverse_round_trip(
        (data, n) in arb_well_conditioned_matrix(2, 4)
    ) {
        let a = mat(data, n, n);

        // Compute inverse
        let inv_result = eval_matrix_op("inverse", &[a.clone()]);
        // Skip if matrix happens to be singular (very unlikely with our generator)
        prop_assume!(inv_result.is_ok());
        let inv_opt = inv_result.unwrap();
        prop_assume!(inv_opt.is_some());
        let inv = inv_opt.unwrap();

        // Compute A * A^(-1)
        let product = eval_matrix_op("mat_mul", &[a, inv])
            .expect("mat_mul should not error")
            .expect("mat_mul should return Some");

        let (prod_data, prod_rows, prod_cols) = unwrap_mat(&product);

        prop_assert_eq!(prod_rows, n);
        prop_assert_eq!(prod_cols, n);

        // Verify approximately identity
        for i in 0..n {
            for j in 0..n {
                let expected = if i == j { 1.0 } else { 0.0 };
                let actual = prod_data[i * n + j];
                prop_assert!(
                    (actual - expected).abs() < 1e-6,
                    "Element ({},{}) of A*A^(-1): got {}, expected {} (tolerance 1e-6)",
                    i, j, actual, expected
                );
            }
        }
    }
}

// =============================================================================
// Property 13: Covariance Matrix Invariants
// Feature: math-stats-portfolio-ops, Property 13: Covariance Matrix Invariants
// =============================================================================

/// Strategy: generate return vectors for cov_matrix testing.
/// Generates n_bars return vectors of n_assets dimensions.
fn arb_return_vectors(min_bars: usize, max_bars: usize, min_assets: usize, max_assets: usize)
    -> impl Strategy<Value = (Vec<Vec<f64>>, usize)>
{
    (min_bars..=max_bars, min_assets..=max_assets).prop_flat_map(|(n_bars, n_assets)| {
        let vecs = proptest::collection::vec(
            proptest::collection::vec(-0.1..0.1f64, n_assets..=n_assets),
            n_bars..=n_bars,
        );
        (vecs, Just(n_assets))
    })
}

proptest! {
    /// **Validates: Requirements 12.1, 12.3, 12.4**
    ///
    /// For any sequence of return vectors fed to cov_matrix over at least `period` bars,
    /// the resulting matrix is symmetric and has non-negative diagonal elements.
    #[test]
    fn prop_covariance_matrix_invariants(
        (return_vecs, n_assets) in arb_return_vectors(3, 5, 2, 4)
    ) {
        let period = return_vecs.len() as i64;
        let mut indicators: HashMap<String, IndicatorStateEntry> = HashMap::new();
        let call_site_key = "test_cov_matrix_0_100";

        // Feed all return vectors into cov_matrix
        let mut last_result = None;
        for returns in &return_vecs {
            let args = vec![
                vecf(returns.clone()),
                Value::Int(period),
            ];
            let result = eval_portfolio_op("cov_matrix", &args, &mut indicators, call_site_key)
                .expect("cov_matrix should not error")
                .expect("cov_matrix should return Some");
            last_result = Some(result);
        }

        let cov = last_result.unwrap();
        let (cov_data, rows, cols) = unwrap_mat(&cov);

        // Verify square matrix of correct dimension
        prop_assert_eq!(rows, n_assets);
        prop_assert_eq!(cols, n_assets);

        // Verify symmetry: cov[i][j] ≈ cov[j][i]
        for i in 0..n_assets {
            for j in 0..n_assets {
                let ij = cov_data[i * n_assets + j];
                let ji = cov_data[j * n_assets + i];
                prop_assert!(
                    (ij - ji).abs() < 1e-12,
                    "Covariance not symmetric: cov[{}][{}]={} vs cov[{}][{}]={}",
                    i, j, ij, j, i, ji
                );
            }
        }

        // Verify non-negative diagonal (variances >= 0)
        for i in 0..n_assets {
            let diag = cov_data[i * n_assets + i];
            prop_assert!(
                diag >= -1e-15,
                "Covariance diagonal[{}] is negative: {}",
                i, diag
            );
        }
    }
}

// =============================================================================
// Property 14: Correlation Matrix Invariants
// Feature: math-stats-portfolio-ops, Property 14: Correlation Matrix Invariants
// =============================================================================

proptest! {
    /// **Validates: Requirements 12.2, 12.5**
    ///
    /// For any sequence of return vectors fed to corr_matrix over at least `period` bars,
    /// the resulting matrix has diagonal ≈ 1.0, all elements in [-1, 1], and is symmetric.
    #[test]
    fn prop_correlation_matrix_invariants(
        (return_vecs, n_assets) in arb_return_vectors(3, 5, 2, 4)
    ) {
        let period = return_vecs.len() as i64;
        let mut indicators: HashMap<String, IndicatorStateEntry> = HashMap::new();
        let call_site_key = "test_corr_matrix_0_200";

        // Feed all return vectors into corr_matrix
        let mut last_result = None;
        for returns in &return_vecs {
            let args = vec![
                vecf(returns.clone()),
                Value::Int(period),
            ];
            let result = eval_portfolio_op("corr_matrix", &args, &mut indicators, call_site_key)
                .expect("corr_matrix should not error")
                .expect("corr_matrix should return Some");
            last_result = Some(result);
        }

        let corr = last_result.unwrap();
        let (corr_data, rows, cols) = unwrap_mat(&corr);

        // Verify square matrix of correct dimension
        prop_assert_eq!(rows, n_assets);
        prop_assert_eq!(cols, n_assets);

        // Verify symmetry
        for i in 0..n_assets {
            for j in 0..n_assets {
                let ij = corr_data[i * n_assets + j];
                let ji = corr_data[j * n_assets + i];
                prop_assert!(
                    (ij - ji).abs() < 1e-12,
                    "Correlation not symmetric: corr[{}][{}]={} vs corr[{}][{}]={}",
                    i, j, ij, j, i, ji
                );
            }
        }

        // Verify diagonal ≈ 1.0
        for i in 0..n_assets {
            let diag = corr_data[i * n_assets + i];
            prop_assert!(
                (diag - 1.0).abs() < 1e-10,
                "Correlation diagonal[{}] should be ≈1.0, got {}",
                i, diag
            );
        }

        // Verify all elements in [-1, 1]
        for i in 0..n_assets {
            for j in 0..n_assets {
                let val = corr_data[i * n_assets + j];
                prop_assert!(
                    val >= -1.0 - 1e-10 && val <= 1.0 + 1e-10,
                    "Correlation[{}][{}]={} outside [-1, 1]",
                    i, j, val
                );
            }
        }
    }
}

// =============================================================================
// Property 15: Min-Variance Weights Constraint Satisfaction
// Feature: math-stats-portfolio-ops, Property 15: Min-Variance Weights Constraint Satisfaction
// =============================================================================

/// Strategy: generate PSD covariance matrices as A^T * A + eps*I.
/// This guarantees positive definiteness (invertibility).
fn arb_psd_matrix(min_n: usize, max_n: usize)
    -> impl Strategy<Value = (Vec<f64>, usize)>
{
    (min_n..=max_n).prop_flat_map(|n| {
        let size = n * n;
        (
            proptest::collection::vec(-1.0..1.0f64, size..=size),
            Just(n),
        )
    }).prop_map(|(raw, n)| {
        // Compute A^T * A (guaranteed PSD)
        let mut result = vec![0.0; n * n];
        for i in 0..n {
            for j in 0..n {
                let mut sum = 0.0;
                for k in 0..n {
                    sum += raw[k * n + i] * raw[k * n + j];
                }
                result[i * n + j] = sum;
            }
        }
        // Add small diagonal to guarantee positive definiteness (invertibility)
        for i in 0..n {
            result[i * n + i] += 0.1;
        }
        (result, n)
    })
}

proptest! {
    /// **Validates: Requirements 13.1, 13.2, 13.3, 13.5**
    ///
    /// For any valid (invertible, PSD) covariance matrix and constraints [0.0, 1.0],
    /// min_variance_weights returns weights that sum ≈ 1.0, are all in [0.0, 1.0],
    /// and the vector length equals the matrix dimension.
    #[test]
    fn prop_min_variance_weights_constraints(
        (cov_data, n) in arb_psd_matrix(2, 4)
    ) {
        let cov = mat(cov_data, n, n);
        let constraints = vecf(vec![0.0, 1.0]);

        let mut indicators: HashMap<String, IndicatorStateEntry> = HashMap::new();
        let result = eval_portfolio_op(
            "min_variance_weights",
            &[cov, constraints],
            &mut indicators,
            "test_mvw_0_300",
        )
            .expect("min_variance_weights should not error")
            .expect("min_variance_weights should return Some");

        let weights = unwrap_vec(&result);

        // Verify vector length equals matrix dimension
        prop_assert_eq!(
            weights.len(), n,
            "Weight vector length {} should equal matrix dimension {}",
            weights.len(), n
        );

        // Verify weights sum ≈ 1.0
        let sum: f64 = weights.iter().sum();
        prop_assert!(
            (sum - 1.0).abs() < 1e-6,
            "Weights sum to {}, expected ≈1.0",
            sum
        );

        // Verify all weights in [0.0, 1.0]
        for (i, &w) in weights.iter().enumerate() {
            prop_assert!(
                w >= -1e-6 && w <= 1.0 + 1e-6,
                "Weight[{}]={} outside [0.0, 1.0]",
                i, w
            );
        }
    }
}

// =============================================================================
// Property 16: Portfolio Variance Non-Negative
// Feature: math-stats-portfolio-ops, Property 16: Portfolio Variance Non-Negative
// =============================================================================

proptest! {
    /// **Validates: Requirements 14.1**
    ///
    /// For any weight vector and PSD covariance matrix of matching dimensions,
    /// portfolio_var(weights, cov) >= 0.0.
    #[test]
    fn prop_portfolio_variance_non_negative(
        (cov_data, n) in arb_psd_matrix(2, 4),
        raw_weights in proptest::collection::vec(0.1..10.0f64, 2..=4usize),
    ) {
        // Only use cases where dimensions match
        prop_assume!(raw_weights.len() == n);

        // Normalize weights to sum to 1
        let sum: f64 = raw_weights.iter().sum();
        let weights: Vec<f64> = raw_weights.iter().map(|x| x / sum).collect();

        let args = vec![
            vecf(weights),
            mat(cov_data, n, n),
        ];

        let mut indicators: HashMap<String, IndicatorStateEntry> = HashMap::new();
        let result = eval_portfolio_op("portfolio_var", &args, &mut indicators, "test_pv_0_400")
            .expect("portfolio_var should not error")
            .expect("portfolio_var should return Some");

        let var = unwrap_float(&result);
        prop_assert!(
            var >= -1e-15,
            "Portfolio variance should be non-negative, got {}",
            var
        );
    }
}


// =============================================================================
// Tier 1 Property Tests — Core Math Builtins
// =============================================================================

// =============================================================================
// Property 1: Exp/Log Round-Trip
// Feature: math-stats-portfolio-ops, Property 1: Exp/Log Round-Trip
// =============================================================================

proptest! {
    /// **Validates: Requirements 1.5, 1.6**
    ///
    /// For any finite positive Float value `x`, evaluating `exp(log(x))` produces
    /// a result approximately equal to `x` within floating-point tolerance
    /// (relative error < 1e-10).
    #[test]
    fn prop_exp_log_round_trip(
        x in 0.001..1e10f64,
    ) {
        // Compute log(x)
        let log_result = eval_math_builtin("log", &[Value::Float(x)])
            .expect("log should not error")
            .expect("log should return Some");

        let log_val = match log_result {
            Value::Float(v) => v,
            _ => panic!("Expected Float from log"),
        };

        // log of positive values should not be NaN
        prop_assume!(!log_val.is_nan());

        // Compute exp(log(x))
        let exp_result = eval_math_builtin("exp", &[Value::Float(log_val)])
            .expect("exp should not error")
            .expect("exp should return Some");

        let result = match exp_result {
            Value::Float(v) => v,
            _ => panic!("Expected Float from exp"),
        };

        // Verify relative error < 1e-10
        let relative_error = ((result - x) / x).abs();
        prop_assert!(
            relative_error < 1e-10,
            "exp(log({})) = {}, relative error = {} (exceeds 1e-10)",
            x, result, relative_error
        );
    }
}

// =============================================================================
// Property 2: Floor/Ceil Bounds
// Feature: math-stats-portfolio-ops, Property 2: Floor/Ceil Bounds
// =============================================================================

proptest! {
    /// **Validates: Requirements 1.8, 1.9**
    ///
    /// For any finite Float value `x`:
    /// - `floor(x) <= x < floor(x) + 1`
    /// - `ceil(x) - 1 < x <= ceil(x)`
    #[test]
    fn prop_floor_ceil_bounds(
        x in -1e6..1e6f64,
    ) {
        // Compute floor(x)
        let floor_result = eval_math_builtin("floor", &[Value::Float(x)])
            .expect("floor should not error")
            .expect("floor should return Some");

        let floor_val = match floor_result {
            Value::Float(v) => v,
            _ => panic!("Expected Float from floor"),
        };

        // Compute ceil(x)
        let ceil_result = eval_math_builtin("ceil", &[Value::Float(x)])
            .expect("ceil should not error")
            .expect("ceil should return Some");

        let ceil_val = match ceil_result {
            Value::Float(v) => v,
            _ => panic!("Expected Float from ceil"),
        };

        // Verify floor(x) <= x < floor(x) + 1
        prop_assert!(
            floor_val <= x,
            "floor({}) = {} > x (violates floor(x) <= x)",
            x, floor_val
        );
        prop_assert!(
            x < floor_val + 1.0,
            "x = {} >= floor(x) + 1 = {} (violates x < floor(x) + 1)",
            x, floor_val + 1.0
        );

        // Verify ceil(x) - 1 < x <= ceil(x)
        prop_assert!(
            ceil_val - 1.0 < x,
            "ceil({}) - 1 = {} >= x (violates ceil(x) - 1 < x)",
            x, ceil_val - 1.0
        );
        prop_assert!(
            x <= ceil_val,
            "x = {} > ceil(x) = {} (violates x <= ceil(x))",
            x, ceil_val
        );
    }
}

// =============================================================================
// Property 3: Sign × Abs Identity
// Feature: math-stats-portfolio-ops, Property 3: Sign × Abs Identity
// =============================================================================

proptest! {
    /// **Validates: Requirements 1.1, 1.11**
    ///
    /// For any finite non-zero Float value `x`, `sign(x) * abs(x) == x` and
    /// `sign(x)` is exactly one of {-1.0, 0.0, 1.0}.
    #[test]
    fn prop_sign_abs_identity(
        x in prop::num::f64::ANY.prop_filter("non-zero finite", |v| *v != 0.0 && v.is_finite()),
    ) {
        // Compute sign(x)
        let sign_result = eval_math_builtin("sign", &[Value::Float(x)])
            .expect("sign should not error")
            .expect("sign should return Some");

        let sign_val = match sign_result {
            Value::Float(v) => v,
            _ => panic!("Expected Float from sign"),
        };

        // Compute abs(x)
        let abs_result = eval_math_builtin("abs", &[Value::Float(x)])
            .expect("abs should not error")
            .expect("abs should return Some");

        let abs_val = match abs_result {
            Value::Float(v) => v,
            _ => panic!("Expected Float from abs"),
        };

        // Verify sign(x) is one of {-1.0, 0.0, 1.0}
        prop_assert!(
            sign_val == -1.0 || sign_val == 0.0 || sign_val == 1.0,
            "sign({}) = {} is not one of {{-1.0, 0.0, 1.0}}",
            x, sign_val
        );

        // Verify sign(x) * abs(x) ≈ x
        let product = sign_val * abs_val;
        let tolerance = 1e-10 * x.abs().max(1.0);
        prop_assert!(
            (product - x).abs() < tolerance,
            "sign({}) * abs({}) = {} * {} = {}, expected {} (diff={})",
            x, x, sign_val, abs_val, product, x, (product - x).abs()
        );
    }
}

// =============================================================================
// Property 4: Min/Max Semantics
// Feature: math-stats-portfolio-ops, Property 4: Min/Max Semantics
// =============================================================================

proptest! {
    /// **Validates: Requirements 2.2, 2.3**
    ///
    /// For any two numeric values `a` and `b`:
    /// - min(a, b) <= a and min(a, b) <= b
    /// - min(a, b) equals either a or b
    /// - max(a, b) >= a and max(a, b) >= b
    /// - max(a, b) equals either a or b
    #[test]
    fn prop_min_max_semantics(
        a in -1e6..1e6f64,
        b in -1e6..1e6f64,
    ) {
        // Compute min(a, b)
        let min_result = eval_math_builtin("min", &[Value::Float(a), Value::Float(b)])
            .expect("min should not error")
            .expect("min should return Some");

        let min_val = match min_result {
            Value::Float(v) => v,
            _ => panic!("Expected Float from min"),
        };

        // Compute max(a, b)
        let max_result = eval_math_builtin("max", &[Value::Float(a), Value::Float(b)])
            .expect("max should not error")
            .expect("max should return Some");

        let max_val = match max_result {
            Value::Float(v) => v,
            _ => panic!("Expected Float from max"),
        };

        // min(a, b) <= a
        prop_assert!(
            min_val <= a,
            "min({}, {}) = {} > a",
            a, b, min_val
        );
        // min(a, b) <= b
        prop_assert!(
            min_val <= b,
            "min({}, {}) = {} > b",
            a, b, min_val
        );
        // min(a, b) equals one of {a, b}
        prop_assert!(
            min_val == a || min_val == b,
            "min({}, {}) = {} is neither a nor b",
            a, b, min_val
        );

        // max(a, b) >= a
        prop_assert!(
            max_val >= a,
            "max({}, {}) = {} < a",
            a, b, max_val
        );
        // max(a, b) >= b
        prop_assert!(
            max_val >= b,
            "max({}, {}) = {} < b",
            a, b, max_val
        );
        // max(a, b) equals one of {a, b}
        prop_assert!(
            max_val == a || max_val == b,
            "max({}, {}) = {} is neither a nor b",
            a, b, max_val
        );
    }
}


// =============================================================================
// Cross-Tier Property Tests
// =============================================================================

// =============================================================================
// Property 17: Stateless Function Determinism
// Feature: math-stats-portfolio-ops, Property 17: Stateless Function Determinism
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// **Validates: Requirements 16.1, 16.2**
    ///
    /// For any Tier 1 math function, calling the function twice with identical
    /// arguments produces identical results.
    #[test]
    fn prop_stateless_determinism_tier1(
        x in -1000.0..1000.0f64,
        y in -1000.0..1000.0f64,
    ) {
        // Tier 1 single-arg functions
        let single_arg_fns = ["abs", "sqrt", "exp", "log", "floor", "ceil", "round", "sign"];
        for func_name in &single_arg_fns {
            let args = [Value::Float(x)];
            let result1 = eval_math_builtin(func_name, &args);
            let result2 = eval_math_builtin(func_name, &args);

            match (result1, result2) {
                (Ok(Some(Value::Float(a))), Ok(Some(Value::Float(b)))) => {
                    // NaN == NaN for determinism purposes (both should be NaN or both equal)
                    if a.is_nan() {
                        prop_assert!(b.is_nan(),
                            "{}: first call returned NaN but second returned {}",
                            func_name, b);
                    } else {
                        prop_assert_eq!(a, b,
                            "{}: results differ: {} vs {}", func_name, a, b);
                    }
                }
                (Ok(Some(Value::Int(a))), Ok(Some(Value::Int(b)))) => {
                    prop_assert_eq!(a, b,
                        "{}: Int results differ: {} vs {}", func_name, a, b);
                }
                (Ok(r1), Ok(r2)) => {
                    prop_assert_eq!(format!("{:?}", r1), format!("{:?}", r2),
                        "{}: results differ", func_name);
                }
                (Err(e1), Err(e2)) => {
                    prop_assert_eq!(e1, e2,
                        "{}: errors differ", func_name);
                }
                (r1, r2) => {
                    prop_assert!(false,
                        "{}: result types differ: {:?} vs {:?}", func_name, r1, r2);
                }
            }
        }

        // Tier 1 two-arg functions
        let two_arg_fns = ["pow", "min", "max"];
        for func_name in &two_arg_fns {
            let args = [Value::Float(x), Value::Float(y)];
            let result1 = eval_math_builtin(func_name, &args);
            let result2 = eval_math_builtin(func_name, &args);

            match (result1, result2) {
                (Ok(Some(Value::Float(a))), Ok(Some(Value::Float(b)))) => {
                    if a.is_nan() {
                        prop_assert!(b.is_nan(),
                            "{}: first call returned NaN but second returned {}",
                            func_name, b);
                    } else {
                        prop_assert_eq!(a, b,
                            "{}: results differ: {} vs {}", func_name, a, b);
                    }
                }
                (Ok(r1), Ok(r2)) => {
                    prop_assert_eq!(format!("{:?}", r1), format!("{:?}", r2),
                        "{}: results differ", func_name);
                }
                (Err(e1), Err(e2)) => {
                    prop_assert_eq!(e1, e2,
                        "{}: errors differ", func_name);
                }
                (r1, r2) => {
                    prop_assert!(false,
                        "{}: result types differ: {:?} vs {:?}", func_name, r1, r2);
                }
            }
        }
    }

    /// **Validates: Requirements 16.1, 16.2**
    ///
    /// For Tier 3 stateless functions (transpose, det), calling the function
    /// twice with the same matrix produces identical results.
    #[test]
    fn prop_stateless_determinism_tier3(
        (data, rows, cols) in arb_matrix(2, 4, 2, 4),
    ) {
        // Transpose is valid for any matrix
        let m = mat(data.clone(), rows, cols);
        let t1 = eval_matrix_op("transpose", &[m.clone()]);
        let t2 = eval_matrix_op("transpose", &[m.clone()]);
        match (t1, t2) {
            (Ok(Some(ref v1)), Ok(Some(ref v2))) => {
                let (d1, r1, c1) = unwrap_mat(v1);
                let (d2, r2, c2) = unwrap_mat(v2);
                prop_assert_eq!(r1, r2, "transpose: rows differ");
                prop_assert_eq!(c1, c2, "transpose: cols differ");
                for i in 0..d1.len() {
                    prop_assert_eq!(d1[i], d2[i],
                        "transpose: element {} differs: {} vs {}", i, d1[i], d2[i]);
                }
            }
            (Err(e1), Err(e2)) => {
                prop_assert_eq!(e1, e2, "transpose: errors differ");
            }
            (r1, r2) => {
                prop_assert!(false, "transpose: result types differ: {:?} vs {:?}", r1, r2);
            }
        }

        // Det is only valid for square matrices
        if rows == cols {
            let sq = mat(data.clone(), rows, cols);
            let d1 = eval_matrix_op("det", &[sq.clone()]);
            let d2 = eval_matrix_op("det", &[sq.clone()]);
            match (d1, d2) {
                (Ok(Some(Value::Float(a))), Ok(Some(Value::Float(b)))) => {
                    if a.is_nan() {
                        prop_assert!(b.is_nan(), "det: first NaN but second {}", b);
                    } else {
                        prop_assert_eq!(a, b, "det: results differ: {} vs {}", a, b);
                    }
                }
                (Err(e1), Err(e2)) => {
                    prop_assert_eq!(e1, e2, "det: errors differ");
                }
                (r1, r2) => {
                    prop_assert!(false, "det: result types differ: {:?} vs {:?}", r1, r2);
                }
            }
        }
    }
}

// =============================================================================
// Property 18: Argument Count Validation
// Feature: math-stats-portfolio-ops, Property 18: Argument Count Validation
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// **Validates: Requirements 17.1, 17.2**
    ///
    /// For Tier 1 single-arg functions, calling with 0 or 2+ args produces an
    /// error containing the function name.
    #[test]
    fn prop_arg_count_validation_tier1_single(
        x in -100.0..100.0f64,
        y in -100.0..100.0f64,
    ) {
        let single_arg_fns = ["abs", "sqrt", "exp", "log", "floor", "ceil", "round", "sign"];

        for func_name in &single_arg_fns {
            // 0 args — should error
            let result_zero = eval_math_builtin(func_name, &[]);
            match result_zero {
                Err(ref e) => {
                    prop_assert!(
                        e.to_lowercase().contains(&func_name.to_lowercase()),
                        "{} with 0 args: error '{}' does not contain function name",
                        func_name, e
                    );
                }
                other => {
                    prop_assert!(false,
                        "{} with 0 args should error, got {:?}", func_name, other);
                }
            }

            // 2 args — should error
            let result_two = eval_math_builtin(func_name, &[Value::Float(x), Value::Float(y)]);
            match result_two {
                Err(ref e) => {
                    prop_assert!(
                        e.to_lowercase().contains(&func_name.to_lowercase()),
                        "{} with 2 args: error '{}' does not contain function name",
                        func_name, e
                    );
                }
                other => {
                    prop_assert!(false,
                        "{} with 2 args should error, got {:?}", func_name, other);
                }
            }
        }
    }

    /// **Validates: Requirements 17.1, 17.2**
    ///
    /// For Tier 1 two-arg functions (pow, min, max), calling with 0, 1, or 3+
    /// args produces an error containing the function name.
    #[test]
    fn prop_arg_count_validation_tier1_two(
        x in -100.0..100.0f64,
        y in -100.0..100.0f64,
        z in -100.0..100.0f64,
    ) {
        let two_arg_fns = ["pow", "min", "max"];

        for func_name in &two_arg_fns {
            // 0 args — should error
            let result_zero = eval_math_builtin(func_name, &[]);
            match result_zero {
                Err(ref e) => {
                    prop_assert!(
                        e.to_lowercase().contains(&func_name.to_lowercase()),
                        "{} with 0 args: error '{}' does not contain function name",
                        func_name, e
                    );
                }
                other => {
                    prop_assert!(false,
                        "{} with 0 args should error, got {:?}", func_name, other);
                }
            }

            // 1 arg — should error
            let result_one = eval_math_builtin(func_name, &[Value::Float(x)]);
            match result_one {
                Err(ref e) => {
                    prop_assert!(
                        e.to_lowercase().contains(&func_name.to_lowercase()),
                        "{} with 1 arg: error '{}' does not contain function name",
                        func_name, e
                    );
                }
                other => {
                    prop_assert!(false,
                        "{} with 1 arg should error, got {:?}", func_name, other);
                }
            }

            // 3 args — should error
            let result_three = eval_math_builtin(func_name, &[Value::Float(x), Value::Float(y), Value::Float(z)]);
            match result_three {
                Err(ref e) => {
                    prop_assert!(
                        e.to_lowercase().contains(&func_name.to_lowercase()),
                        "{} with 3 args: error '{}' does not contain function name",
                        func_name, e
                    );
                }
                other => {
                    prop_assert!(false,
                        "{} with 3 args should error, got {:?}", func_name, other);
                }
            }
        }
    }

    /// **Validates: Requirements 17.1, 17.2**
    ///
    /// For Tier 3 functions (det, transpose, mat_mul), calling with wrong arg
    /// counts produces an error containing the function name.
    #[test]
    fn prop_arg_count_validation_tier3(
        (data, rows, cols) in arb_matrix(2, 3, 2, 3),
    ) {
        let m = mat(data.clone(), rows, cols);

        // det requires 1 arg — test with 0 and 2
        let det_zero = eval_matrix_op("det", &[]);
        match det_zero {
            Err(ref e) => {
                prop_assert!(
                    e.to_lowercase().contains("det"),
                    "det with 0 args: error '{}' does not contain 'det'", e
                );
            }
            other => {
                prop_assert!(false, "det with 0 args should error, got {:?}", other);
            }
        }

        let det_two = eval_matrix_op("det", &[m.clone(), m.clone()]);
        match det_two {
            Err(ref e) => {
                prop_assert!(
                    e.to_lowercase().contains("det"),
                    "det with 2 args: error '{}' does not contain 'det'", e
                );
            }
            other => {
                prop_assert!(false, "det with 2 args should error, got {:?}", other);
            }
        }

        // transpose requires 1 arg — test with 0 and 2
        let transpose_zero = eval_matrix_op("transpose", &[]);
        match transpose_zero {
            Err(ref e) => {
                prop_assert!(
                    e.to_lowercase().contains("transpose"),
                    "transpose with 0 args: error '{}' does not contain 'transpose'", e
                );
            }
            other => {
                prop_assert!(false, "transpose with 0 args should error, got {:?}", other);
            }
        }

        let transpose_two = eval_matrix_op("transpose", &[m.clone(), m.clone()]);
        match transpose_two {
            Err(ref e) => {
                prop_assert!(
                    e.to_lowercase().contains("transpose"),
                    "transpose with 2 args: error '{}' does not contain 'transpose'", e
                );
            }
            other => {
                prop_assert!(false, "transpose with 2 args should error, got {:?}", other);
            }
        }

        // mat_mul requires 2 args — test with 0 and 1
        let matmul_zero = eval_matrix_op("mat_mul", &[]);
        match matmul_zero {
            Err(ref e) => {
                prop_assert!(
                    e.to_lowercase().contains("mat_mul"),
                    "mat_mul with 0 args: error '{}' does not contain 'mat_mul'", e
                );
            }
            other => {
                prop_assert!(false, "mat_mul with 0 args should error, got {:?}", other);
            }
        }

        let matmul_one = eval_matrix_op("mat_mul", &[m.clone()]);
        match matmul_one {
            Err(ref e) => {
                prop_assert!(
                    e.to_lowercase().contains("mat_mul"),
                    "mat_mul with 1 arg: error '{}' does not contain 'mat_mul'", e
                );
            }
            other => {
                prop_assert!(false, "mat_mul with 1 arg should error, got {:?}", other);
            }
        }
    }
}
