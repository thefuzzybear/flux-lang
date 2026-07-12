# =============================================================================
# Order Book Simulator — Structured Data Modeling Demo
# =============================================================================
#
# Demonstrates structs, impl blocks, traits, and enums for data modeling:
#   1. Enums — FillResult with Filled/PartialFill/Rejected variants
#   2. Match expressions — Multi-field destructuring on FillResult
#   3. Structs + Impl blocks — PriceLevel and OrderBook with methods
#   4. Traits — OrderType interface for LimitOrder and MarketOrder
#   5. HashMap — Multi-symbol position tracking
#
# Trading Logic:
#   Builds a simulated order book each bar, checks if limit/market orders
#   would fill at current prices, and manages positions based on fill results.

from indicators import {sma}

# ---------------------------------------------------------------------------
# TYPE SYSTEM FEATURE: Struct + Impl block
# PriceLevel represents a single price point in the order book with
# quantity and order count. Methods compute derived values.
# ---------------------------------------------------------------------------
struct PriceLevel {
    price: f64,
    quantity: f64,
    order_count: int
}

impl PriceLevel {
    fn new(price: f64, qty: f64, orders: int) -> PriceLevel {
        return PriceLevel {
            price = price,
            quantity = qty,
            order_count = orders
        }
    }

    fn total_value(self) -> f64 {
        return self.price * self.quantity
    }
}

# ---------------------------------------------------------------------------
# TYPE SYSTEM FEATURE: Struct + Impl block
# OrderBook holds the best bid and ask levels and provides methods
# to compute spread, mid price, and fill simulation.
# ---------------------------------------------------------------------------
struct OrderBook {
    best_bid: PriceLevel,
    best_ask: PriceLevel
}

impl OrderBook {
    fn spread(self) -> f64 {
        return self.best_ask.price - self.best_bid.price
    }

    fn mid_price(self) -> f64 {
        return (self.best_bid.price + self.best_ask.price) / 2.0
    }

    fn is_crossed(self) -> bool {
        return self.best_bid.price >= self.best_ask.price
    }
}

# ---------------------------------------------------------------------------
# TYPE SYSTEM FEATURE: Enum definition
# FillResult models the three possible outcomes of attempting to fill
# an order. Filled and PartialFill carry associated price/quantity data.
# Rejected carries a reason string. This demonstrates data variants
# with multiple fields.
# ---------------------------------------------------------------------------
enum FillResult {
    Filled(price: f64, quantity: f64),
    PartialFill(price: f64, filled_qty: f64, remaining_qty: f64),
    Rejected(reason: str)
}

# ---------------------------------------------------------------------------
# TYPE SYSTEM FEATURE: Trait definition and implementation
# OrderType defines a common interface for different order types.
# Both LimitOrder and MarketOrder implement can_fill and fill_price,
# enabling uniform handling regardless of the concrete order type.
# ---------------------------------------------------------------------------
trait OrderType {
    fn can_fill(self, market_price: f64) -> bool
    fn fill_price(self, market_price: f64) -> f64
}

struct LimitOrder {
    limit_price: f64,
    quantity: f64
}

impl OrderType for LimitOrder {
    fn can_fill(self, market_price: f64) -> bool {
        # Buy limit fills if market is at or below limit price
        return market_price <= self.limit_price
    }

    fn fill_price(self, market_price: f64) -> f64 {
        return self.limit_price
    }
}

struct MarketOrder {
    quantity: f64
}

impl OrderType for MarketOrder {
    fn can_fill(self, market_price: f64) -> bool {
        # Market orders always fill
        return true
    }

    fn fill_price(self, market_price: f64) -> f64 {
        return market_price
    }
}

fn try_fill_order(book: OrderBook, order_qty: f64, available_qty: f64) -> FillResult {
    if available_qty <= 0.0 {
        return FillResult.Rejected("no liquidity")
    }
    if order_qty <= available_qty {
        return FillResult.Filled(book.mid_price(), order_qty)
    }
    return FillResult.PartialFill(
        book.mid_price(),
        available_qty,
        order_qty - available_qty
    )
}

data {
    symbols = ["AAPL", "GOOG"]
    period = "1y"
    interval = "1d"
    source = "yahoo"
}

strategy OrderBookStrategy {
    params {
        spread_mult = 0.001
        order_size = 100.0
        sma_period = 20
    }

    state {
        bar_count = 0
    }

    on bar {
        bar_count = bar_count + 1

        # Build simulated order book from bar data
        half_spread = close * spread_mult
        bid = PriceLevel.new(close - half_spread, 500.0, 3)
        ask = PriceLevel.new(close + half_spread, 500.0, 4)
        book = OrderBook { best_bid = bid, best_ask = ask }

        # Use impl methods via dot syntax
        spread = book.spread()
        mid = book.mid_price()

        # -------------------------------------------------------------------
        # TYPE SYSTEM FEATURE: HashMap
        # Track positions across multiple symbols using a HashMap.
        # Demonstrates: new(), insert(), get(), contains_key().
        # -------------------------------------------------------------------
        positions = HashMap.new()

        if not positions.contains_key(symbol) {
            positions.insert(symbol, 0.0)
        }

        # Decide whether to submit a limit order
        avg = sma(close, sma_period)
        if bar_count > sma_period and close < avg {
            # Create a limit order at the bid
            limit = LimitOrder { limit_price = bid.price, quantity = order_size }

            if limit.can_fill(bid.price) {
                result = try_fill_order(book, order_size, bid.quantity)

                # -------------------------------------------------------
                # TYPE SYSTEM FEATURE: Match expression
                # Destructures FillResult variants, binding multiple
                # variables from a single pattern. The Filled arm binds
                # both `price` and `quantity`; PartialFill binds three.
                # -------------------------------------------------------
                match result {
                    FillResult.Filled(price, quantity) => {
                        if not in_position {
                            OPEN(symbol, quantity)
                        }
                        positions.insert(symbol, quantity)
                    }
                    FillResult.PartialFill(price, filled_qty, remaining_qty) => {
                        if not in_position {
                            OPEN(symbol, filled_qty)
                        }
                        positions.insert(symbol, filled_qty)
                    }
                    FillResult.Rejected(reason) => {
                        # Order rejected — no action
                    }
                }
            }
        }

        # Exit on mean reversion above SMA
        if bar_count > sma_period and close > avg and in_position {
            CLOSE(symbol)
            positions.insert(symbol, 0.0)
        }
    }
}
