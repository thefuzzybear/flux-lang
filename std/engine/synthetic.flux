# std/engine/synthetic.flux — Level 1: Synthetic book from OHLCV price paths
#
# Generates a 4-point intra-bar price path from OHLCV data, constructs a
# synthetic order book at each price point, and matches pending orders against
# the simulated liquidity. Slippage emerges naturally from book consumption.
# Configurable depth, spread, and liquidity parameters model different
# market conditions.

from engine::types import {
    Order, Fill, FillResult, OrderSide, OrderType, TimeInForce,
    PositionState, BacktestEngine
}
from engine::book import {OrderBook, PriceLevel}
from market::l1 import {Bar}

# --- Configuration ---

struct SyntheticConfig {
    depth: int,
    spread_pct: f64,
    liquidity_per_side: f64
}

impl SyntheticConfig {
    fn default() -> SyntheticConfig {
        return SyntheticConfig {
            depth = 5,
            spread_pct = 0.1,
            liquidity_per_side = 10000.0
        }
    }
}

# --- Engine ---

struct SyntheticEngine {
    config: SyntheticConfig,
    pending_orders: list,
    fills: list,
    positions: HashMap,
    next_order_id: int
}

impl SyntheticEngine {
    fn new(config: SyntheticConfig) -> SyntheticEngine {
        return SyntheticEngine {
            config = config,
            pending_orders = [],
            fills = [],
            positions = HashMap.new(),
            next_order_id = 0
        }
    }
}

# --- BacktestEngine Trait Implementation ---

impl BacktestEngine for SyntheticEngine {
    fn process_bar(self, bar: Bar) -> SyntheticEngine {
        # Generate 4-point price path from OHLCV
        path = generate_price_path(bar)
        new_fills = []

        # Walk each price point, build synthetic book, match pending orders
        for price_point in path {
            book = build_synthetic_book(price_point, bar.symbol, self.config)

            # Match each pending order against the synthetic book
            remaining_orders = []
            for order in self.pending_orders {
                if order.symbol == bar.symbol {
                    match order.order_type {
                        OrderType.Market => {
                            match order.side {
                                OrderSide.Buy => {
                                    result = book.match_buy(order, bar.timestamp)
                                }
                                OrderSide.Sell => {
                                    result = book.match_sell(order, bar.timestamp)
                                }
                            }
                            match result {
                                FillResult.Filled(fill) => {
                                    new_fills.push(fill)
                                    self = update_synth_position(self, fill)
                                }
                                FillResult.PartialFill(fill, rem) => {
                                    new_fills.push(fill)
                                    self = update_synth_position(self, fill)
                                    # Keep remainder as pending with reduced qty
                                    partial_order = Order {
                                        id = order.id,
                                        symbol = order.symbol,
                                        side = order.side,
                                        order_type = order.order_type,
                                        qty = rem,
                                        tif = order.tif
                                    }
                                    remaining_orders.push(partial_order)
                                }
                                FillResult.Rejected(reason) => {
                                    remaining_orders.push(order)
                                }
                            }
                        }
                        _ => {
                            # Non-market orders retained for future processing
                            remaining_orders.push(order)
                        }
                    }
                } else {
                    # Orders for other symbols pass through unchanged
                    remaining_orders.push(order)
                }
            }
            self.pending_orders = remaining_orders
        }

        return SyntheticEngine {
            config = self.config,
            pending_orders = self.pending_orders,
            fills = new_fills,
            positions = self.positions,
            next_order_id = self.next_order_id
        }
    }

    fn submit_order(self, order: Order) -> SyntheticEngine {
        self.pending_orders.push(order)
        return SyntheticEngine {
            config = self.config,
            pending_orders = self.pending_orders,
            fills = self.fills,
            positions = self.positions,
            next_order_id = self.next_order_id
        }
    }

    fn get_fills(self) -> list {
        return self.fills
    }

    fn get_positions(self) -> list {
        # Return all non-zero positions from the HashMap
        result = []
        keys = self.positions.keys()
        for key in keys {
            pos = self.positions.get(key)
            if pos.qty != 0.0 {
                result.push(pos)
            }
        }
        return result
    }
}

# --- Price Path Generation ---

# Generate a 4-point intra-bar price path: Open → nearer extreme → far extreme → Close
# When equidistant from High and Low, prefer High first.
fn generate_price_path(bar: Bar) -> list {
    path = []
    path.push(bar.open)

    # Determine which extreme is nearer to open
    dist_to_high = abs(bar.high - bar.open)
    dist_to_low = abs(bar.open - bar.low)

    if dist_to_high <= dist_to_low {
        # High is nearer (or equidistant — prefer High first)
        path.push(bar.high)
        path.push(bar.low)
    } else {
        # Low is nearer
        path.push(bar.low)
        path.push(bar.high)
    }

    path.push(bar.close)
    return path
}

# --- Synthetic Book Construction ---

# Build a synthetic order book centered on a price point.
# Creates `depth` ask levels above and `depth` bid levels below the center,
# each offset by spread_step increments. Each level holds liquidity_per_side / depth qty.
# Uses negative IDs for synthetic resting orders.
fn build_synthetic_book(center_price: f64, symbol: str, config: SyntheticConfig) -> OrderBook {
    book = OrderBook.new(symbol)
    qty_per_level = config.liquidity_per_side / config.depth
    spread_step = center_price * config.spread_pct / 100.0

    # Build ask levels (ascending from center + spread_step)
    i = 0
    while i < config.depth {
        ask_price = center_price + spread_step * (i + 1)
        ask_order = Order {
            id = 0 - (i + 1),
            symbol = symbol,
            side = OrderSide.Sell,
            order_type = OrderType.Limit(ask_price),
            qty = qty_per_level,
            tif = TimeInForce.GTC
        }
        level = PriceLevel {
            price = ask_price,
            total_size = qty_per_level,
            orders = [ask_order]
        }
        book.asks.push(level)
        i = i + 1
    }

    # Build bid levels (descending from center - spread_step)
    i = 0
    while i < config.depth {
        bid_price = center_price - spread_step * (i + 1)
        bid_order = Order {
            id = 0 - (config.depth + i + 1),
            symbol = symbol,
            side = OrderSide.Buy,
            order_type = OrderType.Limit(bid_price),
            qty = qty_per_level,
            tif = TimeInForce.GTC
        }
        level = PriceLevel {
            price = bid_price,
            total_size = qty_per_level,
            orders = [bid_order]
        }
        book.bids.push(level)
        i = i + 1
    }

    return book
}

# --- Position Update Logic ---

# Update position state after a fill — same logic as fast.flux.
# Buy: volume-weighted average entry price.
# Sell: realized P&L = (fill_price - avg_entry) * fill_qty.
fn update_synth_position(engine: SyntheticEngine, fill: Fill) -> SyntheticEngine {
    positions = engine.positions
    match fill.side {
        OrderSide.Buy => {
            if positions.contains_key(fill.symbol) {
                pos = positions.get(fill.symbol)
                new_qty = pos.qty + fill.qty
                new_avg = (pos.avg_entry_price * pos.qty + fill.price * fill.qty) / new_qty
                positions.insert(fill.symbol, PositionState {
                    symbol = fill.symbol,
                    qty = new_qty,
                    avg_entry_price = new_avg,
                    unrealized_pnl = 0.0,
                    realized_pnl = pos.realized_pnl
                })
            } else {
                positions.insert(fill.symbol, PositionState {
                    symbol = fill.symbol,
                    qty = fill.qty,
                    avg_entry_price = fill.price,
                    unrealized_pnl = 0.0,
                    realized_pnl = 0.0
                })
            }
        }
        OrderSide.Sell => {
            if positions.contains_key(fill.symbol) {
                pos = positions.get(fill.symbol)
                realized = (fill.price - pos.avg_entry_price) * fill.qty
                new_qty = pos.qty - fill.qty
                positions.insert(fill.symbol, PositionState {
                    symbol = fill.symbol,
                    qty = new_qty,
                    avg_entry_price = pos.avg_entry_price,
                    unrealized_pnl = 0.0,
                    realized_pnl = pos.realized_pnl + realized
                })
            }
        }
    }
    return SyntheticEngine {
        config = engine.config,
        pending_orders = engine.pending_orders,
        fills = engine.fills,
        positions = positions,
        next_order_id = engine.next_order_id
    }
}
