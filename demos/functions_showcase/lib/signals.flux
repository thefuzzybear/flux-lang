# lib/signals.flux — Reusable signal detection helpers
#
# This library file contains shared helper functions for
# breakout detection, volume analysis, and band calculations.
# It demonstrates cross-file imports using Flux's :: module syntax.

from indicators import {sma}

# Compute the upper Bollinger Band value
fn upper_band(price, lookback, num_std) {
    avg = sma(price, lookback)
    vol = stddev(price, lookback)
    return avg + num_std * vol
}

# Compute the lower Bollinger Band value
fn lower_band(price, lookback, num_std) {
    avg = sma(price, lookback)
    vol = stddev(price, lookback)
    return avg - num_std * vol
}

# Compute band width as a percentage of the midline
# Wider bands = higher volatility = stronger breakouts
fn band_width_pct(price, lookback, num_std) {
    upper = upper_band(price, lookback, num_std)
    lower = lower_band(price, lookback, num_std)
    mid = sma(price, lookback)
    if mid > 0.0 {
        return (upper - lower) / mid * 100.0
    }
    return 0.0
}

# Volume surge detector: is today's volume significantly above average?
fn volume_surge(lookback, multiplier) {
    avg_vol = sma(volume, lookback)
    if avg_vol > 0.0 {
        ratio = volume / avg_vol
        if ratio > multiplier {
            return 1.0
        }
    }
    return 0.0
}

# Combined breakout signal: price above upper band + volume surge + minimum band width
fn breakout_signal(lookback, band_std, vol_lookback, vol_mult, min_width) {
    upper = upper_band(close, lookback, band_std)
    width = band_width_pct(close, lookback, band_std)
    vol_ok = volume_surge(vol_lookback, vol_mult)

    # All three conditions must be met
    if close > upper and vol_ok > 0.5 and width > min_width {
        return 1.0
    }
    return 0.0
}

# Exit signal: price drops below the moving average (middle band)
fn exit_signal(lookback) {
    mid = sma(close, lookback)
    if close < mid {
        return 1.0
    }
    return 0.0
}
