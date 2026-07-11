# Pairs Trading — Type System Kitchen Sink Demo

A statistical arbitrage strategy that tracks the rolling spread between two correlated symbols (AAPL and MSFT), computes a z-score of the spread relative to its historical mean, and generates buy/sell signals when the spread deviates beyond a threshold. This demo showcases ALL six type system features together in a single, realistic strategy.

## Features Demonstrated

| Feature | Description | Lines |
|---------|-------------|-------|
| Enum | `Signal` with Buy(strength), Sell(strength), and Hold variants | L26–L30 |
| Struct + Impl | `PairState` struct with `new`, `calculate_zscore`, and `update` methods | L39–L75 |
| Trait | `SignalGenerator` interface with `ZScoreGenerator` and `MomentumGenerator` impls | L84–L121 |
| HashMap | Pair registry mapping symbol names to hedge ratios | L150–L152 |
| Match | Pattern matching on `Signal` variants with destructured `strength` binding | L178–L193 |

## Project Structure

```
demos/pairs_trading/
├── strategy.flux          # All 6 type system features in one strategy
├── data.csv               # 120 rows of AAPL + MSFT daily OHLCV data
└── README.md
```

## Running

```bash
# Type-check
cargo run -p flux-cli -- check demos/pairs_trading/strategy.flux

# Backtest
cargo run -p flux-cli -- backtest demos/pairs_trading/strategy.flux \
  --data demos/pairs_trading/data.csv --capital 100000
```

## Code Walkthrough

### Enum — Signal Type (L26–L30)

The `Signal` enum is a discriminated union with three variants. `Buy` and `Sell` are data variants carrying a `strength: f64` value that controls position sizing. `Hold` is a unit variant with no associated data. This pattern is common in trading systems for representing discrete action categories with optional metadata.

```flux
enum Signal {
    Buy(strength: f64),
    Sell(strength: f64),
    Hold
}
```

### Struct + Impl Block — PairState (L39–L75)

`PairState` groups the rolling spread statistics needed for z-score computation. The impl block attaches three methods: a static constructor `new(lookback)` that doesn't take `self`, and two instance methods `calculate_zscore` and `update` that operate on the struct's fields via `self`. This demonstrates both method kinds in one impl block.

### Trait + Implementations — SignalGenerator (L84–L121)

The `SignalGenerator` trait defines a single method `generate(self, z_score, threshold) -> Signal`. Two structs implement it with different logic:

- **ZScoreGenerator** — produces signals when the z-score crosses the threshold
- **MomentumGenerator** — adjusts the z-score by a momentum weight before comparing

This enables swapping signal generation logic without changing the calling code.

### HashMap — Pair Registry (L150–L152)

A `HashMap` stores the symbol-to-hedge-ratio mapping, demonstrating the key-value container API: `new()`, `insert()`, `get()`, and `contains_key()`. The strategy checks if the current bar's symbol exists in the registry before computing spread.

```flux
pair_registry = HashMap.new()
pair_registry.insert("AAPL", 1.0)
pair_registry.insert("MSFT", -0.85)
```

### Match Expression — Signal Routing (L178–L193)

The match expression destructures `Signal` variants and binds associated data. The `strength` variable is extracted from `Signal.Buy(strength)` and used to compute position size. The wildcard `_` arm catches `Signal.Hold` without binding any data.

```flux
match signal {
    Signal.Buy(strength) => {
        size = base_size * strength
        OPEN(symbol, size)
    }
    Signal.Sell(strength) => {
        CLOSE(symbol)
    }
    _ => {
        # Hold — do nothing
    }
}
```
