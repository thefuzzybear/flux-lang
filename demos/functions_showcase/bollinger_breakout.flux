# Bollinger Band Breakout Strategy
#
# Demonstrates user-defined functions for:
# - Band computation (pure math)
# - Volume confirmation (multi-factor filter)
# - Signal emission from functions
# - Functions calling other functions
#
# Logic: Enter long on upper band breakout with volume confirmation.
# Exit when price falls back inside the bands or hits a trailing stop.

from indicators import {sma, ema}

data {
    symbols = ["AAPL"]
    period = "1y"
    interval = "1d"
    source = "yahoo"
}

# --- Helper Functions ---

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

# Entry execution: opens a position if breakout conditions are met
fn try_enter(lookback, band_std, vol_lookback, vol_mult, min_width, size) {
    signal = breakout_signal(lookback, band_std, vol_lookback, vol_mult, min_width)
    if signal > 0.5 and not in_position {
        OPEN(symbol, size)
    }
}

# Exit execution: closes position if exit conditions are met
fn try_exit(lookback) {
    if in_position {
        should_exit = exit_signal(lookback)
        if should_exit > 0.5 {
            CLOSE(symbol)
        }
    }
}

# --- Strategy ---

strategy BollingerBreakout {
    params {
        lookback = 20
        band_std = 2.0
        vol_lookback = 20
        vol_multiplier = 1.5
        min_band_width = 3.0
        position_size = 100.0
    }

    state {
        bar_count = 0
    }

    on bar {
        bar_count = bar_count + 1

        # Wait for indicators to warm up
        if bar_count > lookback {
            try_enter(lookback, band_std, vol_lookback, vol_multiplier, min_band_width, position_size)
            try_exit(lookback)
        }
    }
}
