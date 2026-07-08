# Functions Showcase — Multi-Strategy with User-Defined Functions

A demonstration of Flux user-defined functions (`fn` declarations) applied to a
realistic multi-strategy trading setup. Each strategy uses functions to organize
logic into reusable, testable units.

## What This Demonstrates

- **User-defined functions** with parameters, return values, and signal emission
- **Functions calling other functions** (chained computation)
- **Functions accessing bar context** (close, open, high, low, volume, symbol)
- **Conditional logic inside functions** (if/else, early return)
- **Signal emission from functions** (OPEN/CLOSE inside helper functions)
- **Multiple strategies** sharing similar function patterns

## Strategies

### 1. Bollinger Breakout (`bollinger_breakout.flux`)

A volatility breakout strategy that uses functions to:
- Compute Bollinger Band width as a volatility gauge
- Detect band breakouts with directional bias
- Apply a volume confirmation filter
- Manage entries and exits through dedicated signal functions

### 2. Adaptive Mean Reversion (`adaptive_reversion.flux`)

An adaptive mean reversion strategy that uses functions to:
- Compute a regime filter (trending vs. mean-reverting market)
- Calculate dynamic position sizing based on conviction
- Apply multi-factor entry scoring (z-score + volume + spread)
- Implement tiered exit logic (profit target vs. stop loss vs. timeout)

### 3. Pair Spread (`pair_spread.flux`)

A pairs/spread strategy on two correlated symbols that uses functions to:
- Calculate the rolling spread ratio
- Score entry signals based on spread deviation
- Apply asymmetric exits (tighter stop, wider target)

## Running

Backtest each strategy independently:

```bash
# Bollinger Breakout on AAPL
cargo run -p flux-cli -- backtest demos/functions_showcase/bollinger_breakout.flux \
  --data demos/functions_showcase/aapl_2024.csv \
  --capital 50000

# Adaptive Mean Reversion on AAPL
cargo run -p flux-cli -- backtest demos/functions_showcase/adaptive_reversion.flux \
  --data demos/functions_showcase/aapl_2024.csv \
  --capital 50000

# Pair Spread on AAPL+MSFT
cargo run -p flux-cli -- backtest demos/functions_showcase/pair_spread.flux \
  --data demos/functions_showcase/pair_data.csv \
  --capital 50000
```

## Key Patterns Shown

### Pattern 1: Pure Computation Functions

```flux
fn band_width(price, lookback) {
    upper = sma(price, lookback) + 2.0 * stddev(price, lookback)
    lower = sma(price, lookback) - 2.0 * stddev(price, lookback)
    return (upper - lower) / sma(price, lookback)
}
```

These take parameters, compute a value, and return it. No side effects.

### Pattern 2: Signal-Emitting Functions

```flux
fn enter_long(size) {
    if not in_position {
        OPEN(symbol, size)
    }
}
```

These encapsulate entry/exit logic and emit signals directly.

### Pattern 3: Multi-Factor Scoring

```flux
fn entry_score(z, vol_ratio, spread_pct) {
    score = 0.0
    if z < 0.0 - 1.5 { score = score + 1.0 }
    if vol_ratio > 1.2 { score = score + 0.5 }
    if spread_pct > 0.5 { score = score + 0.5 }
    return score
}
```

Functions that combine multiple conditions into a single score.

### Pattern 4: Functions Calling Functions

```flux
fn should_enter(price, lookback, threshold) {
    score = compute_signal_strength(price, lookback)
    if score > threshold {
        return 1.0
    }
    return 0.0
}
```

Compose logic by chaining function calls.

## Tuning

Each strategy's `params` block contains tunable values. Experiment with:
- Lookback periods (shorter = more responsive, noisier)
- Thresholds (higher = fewer but higher-conviction trades)
- Position sizes (larger = more P&L per trade, more risk)

## Note on Multi-File

Flux currently compiles each `.flux` file independently (no cross-file imports of
user functions yet — that's Layer 2 of the module system). Each strategy file is
self-contained with its own function definitions. The shared patterns across files
demonstrate how functions make strategies more readable and maintainable.
