# std/engine/book.flux — Reusable order book with price-level FIFO matching
#
# Provides an OrderBook implementation with price levels and FIFO matching
# logic, used by Level 1 (Synthetic) and Level 2 (Replay) engines.
# Supports market order matching (buy/sell), limit order insertion with
# price-time priority, and VWAP computation across N levels.

from engine::types import {Order, OrderSide, Fill, FillResult, OrderType, TimeInForce}

struct PriceLevel {
    price: f64,
    total_size: f64,
    orders: list
}

struct OrderBook {
    bids: list,
    asks: list,
    symbol: str
}

impl OrderBook {
    fn new(sym: str) -> OrderBook {
        return OrderBook { bids = [], asks = [], symbol = sym }
    }

    # Match a market buy against ask levels (lowest first).
    # Returns FillResult with VWAP across consumed levels.
    fn match_buy(self, order: Order, timestamp: f64) -> FillResult {
        remaining = order.qty
        filled_qty = 0.0
        cost = 0.0

        # Capture best ask price before matching for slippage calculation
        best_ask = 0.0
        if self.asks.len() > 0 {
            best_ask = self.asks[0].price
        }

        i = 0
        while i < self.asks.len() and remaining > 0.0 {
            level = self.asks[i]
            j = 0
            while j < level.orders.len() and remaining > 0.0 {
                resting = level.orders[j]
                take = min(resting.qty, remaining)
                cost = cost + level.price * take
                filled_qty = filled_qty + take
                remaining = remaining - take

                if take >= resting.qty {
                    level.orders.remove(j)
                    # j stays same after remove (next element shifts down)
                } else {
                    # Partial consume: reduce resting order qty
                    reduced = Order {
                        id = resting.id, symbol = resting.symbol,
                        side = resting.side, order_type = resting.order_type,
                        qty = resting.qty - take, tif = resting.tif
                    }
                    level.orders[j] = reduced
                    j = j + 1
                }
            }
            # Update level total_size
            level_size = 0.0
            for o in level.orders {
                level_size = level_size + o.qty
            }
            self.asks[i] = PriceLevel {
                price = level.price,
                total_size = level_size,
                orders = level.orders
            }
            if level_size == 0.0 {
                self.asks.remove(i)
            } else {
                i = i + 1
            }
        }

        if filled_qty == 0.0 {
            return FillResult.Rejected("no liquidity")
        }

        vwap = cost / filled_qty
        slippage = vwap - best_ask

        fill = Fill {
            order_id = order.id, symbol = order.symbol,
            side = OrderSide.Buy, price = vwap,
            qty = filled_qty, timestamp = timestamp,
            slippage = slippage
        }

        if remaining > 0.0 {
            return FillResult.PartialFill(fill, remaining)
        }
        return FillResult.Filled(fill)
    }

    # Match a market sell against bid levels (highest first).
    # Returns FillResult with VWAP across consumed levels.
    fn match_sell(self, order: Order, timestamp: f64) -> FillResult {
        remaining = order.qty
        filled_qty = 0.0
        cost = 0.0

        # Capture best bid price before matching for slippage calculation
        best_bid = 0.0
        if self.bids.len() > 0 {
            best_bid = self.bids[0].price
        }

        # Bids are sorted descending (highest first)
        i = 0
        while i < self.bids.len() and remaining > 0.0 {
            level = self.bids[i]
            j = 0
            while j < level.orders.len() and remaining > 0.0 {
                resting = level.orders[j]
                take = min(resting.qty, remaining)
                cost = cost + level.price * take
                filled_qty = filled_qty + take
                remaining = remaining - take

                if take >= resting.qty {
                    level.orders.remove(j)
                } else {
                    reduced = Order {
                        id = resting.id, symbol = resting.symbol,
                        side = resting.side, order_type = resting.order_type,
                        qty = resting.qty - take, tif = resting.tif
                    }
                    level.orders[j] = reduced
                    j = j + 1
                }
            }
            level_size = 0.0
            for o in level.orders {
                level_size = level_size + o.qty
            }
            self.bids[i] = PriceLevel {
                price = level.price,
                total_size = level_size,
                orders = level.orders
            }
            if level_size == 0.0 {
                self.bids.remove(i)
            } else {
                i = i + 1
            }
        }

        if filled_qty == 0.0 {
            return FillResult.Rejected("no liquidity")
        }

        vwap = cost / filled_qty
        slippage = best_bid - vwap

        fill = Fill {
            order_id = order.id, symbol = order.symbol,
            side = OrderSide.Sell, price = vwap,
            qty = filled_qty, timestamp = timestamp,
            slippage = slippage
        }

        if remaining > 0.0 {
            return FillResult.PartialFill(fill, remaining)
        }
        return FillResult.Filled(fill)
    }

    # Insert a limit order into the appropriate side, maintaining price-time priority.
    fn insert_limit(self, order: Order, price: f64) -> OrderBook {
        match order.side {
            OrderSide.Buy => {
                # Insert into bids (sorted descending by price)
                inserted = false
                i = 0
                while i < self.bids.len() and not inserted {
                    if self.bids[i].price == price {
                        self.bids[i].orders.push(order)
                        self.bids[i] = PriceLevel {
                            price = price,
                            total_size = self.bids[i].total_size + order.qty,
                            orders = self.bids[i].orders
                        }
                        inserted = true
                    } elif self.bids[i].price < price {
                        new_level = PriceLevel {
                            price = price, total_size = order.qty,
                            orders = [order]
                        }
                        self.bids.insert(i, new_level)
                        inserted = true
                    }
                    i = i + 1
                }
                if not inserted {
                    new_level = PriceLevel {
                        price = price, total_size = order.qty,
                        orders = [order]
                    }
                    self.bids.push(new_level)
                }
            }
            OrderSide.Sell => {
                # Insert into asks (sorted ascending by price)
                inserted = false
                i = 0
                while i < self.asks.len() and not inserted {
                    if self.asks[i].price == price {
                        self.asks[i].orders.push(order)
                        self.asks[i] = PriceLevel {
                            price = price,
                            total_size = self.asks[i].total_size + order.qty,
                            orders = self.asks[i].orders
                        }
                        inserted = true
                    } elif self.asks[i].price > price {
                        new_level = PriceLevel {
                            price = price, total_size = order.qty,
                            orders = [order]
                        }
                        self.asks.insert(i, new_level)
                        inserted = true
                    }
                    i = i + 1
                }
                if not inserted {
                    new_level = PriceLevel {
                        price = price, total_size = order.qty,
                        orders = [order]
                    }
                    self.asks.push(new_level)
                }
            }
        }
        return self
    }

    # VWAP computation across N levels on a given side.
    fn vwap(self, side: OrderSide, levels: int) -> f64 {
        total_cost = 0.0
        total_qty = 0.0
        match side {
            OrderSide.Buy => {
                n = min(levels, self.bids.len())
                i = 0
                while i < n {
                    level = self.bids[i]
                    total_cost = total_cost + level.price * level.total_size
                    total_qty = total_qty + level.total_size
                    i = i + 1
                }
            }
            OrderSide.Sell => {
                n = min(levels, self.asks.len())
                i = 0
                while i < n {
                    level = self.asks[i]
                    total_cost = total_cost + level.price * level.total_size
                    total_qty = total_qty + level.total_size
                    i = i + 1
                }
            }
        }
        if total_qty == 0.0 {
            return 0.0
        }
        return total_cost / total_qty
    }
}
