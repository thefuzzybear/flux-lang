# Multi-Symbol Demo — AAPL & MSFT Momentum

A simple momentum strategy applied to multiple assets simultaneously, demonstrating how Flux handles multi-symbol trading.

## Strategy

The strategy computes a 20-period SMA for each symbol independently. When a symbol's closing price crosses above its SMA, the strategy opens a position in that symbol. When the price drops below the SMA, it closes. The same logic runs for every symbol in the data block — Flux handles each asset's bars separately.

## What You'll Learn

- How the `data` block's `symbols` array declares multiple assets
- How the `symbol` variable reflects which asset the current bar belongs to
- How position tracking works independently per symbol
- How to pass multiple `--data` files for multi-asset backtesting

## How Multi-Symbol Works

### The `symbols` Array

In the `data` block, the `symbols` array tells Flux which assets this strategy trades:

```flux
data {
    symbols = ["AAPL", "MSFT"]
    period = "1y"
    interval = "1d"
    source = "yahoo"
}
```

When backtesting, you provide a separate CSV file for each symbol. The runtime merges bars from all files in chronological order and feeds them to the strategy one at a time.

### The `symbol` Variable

Inside the `on bar` handler, the built-in `symbol` variable always contains the ticker of the current bar being processed. This is how the strategy knows which asset it's looking at:

```flux
on bar {
    avg = sma(close, lookback)

    if close > avg and not in_position {
        OPEN(symbol, position_size)   # Opens position in the current bar's asset
    }
}
```

If the current bar is for AAPL, `symbol` equals `"AAPL"`. On the next bar (which might be MSFT), `symbol` equals `"MSFT"`. The same strategy logic applies to both — no branching needed unless you want symbol-specific behavior.

### Per-Symbol Position Tracking

Flux tracks positions independently for each symbol. The `in_position` variable reflects whether the strategy currently holds a position in the *current* bar's symbol. Opening a position in AAPL does not affect `in_position` when processing an MSFT bar (and vice versa).

This means the strategy can be long AAPL and flat MSFT at the same time, or hold positions in both simultaneously.

## Running

```bash
flux backtest demos/multi_symbol/strategy.flux \
  --data demos/multi_symbol/aapl_data.csv \
  --data demos/multi_symbol/msft_data.csv \
  --capital 10000
```

Or with cargo:
```bash
cargo run -p flux-cli -- backtest demos/multi_symbol/strategy.flux \
  --data demos/multi_symbol/aapl_data.csv \
  --data demos/multi_symbol/msft_data.csv \
  --capital 10000
```

Note the `--data` flag is passed twice — once for each symbol's data file. The runtime loads both files and merges them by timestamp.

## Expected Output

After the 20-bar SMA warm-up period, you should see:
- OPEN/CLOSE signals for both AAPL and MSFT independently
- Each symbol's trades triggered by its own price crossing its own SMA
- Portfolio summary showing combined P&L across both assets

## Experiments

- **Change `lookback`** to 10 for faster signals (more trades, more noise) or 50 for slower signals (fewer trades, bigger moves)
- **Add a third symbol** by adding it to the `symbols` array and providing another `--data` file
- **Add symbol-specific logic** using conditionals on `symbol` (e.g., different position sizes per asset)

## Next Steps

Ready for the next concept? Check out the [dual_mode demo](../dual_mode/) to learn how a single strategy can work in both backtest and live modes using `data` and `connector` blocks together.
