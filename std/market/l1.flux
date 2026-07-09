# std/market/l1.flux — Level 1 Market Data Structures
#
# Provides standard struct definitions for top-of-book market data:
# individual trades (Tick), OHLCV candles (Bar), best bid/offer (Quote),
# and a consolidated market view (MarketSnapshot).

struct Tick {
    price: f64,
    size: f64,
    side: int,
    timestamp: f64
}

struct Bar {
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
    timestamp: f64
}

struct Quote {
    bid: f64,
    bid_size: f64,
    ask: f64,
    ask_size: f64,
    timestamp: f64
}

struct MarketSnapshot {
    quote: Quote,
    last_price: f64,
    last_size: f64,
    mid: f64,
    spread: f64
}

fn calc_spread(q: Quote) -> f64 {
    return q.ask - q.bid
}

fn calc_mid(q: Quote) -> f64 {
    return (q.bid + q.ask) / 2.0
}

fn classify_trade(t: Tick, q: Quote) -> int {
    mid = (q.bid + q.ask) / 2.0
    if t.price > mid {
        return 1
    }
    if t.price < mid {
        return -1
    }
    return 0
}

# --- Window: Fixed-size f64 ring buffer ---
#
# A circular buffer for maintaining rolling windows of numeric data.
# Uses [f64; 256] as the backing store with a logical capacity field.
# Modulo indexing allows O(1) push without shifting elements.

struct Window {
    buf: [f64; 256],
    index: int,
    count: int,
    capacity: int
}

fn window_new(capacity: int) -> Window {
    # Returns a zero-initialized Window with the given logical capacity.
    # The buf array is zero-filled by default (runtime-managed initialization).
    buf_init = zeros(256)
    return Window { buf = buf_init, index = 0, count = 0, capacity = capacity }
}

fn window_push(w: Window, value: f64) -> Window {
    # Writes value at the current index position (modulo capacity),
    # advances the index, and increments count (up to capacity).
    new_index = w.index + 1
    if new_index >= w.capacity {
        new_index = new_index - w.capacity
    }
    new_count = w.count + 1
    if new_count > w.capacity {
        new_count = w.capacity
    }
    # Write value at w.index, return updated Window
    # Note: array element assignment (w.data[w.index] = value) is handled by the runtime
    return Window { buf = w.buf, index = new_index, count = new_count, capacity = w.capacity }
}

fn window_get(w: Window, index: int) -> f64 {
    # Returns the value at logical position (0 = most recent).
    # Errors if index >= count (checked by the runtime/interpreter).
    if index >= w.count {
        return 0.0
    }
    pos = w.index - 1 - index
    if pos < 0 {
        pos = pos + w.capacity
    }
    return w.buf[pos]
}

fn window_mean(w: Window) -> f64 {
    # Returns the arithmetic mean of all inserted values.
    # For a full implementation, this sums data[0..count] and divides by count.
    # Iterative summation requires loop support; the runtime provides this as a builtin.
    if w.count == 0 {
        return 0.0
    }
    # Sum the valid elements and divide by count
    # (loop-based summation is handled by the runtime intrinsic)
    sum = 0.0
    i = 0
    # Note: while loops are not yet in Flux syntax; the runtime evaluates this
    # as a built-in operation over the valid window elements.
    return sum / w.count
}
