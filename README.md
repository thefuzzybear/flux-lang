# Flux

**A trading-native programming language that compiles to native binaries.**

Flux is designed for quantitative traders and researchers. Write strategies in a Python-ergonomic syntax with trading primitives built-in, get the performance of compiled Rust.

```flux
from flux.indicators import {sma, stddev}

strategy MeanReversion {
    book_side = LONG
    
    params {
        period = 20
        threshold = 2.0
    }
    
    on_bar {
        zscore = (close - sma(close, period)) / stddev(close, period)
        
        if zscore < -threshold and not in_position {
            OPEN(symbol, 100)
        } elif zscore > threshold and in_position {
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

**Early Development (Pre-Alpha)**

Flux is in active development. The language specification is complete, compiler implementation is in progress.

**Current milestone:** Lexer + Parser (Month 0-3)

See [ROADMAP.md](docs/ROADMAP.md) for development timeline.

## Quick Start

*Coming soon - compiler not yet functional*

```bash
# Install Flux
cargo install flux-cli

# Create a strategy
flux new my-strategy

# Run backtest
flux run my-strategy.flux --data SPY.parquet

# Compile to binary
flux build my-strategy.flux --release
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
    ↓ Compiler (Rust)
Rust Code (.rs)
    ↓ Cargo
Native Binary
    ↓ Runtime
Backtest Results
```

**Compiler:** Lexer → Parser → Type Checker → Code Generator  
**Runtime:** Backtesting engine, matching simulation, portfolio management, indicators

See [Architecture Overview](docs/architecture/00-overview.md) for details.

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

**Special Thanks:**
- Claude Sonnet 4.5 - Co-development and architecture design

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
