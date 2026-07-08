# Module Imports Demo — Multi-File Mean Reversion
#
# A mean reversion strategy organized across multiple files:
#   - signals/entry.flux  → entry logic (z-score + volume confirmation)
#   - signals/exit.flux   → exit logic (z-score reversion)
#   - math/stats.flux     → shared z-score and ratio helpers
#
# This demonstrates Flux's :: module import system:
#   1. The main file imports high-level signal functions
#   2. Signal modules import shared math helpers
#   3. The resolver handles transitive dependencies automatically

from signals::entry import {should_enter}
from signals::exit import {should_exit}
from indicators import {sma}

data {
    symbols = ["AAPL"]
    period = "1y"
    interval = "1d"
    source = "yahoo"
}

strategy ModularMeanReversion {
    params {
        lookback = 20
        z_threshold = 2.0
        vol_lookback = 20
        vol_threshold = 1.2
        position_size = 100.0
    }

    state {
        bar_count = 0
    }

    on bar {
        bar_count = bar_count + 1

        # Wait for indicators to warm up
        if bar_count > lookback {
            # Entry: delegate to signals/entry.flux
            if should_enter(lookback, z_threshold, vol_lookback, vol_threshold) > 0.5 and not in_position {
                OPEN(symbol, position_size)
            }

            # Exit: delegate to signals/exit.flux
            if should_exit(lookback, z_threshold) > 0.5 and in_position {
                CLOSE(symbol)
            }
        }
    }
}
