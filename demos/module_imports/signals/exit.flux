# signals/exit.flux — Exit signal detection
#
# Determines whether an open position should be closed.
# Uses a simple moving average crossover for exit timing.

from indicators import {sma}

# Exit signal: price has risen above the moving average by a threshold amount
# Returns 1.0 if we should exit, 0.0 otherwise
fn should_exit(lookback, z_threshold) {
    avg = sma(close, lookback)
    vol = stddev(close, lookback)

    # Compute z-score inline for exit check
    z = 0.0
    if vol > 0.001 {
        z = (close - avg) / vol
    }

    # Exit when price has reverted above threshold (mean reversion complete)
    if z > z_threshold {
        return 1.0
    }
    return 0.0
}
