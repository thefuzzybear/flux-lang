# Multi-Symbol — Trading multiple assets
#
# Demonstrates:
# - data block with multiple symbols
# - Using the `symbol` variable to make per-asset decisions
# - Running the same logic across different markets
#
# A simple momentum strategy applied to both AAPL and MSFT.
# Each symbol is traded independently.
from indicators import {sma}

data {
    symbols = ["AAPL", "MSFT"]
    period = "1y"
    interval = "1d"
    source = "yahoo"
}

strategy MultiMomentum {
    params {
        lookback = 20
        position_size = 100.0
    }

    state {
        bar_count = 0
    }

    on bar {
        bar_count = bar_count + 1
        avg = sma(close, lookback)

        # Same logic applied to whichever symbol is being processed
        if bar_count > lookback {
            if close > avg and not in_position {
                OPEN(symbol, position_size)
            }
            if close < avg and in_position {
                CLOSE(symbol)
            }
        }
    }
}
