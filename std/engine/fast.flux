# std/engine/fast.flux — Level 0: Fill all orders at bar close price
#
# The simplest backtester engine: fills all pending market orders at bar.close
# with zero slippage. Equivalent to the existing Rust PositionTracker behavior.
# Ultra-fast for iteration — no order book, no intra-bar simulation.

from engine::types import {
    Order, Fill, FillResult, OrderSide, OrderType, TimeInForce,
    PositionState, BacktestEngine
}
from market::l1 import {Bar}

struct FastEngine {
    pending_orders: list,
    fills: list,
    positions: HashMap,
    next_fill_id: int
}

impl FastEngine {
    fn new() -> FastEngine {
        return FastEngine {
            pending_orders = [],
            fills = [],
            positions = HashMap.new(),
            next_fill_id = 0
        }
    }
}

impl BacktestEngine for FastEngine {
    fn process_bar(self, bar: Bar) -> FastEngine {
        new_fills = []

        # Process pending orders in submission order (deterministic)
        for order in self.pending_orders {
            if order.symbol == bar.symbol {
                match order.side {
                    OrderSide.Buy => {
                        # Buy orders always fill at bar.close
                        fill = Fill {
                            order_id = order.id,
                            symbol = order.symbol,
                            side = OrderSide.Buy,
                            price = bar.close,
                            qty = order.qty,
                            timestamp = bar.timestamp,
                            slippage = 0.0
                        }
                        new_fills.push(fill)
                        self = update_position(self, fill)
                    }
                    OrderSide.Sell => {
                        # Sell orders: check position exists and has sufficient qty
                        if self.positions.contains_key(order.symbol) {
                            pos = self.positions.get(order.symbol)
                            if pos.qty >= order.qty {
                                # Sufficient position — produce fill
                                fill = Fill {
                                    order_id = order.id,
                                    symbol = order.symbol,
                                    side = OrderSide.Sell,
                                    price = bar.close,
                                    qty = order.qty,
                                    timestamp = bar.timestamp,
                                    slippage = 0.0
                                }
                                new_fills.push(fill)
                                self = update_position(self, fill)
                            }
                            # Insufficient qty: discard silently (no fill produced)
                        }
                        # No position for symbol: discard silently (CLOSE on no-position)
                    }
                }
            }
        }

        # Remove filled orders, keep orders for other symbols
        remaining = []
        for order in self.pending_orders {
            if order.symbol != bar.symbol {
                remaining.push(order)
            }
        }

        return FastEngine {
            pending_orders = remaining,
            fills = new_fills,
            positions = self.positions,
            next_fill_id = self.next_fill_id
        }
    }

    fn submit_order(self, order: Order) -> FastEngine {
        self.pending_orders.push(order)
        return FastEngine {
            pending_orders = self.pending_orders,
            fills = self.fills,
            positions = self.positions,
            next_fill_id = self.next_fill_id
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

# Position update logic — volume-weighted avg entry on buy, realized P&L on sell
fn update_position(engine: FastEngine, fill: Fill) -> FastEngine {
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
    return FastEngine {
        pending_orders = engine.pending_orders,
        fills = engine.fills,
        positions = positions,
        next_fill_id = engine.next_fill_id
    }
}
