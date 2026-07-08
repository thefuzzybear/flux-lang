from lib::nonexistent import {some_func}

strategy MissingImport {
    on bar {
        val = some_func(close)
        if val > 0.0 {
            OPEN(symbol, 100.0)
        }
    }
}
