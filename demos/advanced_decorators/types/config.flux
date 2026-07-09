# =============================================================================
# Strategy Configuration — Immutable, Zero-Init Patterns
# =============================================================================

# @immutable — Compile-Time Mutation Prevention
# After construction, no field can be reassigned. The compiler rejects any
# `config.field = value` statement at type-check time.
# Use for strategy parameters that should never change mid-execution.
# Prevents accidental state corruption in complex strategies.

@immutable
struct StrategyConfig {
    max_spread_bps: f64,
    min_imbalance: f64,
    position_limit: f64,
    skew_factor: f64,
    fade_ticks: int
}

# @zero_init — Guaranteed Zero Initialization
# All fields are zero-initialized by type (f64→0.0, int→0, bool→false).
# Use for accumulators, counters, and statistics structs that must start
# from a clean state each session. No need to manually set every field.

@zero_init
struct SessionStats {
    total_trades: int,
    total_volume: f64,
    pnl: f64,
    max_drawdown: f64,
    win_count: int,
    loss_count: int
}
