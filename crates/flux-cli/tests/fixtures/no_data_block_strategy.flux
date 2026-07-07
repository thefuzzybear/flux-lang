strategy NoData {
    params {
        period = 20
    }

    on bar {
        if close > open and not in_position {
            OPEN(symbol, 100.0)
        }
    }
}
