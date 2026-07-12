from engine::types import {Order, Fill, OrderSide, OrderType, TimeInForce, BacktestEngine, PositionState, FillResult}
from engine::replay import {ReplayEngine, L2Event, L2Action, process_l2_event, QueuedOrder, get_queue_ahead, advance_queues, check_queue_fills, update_replay_position, trim_book}
from engine::book import {OrderBook, PriceLevel}
from market::l1 import {Bar}

strategy Test {
    state { x = 0 }
    on bar {
        engine = ReplayEngine.new()
        engine = process_l2_event(engine, L2Event {
            timestamp = 1.0, side = OrderSide.Buy,
            price = 100.0, size = 50.0,
            action = L2Action.Add
        })
        x = 1
    }
}
