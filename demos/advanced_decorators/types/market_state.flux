# =============================================================================
# Market State Types — Cache-Optimized Hot-Path Data
# =============================================================================

# @aligned(64) — Cache-Line Alignment
# Forces the struct to start at a 64-byte boundary (one full cache line).
# Prevents false sharing when multiple threads access different MarketState
# instances that might otherwise land on the same cache line.
# The argument must be a power of 2 between 1 and 4096.

@aligned(64)
struct MarketState {
    mid_price: f64,
    spread_bps: f64,
    imbalance: f64,
    microprice: f64,
    last_trade_side: int,
    tick_count: int
}

# @volatile — Prevent Compiler Reordering
# All reads/writes use volatile semantics. The compiler cannot cache field
# values in registers or reorder access across fields.
# Use for shared-memory feeds written by an external process (e.g., kernel-
# bypass NIC → shared memory → strategy). Guarantees fresh values every read.

@volatile
struct SharedFeedState {
    bid: f64,
    ask: f64,
    last_price: f64,
    sequence: int
}

# @prefetch — CPU Cache Prefetch Hints
# Inserts prefetch intrinsics before accessing the struct in hot loops.
# The CPU starts loading it into L1 cache ahead of time, eliminating
# cache-miss stalls on the critical path.

@prefetch
struct SignalVector {
    bid_pressure: f64,
    ask_pressure: f64,
    momentum: f64,
    mean_reversion: f64,
    volatility: f64
}

fn build_market_state(mid: f64, spread: f64, imb: f64, micro: f64) -> MarketState {
    return MarketState {
        mid_price = mid,
        spread_bps = spread,
        imbalance = imb,
        microprice = micro,
        last_trade_side = 0,
        tick_count = 0
    }
}
