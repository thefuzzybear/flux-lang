# Dual Mode — Backtest & Live with One Strategy

A momentum strategy that works in both backtest mode (historical CSV data) and live mode (websocket stream) without changing any strategy logic.

## Strategy

DualMomentum computes a 20-period SMA of closing prices. When price crosses above the average, it opens a long position. When price drops below the average, it closes. The key insight: this logic is identical whether the bars come from a CSV file or a live websocket feed.

## What You'll Learn

- How the `data` block provides historical bars for backtesting
- How the `connector` block provides live bars for real-time trading
- How Flux separates data sourcing from strategy logic so you write once, run both ways

## How It Works

Flux strategies can include both a `data` block and a `connector` block:

```flux
data {
    symbols = ["AAPL"]
    period = "1y"
    interval = "1d"
    source = "yahoo"
}

connector {
    type = "websocket"
    url = "wss://stream.example.com/v1"
    symbols = ["AAPL"]
    interval = "1m"
}
```

- **`data` block** — Used by `flux backtest`. Defines what historical data to load (source, symbols, period, interval). The backtest engine reads bars from CSV and feeds them to your `on bar` handler sequentially.
- **`connector` block** — Used by `flux run`. Defines a live data source (websocket, poll, etc.). The live harness connects to the endpoint and feeds real-time bars to the same `on bar` handler.

Your strategy logic in the `on bar` block doesn't know or care where the bars come from. It just processes each bar the same way.

## Running — Backtest Mode

Use `flux backtest` to run against historical data:

```bash
flux backtest demos/dual_mode/strategy.flux \
  --data demos/dual_mode/sample_data.csv \
  --capital 10000
```

Or with cargo:

```bash
cargo run -p flux-cli -- backtest demos/dual_mode/strategy.flux \
  --data demos/dual_mode/sample_data.csv \
  --capital 10000
```

This uses the `data` block configuration. The `connector` block is ignored in backtest mode.

## Running — Live Mode

Use `flux run` to connect to a live data feed:

```bash
flux run demos/dual_mode/strategy.flux
```

> **Note:** The `connector` block in this demo uses a placeholder URL (`wss://stream.example.com/v1`). To run live, replace it with a real websocket endpoint that streams market data. The `data` block is ignored in live mode.

## Same Logic, Two Modes

The strategy block is completely unchanged between modes:

```flux
strategy DualMomentum {
    params {
        period = 20
        position_size = 100.0
    }

    state {
        bar_count = 0
    }

    on bar {
        bar_count = bar_count + 1
        avg = sma(close, period)

        if bar_count > period {
            if close > avg and not in_position {
                OPEN(symbol, position_size)
            }
            if close < avg and in_position {
                CLOSE(symbol)
            }
        }
    }
}
```

Whether you run `flux backtest` or `flux run`, this exact logic executes. The only difference is where bars come from — the runtime handles that for you.

## Expected Output (Backtest)

With the included sample data (30 bars of AAPL daily data with a trending pattern), you should see momentum-based entries and exits after the 20-bar warm-up period.

## Next Steps

Ready to run multiple strategies simultaneously? See the [`multi_strategy`](../multi_strategy/) demo to learn how TOML configuration files let you orchestrate several strategies with shared risk limits.
