from lib::helpers import {double}

strategy ImportTest {
    params {
        factor = 2.0
    }

    on bar {
        val = double(close)
        if val > 100.0 and not in_position {
            OPEN(symbol, 100.0)
        }
    }
}
