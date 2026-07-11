# =============================================================================
# Pairs Trading — Type System Kitchen Sink Demo
# =============================================================================
#
# Demonstrates ALL six type system features in a single strategy:
#   1. Enums — Signal type with Buy/Sell/Hold variants
#   2. Match expressions — Pattern matching on Signal for routing
#   3. Structs + Impl blocks — PairState with methods for spread tracking
#   4. Traits — SignalGenerator interface with multiple implementations
#   5. Generics — (demonstrated via trait polymorphism)
#   6. HashMap — Symbol pair registry for key-value lookup
#
# Trading Logic:
#   Tracks the rolling spread between two correlated symbols.
#   Computes a z-score and generates signals when spread deviates.
#   Uses enum-based signal routing to determine position sizing.

from indicators import {sma}

# ---------------------------------------------------------------------------
# TYPE SYSTEM FEATURE: Enum definition
# Enums are discriminated unions — each variant can hold different data.
# Signal has two data variants (Buy/Sell carry a strength value) and one
# unit variant (Hold carries no data).
# ---------------------------------------------------------------------------
enum Signal {
    Buy(strength: f64),
    Sell(strength: f64),
    Hold
}

# ---------------------------------------------------------------------------
# TYPE SYSTEM FEATURE: Struct + Impl block
# Structs group related data. Impl blocks attach methods to a struct.
# PairState tracks the rolling spread statistics for a symbol pair.
# Instance methods take `self` as the first parameter.
# Static methods (like `new`) do not take `self` — they construct instances.
# ---------------------------------------------------------------------------
struct PairState {
    mean_spread: f64,
    current_spread: f64,
    z_score: f64,
    lookback: int
}

impl PairState {
    # Static method — no `self`, used as a constructor
    fn new(lookback: int) -> PairState {
        return PairState {
            mean_spread = 0.0,
            current_spread = 0.0,
            z_score = 0.0,
            lookback = lookback
        }
    }

    # Instance method — takes `self`, computes derived value
    fn calculate_zscore(self, spread: f64, avg_spread: f64, std_spread: f64) -> f64 {
        if std_spread > 0.0 {
            return (spread - avg_spread) / std_spread
        }
        return 0.0
    }

    # Instance method — updates internal state
    fn update(self, spread: f64, avg: f64, std: f64) -> PairState {
        z = self.calculate_zscore(spread, avg, std)
        return PairState {
            mean_spread = avg,
            current_spread = spread,
            z_score = z,
            lookback = self.lookback
        }
    }
}

# ---------------------------------------------------------------------------
# TYPE SYSTEM FEATURE: Trait definition and implementation
# Traits define an interface — a set of method signatures that types must
# implement. This enables polymorphism: different structs can satisfy the
# same interface with different logic.
# SignalGenerator defines how to produce a Signal from spread data.
# ---------------------------------------------------------------------------
trait SignalGenerator {
    fn generate(self, z_score: f64, threshold: f64) -> Signal
}

# ZScoreGenerator: produces signals based on z-score threshold crossings
struct ZScoreGenerator {
    sensitivity: f64
}

impl SignalGenerator for ZScoreGenerator {
    fn generate(self, z_score: f64, threshold: f64) -> Signal {
        if z_score < 0.0 - threshold {
            return Signal.Buy(0.0 - z_score * self.sensitivity)
        }
        if z_score > threshold {
            return Signal.Sell(z_score * self.sensitivity)
        }
        return Signal.Hold
    }
}

# MomentumGenerator: alternative signal logic based on spread momentum
struct MomentumGenerator {
    momentum_weight: f64
}

impl SignalGenerator for MomentumGenerator {
    fn generate(self, z_score: f64, threshold: f64) -> Signal {
        adjusted = z_score * self.momentum_weight
        if adjusted < 0.0 - threshold {
            return Signal.Buy(0.0 - adjusted)
        }
        if adjusted > threshold {
            return Signal.Sell(adjusted)
        }
        return Signal.Hold
    }
}

data {
    symbols = ["AAPL", "MSFT"]
    period = "1y"
    interval = "1d"
    source = "yahoo"
}

strategy PairsTrading {
    params {
        lookback = 20
        threshold = 2.0
        base_size = 100.0
    }

    state {
        bar_count = 0
    }

    on bar {
        bar_count = bar_count + 1

        # -------------------------------------------------------------------
        # TYPE SYSTEM FEATURE: HashMap
        # HashMap is a built-in associative container for key-value lookups.
        # Here we use it as a pair registry mapping symbol names to their
        # hedge ratios. Demonstrates: new(), insert(), get(), contains_key().
        # -------------------------------------------------------------------
        pair_registry = HashMap.new()
        pair_registry.insert("AAPL", 1.0)
        pair_registry.insert("MSFT", -0.85)

        # Check if the current symbol is in our pair registry
        if pair_registry.contains_key(symbol) {
            hedge_ratio = pair_registry.get(symbol)

            # Compute spread using SMA
            spread = close - sma(close, lookback) * hedge_ratio
            avg_spread = sma(spread, lookback)
            std_spread = stddev(spread, lookback)

            # Use PairState to track and compute z-score
            pair = PairState.new(lookback)
            pair = pair.update(spread, avg_spread, std_spread)

            # Choose signal generator (using ZScoreGenerator here)
            gen = ZScoreGenerator { sensitivity = 1.0 }
            signal = gen.generate(pair.z_score, threshold)

            # -----------------------------------------------------------
            # TYPE SYSTEM FEATURE: Match expression
            # Match destructures enum variants and binds associated data.
            # The `strength` variable is bound from Signal.Buy(strength)
            # and Signal.Sell(strength) arms. The `_` wildcard catches
            # any unmatched variants (here, Signal.Hold).
            # -----------------------------------------------------------
            match signal {
                Signal.Buy(strength) => {
                    if not in_position {
                        size = base_size * strength
                        OPEN(symbol, size)
                    }
                }
                Signal.Sell(strength) => {
                    if in_position {
                        CLOSE(symbol)
                    }
                }
                _ => {
                    # Hold — do nothing
                }
            }
        }
    }
}
