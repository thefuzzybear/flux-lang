from indicators import {sma}

strategy BuiltinImportTest {
    params {
        period = 20
    }

    on bar {
        avg = sma(close, period)
        if close > avg and not in_position {
            OPEN(symbol, 100.0)
        }
    }
}
