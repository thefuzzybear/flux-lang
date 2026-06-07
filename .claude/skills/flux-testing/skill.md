---
name: flux-testing
description: Write tests for Flux features (unit, integration, property tests)
---

# Flux Testing

You are writing tests for Flux. Tests are **the executable specification** - they teach future agents how Flux works.

## Test Philosophy

**Every feature needs:**
1. ✅ **Positive test** - Valid usage compiles/runs correctly
2. ✅ **Negative test** - Invalid usage produces helpful error
3. ✅ **Property test** - Invariants hold for any input

**Tests answer:** "How should this work?" and "What should NOT work?"

## Test Organization

```
crates/flux-compiler/
  src/
    lexer/
      mod.rs
      tests.rs         # Unit tests for lexer
    parser/
      mod.rs
      tests.rs         # Unit tests for parser

tests/
  fixtures/
    valid/             # Valid Flux programs
      simple.flux
      mean_reversion.flux
    invalid/           # Invalid Flux programs (should error)
      lookahead.flux
      type_error.flux
  integration/
    compile_test.rs    # End-to-end compilation tests
    codegen_test.rs    # Generated Rust correctness
```

## Writing Good Tests

### Positive Test (Valid Usage)
```rust
#[test]
fn parse_strategy_with_params() {
    let source = r#"
        strategy Example {
            params {
                period = 20
                threshold = 2.0
            }
            
            on_bar {
                OPEN(symbol, 100)
            }
        }
    "#;
    
    let ast = parse(source).expect("Should parse valid strategy");
    
    // Verify structure
    assert_eq!(ast.strategies.len(), 1);
    let strategy = &ast.strategies[0];
    assert_eq!(strategy.params.len(), 2);
    assert_eq!(strategy.event_handlers.len(), 1);
}
```

### Negative Test (Invalid Usage)
```rust
#[test]
fn error_on_lookahead_access() {
    let source = r#"
        strategy Example {
            on_bar {
                if close[1] > close {  // Accessing future data!
                    OPEN(symbol, 100)
                }
            }
        }
    "#;
    
    let err = type_check(source).expect_err("Should reject lookahead");
    
    // Verify error message quality
    assert_contains!(err.to_string(), "Cannot access future bar data");
    assert_contains!(err.to_string(), "close[1]");
    assert_contains!(err.to_string(), "help: Use close[-1]");
}
```

### Property Test (Invariants)
```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn any_valid_ast_generates_valid_rust(
        strategy in strategy_generator()  // Generate random valid strategies
    ) {
        let rust_code = codegen(&strategy).unwrap();
        
        // Generated Rust must compile
        let result = compile_rust_string(&rust_code);
        prop_assert!(result.is_ok(), 
            "Generated Rust failed to compile:\n{}\n\nError: {}", 
            rust_code, result.unwrap_err());
    }
    
    #[test]
    fn portfolio_pnl_equals_sum_of_trades(
        trades in vec(trade_strategy(), 0..100)
    ) {
        let mut portfolio = Portfolio::new(100_000.0);
        let mut expected_pnl = 0.0;
        
        for trade in trades {
            let fill = trade.to_fill();
            expected_pnl += fill.pnl();
            portfolio.apply_fill(fill);
        }
        
        prop_assert_eq!(portfolio.realized_pnl(), expected_pnl,
            "P&L calculation drifted");
    }
}
```

## Test Fixtures

**Create reusable test data:**

```rust
// tests/fixtures.rs

pub fn simple_strategy() -> &'static str {
    r#"
        strategy Simple {
            on_bar {
                if close > open {
                    OPEN(symbol, 100)
                }
            }
        }
    "#
}

pub fn mean_reversion_strategy() -> &'static str {
    include_str!("fixtures/valid/mean_reversion.flux")
}

pub fn sample_bars() -> Vec<Bar> {
    vec![
        Bar { timestamp: 1, symbol: "SPY".into(), open: 100.0, high: 101.0, low: 99.0, close: 100.5, volume: 1000.0 },
        Bar { timestamp: 2, symbol: "SPY".into(), open: 100.5, high: 102.0, low: 100.0, close: 101.5, volume: 1200.0 },
        // ...
    ]
}
```

## Integration Tests

**Test end-to-end workflows:**

```rust
#[test]
fn compile_and_backtest_simple_strategy() {
    // 1. Compile Flux to Rust
    let flux_source = include_str!("../fixtures/valid/simple.flux");
    let rust_code = flux_compile(flux_source).expect("Should compile");
    
    // 2. Compile Rust to binary (or eval in memory)
    let strategy = load_strategy_from_rust(&rust_code).expect("Should load");
    
    // 3. Run backtest
    let data = sample_bars();
    let results = Backtester::new()
        .with_strategy(strategy)
        .with_data(data)
        .run()
        .unwrap();
    
    // 4. Verify results
    assert!(results.trades.len() > 0, "Strategy should trade");
    assert!(results.sharpe_ratio.is_finite(), "Sharpe should be valid");
}
```

## Snapshot Testing

**For code generation, use snapshot tests:**

```rust
#[test]
fn codegen_snapshot_simple_strategy() {
    let source = simple_strategy();
    let generated = codegen(parse(source).unwrap()).unwrap();
    
    // First run: creates tests/__snapshots__/codegen_snapshot_simple_strategy.snap
    // Subsequent runs: compares against snapshot
    insta::assert_snapshot!(generated);
}
```

Update snapshots with: `cargo insta review`

## Error Message Testing

**Test error quality, not just presence:**

```rust
#[test]
fn helpful_error_for_missing_brace() {
    let source = r#"
        strategy Example {
            on_bar {
                OPEN(symbol, 100)
            // Missing closing brace for on_bar
        }
    "#;
    
    let err = parse(source).unwrap_err();
    let msg = err.to_string();
    
    // Check error content
    assert_contains!(msg, "expected `}`");
    assert_contains!(msg, "on_bar");
    
    // Check it points to right location
    assert!(err.span().start > source.find("on_bar").unwrap());
    
    // Check it has help text
    assert_contains!(msg, "help:");
}
```

## Performance Testing

**Benchmark critical paths:**

```rust
// benches/compiler/lexer_bench.rs
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_lexer_throughput(c: &mut Criterion) {
    let source = include_str!("../tests/fixtures/large_strategy.flux");
    
    c.bench_function("lexer_1mb_strategy", |b| {
        b.iter(|| {
            let tokens = lex(black_box(source));
            black_box(tokens)
        });
    });
}

criterion_group!(benches, bench_lexer_throughput);
criterion_main!(benches);
```

Run: `cargo bench`

## Test-Driven Development Workflow

1. **Write failing test** showing desired behavior
2. **Run test** - verify it fails
3. **Implement** minimum code to pass
4. **Run test** - verify it passes
5. **Refactor** - clean up while keeping tests green
6. **Commit** - atomic commit with test + implementation

## Quick Commands

```bash
# Run all tests
cargo test

# Run with output
cargo test -- --nocapture

# Run specific test
cargo test test_name

# Run tests for specific crate
cargo test -p flux-compiler

# Run integration tests only
cargo test --test '*'

# Update snapshots
cargo insta review

# Run benchmarks
cargo bench

# Coverage report
cargo tarpaulin --out Html
```

## Common Patterns

### Testing Error Recovery
```rust
#[test]
fn parser_recovers_from_syntax_error() {
    let source = r#"
        strategy A {
            on_bar { OPEN(symbol, 100) }
        }
        
        strategy B   // Missing opening brace
            on_bar { CLOSE(symbol) }
        }
        
        strategy C {
            on_bar { OPEN(symbol, 50) }
        }
    "#;
    
    let result = parse(source);
    
    // Should have errors but still parse strategies A and C
    assert!(result.errors.len() > 0, "Should report error");
    assert_eq!(result.strategies.len(), 2, "Should recover and parse A and C");
}
```

### Testing Type Inference
```rust
#[test]
fn infers_type_from_usage() {
    let source = r#"
        strategy Example {
            on_bar {
                count = 0       // Should infer int
                count = count + 1
                
                price = close   // Should infer float
                avg = price / 2.0
            }
        }
    "#;
    
    let typed_ast = type_check(parse(source).unwrap()).unwrap();
    
    let count_var = find_var(&typed_ast, "count");
    assert_eq!(count_var.ty, Type::Int);
    
    let price_var = find_var(&typed_ast, "price");
    assert_eq!(price_var.ty, Type::Float);
}
```

---

**Remember:** Tests are documentation. Write tests that teach agents how Flux works.
