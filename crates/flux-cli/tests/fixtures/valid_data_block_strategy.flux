data {
    symbols = ["AAPL"]
    period = "1y"
    interval = "1d"
    source = "yahoo"
}

strategy SimpleData {
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
    }
}
