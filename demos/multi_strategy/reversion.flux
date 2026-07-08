# Mean reversion strategy — buys dips below SMA
#
# A contrarian approach: go long when price is oversold (z-score
# below negative threshold), close when it reverts above the
# positive threshold. Designed to be run independently or as
# part of a multi-strategy TOML config.
#
# Uses a symbol guard so it only processes MSFT bars when run
# against a multi-symbol CSV file.
from indicators import {sma, zscore}

data {
    symbols = ["MSFT"]
    period = "1y"
    interval = "1d"
    source = "yahoo"
}

strategy Reversion {
    params {
        lookback = 20
        threshold = 1.5
        position_size = 100.0
    }

    state {
        bar_count = 0
    }

    on bar {
        if symbol == "MSFT" {
            bar_count = bar_count + 1
            z = zscore(close, lookback)

            if bar_count > lookback {
                if z < 0.0 - threshold and not in_position {
                    OPEN(symbol, position_size)
                }
                if z > threshold and in_position {
                    CLOSE(symbol)
                }
            }
        }
    }
}
