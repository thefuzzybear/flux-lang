# Architecture Guide

This document explains the Flux compiler pipeline, runtime architecture, and workspace layout for contributors looking to understand or modify the system.

## Compiler Pipeline

Flux source code passes through four sequential stages. Each stage transforms a well-defined input into a well-defined output:

```
.flux source → Lexer → Parser → Type Checker → Code Generator
     (text)     (tokens)  (AST)    (Typed AST)    (Rust source)
```

### Stage 1: Lexer

| | |
|---|---|
| **Input** | Raw `.flux` source text (UTF-8 string) |
| **Output** | Stream of tokens with source spans |
| **Responsibility** | Tokenize keywords, identifiers, literals, operators, and punctuation. Track byte offsets for error reporting. Skip whitespace and comments. |

The lexer is built on the [Logos](https://docs.rs/logos) library for zero-copy tokenization with automatic span tracking.

### Stage 2: Parser

| | |
|---|---|
| **Input** | Token stream from the lexer |
| **Output** | Untyped Abstract Syntax Tree (AST) |
| **Responsibility** | Validate syntactic structure. Build tree nodes for strategy blocks, params, state, on bar handlers, expressions, and statements. Report syntax errors with spans. |

The parser uses recursive descent — no external parser generator. It produces a tree of AST nodes representing the full strategy structure.

### Stage 3: Type Checker

| | |
|---|---|
| **Input** | Untyped AST |
| **Output** | Typed AST (all expressions annotated with resolved types) |
| **Responsibility** | Infer and validate types for all expressions. Resolve built-in function signatures. Check that signal functions receive correct argument types. Report type errors with source spans. |

The type checker registers all built-in functions (math, indicators, portfolio operations) and validates calls against their declared signatures.

### Stage 4: Code Generator

| | |
|---|---|
| **Input** | Typed AST |
| **Output** | Rust source code implementing the `Strategy` trait |
| **Responsibility** | Emit valid Rust code that implements `flux_runtime::Strategy`. Map Flux types to Rust types. Generate the `on_bar` method body from the handler AST. |

### Which Stages Each Command Uses

| Command | Lexer | Parser | Type Checker | Code Generator | Interpreter |
|---------|:-----:|:------:|:------------:|:--------------:|:-----------:|
| `flux check` | ✓ | ✓ | ✓ | | |
| `flux build` | ✓ | ✓ | ✓ | ✓ | |
| `flux backtest` | ✓ | ✓ | ✓ | | ✓ |

- **`flux check`** runs Lex → Parse → TypeCheck and reports any errors. No output is generated.
- **`flux build`** runs the full pipeline through Code Generator, emitting Rust source code.
- **`flux backtest`** runs Lex → Parse → TypeCheck, then hands the Typed AST to the Interpreter (not the Code Generator).

## Interpreter-Based Backtest Path

The `flux backtest` command does not compile to Rust. Instead, it uses an AST-walking interpreter to evaluate the strategy directly against historical bar data.

### Execution Flow

1. **Compile** — The `.flux` file passes through Lexer → Parser → Type Checker to produce a Typed AST.
2. **Load data** — The CSV file is parsed into a `Vec<BarContext>` (one entry per bar per symbol).
3. **Initialize** — An `Interpreter` is created from the Typed AST, and a `PositionTracker` is created with the initial capital.
4. **Bar iteration** — For each bar in the dataset:
   - The `in_position` flag is synced from the tracker's open position count
   - The interpreter evaluates the `on bar` handler with the current bar's context variables (`close`, `open`, `high`, `low`, `volume`, `symbol`)
   - The interpreter returns a `Vec<Signal>` (Open, Close, or CloseQty)
5. **Signal dispatch** — Signals are fed to the `PositionTracker`, which converts them into fills at the bar's close price.
6. **Mark-to-market** — After processing signals, all open positions are marked to market at the bar's close price, updating unrealized P&L.
7. **Output** — After all bars are processed, the system prints Signals, Fills, and Portfolio Summary.

### Diagram

```
                    ┌─────────────────────────────────────────────┐
                    │              Bar Iteration Loop              │
                    │                                             │
  CSV Data ───────▶│  BarContext ──▶ Interpreter ──▶ Vec<Signal>  │
                    │                                     │        │
                    │                                     ▼        │
                    │                            PositionTracker   │
                    │                                     │        │
                    │                              ┌──────┴──────┐ │
                    │                              │   Fills     │ │
                    │                              │   P&L       │ │
                    │                              │   Equity    │ │
                    │                              │   Exposure  │ │
                    │                              └─────────────┘ │
                    └─────────────────────────────────────────────┘
```

## PositionTracker

The `PositionTracker` is the stateful engine that converts trading signals into portfolio state. It lives in `flux-runtime` and is used by the backtest command.

### Signal → Fill Conversion

All fills execute at the **bar close price** (no slippage model):

| Signal | Behavior |
|--------|----------|
| `Open { symbol, qty }` | Creates a BUY fill. If no position exists for the symbol, opens a new one. If a position already exists, adds to it (increases quantity). |
| `Close { symbol }` | Creates a SELL fill for the entire position quantity. Removes the position. Ignored if no position exists. |
| `CloseQty { symbol, qty }` | Creates a SELL fill for `min(qty, position.qty)`. Reduces position quantity. Removes the position if quantity reaches zero. Ignored if no position exists. |

### VWAP Average Entry Price

When adding to an existing position, the average entry price is updated using volume-weighted averaging:

```
new_avg = (existing_qty × existing_avg + new_qty × fill_price) / (existing_qty + new_qty)
```

This means the average entry price reflects the blended cost basis across all fills that built the position.

### Position Quantity

Each position tracks a `qty` field representing the number of shares/units held. Open signals increase it; CloseQty signals decrease it. A full Close zeroes it out and removes the position.

### P&L Computation

- **Realized P&L**: Computed when closing (fully or partially). Formula: `(close_price - avg_entry_price) × close_qty`. Accumulated across all closed trades.
- **Unrealized P&L**: Computed on mark-to-market. Formula: `(current_price - avg_entry_price) × position_qty`. Updated every bar for all open positions.
- **Equity**: `initial_capital + total_realized_pnl + total_unrealized_pnl`
- **Gross Exposure**: Sum of `|qty × price|` for all open positions.
- **Net Exposure**: Sum of `qty × price` for all open positions (preserves sign for short positions).

## Workspace Crates

The project is a Cargo workspace with three crates:

### `flux-compiler`

**Responsibility**: Everything from source text to Typed AST.

Contains the lexer, parser, type checker, and code generator. This crate has no runtime dependencies and can be used independently for static analysis tooling.

- Lexer (Logos-based tokenization with span tracking)
- Parser (recursive descent → AST)
- Type checker (type inference, built-in function registration, validation)
- Code generator (Typed AST → Rust source code)

### `flux-runtime`

**Responsibility**: Runtime primitives shared between compiled strategies and the interpreter.

Defines the core abstractions that both compiled Rust strategies and the interpreter-based backtest use:

- `Signal` enum (`Open`, `Close`, `CloseQty`)
- `Strategy` trait (`on_bar(&mut self, ctx: &BarContext) -> Vec<Signal>`)
- `BarContext` (market data for a single bar: close, open, high, low, volume, symbol)
- `PositionTracker` (signal processing, fill simulation, P&L, portfolio metrics)
- `run_backtest` / `run_backtest_with_tracker` (backtest harnesses)
- Indicators (SMA, EMA with per-call-site state management)

### `flux-cli`

**Responsibility**: User-facing CLI application and the interpreter.

Ties everything together into the `flux` binary:

- Command implementations (`check`, `build`, `backtest`, `init`)
- AST-walking interpreter (evaluates Typed AST against bar data)
- CSV loader (ingests market data into `Vec<BarContext>`)
- Diagnostics formatter (renders errors with source context and spans)
- Built-in function implementations for the interpreter (math, statistical indicators, portfolio operations)

## Data Flow Diagram

```
┌──────────────────────────────────────────────────────────────────────────┐
│                           flux backtest                                    │
│                                                                          │
│  .flux file ──▶ Lexer ──▶ Parser ──▶ Type Checker ──▶ Typed AST         │
│                                                            │              │
│  .csv file ──▶ CSV Loader ──▶ Vec<BarContext>              │              │
│                                    │                       │              │
│                                    ▼                       ▼              │
│                              ┌──────────────────────────────┐            │
│                              │    Interpreter (per bar)      │            │
│                              │                               │            │
│                              │  Context vars + Typed AST     │            │
│                              │         │                     │            │
│                              │         ▼                     │            │
│                              │    Vec<Signal>                │            │
│                              └──────────┬───────────────────┘            │
│                                         │                                 │
│                                         ▼                                 │
│                              ┌──────────────────────┐                    │
│                              │   PositionTracker     │                    │
│                              │                       │                    │
│                              │  Signals → Fills      │                    │
│                              │  Mark-to-Market       │                    │
│                              │  P&L Accumulation     │                    │
│                              └──────────┬───────────┘                    │
│                                         │                                 │
│                                         ▼                                 │
│                              ┌──────────────────────┐                    │
│                              │      Output           │                    │
│                              │                       │                    │
│                              │  • Signals log        │                    │
│                              │  • Fill log           │                    │
│                              │  • Portfolio Summary   │                    │
│                              └───────────────────────┘                    │
└──────────────────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────────────────┐
│                           flux build                                      │
│                                                                          │
│  .flux file ──▶ Lexer ──▶ Parser ──▶ Type Checker ──▶ Code Generator    │
│                                                            │              │
│                                                            ▼              │
│                                                     Rust source (.rs)     │
└──────────────────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────────────────┐
│                           flux check                                      │
│                                                                          │
│  .flux file ──▶ Lexer ──▶ Parser ──▶ Type Checker ──▶ (done)            │
│                                                                          │
│                    Errors reported with source spans if any stage fails   │
└──────────────────────────────────────────────────────────────────────────┘
```

## Key Source Files

| Component | File Path | Purpose |
|-----------|-----------|---------|
| Lexer | `crates/flux-compiler/src/lexer/` | Logos-based lexer with span tracking |
| Parser | `crates/flux-compiler/src/parser/` | Recursive descent parser → AST |
| Type Checker | `crates/flux-compiler/src/typeck/` | Type inference and validation |
| Code Generator | `crates/flux-compiler/src/codegen/emitter.rs` | Rust code generation from Typed AST |
| Signal | `crates/flux-runtime/src/signal.rs` | Signal enum (Open, Close, CloseQty) |
| Strategy Trait | `crates/flux-runtime/src/strategy.rs` | Strategy trait (`on_bar` → `Vec<Signal>`) |
| Bar Context | `crates/flux-runtime/src/context.rs` | BarContext (close, open, high, low, volume, symbol) |
| Position Tracker | `crates/flux-runtime/src/position_tracker.rs` | Fill simulation, position tracking, P&L, portfolio metrics |
| Backtest Harness | `crates/flux-runtime/src/backtest.rs` | `run_backtest`, `run_backtest_with_tracker` |
| Indicators | `crates/flux-runtime/src/indicators/` | SMA, EMA with per-call-site state |
| Interpreter | `crates/flux-cli/src/interpreter.rs` | AST-walking interpreter for backtest mode |
| Backtest Command | `crates/flux-cli/src/commands/backtest.rs` | Backtest command (interpreter + PositionTracker) |
| CSV Loader | `crates/flux-cli/src/csv_loader.rs` | CSV ingestion → `Vec<BarContext>` |
