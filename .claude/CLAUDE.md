# Flux Language Development

## Critical: Flux Is Not In Your Training Data

Flux is a custom programming language. You CANNOT reference external docs, Stack Overflow, or examples. Everything you need is in this repository. When writing Flux code or modifying the compiler, rely ONLY on the patterns shown here and in `demos/`.

## What is Flux?

Flux is a trading-native programming language that compiles to native binaries through Rust. It provides Python-ergonomic syntax with trading primitives built-in and native Rust performance.

```bash
flux check strategy.flux                            # Typecheck only
flux build strategy.flux                            # Compile to Rust source
flux backtest strategy.flux --data data.csv --capital 10000  # Interpret + backtest
flux fmt strategy.flux                              # Format code
flux init my-project                                # Scaffold project
flux fetch strategy.flux                            # Download market data
flux live strategy.flux                             # Live/replay mode
```

## Architecture

```
.flux source → Lexer → Parser → Type Checker → Typed AST
                                                    │
                        ┌───────────────────────────┼──────────────────┐
                        │                           │                  │
                  (flux build)               (flux backtest)     (flux check)
                        │                           │                  │
                  Code Generator              Interpreter         (done)
                        │                           │
                  Rust source (.rs)           Signals per bar
                                                    │
                                            PositionTracker
                                                    │
                                    Fills, P&L, Equity, Exposure
```

## Repository Structure

```
flux-lang/
├── crates/
│   ├── flux-compiler/src/
│   │   ├── lexer/             # Logos-based tokenizer with span tracking
│   │   ├── parser/            # Recursive descent → AST
│   │   ├── typeck/            # Type checker → Typed AST
│   │   └── codegen/           # Rust source emitter
│   ├── flux-runtime/src/
│   │   ├── signal.rs          # Signal enum (Open, Close, CloseQty)
│   │   ├── strategy.rs        # Strategy trait
│   │   ├── context.rs         # BarContext (close, open, high, low, volume, symbol)
│   │   ├── position_tracker.rs # Fill simulation, P&L, portfolio metrics
│   │   ├── backtest.rs        # run_backtest, run_backtest_with_tracker
│   │   └── indicators/        # SMA, EMA (stateful, per-call-site)
│   └── flux-cli/src/
│       ├── interpreter.rs     # AST-walking interpreter (~2500 lines)
│       ├── stat_indicators.rs # stddev, zscore, correlation
│       ├── csv_loader.rs      # CSV → Vec<BarContext>
│       ├── formatter/         # flux fmt
│       ├── live/              # Live trading harness
│       ├── data/              # Yahoo Finance data fetcher
│       └── commands/          # CLI command handlers
├── demos/                     # Working strategy examples (pairs_trading, regime_detector, etc.)
├── std/                       # Standard library modules
├── editors/vscode/            # VS Code extension
├── docs/                      # User-facing docs
└── .planning/                 # Architecture, roadmap, language spec
```

## Flux Language — Complete Syntax Reference

### Strategy Structure

```flux
from indicators import {sma, ema}
from signals::entry import {should_enter}

data {
    symbols = ["AAPL", "MSFT"]
    period = "1y"
    interval = "1d"
    source = "yahoo"
}

strategy StrategyName {
    params {
        period = 20
        threshold = 2.0
    }

    state {
        bar_count = 0
    }

    on bar {
        bar_count = bar_count + 1
        avg = sma(close, period)

        if close > avg and not in_position {
            OPEN(symbol, 100.0)
        }
        if close < avg and in_position {
            CLOSE(symbol)
        }
    }
}
```

### Available Bar Context Variables

- `close`, `open`, `high`, `low` — OHLC prices (f64)
- `volume` — Volume (f64)
- `symbol` — Current symbol (str)
- `in_position` — Whether strategy has an open position (bool)

### Types

| Type | Example | Notes |
|------|---------|-------|
| `int` | `42` | 64-bit integer |
| `f64` | `3.14` | 64-bit float |
| `bool` | `true`, `false` | |
| `str` | `"hello"` | String |
| `HashMap` | `HashMap.new()` | String keys → any value |
| Struct | `MyStruct { field = value }` | User-defined |
| Enum | `Signal.Buy(0.5)` | User-defined tagged union |

### Functions

```flux
fn calculate_spread(price_a: f64, price_b: f64, ratio: f64) -> f64 {
    return price_a - price_b * ratio
}

# Functions can access bar context globals
fn should_enter(lookback: int, threshold: f64) -> f64 {
    z = zscore(close, lookback)
    if z < 0.0 - threshold {
        return 1.0
    }
    return 0.0
}
```

### Structs and Impl Blocks

```flux
struct PairState {
    mean_spread: f64,
    z_score: f64,
    lookback: int
}

impl PairState {
    # Static method (no self) — constructor pattern
    fn new(lookback: int) -> PairState {
        return PairState { mean_spread = 0.0, z_score = 0.0, lookback = lookback }
    }

    # Instance method (takes self) — accesses fields via self.field
    fn update(self, spread: f64, avg: f64, std: f64) -> PairState {
        z = self.calculate_zscore(spread, avg, std)
        return PairState { mean_spread = avg, z_score = z, lookback = self.lookback }
    }

    fn calculate_zscore(self, spread: f64, avg: f64, std: f64) -> f64 {
        if std > 0.0 { return (spread - avg) / std }
        return 0.0
    }
}
```

### Enums and Match

```flux
enum Signal {
    Buy(strength: f64),
    Sell(strength: f64),
    Hold
}

enum FillResult {
    Filled(price: f64, quantity: f64),
    PartialFill(price: f64, filled_qty: f64, remaining_qty: f64),
    Rejected(reason: str)
}

# Pattern matching with destructuring
match signal {
    Signal.Buy(strength) => {
        OPEN(symbol, base_size * strength)
    }
    Signal.Sell(strength) => {
        CLOSE(symbol)
    }
    _ => { }  # Wildcard
}
```

### Traits and Generics

```flux
trait RegimeDetector {
    fn detect(self, fast: f64, slow: f64, vol: f64) -> Regime
}

struct TrendDetector { crossover_pct: f64 }

impl RegimeDetector for TrendDetector {
    fn detect(self, fast: f64, slow: f64, vol: f64) -> Regime {
        diff = (fast - slow) / slow
        if diff > self.crossover_pct { return Regime.Bull }
        return Regime.Sideways
    }
}

# Generic function with trait bound (square brackets)
fn detect_regime[T: RegimeDetector](detector: T, fast: f64, slow: f64, vol: f64) -> Regime {
    return detector.detect(fast, slow, vol)
}
```

### HashMap

```flux
registry = HashMap.new()
registry.insert("AAPL", 1.0)
registry.insert("MSFT", -0.85)

if registry.contains_key(symbol) {
    ratio = registry.get(symbol)    # Returns null if key missing
}

# Mutating methods (insert, remove) auto-reassign when used as statements
registry.insert("GOOG", 0.5)       # No need for registry = registry.insert(...)
```

### Module Imports

```flux
from indicators import {sma, ema}               # Built-in indicators
from signals::entry import {should_enter}       # Project module (:: = directory separator)
from math::stats import {zscore_custom}         # Nested module
```

### Built-in Functions

| Function | Signature | Notes |
|----------|-----------|-------|
| `sma(series, period)` | `(f64, int) → f64` | Simple moving average |
| `ema(series, period)` | `(f64, int) → f64` | Exponential moving average |
| `stddev(series, period)` | `(f64, int) → f64` | Rolling standard deviation |
| `zscore(series, period)` | `(f64, int) → f64` | Rolling z-score |
| `correlation(a, b, period)` | `(f64, f64, int) → f64` | Pearson correlation |
| `max(a, b)` | `(f64, f64) → f64` | Maximum |
| `min(a, b)` | `(f64, f64) → f64` | Minimum |
| `abs(x)` | `(f64) → f64` | Absolute value |

### Signal Emission

```flux
OPEN(symbol, quantity)          # Open position (buy)
CLOSE(symbol)                   # Close entire position
CLOSE_QTY(symbol, quantity)     # Close partial
```

### Control Flow and Operators

```flux
if condition {
} elif other {
} else {
}

# Operators: +, -, *, /, %, ==, !=, <, >, <=, >=, and, or, not
# Comments: # (hash to end of line)
# No loops yet — strategies operate bar-by-bar via on bar { }
```

## Key Development Files

| File | Purpose |
|------|---------|
| `crates/flux-cli/src/interpreter.rs` | AST-walking interpreter (main execution engine) |
| `crates/flux-compiler/src/typeck/checker.rs` | Type checker |
| `crates/flux-compiler/src/parser/mod.rs` | Parser |
| `crates/flux-compiler/src/codegen/emitter.rs` | Rust code emitter |
| `crates/flux-compiler/src/lexer/mod.rs` | Lexer |
| `crates/flux-compiler/src/typeck/typed_ast.rs` | Typed AST node definitions |
| `crates/flux-compiler/src/parser/ast.rs` | AST node definitions |
| `crates/flux-cli/src/stat_indicators.rs` | Statistical indicator implementations |

## Testing

```bash
cargo test                    # Full workspace
cargo test -p flux-cli --lib  # CLI unit tests (500+)
cargo test -p flux-cli --test type_system_interpreter_property  # Property tests
cargo test -p flux-cli --test type_system_demos_integration     # Integration tests
cargo test -p flux-compiler   # Compiler tests
cargo test -p flux-runtime    # Runtime tests
```

Property tests use `proptest`. Convention: `*_property.rs` files in `crates/flux-cli/tests/`.

## Adding New Syntax to Flux

1. Token → `crates/flux-compiler/src/lexer/logos_token.rs`
2. Parse rule → `crates/flux-compiler/src/parser/mod.rs`
3. AST node → `crates/flux-compiler/src/parser/ast.rs`
4. Type check → `crates/flux-compiler/src/typeck/checker.rs`
5. Typed AST node → `crates/flux-compiler/src/typeck/typed_ast.rs`
6. Interpreter eval → `crates/flux-cli/src/interpreter.rs`
7. Codegen → `crates/flux-compiler/src/codegen/emitter.rs`
8. Tests (positive + negative cases)

## Adding a New Built-in Function

1. Implement in `crates/flux-cli/src/stat_indicators.rs`
2. Register in interpreter built-in dispatch (`interpreter.rs`, search for "built-in")
3. Register type signature in typechecker (`checker.rs`, search for "builtin_fn_types")
4. Add test

## Coding Standards

- Rust: rustfmt + clippy, doc comments on public items
- Tests: colocated `mod tests`, property tests for invariants
- Errors: actionable messages with source spans
- Commits: `type(scope): description`
- Types: feat, fix, docs, test, refactor
- Scopes: lexer, parser, typeck, codegen, runtime, cli, interpreter

## CSV Data Format

```csv
timestamp,symbol,open,high,low,close,volume
2024-01-02,AAPL,185.50,186.75,185.10,186.20,1200000
```

## Demo Strategies (Working Examples)

See `demos/` directory for complete working strategies:
- `demos/mean_reversion/` — Basic z-score mean reversion
- `demos/pairs_trading/` — Structs, enums, match, HashMap, traits (kitchen sink)
- `demos/regime_detector/` — Trait-bounded generics, polymorphic dispatch
- `demos/order_book/` — Nested structs, multi-field match destructuring
- `demos/live_connector/` — Dual-mode (backtest + live), trait impls
- `demos/module_imports/` — Multi-file strategy with :: imports
- `demos/functions_showcase/` — User functions, cross-file imports
