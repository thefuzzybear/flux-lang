struct Quote {
    bid: f64,
    bid_size: f64,
    ask: f64,
    ask_size: f64,
    timestamp: f64
}

struct Pair {
    left: f64,
    right: f64
}

fn get_spread(q: Quote) -> f64 {
    return q.ask - q.bid
}

strategy FmtTestStrategy {
    params {
        threshold = 1.0
    }

    state {
        count = 0
    }

    on bar {
        q = Quote { bid = close - 0.1, bid_size = 100.0, ask = close + 0.1, ask_size = 100.0, timestamp = 0.0 }
        s = get_spread(q)
        p = Pair { left = close, right = open }
        if s > threshold and not in_position {
            OPEN(symbol, 100.0)
        }
    }
}
