//! Tier 3 — Matrix Operations and Portfolio Operations.
//!
//! Provides stateless matrix operations (`mat_mul`, `transpose`, `det`, `inverse`)
//! and portfolio-specific operations (`cov_matrix`, `corr_matrix`, `min_variance_weights`,
//! `portfolio_var`, `sharpe`).
//!
//! Uses row-major flat storage: element (i, j) is at index `i * cols + j`.

use std::collections::HashMap;

use crate::interpreter::{IndicatorStateEntry, Value};

/// Attempt to evaluate a Tier 3 matrix operation by name.
///
/// Returns:
/// - `Ok(Some(value))` if `name` is a recognized matrix operation and evaluation succeeds
/// - `Ok(None)` if `name` is not a matrix operation (caller should try next dispatch tier)
/// - `Err(String)` on validation errors (wrong arg count, wrong type, dimension mismatch)
pub fn eval_matrix_op(name: &str, args: &[Value]) -> Result<Option<Value>, String> {
    match name {
        "mat_mul" => eval_mat_mul(args),
        "transpose" => eval_transpose(args),
        "det" => eval_det(args),
        "inverse" => eval_inverse(args),
        _ => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// mat_mul(a, b) — matrix multiplication
// ---------------------------------------------------------------------------

fn eval_mat_mul(args: &[Value]) -> Result<Option<Value>, String> {
    check_arity("mat_mul", args, 2)?;

    let (a_data, a_rows, a_cols) = extract_matrix("mat_mul", &args[0])?;
    let (b_data, b_rows, b_cols) = extract_matrix("mat_mul", &args[1])?;

    if a_cols != b_rows {
        return Err(format!(
            "mat_mul: incompatible dimensions ({}x{}) * ({}x{})",
            a_rows, a_cols, b_rows, b_cols
        ));
    }

    let result = matrix_multiply(a_data, a_rows, a_cols, b_data, b_cols);
    Ok(Some(Value::MatFloat {
        data: result,
        rows: a_rows,
        cols: b_cols,
    }))
}

// ---------------------------------------------------------------------------
// transpose(m) — matrix transpose
// ---------------------------------------------------------------------------

fn eval_transpose(args: &[Value]) -> Result<Option<Value>, String> {
    check_arity("transpose", args, 1)?;

    let (data, rows, cols) = extract_matrix("transpose", &args[0])?;

    let mut result = vec![0.0; rows * cols];
    for i in 0..rows {
        for j in 0..cols {
            result[j * rows + i] = data[i * cols + j];
        }
    }

    Ok(Some(Value::MatFloat {
        data: result,
        rows: cols,
        cols: rows,
    }))
}

// ---------------------------------------------------------------------------
// det(m) — determinant via Gaussian elimination
// ---------------------------------------------------------------------------

fn eval_det(args: &[Value]) -> Result<Option<Value>, String> {
    check_arity("det", args, 1)?;

    let (data, rows, cols) = extract_matrix("det", &args[0])?;

    if rows != cols {
        return Err(format!(
            "det: requires a square matrix, got {}x{}",
            rows, cols
        ));
    }

    let determinant = matrix_determinant(data, rows);
    Ok(Some(Value::Float(determinant)))
}

// ---------------------------------------------------------------------------
// inverse(m) — matrix inverse via Gaussian elimination with partial pivoting
// ---------------------------------------------------------------------------

fn eval_inverse(args: &[Value]) -> Result<Option<Value>, String> {
    check_arity("inverse", args, 1)?;

    let (data, rows, cols) = extract_matrix("inverse", &args[0])?;

    if rows != cols {
        return Err(format!(
            "inverse: requires a square matrix, got {}x{}",
            rows, cols
        ));
    }

    let inv = matrix_inverse(data, rows)?;
    Ok(Some(Value::MatFloat {
        data: inv,
        rows,
        cols,
    }))
}

// ===========================================================================
// Portfolio operations (some stateful)
// ===========================================================================

/// Attempt to evaluate a portfolio-specific operation by name.
///
/// Returns:
/// - `Ok(Some(value))` if `name` is a recognized portfolio operation and evaluation succeeds
/// - `Ok(None)` if `name` is not a portfolio operation (caller should try next dispatch tier)
/// - `Err(String)` on validation errors
pub fn eval_portfolio_op(
    name: &str,
    args: &[Value],
    indicators: &mut HashMap<String, IndicatorStateEntry>,
    call_site_key: &str,
) -> Result<Option<Value>, String> {
    match name {
        "cov_matrix" => eval_cov_matrix(args, indicators, call_site_key),
        "corr_matrix" => eval_corr_matrix(args, indicators, call_site_key),
        "min_variance_weights" => eval_min_variance_weights(args),
        "portfolio_var" => eval_portfolio_var(args),
        "sharpe" => eval_sharpe(args),
        _ => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// cov_matrix(returns: VecFloat, period: Int) -> MatFloat
// ---------------------------------------------------------------------------

fn eval_cov_matrix(
    args: &[Value],
    indicators: &mut HashMap<String, IndicatorStateEntry>,
    call_site_key: &str,
) -> Result<Option<Value>, String> {
    check_arity("cov_matrix", args, 2)?;

    let returns = extract_vec_float("cov_matrix", &args[0])?;
    let period = extract_int("cov_matrix", &args[1])?;

    if period <= 0 {
        return Err("cov_matrix: period must be a positive integer".to_string());
    }
    let period = period as usize;
    let n_assets = returns.len();

    let key = call_site_key.to_string();
    let entry = indicators.entry(key).or_insert_with(|| {
        IndicatorStateEntry::RollingMatrix {
            window: vec![vec![0.0; n_assets]; period],
            period,
            index: 0,
            count: 0,
            n_assets,
        }
    });

    match entry {
        IndicatorStateEntry::RollingMatrix {
            window,
            period: p,
            index,
            count,
            n_assets: n,
        } => {
            if returns.len() != *n {
                return Err(format!(
                    "cov_matrix: expected {} assets, got {}",
                    *n,
                    returns.len()
                ));
            }

            // Push returns into the circular buffer
            window[*index] = returns.to_vec();
            *index = (*index + 1) % *p;
            if *count < *p {
                *count += 1;
            }

            // Compute covariance matrix from the window contents
            let actual_count = *count;
            let cov = compute_covariance_matrix(window, actual_count, *n);

            Ok(Some(Value::MatFloat {
                data: cov,
                rows: *n,
                cols: *n,
            }))
        }
        _ => Err("indicator state mismatch for cov_matrix".to_string()),
    }
}

// ---------------------------------------------------------------------------
// corr_matrix(returns: VecFloat, period: Int) -> MatFloat
// ---------------------------------------------------------------------------

fn eval_corr_matrix(
    args: &[Value],
    indicators: &mut HashMap<String, IndicatorStateEntry>,
    call_site_key: &str,
) -> Result<Option<Value>, String> {
    check_arity("corr_matrix", args, 2)?;

    let returns = extract_vec_float("corr_matrix", &args[0])?;
    let period = extract_int("corr_matrix", &args[1])?;

    if period <= 0 {
        return Err("corr_matrix: period must be a positive integer".to_string());
    }
    let period = period as usize;
    let n_assets = returns.len();

    let key = call_site_key.to_string();
    let entry = indicators.entry(key).or_insert_with(|| {
        IndicatorStateEntry::RollingMatrix {
            window: vec![vec![0.0; n_assets]; period],
            period,
            index: 0,
            count: 0,
            n_assets,
        }
    });

    match entry {
        IndicatorStateEntry::RollingMatrix {
            window,
            period: p,
            index,
            count,
            n_assets: n,
        } => {
            if returns.len() != *n {
                return Err(format!(
                    "corr_matrix: expected {} assets, got {}",
                    *n,
                    returns.len()
                ));
            }

            // Push returns into the circular buffer
            window[*index] = returns.to_vec();
            *index = (*index + 1) % *p;
            if *count < *p {
                *count += 1;
            }

            // Compute covariance matrix first, then normalize to correlation
            let actual_count = *count;
            let cov = compute_covariance_matrix(window, actual_count, *n);
            let corr = normalize_to_correlation(&cov, *n);

            Ok(Some(Value::MatFloat {
                data: corr,
                rows: *n,
                cols: *n,
            }))
        }
        _ => Err("indicator state mismatch for corr_matrix".to_string()),
    }
}

// ---------------------------------------------------------------------------
// min_variance_weights(cov_matrix: MatFloat, constraints: VecFloat) -> VecFloat
// ---------------------------------------------------------------------------

fn eval_min_variance_weights(args: &[Value]) -> Result<Option<Value>, String> {
    check_arity("min_variance_weights", args, 2)?;

    let (cov_data, rows, cols) = extract_matrix("min_variance_weights", &args[0])?;
    let constraints = extract_vec_float("min_variance_weights", &args[1])?;

    if rows != cols {
        return Err(format!(
            "min_variance_weights: requires a square matrix, got {}x{}",
            rows, cols
        ));
    }

    let n = rows;

    // Extract constraints [min_weight, max_weight]
    if constraints.len() != 2 {
        return Err(
            "min_variance_weights: constraints must be [min_weight, max_weight]".to_string(),
        );
    }
    let min_w = constraints[0];
    let max_w = constraints[1];

    // Invert the covariance matrix
    let cov_inv = matrix_inverse(cov_data, n)
        .map_err(|_| "min_variance_weights: covariance matrix is not invertible".to_string())?;

    // Unconstrained solution: w = C⁻¹ * 1 / (1ᵀ * C⁻¹ * 1)
    let mut weights = min_variance_unconstrained(&cov_inv, n);

    // Project weights to [min_w, max_w] and renormalize
    project_weights(&mut weights, min_w, max_w, 100);

    Ok(Some(Value::VecFloat(weights)))
}

// ---------------------------------------------------------------------------
// portfolio_var(weights: VecFloat, cov_matrix: MatFloat) -> Float
// ---------------------------------------------------------------------------

fn eval_portfolio_var(args: &[Value]) -> Result<Option<Value>, String> {
    check_arity("portfolio_var", args, 2)?;

    let weights = extract_vec_float("portfolio_var", &args[0])?;
    let (cov_data, rows, cols) = extract_matrix("portfolio_var", &args[1])?;

    if rows != cols {
        return Err(format!(
            "portfolio_var: requires a square covariance matrix, got {}x{}",
            rows, cols
        ));
    }

    let n = weights.len();
    let m = rows;

    if n != m {
        return Err(format!(
            "portfolio_var: weight vector length ({}) doesn't match matrix dimension ({})",
            n, m
        ));
    }

    // Compute w^T * C * w
    // First compute C * w
    let mut cw = vec![0.0; n];
    for i in 0..n {
        for j in 0..n {
            cw[i] += cov_data[i * n + j] * weights[j];
        }
    }

    // Then compute w^T * (C * w)
    let mut var = 0.0;
    for i in 0..n {
        var += weights[i] * cw[i];
    }

    Ok(Some(Value::Float(var)))
}

// ---------------------------------------------------------------------------
// sharpe(returns: VecFloat, rf_rate: Float) -> Float
// ---------------------------------------------------------------------------

fn eval_sharpe(args: &[Value]) -> Result<Option<Value>, String> {
    check_arity("sharpe", args, 2)?;

    let returns = extract_vec_float("sharpe", &args[0])?;
    let rf_rate = extract_float("sharpe", &args[1])?;

    if returns.is_empty() {
        return Ok(Some(Value::Float(0.0)));
    }

    let n = returns.len() as f64;
    let mean = returns.iter().sum::<f64>() / n;

    // Compute population standard deviation
    let variance = returns.iter().map(|r| (r - mean) * (r - mean)).sum::<f64>() / n;
    let stddev = variance.sqrt();

    if stddev == 0.0 {
        return Ok(Some(Value::Float(0.0)));
    }

    let sharpe = (mean - rf_rate) / stddev;
    Ok(Some(Value::Float(sharpe)))
}

// ===========================================================================
// Portfolio helper functions
// ===========================================================================

/// Compute the covariance matrix from a rolling window of return vectors.
/// Uses population covariance (divides by count, not count-1).
fn compute_covariance_matrix(window: &[Vec<f64>], count: usize, n_assets: usize) -> Vec<f64> {
    let mut cov = vec![0.0; n_assets * n_assets];

    if count == 0 {
        return cov;
    }

    // Compute means for each asset
    let mut means = vec![0.0; n_assets];
    for t in 0..count {
        for i in 0..n_assets {
            means[i] += window[t][i];
        }
    }
    for i in 0..n_assets {
        means[i] /= count as f64;
    }

    // Compute covariance: cov(i,j) = (1/count) * sum((r_i_t - mean_i) * (r_j_t - mean_j))
    for t in 0..count {
        for i in 0..n_assets {
            let di = window[t][i] - means[i];
            for j in 0..n_assets {
                let dj = window[t][j] - means[j];
                cov[i * n_assets + j] += di * dj;
            }
        }
    }

    for i in 0..n_assets * n_assets {
        cov[i] /= count as f64;
    }

    cov
}

/// Normalize a covariance matrix to a correlation matrix.
/// diagonal elements become 1.0, off-diagonal elements are bounded to [-1, 1].
fn normalize_to_correlation(cov: &[f64], n: usize) -> Vec<f64> {
    let mut corr = vec![0.0; n * n];

    // Extract standard deviations (sqrt of diagonal)
    let stddevs: Vec<f64> = (0..n).map(|i| cov[i * n + i].sqrt()).collect();

    for i in 0..n {
        for j in 0..n {
            if i == j {
                corr[i * n + j] = 1.0;
            } else if stddevs[i] == 0.0 || stddevs[j] == 0.0 {
                corr[i * n + j] = 0.0;
            } else {
                let r = cov[i * n + j] / (stddevs[i] * stddevs[j]);
                // Clamp to [-1, 1] for numerical safety
                corr[i * n + j] = r.clamp(-1.0, 1.0);
            }
        }
    }

    corr
}

/// Compute unconstrained minimum-variance weights: w = C⁻¹ * 1 / (1ᵀ * C⁻¹ * 1)
fn min_variance_unconstrained(cov_inv: &[f64], n: usize) -> Vec<f64> {
    // C⁻¹ * 1 (sum each row of the inverse)
    let mut cov_inv_ones = vec![0.0; n];
    for i in 0..n {
        for j in 0..n {
            cov_inv_ones[i] += cov_inv[i * n + j];
        }
    }

    // 1ᵀ * C⁻¹ * 1 (sum of all elements in the inverse)
    let denom: f64 = cov_inv_ones.iter().sum();

    if denom.abs() < 1e-15 {
        // Fallback: equal weights
        return vec![1.0 / n as f64; n];
    }

    // w = (C⁻¹ * 1) / (1ᵀ * C⁻¹ * 1)
    cov_inv_ones.iter().map(|x| x / denom).collect()
}

/// Iteratively project weights onto [min_w, max_w] constraints and renormalize to sum=1.
/// Uses up to `max_iters` iterations to converge.
fn project_weights(weights: &mut [f64], min_w: f64, max_w: f64, max_iters: usize) {
    for _ in 0..max_iters {
        let mut clamped = false;

        // Clamp weights to [min_w, max_w]
        for w in weights.iter_mut() {
            if *w < min_w {
                *w = min_w;
                clamped = true;
            } else if *w > max_w {
                *w = max_w;
                clamped = true;
            }
        }

        // Renormalize to sum to 1.0
        let sum: f64 = weights.iter().sum();
        if sum.abs() > 1e-15 {
            for w in weights.iter_mut() {
                *w /= sum;
            }
        }

        if !clamped {
            break;
        }
    }
}

// ===========================================================================
// Internal helpers
// ===========================================================================

/// Validate argument count.
fn check_arity(name: &str, args: &[Value], expected: usize) -> Result<(), String> {
    if args.len() != expected {
        Err(format!("{} requires {} argument(s)", name, expected))
    } else {
        Ok(())
    }
}

/// Extract matrix data from a Value, returning an error if the value is not MatFloat.
fn extract_matrix<'a>(name: &str, value: &'a Value) -> Result<(&'a [f64], usize, usize), String> {
    match value {
        Value::MatFloat { data, rows, cols } => Ok((data.as_slice(), *rows, *cols)),
        other => Err(format!(
            "{}: expected MatFloat, got {}",
            name,
            value_type_name(other)
        )),
    }
}

/// Extract a VecFloat from a Value, returning an error if the value is not VecFloat.
fn extract_vec_float<'a>(name: &str, value: &'a Value) -> Result<&'a [f64], String> {
    match value {
        Value::VecFloat(v) => Ok(v.as_slice()),
        other => Err(format!(
            "{}: expected VecFloat, got {}",
            name,
            value_type_name(other)
        )),
    }
}

/// Extract an integer from a Value, returning an error if the value is not Int.
fn extract_int(name: &str, value: &Value) -> Result<i64, String> {
    match value {
        Value::Int(i) => Ok(*i),
        other => Err(format!(
            "{}: expected Int, got {}",
            name,
            value_type_name(other)
        )),
    }
}

/// Extract a float from a Value, returning an error if the value is not Float (or Int promoted).
fn extract_float(name: &str, value: &Value) -> Result<f64, String> {
    match value {
        Value::Float(f) => Ok(*f),
        Value::Int(i) => Ok(*i as f64),
        other => Err(format!(
            "{}: expected Float, got {}",
            name,
            value_type_name(other)
        )),
    }
}

/// Return a human-readable type name for a Value variant.
fn value_type_name(v: &Value) -> &'static str {
    match v {
        Value::Int(_) => "Int",
        Value::Float(_) => "Float",
        Value::Str(_) => "Str",
        Value::Bool(_) => "Bool",
        Value::Null => "Null",
        Value::List(_) => "List",
        Value::Signal(_) => "Signal",
        Value::VecFloat(_) => "VecFloat",
        Value::MatFloat { .. } => "MatFloat",
        Value::Struct { .. } => "Struct",
    }
}

// ===========================================================================
// Core matrix algorithms
// ===========================================================================

/// Multiply matrix A (a_rows × a_cols) by matrix B (a_cols × b_cols).
/// Returns the product matrix (a_rows × b_cols) in row-major order.
fn matrix_multiply(a: &[f64], a_rows: usize, a_cols: usize, b: &[f64], b_cols: usize) -> Vec<f64> {
    let mut result = vec![0.0; a_rows * b_cols];
    for i in 0..a_rows {
        for k in 0..a_cols {
            let a_ik = a[i * a_cols + k];
            for j in 0..b_cols {
                result[i * b_cols + j] += a_ik * b[k * b_cols + j];
            }
        }
    }
    result
}

/// Compute the determinant of an n×n matrix using Gaussian elimination
/// with partial pivoting.
fn matrix_determinant(data: &[f64], n: usize) -> f64 {
    if n == 0 {
        return 1.0;
    }
    if n == 1 {
        return data[0];
    }

    // Work on a copy
    let mut m = data.to_vec();
    let mut det = 1.0;

    for col in 0..n {
        // Partial pivoting: find the row with the largest absolute value in this column
        let mut max_row = col;
        let mut max_val = m[col * n + col].abs();
        for row in (col + 1)..n {
            let val = m[row * n + col].abs();
            if val > max_val {
                max_val = val;
                max_row = row;
            }
        }

        // If the pivot is effectively zero, determinant is zero
        if max_val < 1e-15 {
            return 0.0;
        }

        // Swap rows if needed
        if max_row != col {
            for j in 0..n {
                m.swap(col * n + j, max_row * n + j);
            }
            det = -det; // Row swap flips sign
        }

        let pivot = m[col * n + col];
        det *= pivot;

        // Eliminate below
        for row in (col + 1)..n {
            let factor = m[row * n + col] / pivot;
            for j in col..n {
                m[row * n + j] -= factor * m[col * n + j];
            }
        }
    }

    det
}

/// Compute the inverse of an n×n matrix using Gaussian elimination
/// with partial pivoting. Returns Err if the matrix is singular.
fn matrix_inverse(data: &[f64], n: usize) -> Result<Vec<f64>, String> {
    if n == 0 {
        return Ok(vec![]);
    }

    // Augmented matrix [A | I] stored as n × 2n
    let mut aug = vec![0.0; n * 2 * n];
    for i in 0..n {
        for j in 0..n {
            aug[i * (2 * n) + j] = data[i * n + j];
        }
        aug[i * (2 * n) + n + i] = 1.0; // Identity on the right
    }

    let cols = 2 * n;

    // Forward elimination with partial pivoting
    for col in 0..n {
        // Find pivot
        let mut max_row = col;
        let mut max_val = aug[col * cols + col].abs();
        for row in (col + 1)..n {
            let val = aug[row * cols + col].abs();
            if val > max_val {
                max_val = val;
                max_row = row;
            }
        }

        if max_val < 1e-12 {
            return Err("inverse: matrix is singular (determinant ≈ 0)".to_string());
        }

        // Swap rows
        if max_row != col {
            for j in 0..cols {
                aug.swap(col * cols + j, max_row * cols + j);
            }
        }

        // Scale pivot row
        let pivot = aug[col * cols + col];
        for j in 0..cols {
            aug[col * cols + j] /= pivot;
        }

        // Eliminate all other rows in this column
        for row in 0..n {
            if row == col {
                continue;
            }
            let factor = aug[row * cols + col];
            for j in 0..cols {
                aug[row * cols + j] -= factor * aug[col * cols + j];
            }
        }
    }

    // Extract the inverse from the right half
    let mut inv = vec![0.0; n * n];
    for i in 0..n {
        for j in 0..n {
            inv[i * n + j] = aug[i * cols + n + j];
        }
    }

    Ok(inv)
}

// ===========================================================================
// Unit tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to create a MatFloat value
    fn mat(data: Vec<f64>, rows: usize, cols: usize) -> Value {
        Value::MatFloat { data, rows, cols }
    }

    // -----------------------------------------------------------------------
    // mat_mul tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_mat_mul_2x2() {
        // [1 2] * [5 6] = [1*5+2*7  1*6+2*8] = [19 22]
        // [3 4]   [7 8]   [3*5+4*7  3*6+4*8]   [43 50]
        let a = mat(vec![1.0, 2.0, 3.0, 4.0], 2, 2);
        let b = mat(vec![5.0, 6.0, 7.0, 8.0], 2, 2);
        let result = eval_matrix_op("mat_mul", &[a, b]).unwrap().unwrap();
        match result {
            Value::MatFloat { data, rows, cols } => {
                assert_eq!(rows, 2);
                assert_eq!(cols, 2);
                assert_eq!(data, vec![19.0, 22.0, 43.0, 50.0]);
            }
            _ => panic!("Expected MatFloat"),
        }
    }

    #[test]
    fn test_mat_mul_2x3_times_3x2() {
        // [1 2 3] * [7  8 ]   = [1*7+2*9+3*11   1*8+2*10+3*12]  = [58  64]
        // [4 5 6]   [9  10]     [4*7+5*9+6*11   4*8+5*10+6*12]    [139 154]
        //           [11 12]
        let a = mat(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 2, 3);
        let b = mat(vec![7.0, 8.0, 9.0, 10.0, 11.0, 12.0], 3, 2);
        let result = eval_matrix_op("mat_mul", &[a, b]).unwrap().unwrap();
        match result {
            Value::MatFloat { data, rows, cols } => {
                assert_eq!(rows, 2);
                assert_eq!(cols, 2);
                assert_eq!(data, vec![58.0, 64.0, 139.0, 154.0]);
            }
            _ => panic!("Expected MatFloat"),
        }
    }

    #[test]
    fn test_mat_mul_dimension_mismatch() {
        let a = mat(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 2, 3);
        let b = mat(vec![1.0, 2.0, 3.0, 4.0], 2, 2);
        let err = eval_matrix_op("mat_mul", &[a, b]).unwrap_err();
        assert_eq!(err, "mat_mul: incompatible dimensions (2x3) * (2x2)");
    }

    #[test]
    fn test_mat_mul_wrong_arg_count() {
        let a = mat(vec![1.0], 1, 1);
        let err = eval_matrix_op("mat_mul", &[a]).unwrap_err();
        assert_eq!(err, "mat_mul requires 2 argument(s)");
    }

    #[test]
    fn test_mat_mul_non_matrix_arg() {
        let a = mat(vec![1.0], 1, 1);
        let b = Value::Float(5.0);
        let err = eval_matrix_op("mat_mul", &[a, b]).unwrap_err();
        assert_eq!(err, "mat_mul: expected MatFloat, got Float");
    }

    // -----------------------------------------------------------------------
    // transpose tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_transpose_2x3() {
        // [1 2 3]^T = [1 4]
        // [4 5 6]     [2 5]
        //             [3 6]
        let m = mat(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 2, 3);
        let result = eval_matrix_op("transpose", &[m]).unwrap().unwrap();
        match result {
            Value::MatFloat { data, rows, cols } => {
                assert_eq!(rows, 3);
                assert_eq!(cols, 2);
                assert_eq!(data, vec![1.0, 4.0, 2.0, 5.0, 3.0, 6.0]);
            }
            _ => panic!("Expected MatFloat"),
        }
    }

    #[test]
    fn test_transpose_involution() {
        let m = mat(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 2, 3);
        let t1 = eval_matrix_op("transpose", &[m.clone()]).unwrap().unwrap();
        let t2 = eval_matrix_op("transpose", &[t1]).unwrap().unwrap();
        match (m, t2) {
            (
                Value::MatFloat { data: d1, rows: r1, cols: c1 },
                Value::MatFloat { data: d2, rows: r2, cols: c2 },
            ) => {
                assert_eq!(r1, r2);
                assert_eq!(c1, c2);
                assert_eq!(d1, d2);
            }
            _ => panic!("Expected MatFloat"),
        }
    }

    #[test]
    fn test_transpose_1x1() {
        let m = mat(vec![42.0], 1, 1);
        let result = eval_matrix_op("transpose", &[m]).unwrap().unwrap();
        match result {
            Value::MatFloat { data, rows, cols } => {
                assert_eq!(rows, 1);
                assert_eq!(cols, 1);
                assert_eq!(data, vec![42.0]);
            }
            _ => panic!("Expected MatFloat"),
        }
    }

    #[test]
    fn test_transpose_wrong_arg_count() {
        let err = eval_matrix_op("transpose", &[]).unwrap_err();
        assert_eq!(err, "transpose requires 1 argument(s)");
    }

    #[test]
    fn test_transpose_non_matrix() {
        let err = eval_matrix_op("transpose", &[Value::Int(5)]).unwrap_err();
        assert_eq!(err, "transpose: expected MatFloat, got Int");
    }

    // -----------------------------------------------------------------------
    // det tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_det_2x2() {
        // det([1 2; 3 4]) = 1*4 - 2*3 = -2
        let m = mat(vec![1.0, 2.0, 3.0, 4.0], 2, 2);
        let result = eval_matrix_op("det", &[m]).unwrap().unwrap();
        match result {
            Value::Float(d) => assert!((d - (-2.0)).abs() < 1e-10),
            _ => panic!("Expected Float"),
        }
    }

    #[test]
    fn test_det_3x3() {
        // det([1 2 3; 0 1 4; 5 6 0]) = 1(0-24) - 2(0-20) + 3(0-5) = -24 + 40 - 15 = 1
        let m = mat(vec![1.0, 2.0, 3.0, 0.0, 1.0, 4.0, 5.0, 6.0, 0.0], 3, 3);
        let result = eval_matrix_op("det", &[m]).unwrap().unwrap();
        match result {
            Value::Float(d) => assert!((d - 1.0).abs() < 1e-10),
            _ => panic!("Expected Float"),
        }
    }

    #[test]
    fn test_det_identity() {
        let m = mat(vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0], 3, 3);
        let result = eval_matrix_op("det", &[m]).unwrap().unwrap();
        match result {
            Value::Float(d) => assert!((d - 1.0).abs() < 1e-10),
            _ => panic!("Expected Float"),
        }
    }

    #[test]
    fn test_det_singular() {
        // Row 2 is 2 * row 1
        let m = mat(vec![1.0, 2.0, 2.0, 4.0], 2, 2);
        let result = eval_matrix_op("det", &[m]).unwrap().unwrap();
        match result {
            Value::Float(d) => assert!(d.abs() < 1e-10),
            _ => panic!("Expected Float"),
        }
    }

    #[test]
    fn test_det_non_square() {
        let m = mat(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 2, 3);
        let err = eval_matrix_op("det", &[m]).unwrap_err();
        assert_eq!(err, "det: requires a square matrix, got 2x3");
    }

    #[test]
    fn test_det_wrong_arg_count() {
        let err = eval_matrix_op("det", &[]).unwrap_err();
        assert_eq!(err, "det requires 1 argument(s)");
    }

    #[test]
    fn test_det_non_matrix() {
        let err = eval_matrix_op("det", &[Value::Bool(true)]).unwrap_err();
        assert_eq!(err, "det: expected MatFloat, got Bool");
    }

    // -----------------------------------------------------------------------
    // inverse tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_inverse_2x2() {
        // A = [1 2; 3 4], A^-1 = [-2 1; 1.5 -0.5]
        let a = mat(vec![1.0, 2.0, 3.0, 4.0], 2, 2);
        let result = eval_matrix_op("inverse", &[a]).unwrap().unwrap();
        match result {
            Value::MatFloat { data, rows, cols } => {
                assert_eq!(rows, 2);
                assert_eq!(cols, 2);
                assert!((data[0] - (-2.0)).abs() < 1e-10);
                assert!((data[1] - 1.0).abs() < 1e-10);
                assert!((data[2] - 1.5).abs() < 1e-10);
                assert!((data[3] - (-0.5)).abs() < 1e-10);
            }
            _ => panic!("Expected MatFloat"),
        }
    }

    #[test]
    fn test_inverse_identity() {
        let m = mat(vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0], 3, 3);
        let result = eval_matrix_op("inverse", &[m]).unwrap().unwrap();
        match result {
            Value::MatFloat { data, rows, cols } => {
                assert_eq!(rows, 3);
                assert_eq!(cols, 3);
                // Should be identity
                for i in 0..3 {
                    for j in 0..3 {
                        let expected = if i == j { 1.0 } else { 0.0 };
                        assert!(
                            (data[i * 3 + j] - expected).abs() < 1e-10,
                            "inv[{}][{}] = {}, expected {}",
                            i, j, data[i * 3 + j], expected
                        );
                    }
                }
            }
            _ => panic!("Expected MatFloat"),
        }
    }

    #[test]
    fn test_inverse_roundtrip() {
        // A * A^-1 should be approximately I
        let a_data = vec![1.0, 2.0, 3.0, 0.0, 1.0, 4.0, 5.0, 6.0, 0.0];
        let a = mat(a_data.clone(), 3, 3);
        let inv = eval_matrix_op("inverse", &[a]).unwrap().unwrap();
        match inv {
            Value::MatFloat { data: inv_data, .. } => {
                let product = matrix_multiply(&a_data, 3, 3, &inv_data, 3);
                for i in 0..3 {
                    for j in 0..3 {
                        let expected = if i == j { 1.0 } else { 0.0 };
                        assert!(
                            (product[i * 3 + j] - expected).abs() < 1e-10,
                            "product[{}][{}] = {}, expected {}",
                            i, j, product[i * 3 + j], expected
                        );
                    }
                }
            }
            _ => panic!("Expected MatFloat"),
        }
    }

    #[test]
    fn test_inverse_singular() {
        // Singular matrix (row 2 = 2 * row 1)
        let m = mat(vec![1.0, 2.0, 2.0, 4.0], 2, 2);
        let err = eval_matrix_op("inverse", &[m]).unwrap_err();
        assert_eq!(err, "inverse: matrix is singular (determinant ≈ 0)");
    }

    #[test]
    fn test_inverse_non_square() {
        let m = mat(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 2, 3);
        let err = eval_matrix_op("inverse", &[m]).unwrap_err();
        assert_eq!(err, "inverse: requires a square matrix, got 2x3");
    }

    #[test]
    fn test_inverse_wrong_arg_count() {
        let m = mat(vec![1.0], 1, 1);
        let err = eval_matrix_op("inverse", &[m.clone(), m]).unwrap_err();
        assert_eq!(err, "inverse requires 1 argument(s)");
    }

    #[test]
    fn test_inverse_non_matrix() {
        let err = eval_matrix_op("inverse", &[Value::VecFloat(vec![1.0, 2.0])]).unwrap_err();
        assert_eq!(err, "inverse: expected MatFloat, got VecFloat");
    }

    // -----------------------------------------------------------------------
    // Unrecognized name
    // -----------------------------------------------------------------------

    #[test]
    fn test_unrecognized_returns_none() {
        let result = eval_matrix_op("unknown_func", &[]).unwrap();
        assert!(result.is_none());
    }

    // -----------------------------------------------------------------------
    // 1x1 matrix edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn test_det_1x1() {
        let m = mat(vec![7.5], 1, 1);
        let result = eval_matrix_op("det", &[m]).unwrap().unwrap();
        match result {
            Value::Float(d) => assert!((d - 7.5).abs() < 1e-10),
            _ => panic!("Expected Float"),
        }
    }

    #[test]
    fn test_inverse_1x1() {
        let m = mat(vec![4.0], 1, 1);
        let result = eval_matrix_op("inverse", &[m]).unwrap().unwrap();
        match result {
            Value::MatFloat { data, rows, cols } => {
                assert_eq!(rows, 1);
                assert_eq!(cols, 1);
                assert!((data[0] - 0.25).abs() < 1e-10);
            }
            _ => panic!("Expected MatFloat"),
        }
    }

    #[test]
    fn test_mat_mul_identity() {
        // A * I = A
        let a = mat(vec![1.0, 2.0, 3.0, 4.0], 2, 2);
        let i = mat(vec![1.0, 0.0, 0.0, 1.0], 2, 2);
        let result = eval_matrix_op("mat_mul", &[a, i]).unwrap().unwrap();
        match result {
            Value::MatFloat { data, rows, cols } => {
                assert_eq!(rows, 2);
                assert_eq!(cols, 2);
                assert_eq!(data, vec![1.0, 2.0, 3.0, 4.0]);
            }
            _ => panic!("Expected MatFloat"),
        }
    }

    // -----------------------------------------------------------------------
    // portfolio_var tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_portfolio_var_equal_weights_identity_cov() {
        // 2 assets, equal weights [0.5, 0.5], identity covariance matrix
        // portfolio_var = w^T * I * w = 0.5^2 + 0.5^2 = 0.5
        let weights = Value::VecFloat(vec![0.5, 0.5]);
        let cov = mat(vec![1.0, 0.0, 0.0, 1.0], 2, 2);
        let result = eval_portfolio_op(
            "portfolio_var",
            &[weights, cov],
            &mut HashMap::new(),
            "test_site",
        )
        .unwrap()
        .unwrap();
        match result {
            Value::Float(v) => assert!((v - 0.5).abs() < 1e-10),
            _ => panic!("Expected Float"),
        }
    }

    #[test]
    fn test_portfolio_var_with_covariance() {
        // weights = [0.6, 0.4], cov = [[0.04, 0.01], [0.01, 0.09]]
        // var = 0.6^2*0.04 + 2*0.6*0.4*0.01 + 0.4^2*0.09
        //     = 0.0144 + 0.0048 + 0.0144 = 0.0336
        let weights = Value::VecFloat(vec![0.6, 0.4]);
        let cov = mat(vec![0.04, 0.01, 0.01, 0.09], 2, 2);
        let result = eval_portfolio_op(
            "portfolio_var",
            &[weights, cov],
            &mut HashMap::new(),
            "test_site",
        )
        .unwrap()
        .unwrap();
        match result {
            Value::Float(v) => assert!((v - 0.0336).abs() < 1e-10),
            _ => panic!("Expected Float"),
        }
    }

    #[test]
    fn test_portfolio_var_dimension_mismatch() {
        // 3 weights but 2x2 cov matrix
        let weights = Value::VecFloat(vec![0.3, 0.3, 0.4]);
        let cov = mat(vec![1.0, 0.0, 0.0, 1.0], 2, 2);
        let err = eval_portfolio_op(
            "portfolio_var",
            &[weights, cov],
            &mut HashMap::new(),
            "test_site",
        )
        .unwrap_err();
        assert!(err.contains("doesn't match matrix dimension"));
    }

    // -----------------------------------------------------------------------
    // sharpe tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_sharpe_known_returns() {
        // returns = [0.05, 0.10, 0.15], rf_rate = 0.02
        // mean = 0.10, stddev = sqrt(((0.05-0.10)^2 + (0.10-0.10)^2 + (0.15-0.10)^2) / 3)
        //       = sqrt((0.0025 + 0 + 0.0025) / 3) = sqrt(0.005/3) ≈ 0.04082
        // sharpe = (0.10 - 0.02) / 0.04082 ≈ 1.9596
        let returns = Value::VecFloat(vec![0.05, 0.10, 0.15]);
        let rf_rate = Value::Float(0.02);
        let result = eval_portfolio_op(
            "sharpe",
            &[returns, rf_rate],
            &mut HashMap::new(),
            "test_site",
        )
        .unwrap()
        .unwrap();
        match result {
            Value::Float(s) => {
                let expected = (0.10 - 0.02) / (0.005_f64 / 3.0).sqrt();
                assert!((s - expected).abs() < 1e-10);
            }
            _ => panic!("Expected Float"),
        }
    }

    #[test]
    fn test_sharpe_zero_stddev_returns_zero() {
        // All returns are exactly zero → stddev = 0 → sharpe = 0.0
        let returns = Value::VecFloat(vec![0.0, 0.0, 0.0]);
        let rf_rate = Value::Float(0.02);
        let result = eval_portfolio_op(
            "sharpe",
            &[returns, rf_rate],
            &mut HashMap::new(),
            "test_site",
        )
        .unwrap()
        .unwrap();
        match result {
            Value::Float(s) => assert_eq!(s, 0.0),
            _ => panic!("Expected Float"),
        }
    }

    #[test]
    fn test_sharpe_empty_returns() {
        // Empty returns → sharpe = 0.0
        let returns = Value::VecFloat(vec![]);
        let rf_rate = Value::Float(0.01);
        let result = eval_portfolio_op(
            "sharpe",
            &[returns, rf_rate],
            &mut HashMap::new(),
            "test_site",
        )
        .unwrap()
        .unwrap();
        match result {
            Value::Float(s) => assert_eq!(s, 0.0),
            _ => panic!("Expected Float"),
        }
    }

    // -----------------------------------------------------------------------
    // cov_matrix tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_cov_matrix_symmetry_and_nonneg_diagonal() {
        // Feed 5 bars of 3-asset returns, verify result is symmetric
        // with non-negative diagonal entries.
        let mut indicators = HashMap::new();
        let key = "cov_test_site";
        let period = Value::Int(5);

        let return_data = vec![
            vec![0.01, -0.02, 0.03],
            vec![0.02, 0.01, -0.01],
            vec![-0.01, 0.03, 0.02],
            vec![0.03, -0.01, 0.01],
            vec![0.00, 0.02, -0.02],
        ];

        let mut result = None;
        for returns in &return_data {
            let args = vec![Value::VecFloat(returns.clone()), period.clone()];
            result = Some(
                eval_portfolio_op("cov_matrix", &args, &mut indicators, key)
                    .unwrap()
                    .unwrap(),
            );
        }

        match result.unwrap() {
            Value::MatFloat { data, rows, cols } => {
                assert_eq!(rows, 3);
                assert_eq!(cols, 3);
                // Symmetry: cov[i][j] == cov[j][i]
                for i in 0..3 {
                    for j in 0..3 {
                        assert!(
                            (data[i * 3 + j] - data[j * 3 + i]).abs() < 1e-12,
                            "cov[{}][{}] != cov[{}][{}]",
                            i, j, j, i
                        );
                    }
                }
                // Non-negative diagonal (variances)
                for i in 0..3 {
                    assert!(
                        data[i * 3 + i] >= 0.0,
                        "cov[{}][{}] = {} is negative",
                        i, i, data[i * 3 + i]
                    );
                }
            }
            _ => panic!("Expected MatFloat"),
        }
    }

    // -----------------------------------------------------------------------
    // corr_matrix tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_corr_matrix_diagonal_one_and_bounds() {
        // Feed 5 bars of 3-asset returns, verify diagonal=1.0 and off-diagonal in [-1,1].
        let mut indicators = HashMap::new();
        let key = "corr_test_site";
        let period = Value::Int(5);

        let return_data = vec![
            vec![0.01, -0.02, 0.03],
            vec![0.02, 0.01, -0.01],
            vec![-0.01, 0.03, 0.02],
            vec![0.03, -0.01, 0.01],
            vec![0.00, 0.02, -0.02],
        ];

        let mut result = None;
        for returns in &return_data {
            let args = vec![Value::VecFloat(returns.clone()), period.clone()];
            result = Some(
                eval_portfolio_op("corr_matrix", &args, &mut indicators, key)
                    .unwrap()
                    .unwrap(),
            );
        }

        match result.unwrap() {
            Value::MatFloat { data, rows, cols } => {
                assert_eq!(rows, 3);
                assert_eq!(cols, 3);
                // Diagonal must be 1.0
                for i in 0..3 {
                    assert!(
                        (data[i * 3 + i] - 1.0).abs() < 1e-12,
                        "corr[{}][{}] = {}, expected 1.0",
                        i, i, data[i * 3 + i]
                    );
                }
                // Off-diagonal must be in [-1, 1]
                for i in 0..3 {
                    for j in 0..3 {
                        if i != j {
                            assert!(
                                data[i * 3 + j] >= -1.0 && data[i * 3 + j] <= 1.0,
                                "corr[{}][{}] = {} is out of [-1, 1]",
                                i, j, data[i * 3 + j]
                            );
                        }
                    }
                }
            }
            _ => panic!("Expected MatFloat"),
        }
    }

    // -----------------------------------------------------------------------
    // min_variance_weights tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_min_variance_weights_identity_gives_equal() {
        // With identity covariance, min-variance weights should be equal (1/n each)
        let cov = mat(vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0], 3, 3);
        let constraints = Value::VecFloat(vec![0.0, 1.0]); // no binding constraints
        let result = eval_portfolio_op(
            "min_variance_weights",
            &[cov, constraints],
            &mut HashMap::new(),
            "test_site",
        )
        .unwrap()
        .unwrap();
        match result {
            Value::VecFloat(w) => {
                assert_eq!(w.len(), 3);
                let expected = 1.0 / 3.0;
                for (i, wi) in w.iter().enumerate() {
                    assert!(
                        (wi - expected).abs() < 1e-10,
                        "w[{}] = {}, expected {}",
                        i, wi, expected
                    );
                }
                // Weights should sum to 1.0
                let sum: f64 = w.iter().sum();
                assert!((sum - 1.0).abs() < 1e-10);
            }
            _ => panic!("Expected VecFloat"),
        }
    }

    #[test]
    fn test_min_variance_weights_singular_matrix_error() {
        // Singular matrix (row 2 = row 1) → not invertible
        let cov = mat(vec![1.0, 2.0, 3.0, 1.0, 2.0, 3.0, 1.0, 2.0, 3.0], 3, 3);
        let constraints = Value::VecFloat(vec![0.0, 1.0]);
        let err = eval_portfolio_op(
            "min_variance_weights",
            &[cov, constraints],
            &mut HashMap::new(),
            "test_site",
        )
        .unwrap_err();
        assert!(err.contains("not invertible"));
    }
}
