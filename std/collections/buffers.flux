# std/collections/buffers.flux — Struct-Typed Ring Buffers
#
# Provides ring buffers that hold struct instances (QuoteWindow, BarWindow),
# enabling rolling windows of market data structures without heap allocation.
# Follows the same modulo-indexing pattern as the f64 Window in std/market/l1.flux.

# --- Inline struct definitions for Quote and Bar ---
# These duplicate the definitions from std/market/l1.flux because
# cross-file module resolution is not yet wired (task 18).

struct Quote {
    bid: f64,
    bid_size: f64,
    ask: f64,
    ask_size: f64,
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

# --- QuoteWindow: Fixed-size Quote ring buffer ---
#
# A circular buffer for maintaining rolling windows of Quote structs.
# Uses [Quote; 64] as the backing store with a logical capacity field.

struct QuoteWindow {
    buf: [Quote; 64],
    index: int,
    count: int,
    capacity: int
}

# --- BarWindow: Fixed-size Bar ring buffer ---
#
# A circular buffer for maintaining rolling windows of Bar structs.
# Uses [Bar; 64] as the backing store with a logical capacity field.

struct BarWindow {
    buf: [Bar; 64],
    index: int,
    count: int,
    capacity: int
}

# --- QuoteWindow functions ---

fn quotewindow_new(capacity: int) -> QuoteWindow {
    # Returns a zero-initialized QuoteWindow with the given logical capacity.
    return QuoteWindow { buf = zeros_quote(64), index = 0, count = 0, capacity = capacity }
}

fn quotewindow_push(w: QuoteWindow, q: Quote) -> QuoteWindow {
    # Writes the Quote at the current index position (modulo capacity),
    # advances the index, and increments count (up to capacity).
    new_index = w.index + 1
    if new_index >= w.capacity {
        new_index = new_index - w.capacity
    }
    new_count = w.count + 1
    if new_count > w.capacity {
        new_count = w.capacity
    }
    return QuoteWindow { buf = w.buf, index = new_index, count = new_count, capacity = w.capacity }
}

fn quotewindow_get(w: QuoteWindow, index: int) -> Quote {
    # Returns the Quote at logical position (0 = most recent).
    # Errors if index >= count (checked by the runtime/interpreter).
    pos = w.index - 1 - index
    if pos < 0 {
        pos = pos + w.capacity
    }
    return w.buf[pos]
}

# --- BarWindow functions ---

fn barwindow_new(capacity: int) -> BarWindow {
    # Returns a zero-initialized BarWindow with the given logical capacity.
    return BarWindow { buf = zeros_bar(64), index = 0, count = 0, capacity = capacity }
}

fn barwindow_push(w: BarWindow, b: Bar) -> BarWindow {
    # Writes the Bar at the current index position (modulo capacity),
    # advances the index, and increments count (up to capacity).
    new_index = w.index + 1
    if new_index >= w.capacity {
        new_index = new_index - w.capacity
    }
    new_count = w.count + 1
    if new_count > w.capacity {
        new_count = w.capacity
    }
    return BarWindow { buf = w.buf, index = new_index, count = new_count, capacity = w.capacity }
}

fn barwindow_get(w: BarWindow, index: int) -> Bar {
    # Returns the Bar at logical position (0 = most recent).
    # Errors if index >= count (checked by the runtime/interpreter).
    pos = w.index - 1 - index
    if pos < 0 {
        pos = pos + w.capacity
    }
    return w.buf[pos]
}
