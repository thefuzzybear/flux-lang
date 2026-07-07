data {
    symbols = ["AAPL"]
    period = "6mo"
    interval = "1d"
    source = "yahoo"
}

strategy Override {
    on bar {
        if close > open and not in_position {
            OPEN(symbol, 100.0)
        }
    }
}
