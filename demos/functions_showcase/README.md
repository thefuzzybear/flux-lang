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
- **Cross-file imports** using `::` module syntax (`from lib::signals import {...}`)
- **Library files** containing only `fn` definitions (no strategy block)
- **Multiple strategies** sharing similar function patterns

## Project Structure

```
demos/functions_showcase/
├── README.md
├── aapl_2024.csv              # AAPL daily price data
├── pair_data.csv              # AAPL+MSFT pair data
├── bollinger_breakout.flux    # Strategy using cross-file imports
├── adaptive_reversion.flux    # Self-contained strategy
├── pair_spread.flux           # Self-contained strategy
└── lib/
    └── signals.flux           # Shared signal detection library
```

## Multi-File Imports

The `bollinger_breakout.flux` strategy demonstrates Flux's cross-file import system.
Signal detection helpers (band calculations, volume surge, breakout/exit signals) live
in `lib/signals.flux`, while entry/exit execution logic stays in the main strategy file.

### Import Syntax

Use `::` as the path separator to import functions from other `.flux` files:

```flux
from lib::signals import {breakout_signal, exit_signal}
```

The path `lib::signals` maps to the file `lib/signals.flux` relative to the importing
file's directory. The `::` separator distinguishes file-module imports from built-in
imports (which use `.` or single-segment names like `from indicators import {sma}`).

### Library Files

A library file (like `lib/signals.flux`) contains only `fn` definitions and `import`
statements — no `strategy`, `data`, `state`, or `connector` blocks. This makes them
pure function collections that can be shared across multiple strategies.

```flux
# lib/signals.flux
from indicators import {sma}

fn upper_band(price, lookback, num_std) {
    avg = sma(price, lookback)
    vol = stddev(price, lookback)
    return avg + num_std * vol
}

fn breakout_signal(lookback, band_std, vol_lookback, vol_mult, min_width) {
    upper = upper_band(close, lookback, band_std)
    # ... breakout logic
}
```

### Selective Inclusion

Only the functions you explicitly import (plus their transitive dependencies) are
included in compilation. If `lib/signals.flux` has 6 functions but you only import
`breakout_signal`, the resolver automatically pulls in `upper_band`, `band_width_pct`,
and `volume_surge` (because `breakout_signal` calls them) but leaves `lower_band` and
`exit_signal` out.

## Strategies

### 1. Bollinger Breakout (`bollinger_breakout.flux`)

A volatility breakout strategy that uses **cross-file imports** to:
- Import signal detection helpers from `lib/signals.flux`
- Keep entry/exit execution logic local to the strategy
- Demonstrate the `from lib::signals import {...}` syntax

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
# Bollinger Breakout on AAPL (uses cross-file imports from lib/)
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

You can also type-check without running a backtest:

```bash
# Verify the multi-file project compiles correctly
cargo run -p flux-cli -- check demos/functions_showcase/bollinger_breakout.flux
```

## Key Patterns Shown

### Pattern 1: Cross-File Import

```flux
# Import shared helpers from a library file
from lib::signals import {breakout_signal, exit_signal}

# Use them just like locally-defined functions
fn try_enter(lookback, band_std, vol_lookback, vol_mult, min_width, size) {
    signal = breakout_signal(lookback, band_std, vol_lookback, vol_mult, min_width)
    if signal > 0.5 and not in_position {
        OPEN(symbol, size)
    }
}
```

### Pattern 2: Pure Computation Functions

```flux
fn band_width(price, lookback) {
    upper = sma(price, lookback) + 2.0 * stddev(price, lookback)
    lower = sma(price, lookback) - 2.0 * stddev(price, lookback)
    return (upper - lower) / sma(price, lookback)
}
```

These take parameters, compute a value, and return it. No side effects.

### Pattern 3: Signal-Emitting Functions

```flux
fn enter_long(size) {
    if not in_position {
        OPEN(symbol, size)
    }
}
```

These encapsulate entry/exit logic and emit signals directly.

### Pattern 4: Multi-Factor Scoring

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

### Pattern 5: Functions Calling Functions

```flux
fn should_enter(price, lookback, threshold) {
    score = compute_signal_strength(price, lookback)
    if score > threshold {
        return 1.0
    }
    return 0.0
}
```

Compose logic by chaining function calls — works across file boundaries too.

## Tuning

Each strategy's `params` block contains tunable values. Experiment with:
- Lookback periods (shorter = more responsive, noisier)
- Thresholds (higher = fewer but higher-conviction trades)
- Position sizes (larger = more P&L per trade, more risk)
