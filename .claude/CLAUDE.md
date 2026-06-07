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
from flux.indicators import {sma, stddev}

strategy MeanReversion {
    book_side = LONG
    
    params {
        period = 20
        threshold = 2.0
    }
    
    state {
        prices = []
    }
    
    on_bar {
        prices.append(close)
        
        if len(prices) < period {
            return
        }
        
        zscore = (close - sma(prices, period)) / stddev(prices, period)
        
        if zscore < -threshold and not in_position {
            OPEN(symbol, 100)
        } elif zscore > threshold and in_position {
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
    ↓ Code Generator (crates/flux-compiler/src/codegen/)
Rust Source Code (.rs)
    ↓ Cargo
Native Binary
```

## Repository Structure

```
flux-lang/
├── .claude/
│   ├── CLAUDE.md              # This file - main context
│   ├── skills/                # Development skills (see below)
│   └── prompts/               # Common prompts/templates
├── .kiro/
│   └── steering/              # Kiro AI steering files
├── crates/
│   ├── flux-compiler/         # Compiler (lexer, parser, typeck, codegen)
│   ├── flux-runtime/          # Runtime library (backtesting, indicators)
│   └── flux-cli/              # CLI tool (flux compile, flux run)
├── docs/
│   ├── architecture/          # How Flux works internally
│   ├── contributing/          # How to contribute
│   ├── language-spec/         # Flux language specification
│   ├── examples/              # Example Flux strategies
│   └── tutorials/             # Step-by-step guides
├── tests/
│   ├── fixtures/              # Test Flux code
│   └── integration/           # End-to-end tests
└── Cargo.toml                 # Workspace root
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

**Phase:** Foundation (Month 0-3)
**Focus:** Lexer + Parser implementation
**Next Milestone:** Parse all example strategies without errors

See `docs/ROADMAP.md` for full development timeline.

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
