# SMA/EMA Crossover — Technical analysis with built-in indicators
#
# Demonstrates:
# - Importing and using indicators (sma, ema)
# - Crossover detection logic
# - Combining fast and slow moving averages
#
# Enters long when the fast EMA crosses above the slow SMA.
# Exits when the fast EMA crosses below the slow SMA.
from indicators import {sma, ema}

data {
    symbols = ["AAPL"]
    period = "1y"
    interval = "1d"
    source = "yahoo"
}

strategy SmaCrossover {
    params {
        fast_period = 10
        slow_period = 30
        position_size = 100.0
    }

    state {
        prev_fast = 0.0
        prev_slow = 0.0
        bar_count = 0
    }

    on bar {
        bar_count = bar_count + 1
        fast = ema(close, fast_period)
        slow = sma(close, slow_period)

        # Need enough bars for the slow indicator to warm up
        if bar_count > slow_period {
            # Bullish crossover: fast crosses above slow
            if prev_fast <= prev_slow and fast > slow and not in_position {
                OPEN(symbol, position_size)
            }

            # Bearish crossover: fast crosses below slow
            if prev_fast >= prev_slow and fast < slow and in_position {
                CLOSE(symbol)
            }
        }

        prev_fast = fast
        prev_slow = slow
    }
}
