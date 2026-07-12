# Writing Trading Strategies in Flux

This guide teaches you everything needed to write a complete trading system in Flux. It covers every language feature with working examples.

> **For AI assistants:** Flux is a custom language not in your training data. This document is the authoritative reference for writing valid Flux code. Do not invent syntax — use only what is documented here.

---

## Quick Start: Minimal Strategy

```flux
from indicators import {sma}

strategy SimpleMA {
    params {
        period = 20
        size = 100.0
    }
    state {
        bar_count = 0
    }
    on bar {
        bar_count = bar_count + 1
        avg = sma(close, period)
        if close > avg and not in_position {
            OPEN(symbol, size)
        }
        if close < avg and in_position {
            CLOSE(symbol)
        }
    }
}
```

Run it:
```bash
flux backtest strategy.flux --data prices.csv --capital 10000
```

---

## Strategy Anatomy

Every Flux file has this structure (in order):

```flux
# 1. Imports (optional)
from indicators import {sma, ema}
from mymodule::helpers import {my_function}

# 2. Type definitions (optional): enums, structs, traits, impl blocks
enum Signal { Buy(strength: f64), Sell(strength: f64), Hold }
struct MyState { value: f64, count: int }
impl MyState { ... }

# 3. Free functions (optional)
fn helper(x: f64, y: f64) -> f64 { return x + y }

# 4. Data block (optional — used by flux fetch and flux backtest)
data {
    symbols = ["AAPL"]
    period = "1y"
    interval = "1d"
    source = "yahoo"
}

# 5. Strategy block (required)
strategy Name {
    params { ... }
    state { ... }
    on bar { ... }
}
```

---

## Bar Context Variables

Available inside `on bar`:

| Variable | Type | Description |
|----------|------|-------------|
| `close` | f64 | Current bar close price |
| `open` | f64 | Current bar open price |
| `high` | f64 | Current bar high price |
| `low` | f64 | Current bar low price |
| `volume` | f64 | Current bar volume |
| `symbol` | str | Current symbol (e.g., "AAPL") |
| `in_position` | bool | Whether a position is currently open |

---

## Trading Signals

```flux
OPEN(symbol, 100.0)         # Open a position (buy quantity)
CLOSE(symbol)               # Close entire position
CLOSE_QTY(symbol, 50.0)    # Close partial position
```

---

## Types

| Type | Literal | Notes |
|------|---------|-------|
| `int` | `42`, `-1` | 64-bit signed integer |
| `f64` | `3.14`, `0.0` | 64-bit float |
| `bool` | `true`, `false` | Boolean |
| `str` | `"hello"` | String |

---

## User-Defined Functions

```flux
fn spread(price_a: f64, price_b: f64, ratio: f64) -> f64 {
    return price_a - price_b * ratio
}

# Functions can access bar context (close, open, etc.)
fn is_oversold(lookback: int, threshold: f64) -> bool {
    z = zscore(close, lookback)
    return z < 0.0 - threshold
}

# Call in on bar:
on bar {
    if is_oversold(20, 2.0) and not in_position {
        OPEN(symbol, 100.0)
    }
}
```

---

## Structs

Group related data and attach behavior:

```flux
struct Position {
    entry_price: f64,
    quantity: f64,
    bars_held: int
}

impl Position {
    # Static method (no self) — constructor
    fn new(price: f64, qty: f64) -> Position {
        return Position {
            entry_price = price,
            quantity = qty,
            bars_held = 0
        }
    }

    # Instance method (takes self)
    fn pnl(self, current_price: f64) -> f64 {
        return (current_price - self.entry_price) * self.quantity
    }

    fn tick(self) -> Position {
        return Position {
            entry_price = self.entry_price,
            quantity = self.quantity,
            bars_held = self.bars_held + 1
        }
    }
}

# Usage:
on bar {
    pos = Position.new(close, 100.0)    # Static call
    profit = pos.pnl(close)             # Instance call
    pos = pos.tick()                    # Returns new struct (immutable values)
}
```

**Key rules:**
- Fields use `=` for assignment in literals: `MyStruct { field = value }`
- Methods that "mutate" return a new struct (values are immutable)
- Access fields with dot notation: `pos.entry_price`
- Nested access works: `book.best_bid.price`

---

## Enums

Tagged unions with optional data:

```flux
enum OrderResult {
    Filled(price: f64, qty: f64),       # Data variant (named fields)
    Rejected(reason: str),              # Data variant
    Pending                              # Unit variant (no data)
}

# Construction:
result = OrderResult.Filled(150.25, 100.0)
result = OrderResult.Pending
```

---

## Match Expressions

Pattern match on enums with destructuring:

```flux
match result {
    OrderResult.Filled(price, qty) => {
        # price and qty are bound from the enum fields
        total = price * qty
        OPEN(symbol, qty)
    }
    OrderResult.Rejected(reason) => {
        # reason is bound — contains the string
    }
    _ => {
        # Wildcard catches everything else
    }
}
```

**Rules:**
- Each arm: `EnumName.Variant(bindings) => { body }`
- Bindings are positional — they match field order in the enum definition
- Wildcard `_` matches any unmatched variant
- Match must be exhaustive (cover all variants or include `_`)

---

## Traits

Define interfaces that multiple types can implement:

```flux
trait Indicator {
    fn value(self) -> f64
    fn is_ready(self) -> bool
}

struct RSIState {
    current: f64,
    period: int,
    bars_seen: int
}

impl Indicator for RSIState {
    fn value(self) -> f64 {
        return self.current
    }
    fn is_ready(self) -> bool {
        return self.bars_seen >= self.period
    }
}
```

---

## Generics with Trait Bounds

Write functions that work with any type implementing a trait:

```flux
# [T: Indicator] means "T must implement the Indicator trait"
fn check_signal[T: Indicator](ind: T, threshold: f64) -> bool {
    if ind.is_ready() {
        return ind.value() > threshold
    }
    return false
}

# Call with concrete type:
rsi_state = RSIState { current = 75.0, period = 14, bars_seen = 20 }
should_sell = check_signal(rsi_state, 70.0)
```

---

## HashMap

Key-value store with string keys:

```flux
on bar {
    # Create
    weights = HashMap.new()

    # Insert (mutating — auto-reassigns when used as statement)
    weights.insert("AAPL", 0.4)
    weights.insert("GOOG", 0.3)
    weights.insert("MSFT", 0.3)

    # Lookup
    if weights.contains_key(symbol) {
        w = weights.get(symbol)         # Returns null if key missing
        size = base_size * w
    }

    # Remove
    weights.remove("MSFT")
}
```

**Available methods:**
| Method | Returns | Description |
|--------|---------|-------------|
| `HashMap.new()` | HashMap | Create empty map |
| `.insert(key, value)` | HashMap | Add/update entry |
| `.get(key)` | Value or null | Lookup by key |
| `.contains_key(key)` | bool | Check if key exists |
| `.remove(key)` | HashMap | Remove entry |
| `.len()` | int | Number of entries |

---

## Module Imports

Organize code across files:

```flux
# Import built-in indicators
from indicators import {sma, ema}

# Import from project files (:: = directory separator)
# File at: signals/entry.flux containing fn should_enter(...)
from signals::entry import {should_enter}

# Nested modules
from lib::math::stats import {custom_zscore}
```

File resolution: `from signals::entry import {fn_name}` looks for `signals/entry.flux` relative to the strategy file.

---

## Data Block

Tells `flux fetch` where to download data and `flux backtest` what symbols to use:

```flux
data {
    symbols = ["AAPL", "MSFT", "GOOG"]
    period = "1y"           # 1d, 5d, 1mo, 3mo, 6mo, 1y, 2y, 5y, max
    interval = "1d"         # 1m, 5m, 15m, 1h, 1d, 1wk
    source = "yahoo"        # Data provider
}
```

---

## Complete Strategy Patterns

### Mean Reversion with Z-Score

```flux
from indicators import {sma}

strategy ZScoreMeanReversion {
    params {
        lookback = 20
        entry_threshold = 2.0
        exit_threshold = 0.5
        position_size = 100.0
    }
    state {
        bar_count = 0
    }
    on bar {
        bar_count = bar_count + 1
        z = zscore(close, lookback)
        vol = stddev(close, lookback)

        # Enter when oversold
        if z < 0.0 - entry_threshold and not in_position and vol > 0.5 {
            OPEN(symbol, position_size)
        }
        # Exit when reverted
        if z > exit_threshold and in_position {
            CLOSE(symbol)
        }
    }
}
```

### Pairs Trading with Type System

```flux
from indicators import {sma}

enum Signal {
    Buy(strength: f64),
    Sell(strength: f64),
    Hold
}

struct PairState {
    z_score: f64,
    lookback: int
}

impl PairState {
    fn new(lb: int) -> PairState {
        return PairState { z_score = 0.0, lookback = lb }
    }
    fn update(self, spread: f64, avg: f64, std: f64) -> PairState {
        z = 0.0
        if std > 0.0 {
            z = (spread - avg) / std
        }
        return PairState { z_score = z, lookback = self.lookback }
    }
}

fn generate_signal(z: f64, threshold: f64) -> Signal {
    if z < 0.0 - threshold {
        return Signal.Buy(0.0 - z)
    }
    if z > threshold {
        return Signal.Sell(z)
    }
    return Signal.Hold
}

data {
    symbols = ["AAPL", "MSFT"]
    period = "1y"
    interval = "1d"
    source = "yahoo"
}

strategy PairsTrader {
    params {
        lookback = 20
        threshold = 1.5
        base_size = 100.0
    }
    state {
        bar_count = 0
    }
    on bar {
        bar_count = bar_count + 1
        spread = close - sma(close, lookback)
        avg_spread = sma(spread, lookback)
        std_spread = stddev(spread, lookback)

        pair = PairState.new(lookback)
        pair = pair.update(spread, avg_spread, std_spread)
        signal = generate_signal(pair.z_score, threshold)

        match signal {
            Signal.Buy(strength) => {
                if not in_position {
                    OPEN(symbol, base_size * strength)
                }
            }
            Signal.Sell(strength) => {
                if in_position {
                    CLOSE(symbol)
                }
            }
            _ => { }
        }
    }
}
```

### Regime-Based Position Sizing with Generics

```flux
from indicators import {sma, ema}

enum Regime { Bull, Bear, Sideways }

trait RegimeDetector {
    fn detect(self, fast: f64, slow: f64) -> Regime
}

struct TrendFollower {
    threshold: f64
}

impl RegimeDetector for TrendFollower {
    fn detect(self, fast: f64, slow: f64) -> Regime {
        diff = (fast - slow) / slow
        if diff > self.threshold { return Regime.Bull }
        if diff < 0.0 - self.threshold { return Regime.Bear }
        return Regime.Sideways
    }
}

fn classify[T: RegimeDetector](detector: T, fast: f64, slow: f64) -> Regime {
    return detector.detect(fast, slow)
}

data {
    symbols = ["SPY"]
    period = "1y"
    interval = "1d"
    source = "yahoo"
}

strategy RegimeAdaptive {
    params {
        fast_period = 10
        slow_period = 50
        bull_size = 200.0
        bear_size = 50.0
    }
    state {
        bar_count = 0
    }
    on bar {
        bar_count = bar_count + 1
        if bar_count > slow_period {
            fast = ema(close, fast_period)
            slow = sma(close, slow_period)
            detector = TrendFollower { threshold = 0.02 }
            regime = classify(detector, fast, slow)

            match regime {
                Regime.Bull => {
                    if not in_position { OPEN(symbol, bull_size) }
                }
                Regime.Bear => {
                    if in_position { CLOSE(symbol) }
                }
                Regime.Sideways => {
                    # Hold current position
                }
            }
        }
    }
}
```

---

## CSV Data Format

```csv
timestamp,symbol,open,high,low,close,volume
2024-01-02,AAPL,185.50,186.75,185.10,186.20,1200000
2024-01-03,AAPL,186.20,187.00,185.80,186.50,1100000
```

- Columns are case-insensitive, any order
- `flux fetch strategy.flux` downloads this format automatically from Yahoo Finance

---

## Common Pitfalls

1. **No negation operator for threshold comparison** — Use `0.0 - threshold` not `-threshold` in comparisons:
   ```flux
   # Correct:
   if z < 0.0 - threshold { ... }
   # Also works for literal negation:
   if z < -2.0 { ... }
   ```

2. **Structs are immutable** — Methods that "update" state return a new struct:
   ```flux
   pair = pair.update(spread, avg, std)  # Reassign!
   ```

3. **HashMap.get returns null for missing keys** — Always check with `contains_key` first or handle null.

4. **No loops in on bar (use indicators instead)** — Flux strategies operate bar-by-bar. Use rolling indicators (sma, stddev, zscore) rather than manual loops over history.

5. **Params are immutable** — You cannot reassign params. Use state for mutable values.

6. **Type annotations required on function params** — Return type annotation uses `->`:
   ```flux
   fn foo(x: f64, n: int) -> f64 { ... }
   ```

7. **Match must be exhaustive** — Include a wildcard `_` arm or cover all variants.
