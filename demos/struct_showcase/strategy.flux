# Struct Showcase — Spread-Based Strategy Using Stdlib Structs
#
# Demonstrates:
#   - Importing stdlib structs (Quote from market::l1)
#   - Constructing struct literals
#   - Accessing struct fields with dot notation
#   - Passing structs to helper functions
#   - Using calc_spread and calc_mid from market::l1
#
# The strategy builds a Quote from bar data each tick, computes the
# bid-ask spread, and enters when the spread is tight (liquidity signal).

from market::l1 import {Quote, calc_spread, calc_mid}

struct TradeSignal {
    score: f64,
    direction: int,
    size: f64
}

fn build_quote(bid_price: f64, ask_price: f64, qty: f64) -> Quote {
    return Quote {
        bid = bid_price,
        bid_size = qty,
        ask = ask_price,
        ask_size = qty,
        timestamp = 0.0
    }
}

fn evaluate_spread(q: Quote, max_spread: f64, base_size: f64) -> TradeSignal {
    spread = calc_spread(q)
    mid = calc_mid(q)

    # Tight spread = good liquidity = enter long
    direction = 0
    score = 0.0
    size = 0.0

    if spread < max_spread and spread > 0.0 {
        direction = 1
        score = 1.0 - (spread / max_spread)
        size = base_size * score
    }

    return TradeSignal { score = score, direction = direction, size = size }
}

strategy SpreadStrategy {
    params {
        max_spread = 2.0
        base_size = 100.0
        exit_bars = 5
    }

    state {
        bars_in_position = 0
    }

    on bar {
        # Simulate a quote from bar data (bid slightly below close, ask above)
        q = build_quote(close - 0.25, close + 0.25, 500.0)

        # Evaluate the spread signal
        signal = evaluate_spread(q, max_spread, base_size)

        # Entry: spread is tight and signal is strong
        if signal.direction > 0 and signal.score > 0.5 and not in_position {
            OPEN(symbol, signal.size)
            bars_in_position = 0
        }

        # Exit: time-based exit after holding for exit_bars
        if in_position {
            bars_in_position = bars_in_position + 1
            if bars_in_position >= exit_bars {
                CLOSE(symbol)
                bars_in_position = 0
            }
        }
    }
}
