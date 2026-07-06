# Flux Language Development Context

## What is Flux?

Flux is a trading-native programming language that compiles to native binaries through Rust. It's designed for quantitative traders — write strategies in a Python-ergonomic syntax with trading primitives built-in, get native Rust performance.

## Current Capabilities (Working End-to-End)

The full pipeline works today:

```bash
# Write a .flux strategy → backtest with CSV data → see portfolio results
flux backtest strategy.flux --data prices.csv --capital 10000
```

### CLI Commands
- `flux check <file>` — Lex → Parse → Typecheck (reports errors with source spans)
- `flux build <file> [--output path]` — Full compile to Rust source code
- `flux backtest <file> --data <csv> [--capital N]` — Run strategy against data, show P&L
- `flux init [name]` — Scaffold a new Flux project

### Backtest Output Includes
- Raw signals (Open/Close per bar)
- Fill log (BUY/SELL with price and quantity)
- Portfolio summary (equity, realized/unrealized P&L, return %, exposure)

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

## Key Source Files

| File | Purpose |
|------|---------|
| `crates/flux-compiler/src/lexer/` | Logos-based lexer with span tracking |
| `crates/flux-compiler/src/parser/` | Recursive descent parser → AST |
| `crates/flux-compiler/src/typeck/` | Type inference and validation |
| `crates/flux-compiler/src/codegen/emitter.rs` | Rust code generation |
| `crates/flux-runtime/src/signal.rs` | Signal enum (Open, Close, CloseQty) |
| `crates/flux-runtime/src/strategy.rs` | Strategy trait (on_bar → Vec<Signal>) |
| `crates/flux-runtime/src/context.rs` | BarContext (close, open, high, low, volume, symbol) |
| `crates/flux-runtime/src/position_tracker.rs` | Fill simulation, position tracking, P&L, portfolio metrics |
| `crates/flux-runtime/src/backtest.rs` | run_backtest (signal-only), run_backtest_with_tracker |
| `crates/flux-runtime/src/indicators/` | SMA, EMA with per-call-site state |
| `crates/flux-cli/src/interpreter.rs` | AST-walking interpreter for backtest mode |
| `crates/flux-cli/src/commands/backtest.rs` | Backtest command (interpreter + PositionTracker) |
| `crates/flux-cli/src/csv_loader.rs` | CSV ingestion → Vec<BarContext> |

## Flux Language Syntax (What Works Today)

```flux
# Optional imports (for future use)
from indicators import {sma}

strategy StrategyName {
    # Parameters (configurable constants)
    params {
        period = 20
        threshold = 2.5
    }

    # State variables (persist across bars)
    state {
        bar_count = 0
    }

    # Event handler — called once per bar
    on bar {
        # Market data available: close, open, high, low, volume, symbol
        # Position tracking: in_position (bool)
        
        bar_count = bar_count + 1
        avg = sma(close, period)

        # Open a position
        if close > avg and not in_position {
            OPEN(symbol, 100.0)
        }

        # Close entire position
        if close < avg and in_position {
            CLOSE(symbol)
        }

        # Close partial position (also available)
        # CLOSE_QTY(symbol, 50.0)
    }
}
```

## CSV Data Format

Required columns (case-insensitive, any order):
```csv
timestamp,symbol,open,high,low,close,volume
2024-01-02,AAPL,185.50,186.75,185.10,186.20,1200000
```

## Testing

```bash
cargo test                    # All workspace tests
cargo test -p flux-runtime    # Runtime tests (includes position tracker property tests)
cargo test -p flux-compiler   # Compiler tests
cargo test -p flux-cli        # CLI tests (unit + integration)
```

Property-based tests use `proptest`. The position tracker has 10 property tests validating invariants (equity, quantity conservation, VWAP, P&L decomposition, etc.).

## Coding Standards

- Rust: rustfmt + clippy, doc comments on public items
- Tests: colocated `mod tests`, property tests for invariants
- Errors: actionable messages with source spans
- Commits: `type(scope): description` (feat, fix, docs, test, refactor)
- Scopes: lexer, parser, typeck, codegen, runtime, cli
