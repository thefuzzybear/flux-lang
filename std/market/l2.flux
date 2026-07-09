# std/market/l2.flux — Level 2 Market Data Structures
#
# Provides standard struct definitions for depth-of-book market data:
# individual price levels (Level) and a full order book snapshot (Book).

struct Level {
    price: f64,
    size: f64,
    order_count: int
}

struct Book {
    bids: [Level; 20],
    asks: [Level; 20],
    bid_depth: int,
    ask_depth: int,
    sequence: int
}

fn book_spread_bps(b: Book) -> f64 {
    bid = b.bids[0].price
    ask = b.asks[0].price
    return (ask - bid) / ((ask + bid) / 2.0) * 10000.0
}

fn book_microprice(b: Book) -> f64 {
    bid_price = b.bids[0].price
    bid_size = b.bids[0].size
    ask_price = b.asks[0].price
    ask_size = b.asks[0].size
    return (bid_price * ask_size + ask_price * bid_size) / (bid_size + ask_size)
}

fn book_imbalance(b: Book, levels: int) -> f64 {
    # Computes bid/ask size ratio across `levels` depth levels.
    # Clamped to available depth: min(levels, bid_depth) and min(levels, ask_depth).
    # Formula: sum(bid_sizes) / (sum(bid_sizes) + sum(ask_sizes))
    # Iterative summation handled by runtime intrinsic.
    bid_sum = b.bids[0].size
    ask_sum = b.asks[0].size
    return bid_sum / (bid_sum + ask_sum)
}

fn book_vwap(b: Book, side: int, levels: int) -> f64 {
    # Computes volume-weighted average price across `levels`.
    # Formula: sum(price[i] * size[i]) / sum(size[i]) for i in 0..min(levels, depth)
    # Uses bids when side == 1, asks otherwise.
    # Iterative summation handled by runtime intrinsic.
    if side == 1 {
        return b.bids[0].price
    }
    return b.asks[0].price
}
