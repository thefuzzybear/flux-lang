# RSI Strategy — Overbought/Oversold Mean Reversion
#
# The Relative Strength Index (RSI) measures momentum on a 0-100 scale.
# When RSI drops below 30, the asset is considered "oversold" — selling
# pressure may be exhausted, signaling a potential entry. When RSI rises
# above 70, the asset is "overbought" — buying pressure may be exhausted,
# signaling a potential exit.
#
# This strategy:
#   1. Enters a long position when RSI falls below the oversold threshold
#   2. Exits the position when RSI rises above the overbought threshold

strategy RsiStrategy {
    # Configurable parameters — adjust these to tune sensitivity
    params {
        rsi_period = 14          # Lookback period for RSI calculation
        overbought = 70.0        # RSI above this level triggers exit
        oversold = 30.0          # RSI below this level triggers entry
        position_size = 100.0    # Number of shares to buy on entry
    }

    # Persistent state across bars
    state {
        bar_count = 0            # Track how many bars we have processed
    }

    # Called once per bar with market data (close, open, high, low, volume, symbol)
    on bar {
        # Increment bar counter for tracking
        bar_count = bar_count + 1

        # Calculate the RSI indicator for the current bar
        # rsi() returns a value between 0 and 100
        strength = rsi(close, rsi_period)

        # Entry condition: RSI below oversold threshold and no open position
        # This suggests the asset may be undervalued and due for a bounce
        if strength < oversold and not in_position {
            OPEN(symbol, position_size)
        }

        # Exit condition: RSI above overbought threshold and holding a position
        # This suggests the asset may be overvalued and due for a pullback
        if strength > overbought and in_position {
            CLOSE(symbol)
        }
    }
}
