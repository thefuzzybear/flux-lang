---
name: flux-compiler-dev
description: Work on the Flux compiler with full context and coding standards
---

# Flux Compiler Development

You are working on the **Flux compiler**, which transforms Flux source code into Rust code.

## Compiler Pipeline

```
Flux Source (.flux)
    ↓
[LEXER] - String → Tokens
    ↓
[PARSER] - Tokens → AST
    ↓
[TYPE CHECKER] - AST → Typed AST + Errors
    ↓
[CODE GENERATOR] - Typed AST → Rust Code
    ↓
Rust Source (.rs)
```

## Module Structure

```
crates/flux-compiler/src/
├── lib.rs              # Public API, compile() function
├── lexer/
│   ├── mod.rs          # Lexer entry point
│   ├── token.rs        # Token definitions
│   └── cursor.rs       # Source code cursor
├── parser/
│   ├── mod.rs          # Parser entry point
│   ├── grammar/        # Parsing rules per construct
│   │   ├── expr.rs     # Expression parsing
│   │   ├── stmt.rs     # Statement parsing
│   │   └── strategy.rs # Strategy block parsing
│   └── ast.rs          # AST node definitions
├── typeck/
│   ├── mod.rs          # Type checker entry point
│   ├── types.rs        # Type definitions
│   ├── infer.rs        # Type inference
│   ├── check.rs        # Type checking
│   └── errors.rs       # Type errors
├── codegen/
│   ├── mod.rs          # Code generator entry point
│   ├── strategy.rs     # Generate Strategy trait impl
│   ├── expr.rs         # Generate expressions
│   └── runtime.rs      # Runtime library bindings
└── error.rs            # Compiler error types
```

## Current Module: {inferred from files you're reading}

## Coding Standards

### Lexer
- Use `logos` crate for tokenization (fast, zero-cost)
- Tokens should carry span information (start, end)
- Support both Python (`and`, `or`) and TypeScript (`&&`, `||`) operators
- Comments are tokens (useful for doc generation)

### Parser
- Use recursive descent (hand-written, not generated)
- **Why not lalrpop?** Need better error recovery and custom messages
- Pratt parsing for expressions (operator precedence)
- Error recovery: synchronize at statement boundaries
- AST nodes carry full span information

### Type Checker
- Hindley-Milner-style inference with constraints
- Special handling for `Series[T]` (prevents future access)
- Trading types (`Bar`, `Signal`, `Position`) are first-class
- Type errors must suggest fixes

### Code Generator
- Generate idiomatic Rust (follows Rust naming conventions)
- Emit doc comments from Flux code
- Runtime library functions imported as needed
- Optimize common patterns (e.g., inline small indicator calls)

## Error Message Philosophy

**Every error must be actionable.** Users should know exactly what to fix.

**Bad error:**
```
error: syntax error
```

**Good error:**
```
error: expected `}` to close strategy block
  --> strategy.flux:10:5
   |
8  |     strategy Example {
   |                      - strategy block opened here
9  |         on_bar {
10 |             OPEN(symbol, 100)
   |     ^ expected `}`, found end of file
   |
   = help: add `}` to close the strategy block
```

**Template for errors:**
```rust
Error::new(span)
    .with_message("Clear description of what went wrong")
    .with_label(primary_span, "What's wrong here")
    .with_label(related_span, "Related context")
    .with_note("Additional explanation")
    .with_help("How to fix it")
```

## Common Tasks

### Adding New Syntax

**Example: Adding `on_bar_daily` handler**

1. **Lexer**: No changes (uses existing `on_bar` keyword + identifier)

2. **Parser** (`parser/grammar/strategy.rs`):
```rust
fn parse_event_handler(&mut self) -> Result<EventHandler> {
    self.expect(Token::On)?;
    
    // Parse handler name: "bar", "bar_daily", "fill", etc.
    let name = self.parse_identifier()?;
    
    // Validate known handlers
    match name.as_str() {
        "bar" | "bar_daily" | "bar_hourly" => {
            // Parse body
        }
        _ => return Err(Error::unknown_event_handler(name)),
    }
}
```

3. **AST** (`parser/ast.rs`):
```rust
pub enum EventHandler {
    OnBar { body: Block, span: Span },
    OnBarDaily { body: Block, span: Span },
    OnBarHourly { body: Block, span: Span },
    // ...
}
```

4. **Type Checker**: Validate event handler has correct signature

5. **Codegen** (`codegen/strategy.rs`):
```rust
match handler {
    EventHandler::OnBarDaily { body, .. } => {
        writeln!(w, "fn on_bar_daily(&mut self, bar: &Bar) {{")?;
        self.gen_block(body, w)?;
        writeln!(w, "}}")?;
    }
}
```

6. **Tests**:
```rust
#[test]
fn parse_on_bar_daily() {
    let source = r#"
        strategy Test {
            on_bar_daily {
                log("Daily bar")
            }
        }
    "#;
    let ast = parse(source).unwrap();
    // Assert structure
}

#[test]
fn error_unknown_event_handler() {
    let source = r#"
        strategy Test {
            on_bar_yearly {  // Invalid
                log("Yearly bar")
            }
        }
    "#;
    let err = parse(source).unwrap_err();
    assert_contains!(err, "unknown event handler");
    assert_contains!(err, "did you mean: on_bar_daily?");
}
```

### Adding New Type

**Example: Adding `OrderType` enum**

1. **Types** (`typeck/types.rs`):
```rust
pub enum Type {
    // ... existing types
    OrderType,
}

pub fn is_order_type(&self) -> bool {
    matches!(self, Type::OrderType)
}
```

2. **Type Checking** (`typeck/check.rs`):
```rust
// When checking OPEN_LIMIT(symbol, qty, price):
fn check_signal(&mut self, signal: &Signal) -> TypeResult {
    match signal.kind {
        SignalKind::OpenLimit { price, .. } => {
            self.expect_type(price, Type::Float)?;
        }
    }
}
```

3. **Codegen** (`codegen/types.rs`):
```rust
fn gen_type(&self, ty: &Type, w: &mut impl Write) -> Result {
    match ty {
        Type::OrderType => write!(w, "flux_runtime::OrderType"),
        // ...
    }
}
```

## Testing Strategy

### Unit Tests
- Test each phase independently
- Mock dependencies (e.g., parser doesn't need type checker)
- Focus on edge cases

### Integration Tests
- End-to-end: Flux source → compiled Rust
- Use `tests/fixtures/*.flux` for test input
- Compare generated Rust to expected output

### Property Tests
- Use `proptest` for invariants
- Example: "Any valid AST should produce valid Rust"
- Example: "Type checker never accepts future access"

## Performance Considerations

**Compiler should be fast** (user is waiting):

- Lexer: Stream tokens, don't allocate vector
- Parser: Arena allocation for AST nodes
- Type checker: Union-find for type unification
- Codegen: Buffered writes, not string concat

**Optimize only when measured:**
- Use `cargo flamegraph` to find bottlenecks
- Benchmark with `criterion` (`benches/` directory)
- Target: <5s for small strategy, <30s for large

## Debugging Tips

### Visualize AST
```rust
// parser/ast.rs
impl Debug for Expr {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        // Pretty-print tree structure
    }
}
```

### Trace Type Inference
```rust
#[cfg(debug_assertions)]
{
    eprintln!("Inferring type for {:?}", expr);
    eprintln!("  Constraints: {:?}", self.constraints);
}
```

### Dump Generated Code
```bash
flux compile --emit-rust strategy.flux
# Writes strategy.rs (generated Rust code)
```

## Common Pitfalls

### Parser Error Recovery
**Bad:** Give up on first error
```rust
if self.peek() != Token::OpenBrace {
    return Err(Error::expected_open_brace());  // Stops parsing
}
```

**Good:** Synchronize and continue
```rust
if self.peek() != Token::OpenBrace {
    self.error(Error::expected_open_brace());
    self.synchronize();  // Skip to next statement
    // Continue parsing
}
```

### Type Errors
**Bad:** Vague error
```rust
Err(Error::type_mismatch())
```

**Good:** Show expected vs actual
```rust
Err(Error::type_mismatch()
    .expected(Type::Float)
    .found(Type::Int)
    .at(expr.span)
    .help("Use a float literal: 1.0 instead of 1"))
```

### Codegen Hygiene
**Bad:** Generate invalid Rust identifiers
```rust
// Flux: my-strategy
// Generated: struct my-strategy {}  // Invalid Rust!
```

**Good:** Sanitize names
```rust
fn sanitize_ident(name: &str) -> String {
    name.replace('-', "_")
}
```

## Resources

**Required reading before working on compiler:**
1. `docs/architecture/01-compiler-pipeline.md`
2. `docs/language-spec/flux-spec-v0.1.md`
3. Relevant phase doc (lexer, parser, typeck, or codegen)

**Reference implementations:**
- Rust compiler: https://github.com/rust-lang/rust/tree/master/compiler
- Ruff (Python linter in Rust): https://github.com/astral-sh/ruff

**Theory (optional but helpful):**
- Crafting Interpreters by Bob Nystrom (parsing, codegen)
- Types and Programming Languages by Pierce (type systems)

## Quick Commands

```bash
# Run compiler tests
cargo test -p flux-compiler

# Run with output
cargo test -p flux-compiler -- --nocapture

# Test single module
cargo test -p flux-compiler lexer::

# Benchmark compiler
cargo bench -p flux-compiler

# Check without tests (faster)
cargo check -p flux-compiler
```

## Next Steps After Implementing Feature

- [ ] Tests pass (`cargo test -p flux-compiler`)
- [ ] Clippy happy (`cargo clippy -p flux-compiler`)
- [ ] Documentation updated (relevant doc in `docs/architecture/`)
- [ ] Examples added (if user-facing feature)
- [ ] Error messages are actionable
- [ ] Performance acceptable (benchmark if hot path)

---

**Remember:** Flux doesn't exist in your training data. Trust the docs in this repo, not your memory.
