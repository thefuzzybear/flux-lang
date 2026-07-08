# Params & State — Configurable entry timing with bar counting
#
# Demonstrates:
# - params block: configure when to enter/exit without editing code
# - state block: track bar count across iterations
#
# Params are strategy-level constants that you set once and reference
# throughout your logic. They let you tune behavior without changing code.
#
# State variables persist their values across bars. Unlike local variables
# (which reset each bar), state remembers where you left off.
#
# This strategy enters a position after `wait_bars` bars have passed,
# then exits after holding for `hold_bars` bars. Simple but shows
# the mechanics of params and state working together.

data {
    symbols = ["AAPL"]
    period = "1y"
    interval = "1d"
    source = "yahoo"
}

strategy TimedEntry {
    params {
        wait_bars = 5
        hold_bars = 10
        position_size = 100.0
    }

    state {
        bar_count = 0
        entry_bar = 0
    }

    on bar {
        bar_count = bar_count + 1

        # Enter after waiting
        if bar_count >= wait_bars and not in_position {
            OPEN(symbol, position_size)
            entry_bar = bar_count
        }

        # Exit after holding
        if in_position and (bar_count - entry_bar) >= hold_bars {
            CLOSE(symbol)
            entry_bar = 0
        }
    }
}
