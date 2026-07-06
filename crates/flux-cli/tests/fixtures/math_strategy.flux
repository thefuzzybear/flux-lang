strategy MathTest {
    params {
        period = 5
        threshold = 2.0
    }

    state {
        bar_count = 0
    }

    on bar {
        bar_count = bar_count + 1
        vol = stddev(close, period)
        z = zscore(close, period)
        strength = rsi(close, period)
        range = atr(high, low, close, period)

        avg = sqrt(pow(close, 2.0))
        capped = min(max(z, -3.0), 3.0)

        if z > threshold and not in_position {
            OPEN(symbol, floor(abs(100.0 / vol)))
        }

        if z < 0.0 and in_position {
            CLOSE(symbol)
        }
    }
}
