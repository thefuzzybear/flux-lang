# Mean Reversion Strategy — AAPL Daily
#
# Buys when price drops 2 standard deviations below its 20-day mean,
# sells when it reverts above 2 standard deviations. Classic z-score
# mean reversion applied to daily AAPL bars.
from indicators import {sma}

data {
    symbols = ["NVDA"]
    period = "1y"
    interval = "1d"
    source = "yahoo"
}

strategy MeanReversion {
    params {
        lookback = 20
        threshold = 2.0
        position_size = 100.0
        min_volatility = 0.5
    }

    state {
        bar_count = 0
    }

    on bar {
        bar_count = bar_count + 1
        # Z-score: how far price is from rolling mean in std dev units
        z = zscore(close, lookback)
        # Volatility filter: only trade when market is active
        vol = stddev(close, lookback)
        # Entry: oversold + sufficient volatility
        if z < 0.0 - threshold and not in_position and vol > min_volatility {
            OPEN(symbol, position_size)
        }
        # Exit: price reverted above threshold
        if z > threshold and in_position {
            CLOSE(symbol)
        }
    }
}
