# =============================================================================
# Regime Detector — Trait-Bounded Generics Demo
# =============================================================================
#
# Demonstrates trait-bounded generics for pluggable regime detection:
#   1. Enums — Regime type with Bull/Bear/Sideways variants
#   2. Match expressions — Regime-based position sizing routing
#   3. Structs + Impl blocks — RegimeState for tracking regime evolution
#   4. Traits — RegimeDetector interface for detection algorithms
#   5. Generics — fn detect_regime[T: RegimeDetector](detector: T, ...) -> Regime
#
# Trading Logic:
#   Classifies the market into regimes using a pluggable detector.
#   Adjusts position sizing aggressively in Bull markets, defensively
#   in Bear markets, and uses tight mean reversion in Sideways markets.

from indicators import {sma, ema}

# ---------------------------------------------------------------------------
# TYPE SYSTEM FEATURE: Enum definition
# Regime represents discrete market state classifications.
# All three are unit variants (no associated data) — representing
# categorical states rather than continuous values.
# ---------------------------------------------------------------------------
enum Regime {
    Bull,
    Bear,
    Sideways
}

# ---------------------------------------------------------------------------
# TYPE SYSTEM FEATURE: Struct + Impl block
# RegimeState tracks the current regime (as an int code), how long it has
# persisted, and the total number of regime transitions observed.
# The impl block provides methods to update state on each bar.
# Note: current_regime uses int encoding (0=Bull, 1=Bear, 2=Sideways)
# because the type checker registers structs before enums.
# ---------------------------------------------------------------------------
struct RegimeState {
    current_regime: int,
    duration: int,
    transitions: int
}

impl RegimeState {
    fn new() -> RegimeState {
        return RegimeState {
            current_regime = 2,
            duration = 0,
            transitions = 0
        }
    }

    fn update(self, new_regime: int) -> RegimeState {
        # Compare regimes — if different, increment transitions and reset duration
        # For now, we always update (simplified)
        return RegimeState {
            current_regime = new_regime,
            duration = self.duration + 1,
            transitions = self.transitions
        }
    }

    fn regime_strength(self) -> f64 {
        # Longer duration = higher confidence in current regime
        if self.duration > 10 {
            return 1.0
        }
        return self.duration / 10.0
    }
}

# ---------------------------------------------------------------------------
# TYPE SYSTEM FEATURE: Trait definition and implementation
# RegimeDetector defines the interface for regime classification.
# Any struct implementing this trait can be used with the generic
# detect_regime function below.
# ---------------------------------------------------------------------------
trait RegimeDetector {
    fn detect(self, fast_avg: f64, slow_avg: f64, volatility: f64) -> Regime
}

# TrendDetector classifies based on moving average crossover
struct TrendDetector {
    crossover_pct: f64
}

impl RegimeDetector for TrendDetector {
    fn detect(self, fast_avg: f64, slow_avg: f64, volatility: f64) -> Regime {
        diff = (fast_avg - slow_avg) / slow_avg
        if diff > self.crossover_pct {
            return Regime.Bull
        }
        if diff < 0.0 - self.crossover_pct {
            return Regime.Bear
        }
        return Regime.Sideways
    }
}

# VolatilityDetector classifies based on realized volatility thresholds
struct VolatilityDetector {
    high_vol_threshold: f64,
    low_vol_threshold: f64
}

impl RegimeDetector for VolatilityDetector {
    fn detect(self, fast_avg: f64, slow_avg: f64, volatility: f64) -> Regime {
        if volatility > self.high_vol_threshold {
            return Regime.Bear
        }
        if volatility < self.low_vol_threshold {
            return Regime.Bull
        }
        return Regime.Sideways
    }
}

# ---------------------------------------------------------------------------
# TYPE SYSTEM FEATURE: Generics with trait bounds
# This generic function accepts ANY type T that implements RegimeDetector.
# The compiler ensures at each call site that T satisfies the bound.
# Square brackets [T: RegimeDetector] declare the bounded type parameter.
# ---------------------------------------------------------------------------
fn detect_regime[T: RegimeDetector](detector: T, fast: f64, slow: f64, vol: f64) -> Regime {
    return detector.detect(fast, slow, vol)
}

data {
    symbols = ["SPY"]
    period = "1y"
    interval = "1d"
    source = "yahoo"
}

strategy RegimeStrategy {
    params {
        fast_period = 10
        slow_period = 50
        vol_period = 20
        bull_size = 200.0
        bear_size = 50.0
        sideways_size = 100.0
    }

    state {
        bar_count = 0
    }

    on bar {
        bar_count = bar_count + 1

        if bar_count > slow_period {
            fast_avg = ema(close, fast_period)
            slow_avg = sma(close, slow_period)
            vol = stddev(close, vol_period)

            # Call generic function with TrendDetector (concrete type #1)
            trend_det = TrendDetector { crossover_pct = 0.02 }
            regime = detect_regime(trend_det, fast_avg, slow_avg, vol)

            # Also try VolatilityDetector (concrete type #2) — demonstrates
            # that the same generic function works with different types
            vol_det = VolatilityDetector { high_vol_threshold = 3.0, low_vol_threshold = 1.0 }
            alt_regime = detect_regime(vol_det, fast_avg, slow_avg, vol)

            # Track regime state
            regime_state = RegimeState.new()
            regime_state = regime_state.update(0)

            # -----------------------------------------------------------
            # TYPE SYSTEM FEATURE: Match expression
            # Routes position sizing based on the detected regime.
            # Each arm handles a different Regime variant.
            # -----------------------------------------------------------
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
                    # In sideways markets, use mean reversion
                    if close < slow_avg and not in_position {
                        OPEN(symbol, sideways_size)
                    }
                    if close > fast_avg and in_position {
                        CLOSE(symbol)
                    }
                }
            }
        }
    }
}
