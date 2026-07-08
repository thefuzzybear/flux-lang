# Pair Spread Strategy (Single-Symbol Simplified)
#
# Demonstrates user-defined functions for:
# - Ratio/spread computation with rolling statistics
# - Asymmetric exit logic (tight stop, wide profit target)
# - Signal strength scoring with multiple thresholds
# - Guard functions that prevent over-trading
#
# This is a simplified "pairs" approach operating on a single symbol:
# it uses the ratio of close-to-moving-average as a "spread" proxy,
# going long when the spread is compressed and exiting on expansion.
#
# In production, a true pairs strategy would use two symbols with
# cross-file imports (Layer 2 modules). This demo shows the function
# patterns that would compose such a strategy.

from indicators import {sma, ema}

data {
    symbols = ["AAPL"]
    period = "1y"
    interval = "1d"
    source = "yahoo"
}

# --- Spread Computation ---

# Compute the "spread" as percent deviation from moving average
fn price_spread(lookback) {
    avg = sma(close, lookback)
    if avg > 0.0 {
        return (close - avg) / avg * 100.0
    }
    return 0.0
}

# Rolling spread statistics: how extreme is current spread?
fn spread_zscore(lookback, z_lookback) {
    spread = price_spread(lookback)
    spread_mean = sma(close, z_lookback)
    spread_vol = stddev(close, z_lookback)

    # Simplified: use raw z-score of price as proxy for spread z-score
    z = zscore(close, z_lookback)
    return z
}

# --- Signal Scoring ---

# Entry score for long: how oversold is the spread?
fn long_entry_score(lookback, z_lookback) {
    z = spread_zscore(lookback, z_lookback)
    spread = price_spread(lookback)

    score = 0.0

    # Z-score component: deep negative = strong buy signal
    if z < 0.0 - 2.0 {
        score = score + 2.0
    } elif z < 0.0 - 1.5 {
        score = score + 1.0
    }

    # Spread deviation component: more compressed = stronger signal
    if spread < 0.0 - 3.0 {
        score = score + 1.0
    } elif spread < 0.0 - 2.0 {
        score = score + 0.5
    }

    # Volume confirmation: high volume on dip
    avg_vol = sma(volume, lookback)
    if avg_vol > 0.0 {
        vol_ratio = volume / avg_vol
        if vol_ratio > 1.5 {
            score = score + 0.5
        }
    }

    return score
}

# --- Trade Guards ---

# Cooldown guard: prevent re-entry too soon after an exit
# Returns 1.0 if enough bars have passed since last trade
fn cooldown_ok(bars_since_exit, min_cooldown) {
    if bars_since_exit >= min_cooldown {
        return 1.0
    }
    return 0.0
}

# Volatility guard: don't trade in extremely low vol environments
fn volatility_ok(lookback, min_vol) {
    vol = stddev(close, lookback)
    if vol > min_vol {
        return 1.0
    }
    return 0.0
}

# Combined entry guard: all conditions must pass
fn entry_allowed(bars_since_exit, min_cooldown, lookback, min_vol) {
    cool = cooldown_ok(bars_since_exit, min_cooldown)
    vol = volatility_ok(lookback, min_vol)
    if cool > 0.5 and vol > 0.5 {
        return 1.0
    }
    return 0.0
}

# --- Exit Logic ---

# Profit target: spread has reverted past the mean into positive territory
fn hit_profit(lookback, target_pct) {
    spread = price_spread(lookback)
    if spread > target_pct {
        return 1.0
    }
    return 0.0
}

# Stop loss: spread went further against us
fn hit_stop(lookback, stop_pct) {
    spread = price_spread(lookback)
    if spread < 0.0 - stop_pct {
        return 1.0
    }
    return 0.0
}

# Time stop: position held too long
fn hit_time_stop(bars_held, max_bars) {
    if bars_held > max_bars {
        return 1.0
    }
    return 0.0
}

# Asymmetric exit: tight stop, wider profit target, time limit
fn should_exit_position(lookback, profit_target, stop_level, bars_held, max_bars) {
    if hit_profit(lookback, profit_target) > 0.5 {
        return 1.0
    }
    if hit_stop(lookback, stop_level) > 0.5 {
        return 1.0
    }
    if hit_time_stop(bars_held, max_bars) > 0.5 {
        return 1.0
    }
    return 0.0
}

# --- Strategy ---

strategy PairSpread {
    params {
        lookback = 20
        z_lookback = 30
        entry_threshold = 2.5
        profit_target = 2.0
        stop_level = 5.0
        max_hold_bars = 10
        cooldown_bars = 3
        min_volatility = 0.5
        position_size = 100.0
    }

    state {
        bar_count = 0
        bars_in_trade = 0
        bars_since_exit = 100
    }

    on bar {
        bar_count = bar_count + 1

        # Track time in/out of positions
        if in_position {
            bars_in_trade = bars_in_trade + 1
        } else {
            bars_since_exit = bars_since_exit + 1
        }

        # Wait for indicators to warm up
        if bar_count > z_lookback {
            # Entry logic
            if not in_position {
                allowed = entry_allowed(bars_since_exit, cooldown_bars, lookback, min_volatility)
                if allowed > 0.5 {
                    score = long_entry_score(lookback, z_lookback)
                    if score >= entry_threshold {
                        OPEN(symbol, position_size)
                        bars_in_trade = 0
                        bars_since_exit = 0
                    }
                }
            }

            # Exit logic
            if in_position {
                exit = should_exit_position(lookback, profit_target, stop_level, bars_in_trade, max_hold_bars)
                if exit > 0.5 {
                    CLOSE(symbol)
                    bars_in_trade = 0
                    bars_since_exit = 0
                }
            }
        }
    }
}
