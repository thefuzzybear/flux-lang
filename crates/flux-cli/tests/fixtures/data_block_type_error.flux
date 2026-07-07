data {
    symbols = ["AAPL"]
    period = "2y2"
    interval = "1d"
    source = "yahoo"
}

strategy TypeErr {
    on bar {
        x = close
    }
}
