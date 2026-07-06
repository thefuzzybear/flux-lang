# Flux

**A trading-native programming language that compiles to native binaries.**

Flux is designed for quantitative traders and researchers. Write strategies in a Python-ergonomic syntax with trading primitives built-in, get the performance of compiled Rust.

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

## Features

- **Trading-native types:** `Bar`, `Signal`, `Position`, `Strategy` are first-class
- **Type safety:** Prevents lookahead bias and position bugs at compile time
- **Book-side polymorphism:** Same code tests both LONG and SHORT strategies
- **Integrated backtesting:** Write in notebook, see results inline
- **Native performance:** Compiles through Rust to optimized binaries
- **Genetic algorithm:** Optimize parameters with built-in genetic algorithm

## Status

**Early Development (Alpha)**

Flux is in active development. The compiler pipeline (lexer → parser → typechecker → codegen) is complete, with an interpreter-based backtest engine that includes full portfolio tracking.

**What works today:**
- Write strategies in `.flux` files
- Compile and type-check (`flux check`, `flux build`)
- Run backtests with CSV data and see P&L results (`flux backtest`)
- Portfolio tracking with fills, positions, equity, and exposure metrics
- Built-in indicators (SMA, EMA)
- Project scaffolding (`flux init`)

**Current milestone:** Position Tracking & Portfolio Metrics (Complete)

## Quick Start

```bash
# Build from source
git clone https://github.com/thefuzzybear/flux-lang.git
cd flux-lang
cargo build --release

# Initialize a new project
cargo run -p flux-cli -- init my-strategy

# Or if installed:
# flux init my-strategy

cd my-strategy
```

Write a strategy in `strategy.flux`:
```flux
from indicators import {sma}

strategy SmaCrossover {
    params {
        period = 5
    }

    state {
        bar_count = 0
    }

    on bar {
        bar_count = bar_count + 1
        avg = sma(close, period)

        if close > avg and not in_position {
            OPEN(symbol, 100.0)
        } elif close < avg and in_position {
            CLOSE(symbol)
        }
    }
}
```

Run a backtest:
```bash
flux backtest strategy.flux --data data.csv --capital 10000
```

Output:
```
--- Signals ---
  2 Open AAPL 100
  5 Close AAPL

--- Fills ---
  Bar    2 |  BUY | AAPL     100.00 @     186.80
  Bar    5 |  SELL | AAPL     100.00 @     186.90

--- Portfolio Summary ---
  Initial Capital:       10000.00
  Final Equity:          10010.00
  Realized P&L:             10.00
  Unrealized P&L:            0.00
  Total Return:             0.10%
  Open Positions:               0
  Gross Exposure:            0.00
  Net Exposure:              0.00
  Total Fills:                  2

--- Summary ---
Total signals: 2
Open: 1
Close: 1
CloseQty: 0
```

### CSV Data Format

Your data CSV needs these columns (case-insensitive, order doesn't matter):
```csv
timestamp,symbol,open,high,low,close,volume
2024-01-02,AAPL,185.50,186.75,185.10,186.20,1200000
2024-01-03,AAPL,186.20,186.90,185.80,186.50,980000
```

### CLI Commands

```bash
flux check strategy.flux          # Type-check only (no execution)
flux build strategy.flux           # Compile to Rust code (stdout)
flux build strategy.flux --output out.rs  # Compile to file
flux backtest strategy.flux --data prices.csv              # Backtest (default $10k capital)
flux backtest strategy.flux --data prices.csv --capital 50000  # Custom capital
flux init my-project              # Scaffold a new project
```

## Documentation

- **[Language Specification](docs/language-spec/flux-spec-v0.1.md)** - Complete Flux language reference
- **[Architecture](docs/architecture/)** - How Flux compiler and runtime work
- **[Contributing](docs/contributing/GETTING_STARTED.md)** - How to contribute
- **[Examples](docs/examples/)** - Example strategies

## Why Flux?

**Problem:** Python is slow, Rust is hard, existing solutions are closed or limited.

**Flux solves this:**
- Write strategies as fast as Python notebooks
- Get performance of native Rust binaries
- Type system prevents common trading bugs (lookahead, position errors)
- Integrated backtesting (no separate tools)
- Open source (MIT license)

## Architecture

```
Flux Source (.flux)
    ↓ Lexer (tokenization)
Tokens
    ↓ Parser (AST construction)
AST
    ↓ Type Checker (type inference + validation)
Typed AST
    ↓ Interpreter (backtest mode)
Signals → PositionTracker → Portfolio Results

    ↓ Code Generator (build mode)
Rust Source (.rs)
    ↓ Cargo
Native Binary
```

**Compiler:** Lexer → Parser → Type Checker → Code Generator  
**Runtime:** PositionTracker (fill simulation, P&L, mark-to-market), Indicators (SMA, EMA)  
**CLI:** check, build, backtest, init

## Project Goals

**Year 1 (Months 1-18):** Language + Runtime
- Complete language specification (DONE)
- Lexer + Parser implementation (IN PROGRESS)
- Type system (PLANNED)
- Code generation (PLANNED)
- Runtime library (PLANNED)
- Notebook environment (PLANNED)
- Genetic algorithm (PLANNED)

**Year 2 (Months 18-36):** Commercial Platform
- Cloud deployment
- Data marketplace
- Collaboration features
- Enterprise features

**Year 3+:** Production use in systematic trading

## Community

- **GitHub Discussions:** Ask questions, share strategies
- **Discord:** Real-time chat (link coming soon)
- **Twitter:** [@fluxlang](https://twitter.com/fluxlang) (placeholder)

## Contributing

We welcome contributions. Flux is built for the era of agentic development - rich documentation and clear patterns help both humans and AI agents contribute effectively.

See [CONTRIBUTING.md](docs/contributing/GETTING_STARTED.md) for:
- Development setup
- Coding standards
- How to pick up issues
- PR guidelines

**Good first issues:** [Issues labeled `good-first-issue`](https://github.com/thefuzzybear/flux-lang/labels/good-first-issue)

## License

MIT License - see [LICENSE](LICENSE) for details.

## Credits

Created by [@thefuzzybear](https://github.com/thefuzzybear)

## Roadmap

See [docs/ROADMAP.md](docs/ROADMAP.md) for detailed development timeline.

**Next milestones:**
- Language specification complete (DONE)
- Lexer implementation (Month 1-2)
- Parser implementation (Month 2-3)
- Type system (Month 3-5)
- Code generation (Month 5-8)

## FAQ

### Is Flux production-ready?
Not yet. Flux is in early development (pre-alpha). The language spec is complete but the compiler is being built. Estimated 18 months to MVP.

### How is Flux different from Pine Script?
- Flux is open source (MIT), Pine Script is proprietary
- Flux compiles to native binaries, Pine Script is interpreted
- Flux has modern type system, Pine Script is dynamically typed
- Flux supports genetic algorithm optimization built-in

### Can I use Python libraries in Flux?
Not directly in v0.1. Future versions may support Python interop via PyO3.

### What's the performance like?
Flux compiles to Rust which compiles to native code. Expect C++/Rust-level performance (approximately 100-1000x faster than Python for backtesting).

### How does lookahead prevention work?
The type system tracks time in `Series[T]` types. `close[0]` (current) and `close[-1]` (past) compile, but `close[1]` (future) is a compile error.

### Can I run strategies live (paper/real trading)?
Not in v0.1 (backtesting only). Live trading support is planned for Year 2-3.

---

**Star this repo** to follow development.
