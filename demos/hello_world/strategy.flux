# Hello World — The simplest possible Flux strategy
#
# Opens a position on the very first bar. That's it.
# No params, no state, no indicators — just one signal.
# Use this to verify your Flux installation works end-to-end.

data {
    symbols = ["AAPL"]
    period = "1y"
    interval = "1d"
    source = "yahoo"
}

strategy HelloWorld {
    on bar {
        if not in_position {
            OPEN(symbol, 100.0)
        }
    }
}
