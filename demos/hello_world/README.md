# Hello World Demo — The Simplest Flux Strategy

Opens a position on the very first bar and holds it forever. Use this to verify your Flux installation works end-to-end.

## What You'll Learn

- The minimal structure of a Flux strategy (just a `strategy` block and `on bar` handler)
- How `OPEN(symbol, quantity)` emits a buy signal
- How `in_position` tracks whether you already hold a position
- That a valid strategy needs no `params` or `state` blocks

## Strategy

The strategy has no parameters and no state. On every bar, it checks `if not in_position` — since no position exists on bar 1, it fires a single `OPEN` signal for 100 shares. On all subsequent bars the condition is false, so it simply holds.

## Running

```bash
# From the project root:
flux backtest demos/hello_world/strategy.flux \
  --data demos/hello_world/sample_data.csv \
  --capital 10000
```

Or with cargo:
```bash
cargo run -p flux-cli -- backtest demos/hello_world/strategy.flux \
  --data demos/hello_world/sample_data.csv \
  --capital 10000
```

## Expected Output

You should see:

- **1 OPEN signal** on bar 1 (buys 100 shares of AAPL at 186.20)
- **No further signals** — the position is held through all 10 bars
- **Final equity** reflects unrealized P&L from holding AAPL as it drifts upward
- Exit code 0 (no errors)

The strategy never closes, so at the end of the backtest the position remains open with unrealized gains.

## Next Steps

Ready to make strategies configurable? Head to the [`params_and_state`](../params_and_state/) demo to learn how `params` and `state` blocks let you control behavior without editing code.
