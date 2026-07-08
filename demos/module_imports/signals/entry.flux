# signals/entry.flux — Entry signal detection
#
# Determines whether conditions are right to open a new position.
# Imports shared math helpers from the project's math/ directory.

from math::stats import {z_score, safe_ratio, exceeds}
from indicators import {sma}

# Volume confirmation: is current volume elevated relative to average?
fn volume_confirmed(vol_lookback, vol_threshold) {
    avg_vol = sma(volume, vol_lookback)
    ratio = safe_ratio(volume, avg_vol)
    return exceeds(ratio, vol_threshold)
}

# Combined entry signal: oversold (negative z-score) + volume confirmation
# Returns 1.0 if we should enter, 0.0 otherwise
fn should_enter(lookback, z_threshold, vol_lookback, vol_threshold) {
    z = z_score(close, lookback)
    vol_ok = volume_confirmed(vol_lookback, vol_threshold)

    # Enter when price is oversold AND volume confirms
    if z < 0.0 - z_threshold and vol_ok > 0.5 {
        return 1.0
    }
    return 0.0
}
