# =============================================================================
# Live Connector — Type System in Streaming Mode Demo
# =============================================================================
#
# Demonstrates that ALL type system features work identically in both
# backtest mode (flux backtest) and live mode (flux live):
#   1. Enums — AlertLevel with unit and data variants
#   2. Match expressions — Destructuring AlertLevel for signal routing
#   3. Structs + Impl blocks — SessionState for tracking live session metrics
#   4. Traits — DataFilter interface for data quality gating
#   5. HashMap — Symbol metadata lookups at runtime
#
# This strategy includes BOTH a data block and a connector block:
#   - `flux backtest strategy.flux --data data.csv` uses the data block
#   - `flux live strategy.flux` uses the connector block (replay from CSV)
#
# The type system features behave identically in both modes — the only
# difference is where the bar data comes from.

from indicators import {sma}

# ---------------------------------------------------------------------------
# TYPE SYSTEM FEATURE: Enum definition
# AlertLevel classifies signal urgency. High carries a score (data variant),
# Low and None are unit variants. Demonstrates both variant kinds in one enum.
# NOTE: These features work identically in backtest and live modes.
# ---------------------------------------------------------------------------
enum AlertLevel {
    High(score: f64),
    Low,
    None
}

# ---------------------------------------------------------------------------
# TYPE SYSTEM FEATURE: Struct + Impl block
# SessionState tracks metrics about the current trading session.
# Instance methods update bar counts and compute session averages.
# NOTE: These features work identically in backtest and live modes.
# ---------------------------------------------------------------------------
struct SessionState {
    bars_processed: int,
    total_volume: f64,
    avg_price: f64
}

impl SessionState {
    fn new() -> SessionState {
        return SessionState {
            bars_processed = 0,
            total_volume = 0.0,
            avg_price = 0.0
        }
    }

    fn update(self, price: f64, vol: f64) -> SessionState {
        new_count = self.bars_processed + 1
        new_vol = self.total_volume + vol
        new_avg = ((self.avg_price * self.bars_processed) + price) / new_count
        return SessionState {
            bars_processed = new_count,
            total_volume = new_vol,
            avg_price = new_avg
        }
    }

    fn is_warmed_up(self) -> bool {
        return self.bars_processed > 20
    }
}

# ---------------------------------------------------------------------------
# TYPE SYSTEM FEATURE: Trait definition and implementation
# DataFilter defines an interface for data quality gating.
# Implementations can filter on different criteria (volume, spread, etc.).
# NOTE: These features work identically in backtest and live modes.
# ---------------------------------------------------------------------------
trait DataFilter {
    fn passes(self, price: f64, volume: f64) -> bool
}

struct VolumeFilter {
    min_volume: f64
}

impl DataFilter for VolumeFilter {
    fn passes(self, price: f64, volume: f64) -> bool {
        return volume >= self.min_volume
    }
}

fn classify_alert(price: f64, avg: f64, threshold: f64) -> AlertLevel {
    deviation = (price - avg) / avg
    if deviation > threshold {
        return AlertLevel.High(deviation)
    }
    if deviation > threshold / 2.0 {
        return AlertLevel.Low
    }
    return AlertLevel.None
}

# ---------------------------------------------------------------------------
# TYPE SYSTEM FEATURE: Dual-mode capability
# The data block is used by `flux backtest`. The connector block is used
# by `flux live`. Both feed bars to the SAME on bar handler below.
# ---------------------------------------------------------------------------
data {
    symbols = ["AAPL"]
    period = "6mo"
    interval = "1d"
    source = "yahoo"
}

# ---------------------------------------------------------------------------
# TYPE SYSTEM FEATURE: Connector block (for flux live mode)
# type = "replay" reads from a local CSV file, simulating a live feed.
# This lets you test live mode without needing a real WebSocket endpoint.
# ---------------------------------------------------------------------------
connector {
    type = "replay"
    file = "data.csv"
    symbols = ["AAPL"]
    interval = "1m"
}

strategy LiveMomentum {
    params {
        period = 20
        threshold = 0.03
        base_size = 100.0
        min_vol = 100000.0
    }

    state {
        bar_count = 0
    }

    on bar {
        bar_count = bar_count + 1

        # -------------------------------------------------------------------
        # TYPE SYSTEM FEATURE: HashMap
        # Symbol metadata lookup table. In a real strategy this might hold
        # tick sizes, lot sizes, or exchange information per symbol.
        # NOTE: These features work identically in backtest and live modes.
        # -------------------------------------------------------------------
        metadata = HashMap.new()
        metadata.insert("AAPL", 0.01)

        # Update session state
        session = SessionState.new()
        session = session.update(close, volume)

        # Apply data quality filter
        vol_filter = VolumeFilter { min_volume = min_vol }

        if session.is_warmed_up() and vol_filter.passes(close, volume) {
            avg = sma(close, period)

            # Get tick size from metadata
            if metadata.contains_key(symbol) {
                tick_size = metadata.get(symbol)
            }

            # Classify alert level
            alert = classify_alert(close, avg, threshold)

            # -----------------------------------------------------------
            # TYPE SYSTEM FEATURE: Match expression
            # Destructures AlertLevel — High(score) binds the score value.
            # Low and None are unit variants matched without bindings.
            # NOTE: These features work identically in backtest and live.
            # -----------------------------------------------------------
            match alert {
                AlertLevel.High(score) => {
                    size = base_size * score
                    if not in_position {
                        OPEN(symbol, size)
                    }
                }
                AlertLevel.Low => {
                    if in_position {
                        CLOSE(symbol)
                    }
                }
                _ => {
                    # No alert — hold current position
                }
            }
        }
    }
}
