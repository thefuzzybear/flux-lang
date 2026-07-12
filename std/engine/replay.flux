# std/engine/replay.flux — Level 2: L2 data replay with queue position modeling
#
# The highest-fidelity backtester engine: reconstructs a real order book from
# L2 market data events and models queue position for limit orders. Fills
# happen when market liquidity reaches the order's queue position. Market
# orders match immediately against the reconstructed book.

from engine::types import {
    Order, Fill, FillResult, OrderSide, OrderType, TimeInForce,
    PositionState, BacktestEngine
}
from engine::book import {OrderBook, PriceLevel}
from market::l1 import {Bar}

# --- Enums ---

enum L2Action { Add, Modify, Delete }

# --- Structs ---

struct L2Event {
    timestamp: f64,
    side: OrderSide,
    price: f64,
    size: f64,
    action: L2Action
}

struct QueuedOrder {
    order: Order,
    queue_position: f64,
    price_level: f64
}

struct ReplayEngine {
    books: HashMap,
    queued_orders: list,
    fills: list,
    positions: HashMap,
    last_timestamp: f64
}

impl ReplayEngine {
    fn new() -> ReplayEngine {
        return ReplayEngine {
            books = HashMap.new(),
            queued_orders = [],
            fills = [],
            positions = HashMap.new(),
            last_timestamp = 0.0
        }
    }
}

impl BacktestEngine for ReplayEngine {
    # Level 2 processes L2 events, not bars. This is a no-op stub.
    fn process_bar(self, bar: Bar) -> ReplayEngine {
        return self
    }

    fn submit_order(self, order: Order) -> ReplayEngine {
        match order.order_type {
            OrderType.Market => {
                # Match immediately against reconstructed book
                sym = order.symbol
                if self.books.contains_key(sym) {
                    book = self.books.get(sym)
                    result = FillResult.Rejected("no liquidity")
                    match order.side {
                        OrderSide.Buy => {
                            result = book.match_buy(order, self.last_timestamp)
                        }
                        OrderSide.Sell => {
                            result = book.match_sell(order, self.last_timestamp)
                        }
                    }
                    match result {
                        FillResult.Filled(fill) => {
                            self.fills.push(fill)
                            self = update_replay_position(self, fill)
                        }
                        FillResult.PartialFill(fill, rem) => {
                            self.fills.push(fill)
                            self = update_replay_position(self, fill)
                        }
                        FillResult.Rejected(reason) => {
                            # No liquidity available — order rejected
                        }
                    }
                    self.books.insert(sym, book)
                }
            }
            OrderType.Limit(price) => {
                # Queue the order with queue position tracking
                sym = order.symbol
                queue_pos = 0.0
                if self.books.contains_key(sym) {
                    book = self.books.get(sym)
                    queue_pos = get_queue_ahead(book, order.side, price)
                }
                queued = QueuedOrder {
                    order = order,
                    queue_position = queue_pos,
                    price_level = price
                }
                self.queued_orders.push(queued)
            }
            _ => {
                # Stop and StopLimit not yet supported in L2 replay
            }
        }
        return self
    }

    fn get_fills(self) -> list {
        return self.fills
    }

    fn get_positions(self) -> list {
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

# --- L2 Event Processing ---

# Get total resting quantity ahead of a new order at a price level
fn get_queue_ahead(book: OrderBook, side: OrderSide, price: f64) -> f64 {
    match side {
        OrderSide.Buy => {
            # For a buy limit, look at bid side
            i = 0
            while i < book.bids.len() {
                if book.bids[i].price == price {
                    return book.bids[i].total_size
                }
                i = i + 1
            }
        }
        OrderSide.Sell => {
            # For a sell limit, look at ask side
            i = 0
            while i < book.asks.len() {
                if book.asks[i].price == price {
                    return book.asks[i].total_size
                }
                i = i + 1
            }
        }
    }
    return 0.0
}

# Process a single L2 event: update the book and advance queue positions
fn process_l2_event(engine: ReplayEngine, event: L2Event) -> ReplayEngine {
    # Reject out-of-order events
    if event.timestamp < engine.last_timestamp {
        return engine
    }

    # L2 events operate on a default symbol (single-symbol replay)
    sym = "default"
    if not engine.books.contains_key(sym) {
        engine.books.insert(sym, OrderBook.new(sym))
    }
    book = engine.books.get(sym)

    match event.action {
        L2Action.Add => {
            # Add a new price level or update size at existing level
            match event.side {
                OrderSide.Buy => {
                    found = false
                    i = 0
                    while i < book.bids.len() {
                        if book.bids[i].price == event.price {
                            book.bids[i] = PriceLevel {
                                price = event.price,
                                total_size = event.size,
                                orders = book.bids[i].orders
                            }
                            found = true
                        }
                        i = i + 1
                    }
                    if not found {
                        new_level = PriceLevel {
                            price = event.price,
                            total_size = event.size,
                            orders = []
                        }
                        # Insert in sorted position (descending by price)
                        inserted = false
                        j = 0
                        while j < book.bids.len() and not inserted {
                            if book.bids[j].price < event.price {
                                book.bids.insert(j, new_level)
                                inserted = true
                            }
                            j = j + 1
                        }
                        if not inserted {
                            book.bids.push(new_level)
                        }
                    }
                }
                OrderSide.Sell => {
                    found = false
                    i = 0
                    while i < book.asks.len() {
                        if book.asks[i].price == event.price {
                            book.asks[i] = PriceLevel {
                                price = event.price,
                                total_size = event.size,
                                orders = book.asks[i].orders
                            }
                            found = true
                        }
                        i = i + 1
                    }
                    if not found {
                        new_level = PriceLevel {
                            price = event.price,
                            total_size = event.size,
                            orders = []
                        }
                        # Insert in sorted position (ascending by price)
                        inserted = false
                        j = 0
                        while j < book.asks.len() and not inserted {
                            if book.asks[j].price > event.price {
                                book.asks.insert(j, new_level)
                                inserted = true
                            }
                            j = j + 1
                        }
                        if not inserted {
                            book.asks.push(new_level)
                        }
                    }
                }
            }
        }
        L2Action.Modify => {
            # Update size at existing price level, advance queues if liquidity consumed
            match event.side {
                OrderSide.Buy => {
                    i = 0
                    while i < book.bids.len() {
                        if book.bids[i].price == event.price {
                            old_size = book.bids[i].total_size
                            consumed = old_size - event.size
                            book.bids[i] = PriceLevel {
                                price = event.price,
                                total_size = event.size,
                                orders = book.bids[i].orders
                            }
                            if consumed > 0.0 {
                                engine = advance_queues(engine, event.price, consumed)
                            }
                        }
                        i = i + 1
                    }
                }
                OrderSide.Sell => {
                    i = 0
                    while i < book.asks.len() {
                        if book.asks[i].price == event.price {
                            old_size = book.asks[i].total_size
                            consumed = old_size - event.size
                            book.asks[i] = PriceLevel {
                                price = event.price,
                                total_size = event.size,
                                orders = book.asks[i].orders
                            }
                            if consumed > 0.0 {
                                engine = advance_queues(engine, event.price, consumed)
                            }
                        }
                        i = i + 1
                    }
                }
            }
        }
        L2Action.Delete => {
            # Remove price level entirely, advance queues by total consumed
            match event.side {
                OrderSide.Buy => {
                    i = 0
                    while i < book.bids.len() {
                        if book.bids[i].price == event.price {
                            consumed = book.bids[i].total_size
                            book.bids.remove(i)
                            engine = advance_queues(engine, event.price, consumed)
                        } else {
                            i = i + 1
                        }
                    }
                }
                OrderSide.Sell => {
                    i = 0
                    while i < book.asks.len() {
                        if book.asks[i].price == event.price {
                            consumed = book.asks[i].total_size
                            book.asks.remove(i)
                            engine = advance_queues(engine, event.price, consumed)
                        } else {
                            i = i + 1
                        }
                    }
                }
            }
        }
    }

    # Trim book to max 20 levels per side
    book = trim_book(book)

    # Store updated book and timestamp
    engine.books.insert(sym, book)
    engine.last_timestamp = event.timestamp

    # Check if any queued orders can now fill
    engine = check_queue_fills(engine)

    return engine
}

# --- Queue Management ---

# Reduce queue positions for orders resting at a price level where liquidity was consumed
fn advance_queues(engine: ReplayEngine, price: f64, consumed: f64) -> ReplayEngine {
    i = 0
    while i < engine.queued_orders.len() {
        qo = engine.queued_orders[i]
        if qo.price_level == price and qo.queue_position > 0.0 {
            new_pos = qo.queue_position - consumed
            if new_pos < 0.0 {
                new_pos = 0.0
            }
            engine.queued_orders[i] = QueuedOrder {
                order = qo.order,
                queue_position = new_pos,
                price_level = qo.price_level
            }
        }
        i = i + 1
    }
    return engine
}

# Check if any queued orders have queue_position <= 0 and can fill
fn check_queue_fills(engine: ReplayEngine) -> ReplayEngine {
    remaining = []
    for qo in engine.queued_orders {
        if qo.queue_position <= 0.0 {
            # Order is at front of queue — fill at limit price
            fill = Fill {
                order_id = qo.order.id,
                symbol = qo.order.symbol,
                side = qo.order.side,
                price = qo.price_level,
                qty = qo.order.qty,
                timestamp = engine.last_timestamp,
                slippage = 0.0
            }
            engine.fills.push(fill)
            engine = update_replay_position(engine, fill)
        } else {
            remaining.push(qo)
        }
    }
    engine.queued_orders = remaining
    return engine
}

# --- Position Tracking ---

# Position update logic — volume-weighted avg entry on buy, realized P&L on sell
fn update_replay_position(engine: ReplayEngine, fill: Fill) -> ReplayEngine {
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
    return ReplayEngine {
        books = engine.books,
        queued_orders = engine.queued_orders,
        fills = engine.fills,
        positions = positions,
        last_timestamp = engine.last_timestamp
    }
}

# --- Book Trimming ---

# Trim book to maximum 20 levels per side
fn trim_book(book: OrderBook) -> OrderBook {
    # Trim bids (sorted descending — keep first 20)
    while book.bids.len() > 20 {
        book.bids.remove(book.bids.len() - 1)
    }
    # Trim asks (sorted ascending — keep first 20)
    while book.asks.len() > 20 {
        book.asks.remove(book.asks.len() - 1)
    }
    return book
}
