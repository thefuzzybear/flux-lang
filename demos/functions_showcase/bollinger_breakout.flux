# Bollinger Band Breakout Strategy
#
# Demonstrates cross-file imports using Flux's :: module syntax.
# Signal detection helpers are imported from lib/signals.flux,
# while entry/exit logic remains in the main strategy file.
#
# Logic: Enter long on upper band breakout with volume confirmation.
# Exit when price falls back inside the bands or hits a trailing stop.

from lib::signals import {breakout_signal, exit_signal}
from indicators import {sma, ema}

data {
    symbols = ["AAPL"]
    period = "1y"
    interval = "1d"
    source = "yahoo"
}

# --- Entry/Exit Functions (strategy-specific) ---

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
