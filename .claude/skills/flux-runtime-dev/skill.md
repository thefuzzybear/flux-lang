---
name: flux-runtime-dev
description: Work on the Flux runtime library (backtesting, indicators, portfolio management)
---

# Flux Runtime Library Development

You are working on the **Flux runtime library**, which provides backtesting infrastructure and trading primitives that compiled Flux code calls.

## Architecture

```
Flux Strategy (compiled to Rust)
    ↓ calls
Runtime Library (crates/flux-runtime)
    ├── Backtester         # Orchestrates backtest execution
    ├── MatchingEngine     # Simulates order execution
    ├── PortfolioManager   # Tracks positions and P&L
    ├── RiskModule         # Enforces limits
    └── Indicators         # Technical indicators (SMA, EMA, etc.)
```

## Module Structure

```
crates/flux-runtime/src/
├── lib.rs              # Public API
├── backtest/
│   ├── mod.rs          # Backtester orchestrator
│   ├── config.rs       # Backtest configuration
│   └── results.rs      # Performance metrics
├── matching/
│   ├── mod.rs          # Matching engine
│   ├── orders.rs       # Order types
│   └── fills.rs        # Fill simulation
├── portfolio/
│   ├── mod.rs          # Portfolio manager
│   ├── position.rs     # Position tracking
│   └── pnl.rs          # P&L calculation
├── risk/
│   ├── mod.rs          # Risk module
│   ├── limits.rs       # Limit definitions
│   └── checks.rs       # Pre-trade validation
├── indicators/
│   ├── mod.rs          # Indicator exports
│   ├── moving_avg.rs   # SMA, EMA, WMA
│   ├── oscillators.rs  # RSI, Stochastic
│   └── volatility.rs   # ATR, Bollinger Bands
├── data/
│   ├── mod.rs          # Data feed abstraction
│   ├── parquet.rs      # Parquet reader
│   └── csv.rs          # CSV reader
└── types.rs            # Core types (Bar, Signal, etc.)
```

## Core Types

**These types are what compiled Flux code interacts with:**

```rust
// Market data
pub struct Bar {
    pub timestamp: i64,
    pub symbol: Symbol,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

// Trading signals
pub enum Signal {
    Open { symbol: Symbol, quantity: i64 },
    Close { symbol: Symbol },
    OpenLimit { symbol: Symbol, quantity: i64, price: f64 },
    // ...
}

// Portfolio state (read-only for strategies)
pub struct PortfolioState {
    cash: f64,
    positions: HashMap<Symbol, Position>,
    total_pnl: f64,
}

// Position (read-only for strategies)
pub struct Position {
    pub symbol: Symbol,
    pub quantity: i64,  // Negative = short
    pub avg_entry_price: f64,
    pub unrealized_pnl: f64,
}
```

## Coding Standards

### Performance Critical
- Backtester processes millions of bars → optimize hot paths
- Use arena allocation for order/fill objects
- Reuse buffers (don't allocate in loops)
- Profile with `criterion` (`benches/runtime/`)

### Financial Correctness
- P&L calculations must be exact (no floating point errors in summation)
- Position tracking must never drift (buys - sells = net position)
- Use property tests (`proptest`) for invariants

### Determinism
- Same input + same seed = same output (critical for genetic algorithm)
- No random() without seed parameter
- No Date.now() without explicit timestamp parameter

## Testing Strategy

### Unit Tests
```rust
#[test]
fn portfolio_long_trade_pnl() {
    let mut portfolio = Portfolio::new(100_000.0);
    
    // Buy 100 @ $50
    portfolio.apply_fill(Fill::buy("SPY", 100, 50.0));
    assert_eq!(portfolio.cash(), 95_000.0);  // 100k - 5k
    assert_eq!(portfolio.position("SPY").quantity, 100);
    
    // Mark to market at $55
    portfolio.update_prices(&[("SPY", 55.0)]);
    assert_eq!(portfolio.position("SPY").unrealized_pnl, 500.0);  // 100 * (55 - 50)
    
    // Sell 100 @ $55
    portfolio.apply_fill(Fill::sell("SPY", 100, 55.0));
    assert_eq!(portfolio.cash(), 100_500.0);  // 95k + 5.5k
    assert_eq!(portfolio.realized_pnl(), 500.0);
}
```

### Property Tests
```rust
proptest! {
    #[test]
    fn position_tracking_invariant(
        trades in vec(trade_strategy(), 0..100)
    ) {
        let mut portfolio = Portfolio::new(100_000.0);
        let mut expected_quantity = 0i64;
        
        for trade in trades {
            portfolio.apply_fill(trade.to_fill());
            expected_quantity += trade.signed_quantity();
        }
        
        let actual = portfolio.position(trade.symbol).quantity;
        prop_assert_eq!(actual, expected_quantity,
            "Position tracking drifted! Expected {}, got {}", 
            expected_quantity, actual);
    }
}
```

### Integration Tests
```rust
#[test]
fn end_to_end_backtest() {
    let strategy = SimpleMA::new(20, 50);
    let data = load_test_data("SPY_2020_2023.parquet");
    
    let results = Backtester::new()
        .with_strategy(strategy)
        .with_data(data)
        .with_capital(100_000.0)
        .run()
        .unwrap();
    
    // Compare to hand-calculated expected results
    assert_approx_eq!(results.total_return, 0.342, 0.01);
    assert_approx_eq!(results.sharpe_ratio, 1.82, 0.01);
}
```

## Key Implementations

### Book-Side Polymorphism

**Compiled Flux code emits `Signal::Open`, runtime converts based on book side:**

```rust
impl Signal {
    pub fn to_order(&self, book_side: BookSide, bar: &Bar) -> Order {
        match (self, book_side) {
            (Signal::Open { symbol, quantity, .. }, BookSide::Long) => {
                Order::market_buy(*symbol, *quantity, bar.ask())
            }
            (Signal::Open { symbol, quantity, .. }, BookSide::Short) => {
                Order::market_sell(*symbol, *quantity, bar.bid())
            }
            // ... other combinations
        }
    }
}
```

### Lookahead Prevention (Runtime)

**Strategies receive bars one at a time, cannot peek ahead:**

```rust
pub trait Strategy {
    fn on_bar(&mut self, bar: &Bar, portfolio: &PortfolioState) -> Vec<Signal>;
}

// Backtester ensures strategies can't see future:
for bar in data_feed {
    let signals = strategy.on_bar(&bar, &portfolio.state());
    // Strategy only sees current bar, not next bars
}
```

## Performance Targets

| Operation | Target | Why |
|-----------|--------|-----|
| Process 1 bar | <10μs | Don't be bottleneck |
| Match 1 order | <1μs | High-frequency capability |
| Update portfolio | <500ns | Called per fill |
| Calculate SMA(20) | <100ns | Called every bar |
| Backtest 1 year daily | <1s | Interactive development |

Use `cargo bench -p flux-runtime` to verify.

## Common Tasks

### Adding Indicator

```rust
// indicators/moving_avg.rs

/// Simple Moving Average
///
/// # Example
/// ```
/// use flux_runtime::indicators::sma;
/// let prices = vec![1.0, 2.0, 3.0, 4.0, 5.0];
/// assert_eq!(sma(&prices, 3), 4.0);  // avg of [3,4,5]
/// ```
pub fn sma(prices: &[f64], period: usize) -> f64 {
    assert!(prices.len() >= period, "Insufficient data for SMA");
    prices[prices.len() - period..].iter().sum::<f64>() / period as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn sma_calculation() {
        let prices = vec![10.0, 20.0, 30.0, 40.0, 50.0];
        assert_eq!(sma(&prices, 3), 40.0);  // (30+40+50)/3
    }
    
    #[test]
    #[should_panic(expected = "Insufficient data")]
    fn sma_insufficient_data() {
        let prices = vec![10.0, 20.0];
        sma(&prices, 3);  // Should panic
    }
}
```

## Resources

**Required reading:**
1. `docs/architecture/06-runtime-library.md`
2. `docs/architecture/07-backtesting.md`
3. Performance targets in `PERFORMANCE.md` (empyrean-labs repo)

**Recommended:**
- Backtrader architecture (Python backtesting library)
- QuantLib (C++ quant library)

---

**Remember:** Financial correctness > performance. Get it right, then make it fast.
