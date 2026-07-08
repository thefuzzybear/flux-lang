# signals/math/stats.flux — Shared statistical helper functions
#
# Pure computation functions for z-scores, volatility measures,
# and normalization. No side effects, no signals.
#
# This file is imported by sibling modules (entry.flux, exit.flux)
# via `from math::stats import {...}` — which resolves relative to
# the importing file's directory.

from indicators import {sma}

# Compute z-score: how many standard deviations price is from its mean
fn z_score(price, lookback) {
    avg = sma(price, lookback)
    vol = stddev(price, lookback)
    if vol > 0.001 {
        return (price - avg) / vol
    }
    return 0.0
}

# Compute a normalized ratio (value / baseline), returns 0 if baseline is near zero
fn safe_ratio(value, baseline) {
    if baseline > 0.001 {
        return value / baseline
    }
    return 0.0
}

# Simple threshold check: returns 1.0 if value exceeds threshold, else 0.0
fn exceeds(value, threshold) {
    if value > threshold {
        return 1.0
    }
    return 0.0
}
