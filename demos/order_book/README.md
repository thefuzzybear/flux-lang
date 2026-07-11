# Order Book Simulator — Structured Data Modeling Demo

Simulates a simplified order book using structs and impl blocks to model price levels, books, and order types. Orders are processed through a trait-based interface (`OrderType`) with limit and market order implementations. Fill outcomes are represented as a multi-field enum (`FillResult`), and match expressions destructure the results to drive position management across multiple symbols via HashMap.

## Features Demonstrated

| Feature | Description | Lines |
|---------|-------------|-------|
| Struct + Impl | `PriceLevel` struct with `new()` constructor and `total_value()` method | L23–L42 |
| Struct + Impl | `OrderBook` struct with `spread()`, `mid_price()`, `is_crossed()` methods | L48–L65 |
| Enum (data variants) | `FillResult` with `Filled(price, quantity)`, `PartialFill(price, filled_qty, remaining_qty)`, `Rejected(reason)` | L74–L78 |
| Trait + Impls | `OrderType` trait with `LimitOrder` and `MarketOrder` implementations | L87–L121 |
| HashMap | Multi-symbol position tracking with `new()`, `insert()`, `contains_key()` | L169–L175 |
| Match (multi-field destructuring) | Destructures `FillResult` variants binding 2–3 fields per arm | L191–L205 |

## Project Structure

```
demos/order_book/
├── strategy.flux    # Order book simulation with struct/enum modeling
├── data.csv         # 120 rows, 2 symbols (AAPL + GOOG)
└── README.md
```

## Running

```bash
# Type-check
cargo run -p flux-cli -- check demos/order_book/strategy.flux

# Backtest
cargo run -p flux-cli -- backtest demos/order_book/strategy.flux \
  --data demos/order_book/data.csv --capital 100000
```

## Code Walkthrough

### Struct Modeling: PriceLevel and OrderBook

The demo builds a domain model using nested structs. `PriceLevel` represents a single price point with quantity and order count:

```flux
struct PriceLevel {
    price: f64,
    quantity: f64,
    order_count: int
}
```

Its `impl` block provides a static constructor (`new`) and an instance method (`total_value`). The `OrderBook` struct composes two `PriceLevel` fields and adds derived computations — `spread()`, `mid_price()`, and `is_crossed()` — showing how impl methods enable encapsulation on structured data.

In the strategy body, the book is constructed each bar from market data and accessed via dot-syntax method calls:

```flux
bid = PriceLevel.new(close - half_spread, 500.0, 3)
ask = PriceLevel.new(close + half_spread, 500.0, 4)
book = OrderBook { best_bid = bid, best_ask = ask }
spread = book.spread()
```

### Enum with Multi-Field Data Variants

`FillResult` demonstrates enums where variants carry different amounts of associated data:

```flux
enum FillResult {
    Filled(price: f64, quantity: f64),
    PartialFill(price: f64, filled_qty: f64, remaining_qty: f64),
    Rejected(reason: str)
}
```

`Filled` carries two fields, `PartialFill` carries three, and `Rejected` carries a string reason. This models the real-world outcomes of order execution where each result type has different relevant data.

### Match Expression with Multi-Field Destructuring

The match expression destructures each variant and binds all associated fields in a single pattern:

```flux
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
```

Each arm binds a different number of variables (2, 3, and 1 respectively), demonstrating that pattern matching handles heterogeneous variant shapes naturally.

### Trait-Based Order Types

The `OrderType` trait defines a uniform interface for any order:

```flux
trait OrderType {
    fn can_fill(self, market_price: f64) -> bool
    fn fill_price(self, market_price: f64) -> f64
}
```

`LimitOrder` fills only at or below the limit price. `MarketOrder` always fills at market. Both are used identically through the trait interface, enabling the strategy to swap order types without changing fill logic.

### HashMap for Position Tracking

Positions are tracked per symbol using a `HashMap`, demonstrating key-value lookups in a multi-asset context:

```flux
positions = HashMap.new()
if not positions.contains_key(symbol) {
    positions.insert(symbol, 0.0)
}
```

This pattern extends naturally to any number of symbols without code changes.
