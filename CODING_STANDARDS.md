# Flux Coding Standards

These standards apply to all code in the Flux project. They ensure consistency, maintainability, and agent-friendliness.

## Principles

1. **Clarity over cleverness** - Code should be obvious at first read
2. **Documentation-first** - Update docs as you code, not after
3. **Test-driven** - Tests are the executable specification
4. **Agent-friendly** - Future AI agents will work on this code
5. **Financial correctness** - Wrong results destroy trust

## Rust Style

### Formatting

**Use rustfmt with default settings. No exceptions.**

```bash
cargo fmt --all
```

All code must pass `rustfmt` before commit.

### Linting

**All Clippy warnings must be fixed.**

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

Exceptions must be explicitly allowed with comment explaining why:

```rust
#[allow(clippy::too_many_arguments)]  // Strategy trait requires these params
fn process_event(&mut self, ...) { }
```

### Naming Conventions

```rust
// Types: PascalCase
struct MatchingEngine { }
enum OrderType { }
trait Strategy { }

// Functions and methods: snake_case
fn process_order() { }
fn calculate_pnl() { }

// Constants: SCREAMING_SNAKE_CASE
const MAX_POSITION_SIZE: usize = 1000;
const DEFAULT_CAPITAL: f64 = 100_000.0;

// Be specific, not terse
let bar_close_price = 123.45;  // Good
let price = 123.45;            // Too vague
let p = 123.45;                // Bad

let fill_quantity = 100;       // Good
let qty = 100;                 // Acceptable in small scope
let q = 100;                   // Bad
```

### Module Organization

```rust
// lib.rs or mod.rs - public API
pub use self::foo::Foo;
pub use self::bar::Bar;

mod foo;
mod bar;

// Private submodules
mod internal;
```

**One primary type per file:**
```
matching/
  mod.rs       # MatchingEngine + re-exports
  order.rs     # Order type
  fill.rs      # Fill type
  queue.rs     # OrderQueue type
```

### Error Handling

**Never panic in library code.** Return `Result<T, E>`.

```rust
// Bad
fn calculate_sma(prices: &[f64], period: usize) -> f64 {
    assert!(prices.len() >= period);  // Panics
    // ...
}

// Good
fn calculate_sma(prices: &[f64], period: usize) -> Result<f64, IndicatorError> {
    if prices.len() < period {
        return Err(IndicatorError::InsufficientData { 
            need: period, 
            have: prices.len() 
        });
    }
    // ...
}
```

**Use custom error types per crate:**

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CompilerError {
    #[error("Syntax error at {span}: {message}")]
    SyntaxError { span: Span, message: String },
    
    #[error("Type mismatch: expected {expected}, found {found}")]
    TypeMismatch { expected: Type, found: Type },
    
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, CompilerError>;
```

### Documentation

**Every public item must have a doc comment with example.**

```rust
/// Simple Moving Average over the last `period` data points.
///
/// # Arguments
/// * `prices` - Price series (must have at least `period` elements)
/// * `period` - Lookback period
///
/// # Returns
/// Average of the last `period` prices
///
/// # Example
/// ```
/// use flux_runtime::indicators::sma;
/// 
/// let prices = vec![1.0, 2.0, 3.0, 4.0, 5.0];
/// let result = sma(&prices, 3).unwrap();
/// assert_eq!(result, 4.0);  // Average of [3, 4, 5]
/// ```
///
/// # Errors
/// Returns `IndicatorError::InsufficientData` if `prices.len() < period`
pub fn sma(prices: &[f64], period: usize) -> Result<f64> {
    // Implementation
}
```

**Private complexity deserves comments:**

```rust
// PERF: Use binary search instead of linear scan. Benchmarked at 10x faster
// for typical order book sizes (100-1000 orders). See benches/matching_bench.rs
let index = orders.binary_search_by(|o| o.price.cmp(&target_price))?;
```

**Explain WHY, not WHAT:**

```rust
// Bad - obvious from code
// Increment counter by 1
counter += 1;

// Good - explains constraint
// Position size must be even due to exchange lot size requirements
if position_size % 2 != 0 {
    position_size += 1;
}
```

### Testing

**Tests colocated with code:**

```rust
// src/indicators/sma.rs

pub fn sma(prices: &[f64], period: usize) -> Result<f64> {
    // Implementation
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn sma_basic() {
        let prices = vec![10.0, 20.0, 30.0];
        assert_eq!(sma(&prices, 3).unwrap(), 20.0);
    }
    
    #[test]
    fn sma_insufficient_data() {
        let prices = vec![10.0, 20.0];
        assert!(sma(&prices, 3).is_err());
    }
}
```

**Every feature needs:**
1. Positive test (valid usage works)
2. Negative test (invalid usage errors correctly)
3. Property test (invariants hold)

**Test names describe behavior:**

```rust
#[test]
fn portfolio_long_trade_updates_cash_correctly() { }

#[test]
fn parser_recovers_from_missing_brace() { }

#[test]
fn type_checker_rejects_future_data_access() { }
```

## Error Messages

**Error messages must be actionable.**

**Template:**
```
error: <clear description>
  --> <file>:<line>:<col>
   |
<line_num> | <source code line>
   |         <pointer to issue>
   |
   = help: <specific suggestion to fix>
   = note: <additional context if needed>
```

**Example:**
```
error: Cannot access future bar data
  --> strategy.flux:10:16
   |
10 |         if close[1] > close {
   |                ^^^ accessing close[1] introduces lookahead bias
   |
   = help: Use close[-1] to access previous bars
   = note: The type system prevents accidental lookahead bias
```

**Bad error:**
```
error: type mismatch
```

**Good error:**
```
error: Type mismatch in signal argument
  --> strategy.flux:15:24
   |
15 |         OPEN(symbol, 123.45)
   |                      ^^^^^^ expected int, found float
   |
   = help: Use an integer for quantity: OPEN(symbol, 123)
   = note: Signal quantities must be whole numbers
```

## Performance

### Measure Before Optimizing

**Never optimize without profiling.**

```bash
# Profile with flamegraph
cargo flamegraph --bin flux -- compile large_strategy.flux

# Benchmark with criterion
cargo bench --bench compiler_bench
```

**Document performance-critical sections:**

```rust
// PERF: This loop processes millions of bars in backtests. 
// Benchmarked at <10μs per iteration. Do not add allocations here.
for bar in data_feed {
    // Hot path
}
```

### Common Patterns

**Reuse buffers, don't allocate in loops:**

```rust
pub struct MatchingEngine {
    fill_buffer: Vec<Fill>,  // Reused across calls
}

impl MatchingEngine {
    pub fn process(&mut self, orders: Vec<Order>) -> &[Fill] {
        self.fill_buffer.clear();  // Don't reallocate
        
        for order in orders {
            if let Some(fill) = self.match_order(order) {
                self.fill_buffer.push(fill);
            }
        }
        
        &self.fill_buffer
    }
}
```

**Use iterators (zero-cost abstractions):**

```rust
// Good - LLVM optimizes away bounds checks
let sum: f64 = prices.iter().sum();
let max = prices.iter().max();

// Avoid manual indexing
let sum = 0.0;
for i in 0..prices.len() {
    sum += prices[i];  // Bounds check on every access
}
```

**Inline small hot functions:**

```rust
#[inline]
pub fn calculate_pnl(entry: f64, exit: f64, quantity: i64) -> f64 {
    (exit - entry) * quantity as f64
}
```

## Git Workflow

### Commit Messages

```
<type>(<scope>): <subject>

<body>

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>
```

**Types:**
- `feat` - New feature
- `fix` - Bug fix
- `docs` - Documentation only
- `refactor` - Code change that neither fixes a bug nor adds a feature
- `perf` - Performance improvement
- `test` - Adding or updating tests
- `chore` - Maintenance tasks

**Scopes:**
- `lexer` - Lexer changes
- `parser` - Parser changes
- `typeck` - Type checker changes
- `codegen` - Code generator changes
- `runtime` - Runtime library changes
- `cli` - CLI tool changes
- `docs` - Documentation changes

**Examples:**

```
feat(parser): Add support for multi-timeframe event handlers

Implements on_bar_daily, on_bar_hourly as specified in language
spec section 4.3. Parser now recognizes bar_{timeframe} syntax
and generates appropriate AST nodes.

Tests included for valid usage and error cases.

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>
```

```
fix(typeck): Prevent future data access in Series[T]

Type checker now correctly rejects positive indices in Series
access (e.g., close[1]). Added comprehensive test suite for
lookahead prevention.

Fixes #42

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>
```

### Branch Strategy

**Main branch:**
- Always stable
- All tests pass
- Ready to release

**Feature branches:**
- `feat/description` - New features
- `fix/description` - Bug fixes
- `docs/description` - Documentation
- `refactor/description` - Refactoring

**Workflow:**
1. Create feature branch from `main`
2. Make changes, commit atomically
3. Ensure tests pass, clippy clean
4. Open PR against `main`
5. Address review comments
6. Squash and merge

## Agent-Friendly Code

**Future AI agents will work on Flux. Make their job easy.**

### Clear File Organization

```
crates/flux-compiler/src/
  lexer/
    mod.rs          # Entry point, exports
    token.rs        # One concept per file
    cursor.rs
  parser/
    mod.rs
    grammar/        # Grouped by purpose
      expr.rs
      stmt.rs
```

### Obvious Naming

```rust
// Good - clear intent
fn parse_strategy_block() -> Result<Strategy>
fn check_lookahead_violation() -> Result<()>
fn generate_rust_code() -> String

// Bad - vague
fn parse() -> Result<Thing>
fn check() -> Result<()>
fn gen() -> String
```

### Self-Documenting Code

```rust
// Good - types explain intent
struct StrategyParams {
    period: usize,
    threshold: f64,
}

fn validate_params(params: &StrategyParams) -> Result<()> {
    if params.period < 1 {
        return Err(Error::invalid_period(params.period));
    }
    Ok(())
}

// Bad - unclear types
fn validate(data: &HashMap<String, Value>) -> Result<()> {
    // Agent has to infer structure
}
```

## Financial Correctness

**Wrong P&L calculations destroy trust.**

### Property Testing

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn portfolio_pnl_equals_sum_of_fills(
        fills in vec(fill_strategy(), 0..100)
    ) {
        let mut portfolio = Portfolio::new(100_000.0);
        let expected_pnl: f64 = fills.iter().map(|f| f.pnl()).sum();
        
        for fill in fills {
            portfolio.apply_fill(fill);
        }
        
        prop_assert_eq!(portfolio.realized_pnl(), expected_pnl);
    }
}
```

### No Floating Point Summation Errors

```rust
// Bad - accumulates floating point errors
let mut total = 0.0;
for pnl in pnls {
    total += pnl;
}

// Good - use Kahan summation for accuracy
fn kahan_sum(values: &[f64]) -> f64 {
    let mut sum = 0.0;
    let mut c = 0.0;
    for &value in values {
        let y = value - c;
        let t = sum + y;
        c = (t - sum) - y;
        sum = t;
    }
    sum
}
```

### Explicit Money Types

```rust
// Consider using rust_decimal for financial amounts
use rust_decimal::Decimal;

pub struct Portfolio {
    cash: Decimal,  // Not f64 - no rounding errors
    positions: HashMap<Symbol, Position>,
}
```

## Review Checklist

Before submitting PR, verify:

- [ ] `cargo fmt --all` (formatted)
- [ ] `cargo clippy --all-targets` (no warnings)
- [ ] `cargo test --all` (tests pass)
- [ ] `cargo doc --no-deps` (docs build)
- [ ] Every public item has doc comment with example
- [ ] Error messages are actionable
- [ ] Performance-critical code is benchmarked
- [ ] Relevant documentation updated
- [ ] Commit messages follow template

## Resources

- **Rust API Guidelines:** https://rust-lang.github.io/api-guidelines/
- **Effective Rust:** https://www.lurklurk.org/effective-rust/
- **Rust Performance Book:** https://nnethercote.github.io/perf-book/

---

**When in doubt, prioritize:** Clarity > Performance > Cleverness
