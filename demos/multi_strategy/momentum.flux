# Momentum strategy — buys breakouts above SMA
#
# A simple trend-following approach: go long when price is above
# its moving average, close when it drops below. Designed to be
# run independently or as part of a multi-strategy TOML config.
#
# Uses a symbol guard so it only processes AAPL bars when run
# against a multi-symbol CSV file.
from indicators import {sma}

data {
    symbols = ["AAPL"]
    period = "1y"
    interval = "1d"
    source = "yahoo"
}

strategy Momentum {
    params {
        period = 20
        position_size = 100.0
    }

    state {
        bar_count = 0
    }

    on bar {
        if symbol == "AAPL" {
            bar_count = bar_count + 1
            avg = sma(close, period)

            if bar_count > period {
                if close > avg and not in_position {
                    OPEN(symbol, position_size)
                }
                if close < avg and in_position {
                    CLOSE(symbol)
                }
            }
        }
    }
}
