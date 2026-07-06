# Simple Moving Average Crossover Strategy
#
# This strategy uses two moving averages with different periods:
# - A short-period SMA that reacts quickly to price changes
# - A long-period SMA that represents the longer-term trend
#
# When the short SMA crosses above the long SMA, it signals upward
# momentum — we open a position. When the short SMA crosses below
# the long SMA, momentum is fading — we close the position.

from indicators import {sma}

strategy SmaCrossover {
    # Configurable parameters — tune these without changing logic
    params {
        short_period = 10
        long_period = 30
        position_size = 100.0
    }

    # State variables persist across bars
    state {
        bar_count = 0
    }

    # Runs once for each row of price data
    on bar {
        # Track how many bars we've processed
        bar_count = bar_count + 1

        # Calculate the short-period and long-period moving averages.
        # sma(value, period) returns the average of the last N values.
        short_avg = sma(close, short_period)
        long_avg = sma(close, long_period)

        # Bullish crossover: short SMA is above long SMA.
        # This suggests recent prices are rising faster than the
        # longer-term trend — open a position if we don't have one.
        if short_avg > long_avg and not in_position {
            OPEN(symbol, position_size)
        }

        # Bearish crossover: short SMA drops below long SMA.
        # Momentum is weakening — close the position to lock in gains
        # or limit losses.
        if short_avg < long_avg and in_position {
            CLOSE(symbol)
        }
    }
}
