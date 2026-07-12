# Flux Language Development Context

## Critical: Flux Is Not In Your Training Data

Flux is a custom programming language built from scratch. You cannot reference external docs, Stack Overflow, or examples. Everything you need is in this repository. When writing Flux code or modifying the compiler, rely ONLY on the patterns shown below and in the `demos/` directory.

## What is Flux?

Flux is a trading-native programming language that compiles to native binaries through Rust. Write strategies in a Python-ergonomic syntax with trading primitives built-in, get native Rust performance.

```bash
# The full workflow today:
flux check strategy.flux          # Typecheck (reports errors with source spans)
flux build strategy.flux          # Compile to Rust source
flux backtest strategy.flux --data prices.csv --capital 10000  # Interpret + backtest
flux fmt strategy.flux            # Format source code
flux init my-project              # Scaffold new project
flux fetch strategy.flux          # Download market data from Yahoo Finance
flux live strategy.flux           # Run with live/replay data feed
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
│   │   ├── parser/            # Recursive descent → AST (ast.rs, mod.rs)
│   │   ├── typeck/            # Type checker → Typed AST (checker.rs, typed_ast.rs)
│   │   └── codegen/           # Rust source emitter (emitter.rs)
│   ├── flux-runtime/src/
│   │   ├── signal.rs          # Signal enum (Open, Close, CloseQty)
│   │   ├── strategy.rs        # Strategy trait (on_bar → Vec<Signal>)
│   │   ├── context.rs         # BarContext (close, open, high, low, volume, symbol)
│   │   ├── position_tracker.rs # Fill simulation, P&L, portfolio metrics
│   │   ├── backtest.rs        # run_backtest, run_backtest_with_tracker
│   │   └── indicators/        # SMA, EMA (stateful, per-call-site)
│   └── flux-cli/src/
│       ├── interpreter.rs     # AST-walking interpreter (2000+ lines)
│       ├── stat_indicators.rs # stddev, zscore, correlation, etc.
│       ├── csv_loader.rs      # CSV → Vec<BarContext>
│       ├── formatter/         # flux fmt implementation
│       ├── live/              # Live trading harness (connector, aggregator)
│       ├── data/              # Data fetching (Yahoo Finance)
│       └── commands/          # CLI command handlers
├── demos/                     # Working strategy examples
├── std/                       # Standard library modules (market/l1.flux, etc.)
├── editors/vscode/            # VS Code extension (syntax highlighting)
├── docs/                      # User-facing documentation
└── .planning/                 # Architecture, roadmap, language spec
```

## Flux Language Syntax — Complete Reference

### Basic Strategy Structure

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
        last_signal = 0.0
    }

    on bar {
        # All bar data available: close, open, high, low, volume, symbol
        # Position state: in_position (bool)
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

### Types

| Type | Example | Notes |
|------|---------|-------|
| `int` | `42`, `-1` | 64-bit integer |
| `f64` | `3.14`, `0.0` | 64-bit float |
| `bool` | `true`, `false` | |
| `str` | `"hello"` | String |
| `HashMap` | `HashMap.new()` | String → Value map |
| Struct | `PairState { ... }` | User-defined |
| Enum | `Signal.Buy(0.5)` | User-defined |

### User-Defined Functions

```flux
fn calculate_spread(price_a: f64, price_b: f64, ratio: f64) -> f64 {
    return price_a - price_b * ratio
}

# Functions can access bar context (close, open, etc.) and built-ins
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
    # Static method (no self) — used as constructor
    fn new(lookback: int) -> PairState {
        return PairState {
            mean_spread = 0.0,
            z_score = 0.0,
            lookback = lookback
        }
    }

    # Instance method (takes self)
    fn update(self, spread: f64, avg: f64, std: f64) -> PairState {
        z = self.calculate_zscore(spread, avg, std)
        return PairState {
            mean_spread = avg,
            z_score = z,
            lookback = self.lookback
        }
    }

    fn calculate_zscore(self, spread: f64, avg: f64, std: f64) -> f64 {
        if std > 0.0 {
            return (spread - avg) / std
        }
        return 0.0
    }
}
```

### Enums and Match Expressions

```flux
enum Signal {
    Buy(strength: f64),
    Sell(strength: f64),
    Hold
}

# Construction
signal = Signal.Buy(0.75)
signal = Signal.Hold

# Pattern matching with destructuring
match signal {
    Signal.Buy(strength) => {
        size = base_size * strength
        OPEN(symbol, size)
    }
    Signal.Sell(strength) => {
        CLOSE(symbol)
    }
    _ => {
        # Wildcard — matches anything
    }
}
```

### Traits and Generics

```flux
trait RegimeDetector {
    fn detect(self, fast: f64, slow: f64, vol: f64) -> Regime
}

struct TrendDetector {
    crossover_pct: f64
}

impl RegimeDetector for TrendDetector {
    fn detect(self, fast: f64, slow: f64, vol: f64) -> Regime {
        diff = (fast - slow) / slow
        if diff > self.crossover_pct {
            return Regime.Bull
        }
        return Regime.Sideways
    }
}

# Generic function with trait bound
fn detect_regime[T: RegimeDetector](detector: T, fast: f64, slow: f64, vol: f64) -> Regime {
    return detector.detect(fast, slow, vol)
}
```

### HashMap Operations

```flux
registry = HashMap.new()
registry.insert("AAPL", 1.0)
registry.insert("MSFT", -0.85)

if registry.contains_key(symbol) {
    ratio = registry.get(symbol)     # Returns Value::Null if key missing
}
```

### Module Imports

```flux
# Import from standard library
from indicators import {sma, ema}

# Import from project modules (:: separator = directory structure)
from signals::entry import {should_enter}
from math::stats import {zscore_custom}
```

### Built-in Indicators and Functions

| Function | Signature | Notes |
|----------|-----------|-------|
| `sma(series, period)` | `(f64, int) → f64` | Simple moving average |
| `ema(series, period)` | `(f64, int) → f64` | Exponential moving average |
| `stddev(series, period)` | `(f64, int) → f64` | Standard deviation |
| `zscore(series, period)` | `(f64, int) → f64` | Z-score |
| `correlation(a, b, period)` | `(f64, f64, int) → f64` | Pearson correlation |
| `max(a, b)` | `(f64, f64) → f64` | Maximum |
| `min(a, b)` | `(f64, f64) → f64` | Minimum |
| `abs(x)` | `(f64) → f64` | Absolute value |

### Signal Emission

```flux
OPEN(symbol, quantity)          # Open a position (buy)
CLOSE(symbol)                   # Close entire position
CLOSE_QTY(symbol, quantity)     # Close partial position
```

### Control Flow

```flux
if condition {
    # body
} elif other_condition {
    # body
} else {
    # body
}

# Boolean operators: and, or, not
if close > avg and not in_position {
    OPEN(symbol, 100.0)
}
```

### Operators

| Category | Operators |
|----------|-----------|
| Arithmetic | `+`, `-`, `*`, `/`, `%` |
| Comparison | `==`, `!=`, `<`, `>`, `<=`, `>=` |
| Logical | `and`, `or`, `not` |
| Unary | `-` (negation) |

### Comments

```flux
# Single-line comment (hash style)
```

## Key Source Files for Development

| File | Purpose | Lines |
|------|---------|-------|
| `crates/flux-cli/src/interpreter.rs` | AST-walking interpreter | ~2500 |
| `crates/flux-compiler/src/typeck/checker.rs` | Type checker | ~2000 |
| `crates/flux-compiler/src/parser/mod.rs` | Parser | ~1500 |
| `crates/flux-compiler/src/codegen/emitter.rs` | Rust code emitter | ~1000 |
| `crates/flux-compiler/src/lexer/mod.rs` | Lexer | ~500 |
| `crates/flux-compiler/src/typeck/typed_ast.rs` | Typed AST definitions | ~500 |
| `crates/flux-compiler/src/parser/ast.rs` | AST node definitions | ~400 |

## Testing

```bash
cargo test                                          # Full workspace
cargo test -p flux-cli --lib                        # CLI unit tests (500+)
cargo test -p flux-cli --test type_system_interpreter_property  # Property tests
cargo test -p flux-cli --test type_system_demos_integration     # Demo backtests
cargo test -p flux-compiler                         # Compiler tests
cargo test -p flux-runtime                          # Runtime tests
```

Property-based tests use `proptest` (already a dev-dependency). Convention: files named `*_property.rs` in `crates/flux-cli/tests/`.

## Coding Standards

- Rust: rustfmt + clippy, doc comments on public items
- Tests: colocated `mod tests`, property tests for invariants
- Errors: actionable messages with source spans (`"runtime error: struct '{}' has no field '{}'"`)
- Commits: `type(scope): description` (feat, fix, docs, test, refactor)
- Scopes: lexer, parser, typeck, codegen, runtime, cli, interpreter

## CSV Data Format

```csv
timestamp,symbol,open,high,low,close,volume
2024-01-02,AAPL,185.50,186.75,185.10,186.20,1200000
```

Columns are case-insensitive, any order. The `flux fetch` command downloads this format from Yahoo Finance.

## Common Development Patterns

### Adding new syntax to Flux:
1. Add token in `lexer/logos_token.rs`
2. Add parse rule in `parser/mod.rs`
3. Add AST node in `parser/ast.rs`
4. Add typeck rule in `typeck/checker.rs` + typed node in `typed_ast.rs`
5. Add interpreter eval in `interpreter.rs`
6. Add codegen in `codegen/emitter.rs`
7. Write tests (positive + negative)

### Adding a new built-in function/indicator:
1. Add to `stat_indicators.rs` (or `indicators/` in runtime)
2. Register in interpreter's built-in dispatch
3. Register in typechecker's built-in function types
4. Add test in `interpreter.rs` mod tests
