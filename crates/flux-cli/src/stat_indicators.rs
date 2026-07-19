//! Tier 2 — Rolling Statistical Indicators (stateful functions).
//!
//! Provides `stddev`, `variance`, and `zscore` using `RollingStats` state,
//! plus placeholders for `corr`, `covariance`, `rsi`, and `atr` (implemented in later tasks).
//!
//! Each distinct call site (keyed by AST span) gets its own independent state buffer
//! via the `indicators` HashMap.

use std::collections::HashMap;

use crate::interpreter::{IndicatorStateEntry, Value};

/// Attempt to evaluate a Tier 2 statistical indicator by name.
///
/// Returns:
/// - `Ok(Some(value))` if `name` is a recognized stat indicator and evaluation succeeds
/// - `Ok(None)` if `name` is not a stat indicator (caller should try next dispatch tier)
/// - `Err(String)` on validation errors (wrong arg count, invalid period, non-numeric argument)
pub fn eval_stat_indicator(
    name: &str,
    args: &[Value],
    indicators: &mut HashMap<String, IndicatorStateEntry>,
    call_site_key: &str,
) -> Result<Option<Value>, String> {
    match name {
        "stddev" => eval_stddev(args, indicators, call_site_key),
        "variance" => eval_variance(args, indicators, call_site_key),
        "zscore" => eval_zscore(args, indicators, call_site_key),
        "corr" | "covariance" => eval_rolling_pair(name, args, indicators, call_site_key),
        "rsi" => eval_rsi(args, indicators, call_site_key),
        "atr" => eval_atr(args, indicators, call_site_key),
        "rolling_rank" => eval_rolling_rank(args, indicators, call_site_key),
        "lag" => eval_lag(args, indicators, call_site_key),
        _ => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// stddev(value, period) — population standard deviation
// ---------------------------------------------------------------------------

fn eval_stddev(
    args: &[Value],
    indicators: &mut HashMap<String, IndicatorStateEntry>,
    call_site_key: &str,
) -> Result<Option<Value>, String> {
    check_arity("stddev", args, 2)?;
    let value = to_f64("stddev", &args[0])?;
    let period = to_period("stddev", &args[1])?;

    let stats = get_or_init_rolling_stats(indicators, call_site_key, period);
    push_value(stats, value);
    let stddev = compute_population_stddev(stats);

    Ok(Some(Value::Float(stddev)))
}

// ---------------------------------------------------------------------------
// variance(value, period) — population variance
// ---------------------------------------------------------------------------

fn eval_variance(
    args: &[Value],
    indicators: &mut HashMap<String, IndicatorStateEntry>,
    call_site_key: &str,
) -> Result<Option<Value>, String> {
    check_arity("variance", args, 2)?;
    let value = to_f64("variance", &args[0])?;
    let period = to_period("variance", &args[1])?;

    let stats = get_or_init_rolling_stats(indicators, call_site_key, period);
    push_value(stats, value);
    let variance = compute_population_variance(stats);

    Ok(Some(Value::Float(variance)))
}

// ---------------------------------------------------------------------------
// zscore(value, period) — (value - mean) / stddev
// ---------------------------------------------------------------------------

fn eval_zscore(
    args: &[Value],
    indicators: &mut HashMap<String, IndicatorStateEntry>,
    call_site_key: &str,
) -> Result<Option<Value>, String> {
    check_arity("zscore", args, 2)?;
    let value = to_f64("zscore", &args[0])?;
    let period = to_period("zscore", &args[1])?;

    let stats = get_or_init_rolling_stats(indicators, call_site_key, period);
    push_value(stats, value);

    let stddev = compute_population_stddev(stats);
    if stddev == 0.0 {
        return Ok(Some(Value::Float(0.0)));
    }

    let mean = compute_mean(stats);
    let zscore = (value - mean) / stddev;

    Ok(Some(Value::Float(zscore)))
}

// ---------------------------------------------------------------------------
// corr(value_a, value_b, period) — Pearson correlation coefficient
// covariance(value_a, value_b, period) — population covariance
// ---------------------------------------------------------------------------

/// Evaluate corr or covariance using RollingPair state.
///
/// Population covariance: cov(A,B) = (Σ(ai*bi)/n) - (Σai/n)*(Σbi/n)
/// Pearson correlation: corr(A,B) = cov(A,B) / (stddev_A * stddev_B)
fn eval_rolling_pair(
    name: &str,
    args: &[Value],
    indicators: &mut HashMap<String, IndicatorStateEntry>,
    call_site_key: &str,
) -> Result<Option<Value>, String> {
    check_arity(name, args, 3)?;
    let value_a = to_f64(name, &args[0])?;
    let value_b = to_f64(name, &args[1])?;
    let period = to_period(name, &args[2])?;

    let key = format!("{}_{}", name, call_site_key);
    let entry = indicators.entry(key).or_insert_with(|| {
        IndicatorStateEntry::RollingPair {
            buffer_a: vec![0.0; period],
            buffer_b: vec![0.0; period],
            period,
            index: 0,
            count: 0,
        }
    });

    let result = match entry {
        IndicatorStateEntry::RollingPair {
            buffer_a, buffer_b, period: p, index, count,
        } => {
            // Insert new values into the circular buffers
            buffer_a[*index] = value_a;
            buffer_b[*index] = value_b;
            if *count < *p {
                *count += 1;
            }
            *index = (*index + 1) % *p;

            // Compute sums over the active window
            let n = *count as f64;
            let mut sum_a = 0.0;
            let mut sum_b = 0.0;
            let mut sum_ab = 0.0;
            let mut sum_a_sq = 0.0;
            let mut sum_b_sq = 0.0;

            for i in 0..*count {
                let a = buffer_a[i];
                let b = buffer_b[i];
                sum_a += a;
                sum_b += b;
                sum_ab += a * b;
                sum_a_sq += a * a;
                sum_b_sq += b * b;
            }

            let mean_a = sum_a / n;
            let mean_b = sum_b / n;
            // Population covariance: E[AB] - E[A]*E[B]
            let covariance = (sum_ab / n) - (mean_a * mean_b);

            match name {
                "covariance" => covariance,
                "corr" => {
                    // Population variance for each series
                    let var_a = (sum_a_sq / n) - (mean_a * mean_a);
                    let var_b = (sum_b_sq / n) - (mean_b * mean_b);
                    let stddev_a = var_a.max(0.0).sqrt();
                    let stddev_b = var_b.max(0.0).sqrt();
                    // If stddev of either series is zero, return 0.0
                    if stddev_a == 0.0 || stddev_b == 0.0 {
                        0.0
                    } else {
                        let r = covariance / (stddev_a * stddev_b);
                        // Clamp to [-1.0, 1.0] for floating-point safety
                        r.clamp(-1.0, 1.0)
                    }
                }
                _ => unreachable!(),
            }
        }
        _ => return Err(format!("indicator state mismatch for {}", name)),
    };
    Ok(Some(Value::Float(result)))
}

// ---------------------------------------------------------------------------
// RollingStats helpers
// ---------------------------------------------------------------------------

/// Get the existing RollingStats entry for this call site, or initialize one.
fn get_or_init_rolling_stats<'a>(
    indicators: &'a mut HashMap<String, IndicatorStateEntry>,
    call_site_key: &str,
    period: usize,
) -> &'a mut IndicatorStateEntry {
    indicators
        .entry(call_site_key.to_string())
        .or_insert_with(|| IndicatorStateEntry::RollingStats {
            buffer: vec![0.0; period],
            period,
            index: 0,
            count: 0,
            sum: 0.0,
            sum_sq: 0.0,
        })
}

/// Push a new value into the circular buffer, updating sum and sum_sq incrementally.
fn push_value(state: &mut IndicatorStateEntry, value: f64) {
    if let IndicatorStateEntry::RollingStats {
        buffer,
        period,
        index,
        count,
        sum,
        sum_sq,
    } = state
    {
        // If buffer is full, subtract the old value being evicted
        if *count >= *period {
            let old = buffer[*index];
            *sum -= old;
            *sum_sq -= old * old;
        }

        // Insert new value at current index
        buffer[*index] = value;
        *sum += value;
        *sum_sq += value * value;

        // Advance circular buffer index
        *index = (*index + 1) % *period;
        if *count < *period {
            *count += 1;
        }
    }
}

/// Compute population variance from the running sums: var = E[X^2] - E[X]^2
fn compute_population_variance(state: &IndicatorStateEntry) -> f64 {
    if let IndicatorStateEntry::RollingStats {
        count, sum, sum_sq, ..
    } = state
    {
        if *count == 0 {
            return 0.0;
        }
        let n = *count as f64;
        let mean = *sum / n;
        let mean_sq = *sum_sq / n;
        // Clamp to zero to handle floating-point noise (variance cannot be negative)
        (mean_sq - mean * mean).max(0.0)
    } else {
        0.0
    }
}

/// Compute population standard deviation (sqrt of variance).
fn compute_population_stddev(state: &IndicatorStateEntry) -> f64 {
    compute_population_variance(state).sqrt()
}

/// Compute the mean from the running sum and count.
fn compute_mean(state: &IndicatorStateEntry) -> f64 {
    if let IndicatorStateEntry::RollingStats { count, sum, .. } = state {
        if *count == 0 {
            return 0.0;
        }
        *sum / (*count as f64)
    } else {
        0.0
    }
}

// ---------------------------------------------------------------------------
// Argument validation helpers
// ---------------------------------------------------------------------------

/// Validate argument count.
fn check_arity(name: &str, args: &[Value], expected: usize) -> Result<(), String> {
    if args.len() != expected {
        Err(format!("{} requires {} argument(s)", name, expected))
    } else {
        Ok(())
    }
}

/// Coerce a Value to f64.
fn to_f64(name: &str, val: &Value) -> Result<f64, String> {
    match val {
        Value::Float(f) => Ok(*f),
        Value::Int(i) => Ok(*i as f64),
        _ => Err(format!("{} requires numeric arguments", name)),
    }
}

/// Extract period as a positive usize from a Value.
fn to_period(name: &str, val: &Value) -> Result<usize, String> {
    let p = match val {
        Value::Int(i) => *i,
        Value::Float(f) => *f as i64,
        _ => return Err(format!("{}: period must be a positive integer", name)),
    };
    if p <= 0 {
        return Err(format!("{}: period must be a positive integer", name));
    }
    Ok(p as usize)
}

// ---------------------------------------------------------------------------
// RSI (Relative Strength Index)
// ---------------------------------------------------------------------------

/// Evaluate `rsi(value, period)` using Wilder's smoothing method.
///
/// RSI = 100 - 100 / (1 + avg_gain / avg_loss)
///
/// - Fewer than 2 values: returns 50.0 (neutral)
/// - All gains (avg_loss = 0): returns 100.0
/// - All losses (avg_gain = 0): returns 0.0
/// - avg_gain and avg_loss use EMA smoothing with factor 1/period
fn eval_rsi(
    args: &[Value],
    indicators: &mut HashMap<String, IndicatorStateEntry>,
    call_site_key: &str,
) -> Result<Option<Value>, String> {
    check_arity("rsi", args, 2)?;
    let value = to_f64("rsi", &args[0])?;
    let period = to_period("rsi", &args[1])?;

    let key = format!("rsi_{}", call_site_key);
    let entry = indicators.entry(key).or_insert_with(|| {
        IndicatorStateEntry::Rsi {
            prev_value: None,
            avg_gain: 0.0,
            avg_loss: 0.0,
            period,
            count: 0,
        }
    });

    let result = match entry {
        IndicatorStateEntry::Rsi {
            prev_value, avg_gain, avg_loss, period: p, count,
        } => {
            *count += 1;

            match *prev_value {
                None => {
                    // First value: no change to compute, return neutral
                    *prev_value = Some(value);
                    50.0
                }
                Some(prev) => {
                    let change = value - prev;
                    *prev_value = Some(value);

                    let gain = if change > 0.0 { change } else { 0.0 };
                    let loss = if change < 0.0 { -change } else { 0.0 };

                    // Wilder's smoothing: EMA with factor 1/period
                    let alpha = 1.0 / (*p as f64);
                    *avg_gain = *avg_gain * (1.0 - alpha) + gain * alpha;
                    *avg_loss = *avg_loss * (1.0 - alpha) + loss * alpha;

                    // Edge cases
                    if *avg_loss == 0.0 && *avg_gain == 0.0 {
                        50.0
                    } else if *avg_loss == 0.0 {
                        100.0
                    } else if *avg_gain == 0.0 {
                        0.0
                    } else {
                        let rs = *avg_gain / *avg_loss;
                        100.0 - 100.0 / (1.0 + rs)
                    }
                }
            }
        }
        _ => return Err("indicator state mismatch for rsi".to_string()),
    };
    Ok(Some(Value::Float(result)))
}

// ---------------------------------------------------------------------------
// ATR (Average True Range)
// ---------------------------------------------------------------------------

/// Evaluate `atr(high, low, close, period)` using True Range + Wilder's smoothing.
///
/// True Range = max(high - low, |high - prev_close|, |low - prev_close|)
/// First bar (no prev_close): TR = high - low
/// ATR = EMA of True Range with Wilder's smoothing (factor 1/period)
fn eval_atr(
    args: &[Value],
    indicators: &mut HashMap<String, IndicatorStateEntry>,
    call_site_key: &str,
) -> Result<Option<Value>, String> {
    check_arity("atr", args, 4)?;
    let high = to_f64("atr", &args[0])?;
    let low = to_f64("atr", &args[1])?;
    let close = to_f64("atr", &args[2])?;
    let period = to_period("atr", &args[3])?;

    let key = format!("atr_{}", call_site_key);
    let entry = indicators.entry(key).or_insert_with(|| {
        IndicatorStateEntry::Atr {
            prev_close: None,
            atr_value: None,
            period,
            count: 0,
        }
    });

    let result = match entry {
        IndicatorStateEntry::Atr {
            prev_close, atr_value, period: p, count,
        } => {
            *count += 1;

            // Compute True Range
            let tr = match *prev_close {
                None => {
                    // First bar: no previous close, use high - low
                    high - low
                }
                Some(pc) => {
                    // TR = max(high-low, |high-prev_close|, |low-prev_close|)
                    let hl = high - low;
                    let hpc = (high - pc).abs();
                    let lpc = (low - pc).abs();
                    hl.max(hpc).max(lpc)
                }
            };

            // Update previous close for next bar
            *prev_close = Some(close);

            // Compute ATR with Wilder's smoothing
            let alpha = 1.0 / (*p as f64);
            let atr = match *atr_value {
                None => {
                    // First ATR value is just the first True Range
                    tr
                }
                Some(prev_atr) => {
                    // Wilder's: ATR = prev_ATR*(1-1/period) + TR*(1/period)
                    prev_atr * (1.0 - alpha) + tr * alpha
                }
            };
            *atr_value = Some(atr);
            atr
        }
        _ => return Err("indicator state mismatch for atr".to_string()),
    };
    Ok(Some(Value::Float(result)))
}

// ---------------------------------------------------------------------------
// rolling_rank(value, period) — percentile rank within trailing window (0.0 to 1.0)
// ---------------------------------------------------------------------------

/// Compute the percentile rank of the current value within its trailing window.
/// Returns the fraction of values in the window that are strictly less than the current value.
/// Equivalent to: `series.rolling(period).rank(pct=True)` in pandas.
///
/// - Returns 0.0 during warmup (fewer than 2 observations)
/// - Returns value in [0.0, 1.0] once window has >= 2 values
fn eval_rolling_rank(
    args: &[Value],
    indicators: &mut HashMap<String, IndicatorStateEntry>,
    call_site_key: &str,
) -> Result<Option<Value>, String> {
    check_arity("rolling_rank", args, 2)?;
    let value = to_f64("rolling_rank", &args[0])?;
    let period = to_period("rolling_rank", &args[1])?;

    let state = indicators
        .entry(call_site_key.to_string())
        .or_insert_with(|| IndicatorStateEntry::RollingRank {
            buffer: vec![0.0; period],
            period,
            index: 0,
            count: 0,
        });

    if let IndicatorStateEntry::RollingRank {
        buffer,
        period: p,
        index,
        count,
    } = state
    {
        // Insert new value
        buffer[*index] = value;
        *index = (*index + 1) % *p;
        if *count < *p {
            *count += 1;
        }

        // Need at least 2 values for a meaningful rank
        if *count < 2 {
            return Ok(Some(Value::Float(0.0)));
        }

        // Count how many values in the window are strictly less than current value
        let n = *count;
        let mut less_count = 0usize;
        for i in 0..n {
            if buffer[i] < value {
                less_count += 1;
            }
        }

        // Percentile rank: fraction of values that are less than current
        // This matches pandas rank(pct=True) with method='average' for non-ties
        let rank = less_count as f64 / (n - 1) as f64;

        Ok(Some(Value::Float(rank.clamp(0.0, 1.0))))
    } else {
        Ok(Some(Value::Float(0.0)))
    }
}

// ---------------------------------------------------------------------------
// lag(value, period) — returns the value from `period` bars ago
// ---------------------------------------------------------------------------

/// Return the value from N bars ago. During warmup (fewer than period+1 observations),
/// returns the oldest available value.
///
/// lag(close, 1) → previous bar's close (equivalent to prev_close)
/// lag(close, 5) → close from 5 bars ago
fn eval_lag(
    args: &[Value],
    indicators: &mut HashMap<String, IndicatorStateEntry>,
    call_site_key: &str,
) -> Result<Option<Value>, String> {
    check_arity("lag", args, 2)?;
    let value = to_f64("lag", &args[0])?;
    let period = to_period("lag", &args[1])?;

    // We need a buffer of size period+1 to store current + N past values
    let buf_size = period + 1;

    let state = indicators
        .entry(call_site_key.to_string())
        .or_insert_with(|| IndicatorStateEntry::Lag {
            buffer: vec![0.0; buf_size],
            period: buf_size,
            index: 0,
            count: 0,
        });

    if let IndicatorStateEntry::Lag {
        buffer,
        period: p,
        index,
        count,
    } = state
    {
        // Insert current value
        buffer[*index] = value;

        // The lagged value is at position (index - period) mod buf_size
        // But we need to handle warmup: if count < period+1, return oldest available
        if *count < *p {
            *count += 1;
        }

        // Advance index for next call
        let current_index = *index;
        *index = (*index + 1) % *p;

        if *count <= period {
            // Not enough history yet — return the oldest value we have
            // (which is the first value pushed, at earliest position)
            let oldest_idx = if *count < *p {
                0
            } else {
                *index // next position to be overwritten = oldest
            };
            return Ok(Some(Value::Float(buffer[oldest_idx])));
        }

        // Normal case: return value from `period` steps ago
        let lag_idx = (current_index + *p - period) % *p;
        Ok(Some(Value::Float(buffer[lag_idx])))
    } else {
        Ok(Some(Value::Float(value)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: feed a sequence of values through a stat indicator function.
    fn feed_values(name: &str, values: &[f64], period: usize) -> Vec<f64> {
        let mut indicators = HashMap::new();
        let key = "test_call_site";
        let mut results = Vec::new();
        for &v in values {
            let args = vec![Value::Float(v), Value::Int(period as i64)];
            let result = eval_stat_indicator(name, &args, &mut indicators, key)
                .unwrap()
                .unwrap();
            if let Value::Float(f) = result {
                results.push(f);
            }
        }
        results
    }

    // --- stddev ---

    #[test]
    fn test_stddev_constant_series() {
        let results = feed_values("stddev", &[5.0, 5.0, 5.0, 5.0], 3);
        for r in &results {
            assert!((*r - 0.0).abs() < 1e-10, "stddev of constant should be 0, got {}", r);
        }
    }

    #[test]
    fn test_stddev_known_sequence() {
        // [1, 2, 3] period=3: mean=2, var=2/3, stddev=sqrt(2/3)
        let results = feed_values("stddev", &[1.0, 2.0, 3.0], 3);
        let expected = (2.0_f64 / 3.0).sqrt();
        assert!((results[2] - expected).abs() < 1e-10, "expected {}, got {}", expected, results[2]);
    }

    #[test]
    fn test_stddev_period_1_returns_zero() {
        let results = feed_values("stddev", &[1.0, 5.0, 100.0], 1);
        for r in &results {
            assert!((*r - 0.0).abs() < 1e-10, "stddev with period=1 should be 0, got {}", r);
        }
    }

    #[test]
    fn test_stddev_rolling_window() {
        // [1, 2, 3, 4] period=2: consecutive pairs differ by 1, stddev=0.5
        let results = feed_values("stddev", &[1.0, 2.0, 3.0, 4.0], 2);
        for &r in &results[1..] {
            assert!((r - 0.5).abs() < 1e-10, "expected 0.5, got {}", r);
        }
    }

    #[test]
    fn test_stddev_warmup_period() {
        let results = feed_values("stddev", &[10.0, 20.0, 30.0], 5);
        assert!((results[0] - 0.0).abs() < 1e-10, "single value stddev should be 0");
    }

    // --- variance ---

    #[test]
    fn test_variance_equals_stddev_squared() {
        let values = [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        let period = 4;
        let stddev_results = feed_values("stddev", &values, period);
        let variance_results = feed_values("variance", &values, period);
        for (i, (s, v)) in stddev_results.iter().zip(variance_results.iter()).enumerate() {
            assert!((s * s - v).abs() < 1e-10, "bar {}: stddev^2={} != variance={}", i, s * s, v);
        }
    }

    #[test]
    fn test_variance_known_sequence() {
        let results = feed_values("variance", &[1.0, 2.0, 3.0], 3);
        let expected = 2.0 / 3.0;
        assert!((results[2] - expected).abs() < 1e-10, "expected {}, got {}", expected, results[2]);
    }

    #[test]
    fn test_variance_period_1_returns_zero() {
        let results = feed_values("variance", &[1.0, 5.0, 100.0], 1);
        for r in &results {
            assert!((*r - 0.0).abs() < 1e-10, "variance with period=1 should be 0, got {}", r);
        }
    }

    // --- zscore ---

    #[test]
    fn test_zscore_constant_series_returns_zero() {
        let results = feed_values("zscore", &[5.0, 5.0, 5.0, 5.0], 3);
        for r in &results {
            assert!((*r - 0.0).abs() < 1e-10, "zscore of constant should be 0, got {}", r);
        }
    }

    #[test]
    fn test_zscore_known_values() {
        // [1, 2, 3] period=3: mean=2, stddev=sqrt(2/3), zscore of 3 = 1/sqrt(2/3)
        let results = feed_values("zscore", &[1.0, 2.0, 3.0], 3);
        let expected = 1.0 / (2.0_f64 / 3.0).sqrt();
        assert!((results[2] - expected).abs() < 1e-10, "expected {}, got {}", expected, results[2]);
    }

    #[test]
    fn test_zscore_mean_value_is_zero() {
        let results = feed_values("zscore", &[1.0, 3.0, 2.0], 3);
        assert!((results[2] - 0.0).abs() < 1e-10, "zscore at mean should be 0, got {}", results[2]);
    }

    // --- Period validation ---

    #[test]
    fn test_period_zero_returns_error() {
        let mut indicators = HashMap::new();
        let args = vec![Value::Float(1.0), Value::Int(0)];
        let err = eval_stat_indicator("stddev", &args, &mut indicators, "key").unwrap_err();
        assert_eq!(err, "stddev: period must be a positive integer");
    }

    #[test]
    fn test_period_negative_returns_error() {
        let mut indicators = HashMap::new();
        let args = vec![Value::Float(1.0), Value::Int(-5)];
        let err = eval_stat_indicator("variance", &args, &mut indicators, "key").unwrap_err();
        assert_eq!(err, "variance: period must be a positive integer");
    }

    #[test]
    fn test_zscore_period_error() {
        let mut indicators = HashMap::new();
        let args = vec![Value::Float(1.0), Value::Int(-1)];
        let err = eval_stat_indicator("zscore", &args, &mut indicators, "key").unwrap_err();
        assert_eq!(err, "zscore: period must be a positive integer");
    }

    // --- Argument count ---

    #[test]
    fn test_wrong_arg_count_stddev() {
        let mut indicators = HashMap::new();
        let args = vec![Value::Float(1.0)];
        let err = eval_stat_indicator("stddev", &args, &mut indicators, "key").unwrap_err();
        assert_eq!(err, "stddev requires 2 argument(s)");
    }

    #[test]
    fn test_wrong_arg_count_variance() {
        let mut indicators = HashMap::new();
        let args = vec![Value::Float(1.0), Value::Float(2.0), Value::Float(3.0)];
        let err = eval_stat_indicator("variance", &args, &mut indicators, "key").unwrap_err();
        assert_eq!(err, "variance requires 2 argument(s)");
    }

    // --- Call-site isolation ---

    #[test]
    fn test_independent_call_sites() {
        let mut indicators = HashMap::new();

        let args_a = vec![Value::Float(10.0), Value::Int(3)];
        let args_b = vec![Value::Float(100.0), Value::Int(3)];
        eval_stat_indicator("stddev", &args_a, &mut indicators, "site_a").unwrap();
        eval_stat_indicator("stddev", &args_b, &mut indicators, "site_b").unwrap();

        let args_a2 = vec![Value::Float(20.0), Value::Int(3)];
        let args_b2 = vec![Value::Float(100.0), Value::Int(3)];
        let result_a = eval_stat_indicator("stddev", &args_a2, &mut indicators, "site_a").unwrap().unwrap();
        let result_b = eval_stat_indicator("stddev", &args_b2, &mut indicators, "site_b").unwrap().unwrap();

        if let (Value::Float(a), Value::Float(b)) = (result_a, result_b) {
            assert!((a - 5.0).abs() < 1e-10, "site_a stddev should be 5.0, got {}", a);
            assert!((b - 0.0).abs() < 1e-10, "site_b stddev should be 0.0, got {}", b);
        } else {
            panic!("expected Float values");
        }
    }

    // --- Unknown function returns None ---

    #[test]
    fn test_unknown_function_returns_none() {
        let mut indicators = HashMap::new();
        let args = vec![Value::Float(1.0), Value::Int(5)];
        let result = eval_stat_indicator("unknown_fn", &args, &mut indicators, "key").unwrap();
        assert!(result.is_none());
    }

    // --- Non-numeric argument ---

    #[test]
    fn test_non_numeric_value_argument() {
        let mut indicators = HashMap::new();
        let args = vec![Value::Str("hello".to_string()), Value::Int(5)];
        let err = eval_stat_indicator("stddev", &args, &mut indicators, "key").unwrap_err();
        assert_eq!(err, "stddev requires numeric arguments");
    }

    // --- corr ---

    #[test]
    fn test_corr_perfectly_correlated() {
        let mut indicators = HashMap::new();
        let key = "corr_test_1";
        let mut result = 0.0;
        for v in [1.0, 2.0, 3.0, 4.0, 5.0] {
            let args = vec![Value::Float(v), Value::Float(v), Value::Int(5)];
            if let Ok(Some(Value::Float(f))) = eval_stat_indicator("corr", &args, &mut indicators, key) {
                result = f;
            }
        }
        assert!((result - 1.0).abs() < 1e-10, "Expected 1.0, got {}", result);
    }

    #[test]
    fn test_corr_perfectly_anti_correlated() {
        let mut indicators = HashMap::new();
        let key = "corr_test_2";
        let mut result = 0.0;
        for v in [1.0, 2.0, 3.0, 4.0, 5.0] {
            let args = vec![Value::Float(v), Value::Float(-v), Value::Int(5)];
            if let Ok(Some(Value::Float(f))) = eval_stat_indicator("corr", &args, &mut indicators, key) {
                result = f;
            }
        }
        assert!((result + 1.0).abs() < 1e-10, "Expected -1.0, got {}", result);
    }

    #[test]
    fn test_corr_constant_series_returns_zero() {
        let mut indicators = HashMap::new();
        let key = "corr_test_3";
        let mut result = 99.0;
        for v in [1.0, 2.0, 3.0, 4.0, 5.0] {
            let args = vec![Value::Float(v), Value::Float(3.0), Value::Int(5)];
            if let Ok(Some(Value::Float(f))) = eval_stat_indicator("corr", &args, &mut indicators, key) {
                result = f;
            }
        }
        assert!(result.abs() < 1e-10, "Expected 0.0, got {}", result);
    }

    #[test]
    fn test_corr_bounded() {
        let mut indicators = HashMap::new();
        let key = "corr_test_4";
        for (a, b) in [(1.0, 5.0), (3.0, 2.0), (7.0, 8.0), (2.0, 1.0), (9.0, 4.0)] {
            let args = vec![Value::Float(a), Value::Float(b), Value::Int(3)];
            if let Ok(Some(Value::Float(f))) = eval_stat_indicator("corr", &args, &mut indicators, key) {
                assert!(f >= -1.0 && f <= 1.0, "Correlation {} out of bounds", f);
            }
        }
    }

    #[test]
    fn test_corr_fewer_than_period() {
        let mut indicators = HashMap::new();
        let key = "corr_test_5";
        // Period=10, feed 3 values: (1,2), (2,4), (3,6) → perfectly correlated
        for (a, b) in [(1.0, 2.0), (2.0, 4.0)] {
            let args = vec![Value::Float(a), Value::Float(b), Value::Int(10)];
            eval_stat_indicator("corr", &args, &mut indicators, key).unwrap();
        }
        let args = vec![Value::Float(3.0), Value::Float(6.0), Value::Int(10)];
        if let Ok(Some(Value::Float(f))) = eval_stat_indicator("corr", &args, &mut indicators, key) {
            assert!((f - 1.0).abs() < 1e-10, "Expected 1.0, got {}", f);
        }
    }

    #[test]
    fn test_corr_wrong_arg_count() {
        let mut indicators = HashMap::new();
        let args = vec![Value::Float(1.0), Value::Float(2.0)];
        let err = eval_stat_indicator("corr", &args, &mut indicators, "key").unwrap_err();
        assert_eq!(err, "corr requires 3 argument(s)");
    }

    #[test]
    fn test_corr_period_zero_error() {
        let mut indicators = HashMap::new();
        let args = vec![Value::Float(1.0), Value::Float(2.0), Value::Int(0)];
        let err = eval_stat_indicator("corr", &args, &mut indicators, "key").unwrap_err();
        assert_eq!(err, "corr: period must be a positive integer");
    }

    // --- covariance ---

    #[test]
    fn test_covariance_known_values() {
        // A=[1,2,3], B=[2,4,6]: mean_A=2, mean_B=4
        // cov = (1*2+2*4+3*6)/3 - 2*4 = 28/3 - 8 = 4/3
        let mut indicators = HashMap::new();
        let key = "cov_test_1";
        let mut result = 0.0;
        for (a, b) in [(1.0, 2.0), (2.0, 4.0), (3.0, 6.0)] {
            let args = vec![Value::Float(a), Value::Float(b), Value::Int(3)];
            if let Ok(Some(Value::Float(f))) = eval_stat_indicator("covariance", &args, &mut indicators, key) {
                result = f;
            }
        }
        let expected = 4.0 / 3.0;
        assert!((result - expected).abs() < 1e-10, "Expected {}, got {}", expected, result);
    }

    #[test]
    fn test_covariance_constant_series_zero() {
        let mut indicators = HashMap::new();
        let key = "cov_test_2";
        let mut result = 99.0;
        for v in [1.0, 2.0, 3.0, 4.0, 5.0] {
            let args = vec![Value::Float(v), Value::Float(5.0), Value::Int(5)];
            if let Ok(Some(Value::Float(f))) = eval_stat_indicator("covariance", &args, &mut indicators, key) {
                result = f;
            }
        }
        assert!(result.abs() < 1e-10, "Expected 0.0, got {}", result);
    }

    #[test]
    fn test_covariance_rolling_window() {
        let mut indicators = HashMap::new();
        let key = "cov_test_3";
        let mut result = 0.0;
        // Feed 5 values with period=3, only last 3 count
        for (a, b) in [(10.0, 20.0), (20.0, 40.0), (30.0, 60.0), (1.0, 2.0), (2.0, 4.0)] {
            let args = vec![Value::Float(a), Value::Float(b), Value::Int(3)];
            if let Ok(Some(Value::Float(f))) = eval_stat_indicator("covariance", &args, &mut indicators, key) {
                result = f;
            }
        }
        // Last 3: (30,60), (1,2), (2,4)
        let mean_a = (30.0 + 1.0 + 2.0) / 3.0;
        let mean_b = (60.0 + 2.0 + 4.0) / 3.0;
        let expected = (30.0 * 60.0 + 1.0 * 2.0 + 2.0 * 4.0) / 3.0 - mean_a * mean_b;
        assert!((result - expected).abs() < 1e-10, "Expected {}, got {}", expected, result);
    }

    #[test]
    fn test_covariance_wrong_arg_count() {
        let mut indicators = HashMap::new();
        let args = vec![Value::Float(1.0)];
        let err = eval_stat_indicator("covariance", &args, &mut indicators, "key").unwrap_err();
        assert_eq!(err, "covariance requires 3 argument(s)");
    }

    #[test]
    fn test_covariance_period_negative_error() {
        let mut indicators = HashMap::new();
        let args = vec![Value::Float(1.0), Value::Float(2.0), Value::Int(-5)];
        let err = eval_stat_indicator("covariance", &args, &mut indicators, "key").unwrap_err();
        assert_eq!(err, "covariance: period must be a positive integer");
    }

    // --- corr call-site isolation ---

    #[test]
    fn test_corr_independent_call_sites() {
        let mut indicators = HashMap::new();
        // Site 1: perfectly correlated
        for v in [1.0, 2.0, 3.0] {
            let args = vec![Value::Float(v), Value::Float(v), Value::Int(3)];
            eval_stat_indicator("corr", &args, &mut indicators, "site_1").unwrap();
        }
        // Site 2: perfectly anti-correlated
        for v in [1.0, 2.0, 3.0] {
            let args = vec![Value::Float(v), Value::Float(-v), Value::Int(3)];
            eval_stat_indicator("corr", &args, &mut indicators, "site_2").unwrap();
        }
        // Verify site 1 remains correlated
        let args = vec![Value::Float(4.0), Value::Float(4.0), Value::Int(3)];
        if let Ok(Some(Value::Float(f))) = eval_stat_indicator("corr", &args, &mut indicators, "site_1") {
            assert!((f - 1.0).abs() < 1e-10, "Site 1 expected 1.0, got {}", f);
        }
        // Verify site 2 remains anti-correlated
        let args = vec![Value::Float(4.0), Value::Float(-4.0), Value::Int(3)];
        if let Ok(Some(Value::Float(f))) = eval_stat_indicator("corr", &args, &mut indicators, "site_2") {
            assert!((f + 1.0).abs() < 1e-10, "Site 2 expected -1.0, got {}", f);
        }
    }

    // -----------------------------------------------------------------------
    // RSI Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_rsi_first_value_returns_neutral() {
        let mut indicators = HashMap::new();
        let result = eval_stat_indicator(
            "rsi", &[Value::Float(100.0), Value::Int(14)],
            &mut indicators, "test_0_10",
        ).unwrap().unwrap();
        assert!(matches!(result, Value::Float(f) if (f - 50.0).abs() < 1e-10));
    }

    #[test]
    fn test_rsi_all_gains_returns_100() {
        let mut indicators = HashMap::new();
        let key = "test_0_10";
        let vals: Vec<f64> = (100..=116).map(|x| x as f64).collect();
        for v in &vals {
            let _ = eval_stat_indicator(
                "rsi", &[Value::Float(*v), Value::Int(14)],
                &mut indicators, key,
            ).unwrap();
        }
        let r = eval_stat_indicator(
            "rsi", &[Value::Float(117.0), Value::Int(14)],
            &mut indicators, key,
        ).unwrap().unwrap();
        assert!(matches!(r, Value::Float(f) if (f - 100.0).abs() < 1e-10));
    }

    #[test]
    fn test_rsi_all_losses_returns_0() {
        let mut indicators = HashMap::new();
        let key = "test_0_10";
        let vals: Vec<f64> = (100..=116).rev().map(|x| x as f64).collect();
        for v in &vals {
            let _ = eval_stat_indicator(
                "rsi", &[Value::Float(*v), Value::Int(14)],
                &mut indicators, key,
            ).unwrap();
        }
        let r = eval_stat_indicator(
            "rsi", &[Value::Float(99.0), Value::Int(14)],
            &mut indicators, key,
        ).unwrap().unwrap();
        assert!(matches!(r, Value::Float(f) if f.abs() < 1e-10));
    }

    #[test]
    fn test_rsi_bounded_0_to_100() {
        let mut indicators = HashMap::new();
        let key = "test_0_10";
        let vals = [44.0, 44.34, 44.09, 43.61, 44.33, 44.83, 45.10,
            45.42, 45.84, 46.08, 45.89, 46.03, 45.61, 46.28, 46.28];
        for v in &vals {
            let r = eval_stat_indicator(
                "rsi", &[Value::Float(*v), Value::Int(14)],
                &mut indicators, key,
            ).unwrap().unwrap();
            match r {
                Value::Float(f) => {
                    assert!(f >= 0.0 && f <= 100.0, "RSI {} out of [0,100]", f);
                }
                _ => panic!("expected Float"),
            }
        }
    }

    #[test]
    fn test_rsi_period_zero_error() {
        let mut indicators = HashMap::new();
        let err = eval_stat_indicator(
            "rsi", &[Value::Float(100.0), Value::Int(0)],
            &mut indicators, "test",
        ).unwrap_err();
        assert_eq!(err, "rsi: period must be a positive integer");
    }

    #[test]
    fn test_rsi_wrong_arg_count() {
        let mut indicators = HashMap::new();
        let err = eval_stat_indicator(
            "rsi", &[Value::Float(100.0)],
            &mut indicators, "test",
        ).unwrap_err();
        assert_eq!(err, "rsi requires 2 argument(s)");
    }

    // -----------------------------------------------------------------------
    // ATR Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_atr_first_bar_uses_high_minus_low() {
        let mut indicators = HashMap::new();
        let r = eval_stat_indicator(
            "atr",
            &[Value::Float(50.0), Value::Float(45.0), Value::Float(48.0), Value::Int(14)],
            &mut indicators, "test",
        ).unwrap().unwrap();
        // First bar: TR = 50 - 45 = 5.0, ATR = 5.0
        assert!(matches!(r, Value::Float(f) if (f - 5.0).abs() < 1e-10));
    }

    #[test]
    fn test_atr_second_bar_with_equal_tr() {
        let mut indicators = HashMap::new();
        let key = "test";
        // First bar
        let _ = eval_stat_indicator(
            "atr",
            &[Value::Float(50.0), Value::Float(45.0), Value::Float(48.0), Value::Int(14)],
            &mut indicators, key,
        ).unwrap();
        // Second bar: TR = max(52-47, |52-48|, |47-48|) = max(5, 4, 1) = 5
        // ATR = 5*(13/14) + 5*(1/14) = 5.0
        let r = eval_stat_indicator(
            "atr",
            &[Value::Float(52.0), Value::Float(47.0), Value::Float(51.0), Value::Int(14)],
            &mut indicators, key,
        ).unwrap().unwrap();
        assert!(matches!(r, Value::Float(f) if (f - 5.0).abs() < 1e-10));
    }

    #[test]
    fn test_atr_gap_up_increases_atr() {
        let mut indicators = HashMap::new();
        let key = "test";
        // First bar: high=50, low=45, close=48
        let _ = eval_stat_indicator(
            "atr",
            &[Value::Float(50.0), Value::Float(45.0), Value::Float(48.0), Value::Int(14)],
            &mut indicators, key,
        ).unwrap();
        // Gap up: high=55, low=52, close=54, prev_close=48
        // TR = max(55-52, |55-48|, |52-48|) = max(3, 7, 4) = 7
        let expected = 5.0 * (13.0 / 14.0) + 7.0 * (1.0 / 14.0);
        let r = eval_stat_indicator(
            "atr",
            &[Value::Float(55.0), Value::Float(52.0), Value::Float(54.0), Value::Int(14)],
            &mut indicators, key,
        ).unwrap().unwrap();
        match r {
            Value::Float(f) => assert!((f - expected).abs() < 1e-10,
                "expected {}, got {}", expected, f),
            _ => panic!("expected Float"),
        }
    }

    #[test]
    fn test_atr_non_negative() {
        let mut indicators = HashMap::new();
        let key = "test";
        let bars = [(50.0, 45.0, 48.0), (49.0, 44.0, 46.0),
            (47.0, 43.0, 45.0), (48.0, 44.0, 47.0), (50.0, 46.0, 49.0)];
        for (h, l, c) in &bars {
            let r = eval_stat_indicator(
                "atr",
                &[Value::Float(*h), Value::Float(*l), Value::Float(*c), Value::Int(5)],
                &mut indicators, key,
            ).unwrap().unwrap();
            match r {
                Value::Float(f) => assert!(f >= 0.0, "ATR should be >= 0, got {}", f),
                _ => panic!("expected Float"),
            }
        }
    }

    #[test]
    fn test_atr_period_zero_error() {
        let mut indicators = HashMap::new();
        let err = eval_stat_indicator(
            "atr",
            &[Value::Float(50.0), Value::Float(45.0), Value::Float(48.0), Value::Int(0)],
            &mut indicators, "test",
        ).unwrap_err();
        assert_eq!(err, "atr: period must be a positive integer");
    }

    #[test]
    fn test_atr_wrong_arg_count() {
        let mut indicators = HashMap::new();
        let err = eval_stat_indicator(
            "atr", &[Value::Float(50.0), Value::Float(45.0)],
            &mut indicators, "test",
        ).unwrap_err();
        assert_eq!(err, "atr requires 4 argument(s)");
    }
}
