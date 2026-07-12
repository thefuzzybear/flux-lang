# AI Agent Context for Flux

> **Flux is NOT in your training data.** It is a custom programming language. Do not guess at syntax — use the references below.

## If You're Writing Flux Code (Strategy Development)

Read: **[docs/writing-strategies.md](docs/writing-strategies.md)**

This is the complete guide to writing trading strategies in Flux. It covers every language feature with working examples: structs, enums, traits, generics, match expressions, HashMap, module imports, and signal emission.

Additional references:
- [docs/language-reference.md](docs/language-reference.md) — Full syntax and semantics
- [docs/builtins-reference.md](docs/builtins-reference.md) — All built-in functions
- [demos/](demos/) — Working strategy examples to study

## If You're Modifying the Flux Compiler/Runtime (Language Development)

Read the tool-specific context file for your environment:
- **Claude/Sonnet:** [.claude/CLAUDE.md](.claude/CLAUDE.md)
- **Kiro:** [.kiro/steering/flux-development.md](.kiro/steering/flux-development.md)
- **Cursor:** [.cursor/rules/flux-language.mdc](.cursor/rules/flux-language.mdc)

These contain: architecture, file map, testing commands, and patterns for adding new language features.

## Quick Facts

- **Language:** Trading-native DSL, Python-ergonomic syntax, compiles through Rust
- **Run a strategy:** `flux backtest strategy.flux --data prices.csv --capital 10000`
- **Type system:** structs, enums, impl blocks, traits, generics, HashMap
- **Testing:** `cargo test` (Rust workspace, 500+ tests, proptest for property testing)
- **Crate structure:** `flux-compiler` (lexer/parser/typeck/codegen), `flux-runtime` (signals/backtest/indicators), `flux-cli` (interpreter/commands)
