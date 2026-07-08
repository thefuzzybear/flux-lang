from circular::a import {func_a}

strategy CircularTest {
    on bar {
        val = func_a(close)
        if val > 0.0 {
            OPEN(symbol, 100.0)
        }
    }
}
