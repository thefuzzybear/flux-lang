# Integration test: strategy using stdlib structs and helper functions
# Validates full pipeline (lex → parse → typecheck → codegen) and interpreter support.

from market::l1 import {Quote, calc_spread, calc_mid}

struct Signal {
    strength: f64,
    direction: int
}

fn make_quote(b: f64, a: f64) -> Quote {
    return Quote { bid = b, bid_size = 100.0, ask = a, ask_size = 100.0, timestamp = 0.0 }
}

fn compute_signal(q: Quote) -> Signal {
    spread = calc_spread(q)
    mid = calc_mid(q)
    direction = 0
    strength = 0.0
    if spread > 0.5 {
        direction = 1
        strength = spread / mid
    }
    return Signal { strength = strength, direction = direction }
}

strategy StructSpreadStrategy {
    params {
        spread_threshold = 0.5
        position_size = 100.0
    }

    state {
        bar_count = 0
    }

    on bar {
        bar_count = bar_count + 1

        # Construct a Quote struct from bar data
        q = make_quote(close - 0.25, close + 0.25)

        # Access struct fields and pass struct to function
        sig = compute_signal(q)

        if sig.direction > 0 and sig.strength > 0.001 and not in_position {
            OPEN(symbol, position_size)
        }

        if sig.direction == 0 and in_position {
            CLOSE(symbol)
        }
    }
}
