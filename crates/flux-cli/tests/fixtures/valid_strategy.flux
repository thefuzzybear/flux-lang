strategy Simple {
    params {
        period = 20
        threshold = 2.5
    }

    state {
        count = 0
    }

    on bar {
        if close > open {
            OPEN(symbol, 100.0)
        }
    }
}
