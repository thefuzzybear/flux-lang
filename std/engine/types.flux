# std/engine/types.flux — Shared types for all backtester engines
#
# Provides common type definitions used across all fidelity levels:
# order types, fill results, position state, and the BacktestEngine trait.

from market::l1 import {Bar}

# --- Enums ---

enum OrderSide { Buy, Sell }

enum OrderType {
    Market,
    Limit(price: f64),
    Stop(stop_price: f64),
    StopLimit(stop_price: f64, limit_price: f64)
}

enum TimeInForce { GTC, IOC, DAY }

enum FillResult {
    Filled(fill: Fill),
    PartialFill(fill: Fill, remaining_qty: f64),
    Rejected(reason: str)
}

# --- Structs ---

struct Order {
    id: int,
    symbol: str,
    side: OrderSide,
    order_type: OrderType,
    qty: f64,
    tif: TimeInForce
}

struct Fill {
    order_id: int,
    symbol: str,
    side: OrderSide,
    price: f64,
    qty: f64,
    timestamp: f64,
    slippage: f64
}

struct PositionState {
    symbol: str,
    qty: f64,
    avg_entry_price: f64,
    unrealized_pnl: f64,
    realized_pnl: f64
}

# --- BacktestEngine Trait ---

trait BacktestEngine {
    fn process_bar(self, bar: Bar) -> BacktestEngine
    fn submit_order(self, order: Order) -> BacktestEngine
    fn get_fills(self) -> list
    fn get_positions(self) -> list
}
