# SMA/EMA Crossover

A moving average crossover strategy that enters long when a fast EMA crosses above a slow SMA, and exits on the reverse crossover.

## Strategy

The strategy computes two moving averages on each bar:
- **Fast line:** EMA(10) — responds quickly to recent price changes
- **Slow line:** SMA(30) — smooths out noise over a longer window

When the fast line crosses above the slow line, it signals upward momentum and opens a long position. When the fast line crosses below the slow line, momentum has shifted and the position is closed.

## What You'll Learn

- Importing and using built-in indicators (`sma`, `ema`)
- The difference between SMA and EMA
- Crossover detection logic using previous-bar state
- Warm-up periods — why indicators need history before producing valid signals

## SMA vs EMA

**Simple Moving Average (SMA)** is the unweighted arithmetic mean of the last *N* closing prices. Every bar in the window contributes equally:

```
SMA(30) = (close[0] + close[1] + ... + close[29]) / 30
```

SMA is stable and easy to reason about, but it reacts slowly to sudden price moves because old bars carry the same weight as recent ones.

**Exponential Moving Average (EMA)** applies exponentially decreasing weights to older bars. Recent prices contribute more, so EMA reacts faster to new information. The smoothing factor is `α = 2 / (period + 1)`:

```
EMA(today) = close * α + EMA(yesterday) * (1 - α)
```

With `fast_period = 10`, the EMA gives ~18% weight to the current bar. The SMA(30) gives each bar only ~3.3%. This difference in responsiveness is what makes crossovers meaningful — the fast line leads and the slow line confirms the trend.

## Crossover Logic

A **bullish crossover** occurs when the fast EMA moves from below (or equal to) the slow SMA to above it. The strategy detects this by comparing the current and previous bar values:

```flux
if prev_fast <= prev_slow and fast > slow and not in_position {
    OPEN(symbol, position_size)
}
```

A **bearish crossover** is the reverse — fast drops below slow:

```flux
if prev_fast >= prev_slow and fast < slow and in_position {
    CLOSE(symbol)
}
```

This uses `state` variables (`prev_fast`, `prev_slow`) to remember the previous bar's indicator values, making the cross detection a simple comparison between two consecutive bars.

## Warm-Up Period

The slow SMA requires 30 bars of history before it produces a meaningful value. During the first 30 bars, the indicators are still "warming up" — their output isn't reliable because they haven't seen enough data.

The strategy handles this with a bar counter:

```flux
if bar_count > slow_period {
    # Only trade after warm-up
}
```

This means the earliest possible trade signal is on bar 31. Your data file needs at least 30+ rows before any crossover can be detected. The sample data includes 60 bars to allow for warm-up plus multiple crossover opportunities.

## Running

```bash
flux backtest demos/indicators/strategy.flux \
  --data demos/indicators/sample_data.csv \
  --capital 10000
```

Or with cargo:

```bash
cargo run -p flux-cli -- backtest demos/indicators/strategy.flux \
  --data demos/indicators/sample_data.csv \
  --capital 10000
```

## Expected Output

After the 30-bar warm-up period, you should see crossover signals as the fast EMA reacts to price movements before the slow SMA. Expect 2-4 round-trip trades depending on the price data's trending behavior.

## Experiments

Try adjusting the moving average periods to see how sensitivity changes:

| Fast | Slow | Effect |
|------|------|--------|
| 5 | 20 | More responsive, more trades, more whipsaws in choppy markets |
| 10 | 30 | Default — balanced between signal frequency and noise filtering |
| 20 | 50 | Slower, fewer trades, catches only major trend shifts |

Edit the `params` block in `strategy.flux`:

```flux
params {
    fast_period = 5    # try 5, 10, 20
    slow_period = 20   # try 20, 30, 50
    position_size = 100.0
}
```

Shorter periods generate more crossovers (and more false signals in sideways markets). Longer periods filter out noise but enter trends later. There's no universally "best" setting — it depends on market conditions.

## Next Steps

Move on to the [`multi_symbol`](../multi_symbol/) demo to learn how Flux handles trading multiple assets in a single strategy, with per-symbol position tracking and independent signal generation.
