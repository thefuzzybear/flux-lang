# Multi-Strategy TOML Demo

Run multiple strategies simultaneously using a TOML configuration file with shared risk controls.

## Strategy

This demo deploys two independent strategies under a single harness:
- **Momentum** (AAPL) — trend-following, buys breakouts above the 20-period SMA
- **Reversion** (MSFT) — mean-reversion, buys oversold dips when z-score drops below -2.0

A replay connector feeds both strategies from the same data file, simulating a live multi-strategy deployment without needing a real market feed.

## What You'll Learn

- TOML configuration format for multi-strategy orchestration
- Running multiple strategies with independent symbol assignments
- Risk management configuration (position limits, exposure caps)
- Replay connectors for safe offline testing
- How to backtest strategies independently vs. running them together

## TOML Configuration Breakdown

The `config.toml` file has four sections:

### Global Settings

```toml
capital = 50000.0
state_file = "harness_state.json"
```

- `capital` — total starting capital shared across all strategies
- `state_file` — where the harness persists state between restarts (positions, fills, equity)

### Risk Configuration

```toml
[risk]
max_position_size = 1000.0
max_exposure = 100000.0
max_positions = 5
```

- `max_position_size` — maximum shares/units in any single position
- `max_exposure` — maximum total dollar exposure across all positions
- `max_positions` — maximum number of concurrent open positions

The risk section acts as a global safety net. If any strategy attempts to exceed these limits, the signal is rejected.

### Strategies Array

```toml
[[strategies]]
path = "momentum.flux"
symbols = ["AAPL"]

[[strategies]]
path = "reversion.flux"
symbols = ["MSFT"]
```

Each `[[strategies]]` entry defines:
- `path` — relative path to the `.flux` file
- `symbols` — which symbols this strategy trades

Strategies run independently. Each only sees bars for its assigned symbols.

### Connectors Array

```toml
[[connectors]]
kind = "replay"
file = "sample_data.csv"
symbols = ["AAPL", "MSFT"]
playback_rate = 0.0
```

- `kind` — connector type (`replay` for file-based, `websocket` for live feeds)
- `file` — CSV file to replay (for replay connectors)
- `symbols` — which symbols this connector provides
- `playback_rate` — seconds between bars (`0.0` = as fast as possible)

## Running

Run both strategies together in replay mode:

```bash
flux live --config demos/multi_strategy/config.toml
```

This starts the live harness with the replay connector, feeding bars from `sample_data.csv` to both strategies simultaneously.

## Backtesting Each Strategy Independently

Each strategy has its own `data` block, so you can backtest them in isolation:

```bash
# Backtest momentum strategy (AAPL)
flux backtest demos/multi_strategy/momentum.flux \
  --data demos/multi_strategy/sample_data.csv \
  --capital 25000

# Backtest reversion strategy (MSFT)
flux backtest demos/multi_strategy/reversion.flux \
  --data demos/multi_strategy/sample_data.csv \
  --capital 25000
```

This lets you tune each strategy's parameters independently before combining them.

## Adding a Third Strategy

To add another strategy to the deployment:

1. Create a new `.flux` file (e.g., `breakout.flux`) in the same directory:

```flux
from indicators import {sma}

data {
    symbols = ["GOOG"]
    period = "1y"
    interval = "1d"
    source = "yahoo"
}

strategy Breakout {
    params {
        period = 10
        position_size = 50.0
    }

    state {
        bar_count = 0
    }

    on bar {
        bar_count = bar_count + 1
        avg = sma(close, period)

        if bar_count > period {
            if close > avg * 1.02 and not in_position {
                OPEN(symbol, position_size)
            }
            if close < avg and in_position {
                CLOSE(symbol)
            }
        }
    }
}
```

2. Add a new `[[strategies]]` entry to `config.toml`:

```toml
[[strategies]]
path = "breakout.flux"
symbols = ["GOOG"]
```

3. Update the connector to include the new symbol:

```toml
[[connectors]]
kind = "replay"
file = "sample_data.csv"
symbols = ["AAPL", "MSFT", "GOOG"]
```

4. Add GOOG rows to `sample_data.csv` (or point to a separate data source).

## Risk Configuration Options

| Option | Type | Description |
|--------|------|-------------|
| `max_position_size` | Float | Max shares/units per position. Rejects OPEN signals that would exceed this. |
| `max_exposure` | Float | Max total dollar value across all open positions. Prevents over-leveraging. |
| `max_positions` | Integer | Max number of concurrent positions. Additional OPENs are rejected once hit. |

These limits apply globally across all strategies. A momentum OPEN that would push total exposure past `max_exposure` is blocked even if the reversion strategy has plenty of room.

## Experiments

- Set `max_positions = 1` to force strategies to compete for a single slot
- Increase `capital` to `100000.0` and observe how both strategies scale
- Change `playback_rate` to `1.0` to watch bars arrive in real-time
- Swap symbol assignments (give momentum MSFT and reversion AAPL)

## Previous Demo

← [Dual Mode](../dual_mode/) — writing strategies that work in both backtest and live modes
