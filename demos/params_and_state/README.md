# Params & State — Configurable Entry Timing

A timed entry strategy that demonstrates how params make strategies configurable and how state persists values across bars.

## Strategy

The `TimedEntry` strategy waits a configurable number of bars before opening a position, then holds for a configurable duration before closing. It uses `bar_count` in state to track progress and `entry_bar` to remember when it entered.

## What You'll Learn

- **params block** — Define strategy-level constants that control behavior without editing code
- **state block** — Declare variables that persist their values across bars (unlike locals, which reset each bar)
- How params and state work together to create configurable, stateful strategies

## Key Concepts

### Params

Parameters are declared once and referenced throughout your strategy logic. They act as configurable constants — change a value in the `params` block and the entire strategy adapts:

```flux
params {
    wait_bars = 5        # How many bars to wait before entering
    hold_bars = 10       # How many bars to hold before exiting
    position_size = 100.0
}
```

This means you can tune when the strategy enters and exits without touching any logic code.

### State

State variables remember their values between bar iterations. Without state, you'd have no way to count bars or remember when you entered a position:

```flux
state {
    bar_count = 0    # Incremented each bar — persists across iterations
    entry_bar = 0    # Records which bar we entered on
}
```

Each time `on bar` fires, state variables retain whatever value they had at the end of the previous bar.

## Running

```bash
flux backtest demos/params_and_state/strategy.flux \
  --data demos/params_and_state/sample_data.csv \
  --capital 10000
```

Or with cargo:

```bash
cargo run -p flux-cli -- backtest demos/params_and_state/strategy.flux \
  --data demos/params_and_state/sample_data.csv \
  --capital 10000
```

## Expected Output

With the default parameters (`wait_bars=5`, `hold_bars=10`):

- Bar 5: OPEN signal (waited 5 bars, enters position)
- Bar 15: CLOSE signal (held for 10 bars, exits)
- Bar 16: OPEN signal (immediately re-enters since `bar_count >= wait_bars`)
- Bar 26: CLOSE signal (held another 10 bars)

You should see multiple entry/exit cycles over the 30+ bar dataset.

## Experiments

Try changing the params in `strategy.flux` to see how behavior shifts:

| Change | Effect |
|--------|--------|
| `wait_bars = 2` | Enters almost immediately — only waits 2 bars |
| `hold_bars = 20` | Holds much longer before exiting — fewer round trips |
| `wait_bars = 1, hold_bars = 5` | Rapid cycling — enters on bar 1, exits on bar 6, repeats |
| `position_size = 50.0` | Smaller position — less capital at risk per trade |

These tweaks change strategy behavior entirely through params alone — no logic edits needed.

## Next Steps

Ready to use technical indicators? Head to the [indicators demo](../indicators/) to learn about SMA/EMA crossover strategies.
