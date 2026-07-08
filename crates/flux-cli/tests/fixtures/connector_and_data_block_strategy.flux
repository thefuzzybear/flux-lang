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

strategy DualBlock {
    params {
        period = 20
    }

    state {
        count = 0
    }

    on bar {
        count = count + 1
        if close > open and not in_position {
            OPEN(symbol, 100.0)
        }
        if close < open and in_position {
            CLOSE(symbol)
        }
    }
}
