# =============================================================================
# Advanced Decorator Showcase — Adaptive Market Making Strategy
# =============================================================================
#
# Demonstrates Flux's decorator system with a realistic HFT market maker.
# Each struct uses a different memory layout decorator — see the type module
# files in types/ for detailed documentation on each decorator.
#
# This file is the executable strategy. Type definitions are kept here
# (inline) since the module resolver currently handles stdlib imports.
# The types/ directory contains the same structs as standalone reference files.
#
# Module imports from the standard library:
from market::l1 import {Quote, calc_spread, calc_mid}
from market::l2 import {Book, book_spread_bps, book_microprice, book_imbalance}

# Project-local module imports:
from types::market_state import {MarketState, SignalVector, build_market_state}
from types::config import {StrategyConfig, SessionStats}
from types::orders import {LiveOrder, OrderFlags, TradeRecord}

# =============================================================================
# Additional decorated types (not in local modules — defined inline)
# =============================================================================

# @volatile: Reads/writes cannot be reordered — for shared-memory feeds
@volatile
struct SharedFeedState {
    bid: f64,
    ask: f64,
    last_price: f64,
    sequence: int
}

# @simd(256): AVX2-aligned (32 bytes) for vectorized price math
@simd(256)
struct PriceVector {
    p0: f64,
    p1: f64,
    p2: f64,
    p3: f64
}

# @soa: Struct-of-arrays transform — enables SIMD over per-field arrays
@soa
struct TickFeature {
    price_delta: f64,
    volume_ratio: f64,
    spread_normalized: f64,
    imbalance_score: f64
}

# @streaming: Non-temporal stores — write-once data bypasses cache
@streaming
struct FillLog {
    timestamp: f64,
    price: f64,
    size: f64,
    side: int,
    order_id: int
}

# @heap: Box<T> allocation — for large structs that exceed stack frame
@heap
struct LargeBuffer {
    prices: [f64; 256],
    volumes: [f64; 256],
    count: int,
    capacity: int
}

# @stack: Explicit stack allocation with Copy semantics (the default)
@stack
struct QuoteUpdate {
    bid: f64,
    ask: f64,
    bid_size: f64,
    ask_size: f64
}

# =============================================================================
# Strategy Logic
# =============================================================================

fn compute_fair_value(mkt: MarketState, config: StrategyConfig) -> f64 {
    skew = mkt.imbalance * config.skew_factor
    return mkt.microprice + skew
}

fn should_quote(mkt: MarketState, config: StrategyConfig) -> int {
    if mkt.spread_bps > config.max_spread_bps {
        return 0
    }
    if mkt.imbalance > config.min_imbalance {
        return 1
    }
    if mkt.imbalance < 0.0 - config.min_imbalance {
        return -1
    }
    return 0
}

fn compute_quote_size(config: StrategyConfig, direction: int) -> f64 {
    if direction > 0 {
        return config.position_limit * 0.1
    }
    if direction < 0 {
        return config.position_limit * 0.1
    }
    return 0.0
}

strategy AdaptiveMarketMaker {
    params {
        max_spread = 5.0
        min_imbalance_threshold = 0.1
        position_cap = 1000.0
        skew_mult = 0.5
        exit_after = 10
    }

    state {
        bars_held = 0
        last_direction = 0
    }

    on bar {
        # Build market state from bar data
        mkt = MarketState {
            mid_price = close,
            spread_bps = 2.0,
            imbalance = (close - open) / close,
            microprice = (close + open) / 2.0,
            last_trade_side = 1,
            tick_count = 0
        }

        # Frozen config — @immutable prevents modification after this point
        config = StrategyConfig {
            max_spread_bps = max_spread,
            min_imbalance = min_imbalance_threshold,
            position_limit = position_cap,
            skew_factor = skew_mult,
            fade_ticks = exit_after
        }

        # Compute signals
        fair = compute_fair_value(mkt, config)
        direction = should_quote(mkt, config)
        size = compute_quote_size(config, direction)

        # Entry
        if direction > 0 and size > 0.0 and not in_position {
            OPEN(symbol, size)
            bars_held = 0
            last_direction = direction
        }

        # Exit — fade after N ticks
        if in_position {
            bars_held = bars_held + 1
            if bars_held >= exit_after {
                CLOSE(symbol)
                bars_held = 0
            }
        }
    }
}
