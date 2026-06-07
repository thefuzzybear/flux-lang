strategy TypeBug {
    params {
        threshold = 10
    }

    on bar {
        if close + "hello" > 0 {
            OPEN(symbol, 100.0)
        }
    }
}
