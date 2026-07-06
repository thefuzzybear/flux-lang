# Flux Language Development

**You are working on Flux**, a domain-specific trading language that compiles to native binaries through Rust.

## Critical Context

**Flux does not exist in your training data.** You cannot reference external documentation, Stack Overflow, or examples. Everything you need to know is in this repository.

## What is Flux?

Flux is a trading-native programming language designed for quantitative traders and researchers. It provides:

1. **Trading-native syntax:** First-class types for `Bar`, `Signal`, `Position`, `Strategy`
2. **Type safety:** Prevents lookahead bias and position management bugs at compile time
3. **Integrated backtesting:** Write strategy in notebook, see results inline
4. **Native performance:** Compiles through Rust to optimized binaries
5. **Book-side polymorphism:** Same code tests both LONG and SHORT strategies

**Example Flux Code:**
```flux
from indicators import {sma}

strategy MeanReversion {
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

        if close < avg and not in_position {
            OPEN(symbol, 100.0)
        } elif close > avg and in_position {
            CLOSE(symbol)
        }
    }
}
```

## Architecture Overview

```
Flux Source Code (.flux)
    ↓ Lexer (crates/flux-compiler/src/lexer/)
Tokens
    ↓ Parser (crates/flux-compiler/src/parser/)
Abstract Syntax Tree (AST)
    ↓ Type Checker (crates/flux-compiler/src/typeck/)
Typed AST
    ├─→ Code Generator (crates/flux-compiler/src/codegen/)
    │       → Rust Source Code (.rs) → Cargo → Native Binary
    │
    └─→ Interpreter (crates/flux-cli/src/interpreter.rs)
            → Signals per bar
                ↓ PositionTracker (crates/flux-runtime/src/position_tracker.rs)
            → Fills, Positions, P&L, Equity, Exposure
```

The **backtest** command uses the interpreter path. The **build** command uses the codegen path.

## Repository Structure

```
flux-lang/
├── .claude/
│   ├── CLAUDE.md              # This file - main context
│   ├── skills/                # Development skills (see below)
│   └── prompts/               # Common prompts/templates
├── .kiro/
│   ├── steering/              # Kiro AI steering files
│   └── specs/                 # Feature specs (requirements, design, tasks)
├── crates/
│   ├── flux-compiler/         # Compiler (lexer, parser, typeck, codegen)
│   │   └── src/
│   │       ├── lexer/         # Logos-based lexer with spans
│   │       ├── parser/        # Recursive descent parser → AST
│   │       ├── typeck/        # Type checker → Typed AST
│   │       └── codegen/       # Rust code emitter
│   ├── flux-runtime/          # Runtime library
│   │   └── src/
│   │       ├── backtest.rs        # run_backtest (signal collection)
│   │       ├── position_tracker.rs # PositionTracker, Fill, Position, run_backtest_with_tracker
│   │       ├── signal.rs          # Signal enum (Open, Close, CloseQty)
│   │       ├── strategy.rs        # Strategy trait
│   │       ├── context.rs         # BarContext struct
│   │       └── indicators/        # SMA, EMA (per-call-site state)
│   └── flux-cli/              # CLI tool
│       └── src/
│           ├── main.rs            # CLI entry (check, build, backtest, init)
│           ├── interpreter.rs     # AST-walking interpreter for backtest
│           ├── csv_loader.rs      # CSV → Vec<BarContext>
│           ├── commands/
│           │   ├── backtest.rs    # backtest command (interpreter + PositionTracker)
│           │   ├── build.rs       # build command (codegen)
│           │   ├── check.rs       # check command (typecheck only)
│           │   └── init.rs        # init command (project scaffold)
│           └── diagnostics.rs     # Error formatting with source spans
├── .planning/                 # Architecture docs, language spec, roadmap
├── Cargo.toml                 # Workspace root
├── CODING_STANDARDS.md        # Coding conventions
└── CONTRIBUTING.md            # How to contribute
```

## Development Skills

Use these skills for focused development tasks:

- **`/flux-compiler-dev`** - Work on compiler (lexer, parser, type checker, codegen)
- **`/flux-parser-dev`** - Deep work on parser implementation
- **`/flux-codegen-dev`** - Work on Rust code generation
- **`/flux-runtime-dev`** - Work on runtime library (backtesting, indicators)
- **`/flux-testing`** - Write tests for Flux features

Each skill loads specific context and coding standards for that area.

## Quick Start for Agents

**New to this codebase? Read these in order:**
1. `docs/architecture/00-overview.md` - High-level architecture
2. `docs/architecture/01-compiler-pipeline.md` - How compilation works
3. `docs/language-spec/flux-spec-v0.1.md` - What Flux the language is
4. `CODING_STANDARDS.md` - How we write code here

**Working on a specific component?**
- Lexer: Read `docs/architecture/02-lexer.md`
- Parser: Read `docs/architecture/03-parser.md`
- Type system: Read `docs/architecture/04-type-system.md`
- Code generation: Read `docs/architecture/05-codegen.md`

## Coding Standards (Summary)

**Full standards in `CODING_STANDARDS.md`. Key points:**

### Rust Style
- Follow rustfmt (no exceptions)
- Clippy warnings are errors
- Every public item has doc comment with example
- Tests colocated with code (`mod tests`)

### Error Messages
- Must be actionable (show code snippet + suggestion)
- Include "help:" line with fix
- Reference Flux language spec section when relevant

### Testing
- Every feature has positive test (valid code)
- Every feature has negative test (invalid code with expected error)
- Property tests for invariants (use proptest)

### Documentation
- Doc comments explain WHY, code shows WHAT
- Examples in doc comments must compile
- Link to relevant spec sections

### Performance
- No premature optimization
- Profile before optimizing (use criterion)
- Document performance-critical sections

## Common Development Patterns

### Adding New Syntax

1. **Update lexer** - Add new token(s) in `lexer/token.rs`
2. **Update parser** - Add parsing rule in `parser/*.rs`
3. **Update AST** - Add node type in `ast.rs`
4. **Update type checker** - Add type checking rule in `typeck/*.rs`
5. **Update codegen** - Add Rust code generation in `codegen/*.rs`
6. **Add tests** - Both valid and invalid usage
7. **Update docs** - Language spec and examples

### Adding New Type

1. **Define type** in `typeck/types.rs`
2. **Add inference rules** in `typeck/infer.rs`
3. **Add type checking** in `typeck/check.rs`
4. **Add codegen** in `codegen/types.rs`
5. **Add tests** with type errors
6. **Document** in type system spec

### Fixing Bugs

1. **Write failing test** that reproduces bug
2. **Fix bug** in relevant module
3. **Verify test passes**
4. **Add regression test** if not covered
5. **Update docs** if behavior clarified

## Key Principles

### Documentation-First
Every agent needs rich context because Flux isn't in training data. When you add features:
- Update relevant docs FIRST
- Code SECOND
- This ensures docs stay accurate

### Test-Driven
Tests are the executable specification. They teach agents "how Flux works":
- Write test before implementation
- Tests show valid AND invalid usage
- Property tests for invariants

### Agent-Friendly
Future agents will work on Flux. Make their job easy:
- Clear file organization
- Rich doc comments
- Obvious naming
- Self-documenting code

### Living Documentation
As you learn Flux patterns, capture them:
- Update `RUST_PATTERNS.md` with Flux-specific patterns
- Add examples to docs when you solve hard problems
- Improve error messages based on confusion
- Update skills with new learnings

## Performance Targets

| Component | Target | Why |
|-----------|--------|-----|
| Compile small strategy | <5s | Interactive development |
| Compile large strategy | <30s | Acceptable for production |
| Lexer throughput | >10MB/s | Not the bottleneck |
| Parser throughput | >5MB/s | Not the bottleneck |
| Type checking | <1s for 1000 LOC | User doesn't wait |

## Current Status

**Phase:** Foundation (Core Pipeline Complete)
**Focus:** The full compile + backtest pipeline is operational
**What's done:**
- Lexer (Logos-based, spans, all tokens)
- Parser (full AST with expressions, statements, strategies)
- Type Checker (type inference, validation, typed AST)
- Code Generator (emits valid Rust implementing Strategy trait)
- Runtime: `run_backtest` (signal collection), `run_backtest_with_tracker` (fills + P&L + portfolio state)
- Position Tracker: fill simulation, VWAP averaging, mark-to-market, portfolio metrics
- CLI: `check`, `build`, `backtest` (with `--capital` flag), `init`
- Interpreter: AST-walking execution for backtest mode (no compile step needed)
- Indicators: SMA, EMA (stateful, per-call-site)
- CSV loader (OHLCV data ingestion)

**End-to-end workflow:**
```bash
flux backtest strategy.flux --data prices.csv --capital 10000
```
This lexes → parses → typechecks → interprets bar-by-bar → feeds signals through PositionTracker → prints fills, P&L, equity, exposure.

**Next milestones:**
- More indicators (stddev, RSI, Bollinger Bands)
- CLOSE_QTY support in language syntax
- Equity curve output (per-bar CSV)
- Multi-symbol backtests
- Performance metrics (Sharpe, max drawdown)

See `.kiro/specs/` for detailed implementation specs.

## Getting Help

**Stuck? Check these resources in order:**
1. Relevant doc in `docs/architecture/`
2. Existing tests for similar features
3. Language spec in `docs/language-spec/`
4. Code comments in related modules

**Found a gap in docs?** Fix it! Documentation PRs are high priority.

## Git Workflow

### Commit Messages
```
<type>(<scope>): <subject>

<body>

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>
```

**Types:** `feat`, `fix`, `docs`, `refactor`, `perf`, `test`, `chore`
**Scopes:** `lexer`, `parser`, `typeck`, `codegen`, `runtime`, `cli`, `docs`

### Examples
```
feat(parser): Add support for on_bar_daily event handler

Implements multi-timeframe event handlers as specified in
language spec section 4.3.

Tests included for valid usage and syntax errors.

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>
```

## Remember

- **Flux doesn't exist in your training data** - Trust the docs in this repo
- **Documentation-first culture** - Update docs as you code
- **Tests teach agents** - Write tests that explain behavior
- **Agent-friendly code** - Future agents will read your code
- **Living skills** - Update skills as patterns emerge

---

**Ready to contribute? Start with:** `docs/contributing/GETTING_STARTED.md`
