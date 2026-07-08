# Adaptive Mean Reversion Strategy
#
# Demonstrates user-defined functions for:
# - Regime detection (trending vs. mean-reverting)
# - Dynamic position sizing based on conviction
# - Multi-factor entry scoring
# - Tiered exit logic (profit target, stop loss, time-based)
# - Functions with complex control flow (if/elif/else, early return)
#
# Logic: Only trade mean reversion when the market is range-bound.
# Size positions proportional to signal strength. Exit adaptively.

from indicators import {sma, ema}

data {
    symbols = ["AAPL"]
    period = "1y"
    interval = "1d"
    source = "yahoo"
}

# --- Regime Detection ---

# Compute trend strength: ratio of directional move to volatility.
# High values indicate trending, low values indicate mean-reverting.
fn trend_strength(lookback) {
    current_price = close
    past_price = sma(close, lookback)
    direction = current_price - past_price
    vol = stddev(close, lookback)

    if vol > 0.01 {
        return abs(direction) / vol
    }
    return 0.0
}

# Regime filter: returns 1.0 if market is mean-reverting (low trend strength)
fn is_mean_reverting(lookback, max_trend) {
    strength = trend_strength(lookback)
    if strength < max_trend {
        return 1.0
    }
    return 0.0
}

# --- Signal Scoring ---

# Z-score component: how far is price from its mean?
fn zscore_component(lookback) {
    z = zscore(close, lookback)
    # Stronger signal when more oversold
    if z < 0.0 - 2.5 {
        return 2.0
    }
    if z < 0.0 - 2.0 {
        return 1.5
    }
    if z < 0.0 - 1.5 {
        return 1.0
    }
    return 0.0
}

# Volume component: elevated volume on dips suggests capitulation
fn volume_component(vol_lookback) {
    avg_vol = sma(volume, vol_lookback)
    if avg_vol > 0.0 {
        ratio = volume / avg_vol
        if ratio > 2.0 {
            return 1.0
        }
        if ratio > 1.5 {
            return 0.5
        }
    }
    return 0.0
}

# Spread component: wide intraday range suggests exhaustion
fn spread_component(threshold) {
    if open > 0.0 {
        spread_pct = (high - low) / open * 100.0
        if spread_pct > threshold {
            return 0.5
        }
    }
    return 0.0
}

# Combined entry score: sum of all components
fn entry_score(lookback, vol_lookback, spread_threshold) {
    z_score = zscore_component(lookback)
    vol_score = volume_component(vol_lookback)
    spread_score = spread_component(spread_threshold)
    return z_score + vol_score + spread_score
}

# --- Position Sizing ---

# Dynamic size: scale position based on conviction (entry score)
fn compute_position_size(score, base_size, max_multiplier) {
    # Linear scaling: score of 2.0 gets full size, below 2.0 gets partial
    multiplier = score / 2.0
    if multiplier > max_multiplier {
        multiplier = max_multiplier
    }
    if multiplier < 0.5 {
        multiplier = 0.5
    }
    return base_size * multiplier
}

# --- Exit Logic ---

# Profit target: z-score reverted past zero into positive territory
fn hit_profit_target(lookback, target_z) {
    z = zscore(close, lookback)
    if z > target_z {
        return 1.0
    }
    return 0.0
}

# Stop loss: z-score went even more extreme (wrong direction)
fn hit_stop_loss(lookback, stop_z) {
    z = zscore(close, lookback)
    if z < 0.0 - stop_z {
        return 1.0
    }
    return 0.0
}

# Combined exit decision
fn should_exit(lookback, target_z, stop_z, max_bars, bars_held) {
    # Profit target hit
    if hit_profit_target(lookback, target_z) > 0.5 {
        return 1.0
    }
    # Stop loss hit
    if hit_stop_loss(lookback, stop_z) > 0.5 {
        return 1.0
    }
    # Time-based exit: held too long without resolution
    if bars_held > max_bars {
        return 1.0
    }
    return 0.0
}

# --- Strategy ---

strategy AdaptiveReversion {
    params {
        lookback = 20
        vol_lookback = 15
        max_trend_strength = 1.5
        entry_threshold = 2.0
        spread_threshold = 1.5
        base_size = 100.0
        max_size_mult = 2.0
        profit_z = 0.5
        stop_z = 3.5
        max_hold_bars = 15
    }

    state {
        bar_count = 0
        bars_in_position = 0
    }

    on bar {
        bar_count = bar_count + 1

        # Track how long we've been in a position
        if in_position {
            bars_in_position = bars_in_position + 1
        }

        # Wait for warmup
        if bar_count > lookback {
            # Only trade in mean-reverting regimes
            regime_ok = is_mean_reverting(lookback, max_trend_strength)

            if regime_ok > 0.5 and not in_position {
                # Compute entry score
                score = entry_score(lookback, vol_lookback, spread_threshold)

                # Enter if score exceeds threshold
                if score >= entry_threshold {
                    size = compute_position_size(score, base_size, max_size_mult)
                    OPEN(symbol, size)
                    bars_in_position = 0
                }
            }

            # Adaptive exit
            if in_position {
                exit = should_exit(lookback, profit_z, stop_z, max_hold_bars, bars_in_position)
                if exit > 0.5 {
                    CLOSE(symbol)
                    bars_in_position = 0
                }
            }
        }
    }
}
