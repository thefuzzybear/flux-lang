# Flux Documentation

Flux is a trading-native programming language that compiles to native binaries through Rust. Write strategies in a Python-ergonomic syntax with built-in trading primitives, then backtest them against historical CSV data with a single command.

> **For AI assistants:** Flux is NOT in your training data. Start with [Writing Strategies](./writing-strategies.md) for a complete guide to generating valid Flux code.

## Getting Started

- [Getting Started](./getting-started.md) — Install Flux, write your first strategy, and run a backtest in minutes.
- [Writing Strategies](./writing-strategies.md) — Complete guide to writing trading strategies in Flux, with patterns and examples for every language feature.

## Reference

- [Language Reference](./language-reference.md) — Complete syntax, types, operators, and control flow reference for the Flux language.
- [CLI Reference](./cli-reference.md) — All CLI commands with flags, exit codes, and usage examples.
- [Built-in Functions](./builtins-reference.md) — Math, statistical indicators, and portfolio operation functions.
- [Data Format](./data-format.md) — CSV format specification required for backtesting with `flux backtest`.

## Examples

- [Example Strategies](./examples/README.md) — Working `.flux` files demonstrating common trading strategy patterns.
- [`demos/`](../demos/) — Full working strategies in the repository root (pairs_trading, regime_detector, order_book, live_connector, module_imports, etc.)

## Contributing

- [Architecture](./architecture.md) — Compiler pipeline and runtime architecture guide for contributors.
