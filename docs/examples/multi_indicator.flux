# Multi-Indicator Combination Strategy
#
# This strategy combines a trend filter (SMA) with a momentum oscillator (RSI)
# to generate trading signals. The idea is to only buy during established uptrends
# when momentum suggests a temporary dip (oversold), and to exit when momentum
# signals the move is exhausted (overbought).
#
# Logic:
#   - BUY when price is above the SMA (uptrend confirmed)
#     AND RSI is below the oversold threshold (momentum dip)
#   - SELL when RSI rises above the overbought threshold
#     OR price drops below the SMA (trend reversal)
#
# This approach filters out false signals that either indicator would
# generate alone — the SMA keeps you on the right side of the trend,
# while RSI times the entries and exits.

from indicators import {sma}

strategy MultiIndicator {
    # Configurable parameters for tuning the strategy
    params {
        sma_period = 50        # Lookback period for the trend filter
        rsi_period = 14        # Lookback period for RSI momentum
        oversold = 30.0        # RSI level indicating oversold condition
        overbought = 70.0      # RSI level indicating overbought condition
        position_size = 100.0  # Number of shares per trade
    }

    # Persistent state across bars
    state {
        bar_count = 0
    }

    on bar {
        bar_count = bar_count + 1

        # Calculate the trend filter — SMA smooths price to show direction
        trend = sma(close, sma_period)

        # Calculate momentum — RSI measures speed of price changes (0-100)
        strength = rsi(close, rsi_period)

        # Entry: price above SMA (uptrend) AND RSI oversold (dip buying)
        # Both conditions must be true — this is the compound filter
        if close > trend and strength < oversold and not in_position {
            OPEN(symbol, position_size)
        }

        # Exit: RSI overbought (momentum exhausted) OR trend lost
        # Either condition triggers an exit to protect gains
        if in_position and (strength > overbought or close < trend) {
            CLOSE(symbol)
        }
    }
}
