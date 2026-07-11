# Regime Detector — Trait-Bounded Generics for Pluggable Market Classification

Classifies the market into Bull, Bear, or Sideways regimes using pluggable detection
algorithms. A generic function `detect_regime[T: RegimeDetector]` accepts any struct
that implements the `RegimeDetector` trait, demonstrating how generics and trait bounds
work together for type-safe polymorphism. Position sizing varies by regime: aggressive
in Bull markets, defensive in Bear markets, and tight mean reversion in Sideways markets.

## Features Demonstrated

| Feature | Description | Lines |
|---------|-------------|-------|
| Enum | `Regime` with unit variants Bull, Bear, Sideways | L25–L29 |
| Struct + Impl | `RegimeState` tracking regime duration and transitions | L40–L78 |
| Trait definition | `RegimeDetector` interface for classification algorithms | L85–L87 |
| Trait impl #1 | `TrendDetector` — moving average crossover detection | L90–L105 |
| Trait impl #2 | `VolatilityDetector` — realized vol threshold detection | L108–L123 |
| Generics + Trait bounds | `fn detect_regime[T: RegimeDetector](detector: T, ...)` | L131–L133 |
| Match expression | Regime-based position sizing routing | L169–L188 |

## Project Structure

```
demos/regime_detector/
├── strategy.flux    # Regime detection strategy with generics focus
├── data.csv         # 150 rows of SPY OHLCV data with trending/ranging phases
└── README.md
```

## Running

```bash
# Type-check
cargo run -p flux-cli -- check demos/regime_detector/strategy.flux

# Backtest
cargo run -p flux-cli -- backtest demos/regime_detector/strategy.flux \
  --data demos/regime_detector/data.csv --capital 100000
```

## Code Walkthrough

### Enum: Discrete Market States (L25–L29)

```flux
enum Regime {
    Bull,
    Bear,
    Sideways
}
```

All three variants are unit variants (no associated data). This models a categorical
classification — the market is in exactly one of three states at any time. Unlike the
Pairs Trading demo's `Signal` enum (which carries strength data), `Regime` is purely
categorical.

### Struct + Impl: Regime State Tracking (L40–L78)

```flux
struct RegimeState {
    current_regime: int,
    duration: int,
    transitions: int
}

impl RegimeState {
    fn new() -> RegimeState { ... }
    fn update(self, new_regime: int) -> RegimeState { ... }
    fn regime_strength(self) -> f64 { ... }
}
```

`RegimeState` encapsulates regime evolution over time. The `new()` static method
constructs a default state starting in Sideways. The `update()` instance method
records a new regime detection, and `regime_strength()` returns a confidence score
based on how long the current regime has persisted (longer duration = higher confidence).

### Trait: RegimeDetector Interface (L85–L87)

```flux
trait RegimeDetector {
    fn detect(self, fast_avg: f64, slow_avg: f64, volatility: f64) -> Regime
}
```

This trait defines the contract for any regime classification algorithm. Any struct
implementing `RegimeDetector` must provide a `detect` method that takes market
indicators and returns a `Regime` variant.

### Trait Implementations: Two Detection Algorithms (L90–L123)

**TrendDetector** (L90–L105) classifies based on moving average crossover percentage.
When the fast MA leads the slow MA by more than `crossover_pct`, the market is Bull;
when it lags by that amount, Bear; otherwise Sideways.

**VolatilityDetector** (L108–L123) classifies based on realized volatility thresholds.
High volatility maps to Bear (risk-off), low volatility to Bull (calm trend), and
medium volatility to Sideways.

Both structs satisfy the same `RegimeDetector` trait but use completely different
logic — this is the core value of traits as interfaces.

### Generics with Trait Bounds (L131–L133)

```flux
fn detect_regime[T: RegimeDetector](detector: T, fast: f64, slow: f64, vol: f64) -> Regime {
    return detector.detect(fast, slow, vol)
}
```

This is the key feature of this demo. The type parameter `[T: RegimeDetector]` means:
- `T` can be **any** type that implements the `RegimeDetector` trait
- The compiler verifies at each call site that the concrete type satisfies the bound
- The function body can call any method defined in the `RegimeDetector` trait on `detector`

In the strategy's `on bar` handler, this generic function is called twice with
different concrete types:

```flux
# Call with TrendDetector (concrete type #1)
trend_det = TrendDetector { crossover_pct = 0.02 }
regime = detect_regime(trend_det, fast_avg, slow_avg, vol)

# Call with VolatilityDetector (concrete type #2)
vol_det = VolatilityDetector { high_vol_threshold = 3.0, low_vol_threshold = 1.0 }
alt_regime = detect_regime(vol_det, fast_avg, slow_avg, vol)
```

The compiler monomorphizes the generic function — generating specialized versions
for each concrete type used. This gives the flexibility of polymorphism with the
performance of static dispatch.

### Match Expression: Regime-Based Routing (L169–L188)

```flux
match regime {
    Regime.Bull => {
        if not in_position {
            OPEN(symbol, bull_size)
        }
    }
    Regime.Bear => {
        if in_position {
            CLOSE(symbol)
        }
    }
    Regime.Sideways => {
        if close < slow_avg and not in_position {
            OPEN(symbol, sideways_size)
        }
        if close > fast_avg and in_position {
            CLOSE(symbol)
        }
    }
}
```

The match expression routes to different position sizing strategies:
- **Bull**: Open aggressively with `bull_size` (200 shares)
- **Bear**: Close existing positions defensively
- **Sideways**: Use tight mean reversion with `sideways_size` (100 shares)

Each arm handles exactly one `Regime` variant, making the match exhaustive. The
compiler can verify that all variants are covered, preventing unhandled cases at
compile time.
