# Struct Showcase — Spread-Based Strategy with Stdlib Structs

A demonstration of Flux's struct system applied to a spread-based trading strategy.
Imports `Quote` from the standard library (`market::l1`) and uses helper functions
that accept and return struct values.

## What This Demonstrates

- **Stdlib struct imports** — `from market::l1 import {Quote, calc_spread, calc_mid}`
- **User-defined structs** — `TradeSignal` with typed fields
- **Struct literal construction** — `Quote { bid = ..., ask = ..., ... }`
- **Field access** — `signal.score`, `signal.direction`, `q.bid`
- **Structs as function parameters and return types** — `fn build_quote(...) -> Quote`
- **Struct-based strategy logic** — decisions driven by struct field values

## Project Structure

```
demos/struct_showcase/
├── strategy.flux    # Main strategy with struct usage
└── README.md        # This file
```

## Strategy Logic

1. Each bar, construct a `Quote` struct simulating top-of-book data from the bar's close price
2. Pass the `Quote` to `evaluate_spread()` which computes spread and mid using stdlib helpers
3. If the spread is tight (below `max_spread`), generate a `TradeSignal` with a score and size
4. Enter when the signal score exceeds 0.5; exit after holding for `exit_bars` bars

## Stdlib Usage

The strategy imports from `market::l1`:

| Import | Type | Purpose |
|--------|------|---------|
| `Quote` | struct | Represents best bid/offer with sizes and timestamp |
| `calc_spread` | function | Returns `ask - bid` from a Quote |
| `calc_mid` | function | Returns `(bid + ask) / 2.0` from a Quote |

## Running

```bash
# Type-check the strategy
cargo run -p flux-cli -- check demos/struct_showcase/strategy.flux

# Backtest with sample data (use any OHLCV CSV)
cargo run -p flux-cli -- backtest demos/struct_showcase/strategy.flux \
  --data demos/hello_world/sample_data.csv \
  --capital 10000

# Build to Rust source code
cargo run -p flux-cli -- build demos/struct_showcase/strategy.flux
```

## Key Patterns

### Importing Stdlib Structs

```flux
from market::l1 import {Quote, calc_spread, calc_mid}
```

The `market::l1` module provides L1 (top-of-book) market data types. Import both
the struct type and its associated helper functions together.

### Constructing Structs

```flux
q = Quote {
    bid = close - 0.25,
    bid_size = 500.0,
    ask = close + 0.25,
    ask_size = 500.0,
    timestamp = 0.0
}
```

All fields must be provided. Field order doesn't matter.

### Passing Structs to Functions

```flux
fn evaluate_spread(q: Quote, max_spread: f64, base_size: f64) -> TradeSignal {
    spread = calc_spread(q)  # Pass struct to stdlib function
    # ...
    return TradeSignal { score = score, direction = direction, size = size }
}
```

Structs have value semantics — passing to a function copies the struct.

### Accessing Fields

```flux
if signal.direction > 0 and signal.score > 0.5 {
    OPEN(symbol, signal.size)
}
```

Dot notation reads struct fields. The type checker verifies field names at compile time.
