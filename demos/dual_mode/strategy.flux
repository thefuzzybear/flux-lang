# Dual Mode — Backtest and live with the same strategy
#
# Demonstrates:
# - data block for historical backtesting
# - connector block for live data feeds
# - Same strategy logic works in both modes
#
# The strategy uses a simple momentum approach. In backtest mode,
# it reads from the data block. In live mode, it connects via websocket.
from indicators import {sma}

data {
    symbols = ["AAPL"]
    period = "1y"
    interval = "1d"
    source = "yahoo"
}

connector {
    type = "websocket"
    url = "wss://stream.example.com/v1"
    symbols = ["AAPL"]
    interval = "1m"
}

strategy DualMomentum {
    params {
        period = 20
        position_size = 100.0
    }

    state {
        bar_count = 0
    }

    on bar {
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
