# Mean Reversion Strategy
#
# This strategy uses the z-score to identify when price has deviated
# significantly from its recent mean. The z-score measures how many
# standard deviations the current price is from the rolling average.
#
# Logic:
#   - When z-score drops below the negative threshold (e.g., -2.0),
#     price is "oversold" — we open a long position expecting a bounce.
#   - When z-score rises above the positive threshold (e.g., +2.0),
#     price is "overbought" — we close the position expecting a pullback.
#
# Key concepts:
#   - zscore(value, period) returns how many std devs value is from its mean
#   - stddev(value, period) gives the rolling standard deviation (used here
#     as a volatility filter to avoid trading in low-volatility regimes)
#   - Threshold comparison uses 0.0 - threshold because Flux does not
#     support unary negation on expressions in conditions

strategy MeanReversion {
    # Strategy parameters — tune these for different assets/timeframes
    params {
        lookback = 20
        threshold = 2.0
        position_size = 100.0
        min_volatility = 0.5
    }

    # Persistent state across bars
    state {
        bar_count = 0
    }

    on bar {
        # Track how many bars have elapsed
        bar_count = bar_count + 1

        # Compute the z-score: how far price is from its rolling mean
        # A z-score of -2.0 means price is 2 standard deviations below average
        z = zscore(close, lookback)

        # Compute rolling volatility (standard deviation of closing prices)
        # We use this as a filter to avoid trading in dead markets
        vol = stddev(close, lookback)

        # Entry: price is oversold (z-score below negative threshold)
        # and volatility is high enough to suggest a tradeable move
        if z < 0.0 - threshold and not in_position and vol > min_volatility {
            OPEN(symbol, position_size)
        }

        # Exit: price has reverted above the positive threshold
        # (overbought), so we take profit and close the position
        if z > threshold and in_position {
            CLOSE(symbol)
        }
    }
}
