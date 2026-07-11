# Live Connector — Type System in Streaming Mode

A momentum strategy that classifies alert urgency, filters data quality, and
tracks session state — demonstrating that ALL type system features work
identically whether you run in backtest mode (`flux backtest`) or live streaming
mode (`flux live`). The same `on bar` handler receives bars from either the
`data` block or the `connector` block with no code changes.

## Features Demonstrated

| Feature | Description | Lines |
|---------|-------------|-------|
| Enum (unit + data variants) | `AlertLevel` with `High(score: f64)`, `Low`, `None` | L28–L32 |
| Struct + Impl block | `SessionState` with `new()`, `update()`, `is_warmed_up()` methods | L41–L72 |
| Trait + Implementation | `DataFilter` trait with `VolumeFilter` impl for data quality gating | L80–L93 |
| HashMap | Runtime symbol metadata lookups via `new()`, `insert()`, `contains_key()`, `get()` | L147–L148, L161–L162 |
| Connector block | `type = "replay"` for live mode — reads bars from a local CSV | L122–L128 |
| Data block | Backward-compatible backtest source — same strategy, different data path | L110–L115 |
| Match expression | Destructures `AlertLevel.High(score)` to size positions dynamically | L175–L189 |

## Dual-Mode Capability

This demo includes **both** a `data` block and a `connector` block:

- **Backtest mode** (`flux backtest`): Uses the `data` block to load historical bars from CSV. Great for validating strategy logic against past prices.
- **Live mode** (`flux live`): Uses the `connector` block with `type = "replay"` to simulate a streaming data source. The replay connector reads the same CSV line-by-line as if it were a live feed.

The type system features (enums, structs, traits, match, HashMap) behave identically
in both modes. The only difference is _where_ the bar data originates.

## Project Structure

```
demos/live_connector/
├── strategy.flux          # Streaming momentum strategy with all type features
├── data.csv               # 100 rows AAPL daily OHLCV (backtest + replay source)
└── README.md              # This file
```

## Running

```bash
# Type-check
cargo run -p flux-cli -- check demos/live_connector/strategy.flux

# Backtest mode (uses data block)
cargo run -p flux-cli -- backtest demos/live_connector/strategy.flux \
  --data demos/live_connector/data.csv --capital 100000

# Live replay mode (uses connector block)
cargo run -p flux-cli -- live demos/live_connector/strategy.flux
```

## Code Walkthrough

### Enum — AlertLevel (L28–L32)

```flux
enum AlertLevel {
    High(score: f64),
    Low,
    None
}
```

`AlertLevel` demonstrates both variant kinds in a single enum. `High` is a **data variant** carrying a deviation score; `Low` and `None` are **unit variants** with no associated data. The `classify_alert` helper function (L95–L104) constructs each variant based on price deviation from the moving average.

### Struct + Impl — SessionState (L41–L72)

```flux
struct SessionState { bars_processed: int, total_volume: f64, avg_price: f64 }

impl SessionState {
    fn new() -> SessionState { ... }
    fn update(self, price: f64, vol: f64) -> SessionState { ... }
    fn is_warmed_up(self) -> bool { ... }
}
```

`SessionState` tracks live session metrics. The impl block attaches a static constructor (`new`) and two instance methods (`update`, `is_warmed_up`). Instance methods accept `self` as the first parameter. The strategy creates and updates a `SessionState` each bar to decide when enough data has been seen to start trading.

### Trait + Implementation — DataFilter (L80–L93)

```flux
trait DataFilter {
    fn passes(self, price: f64, volume: f64) -> bool
}

impl DataFilter for VolumeFilter {
    fn passes(self, price: f64, volume: f64) -> bool {
        return volume >= self.min_volume
    }
}
```

`DataFilter` defines an interface for data quality gating. `VolumeFilter` implements it by rejecting bars below a minimum volume threshold. This pattern lets you swap in alternative filters (spread-based, time-based) without changing the strategy logic.

### HashMap — Symbol Metadata (L147–L148)

```flux
metadata = HashMap.new()
metadata.insert("AAPL", 0.01)
```

A `HashMap` stores per-symbol metadata (tick sizes, lot sizes, etc.). The strategy uses `contains_key` and `get` to retrieve tick size at runtime. In a multi-symbol strategy this table would hold exchange-specific configuration.

### Connector Block — Replay Source (L122–L128)

```flux
connector {
    type = "replay"
    file = "data.csv"
    symbols = ["AAPL"]
    interval = "1m"
}
```

The `connector` block configures live data ingestion. `type = "replay"` reads bars from a local CSV file, simulating a live WebSocket feed without needing a real endpoint. This lets you test live-mode behavior (bar-by-bar streaming, session state) entirely offline.

### Data Block — Backtest Source (L110–L115)

```flux
data {
    symbols = ["AAPL"]
    period = "6m"
    interval = "1d"
    source = "yahoo"
}
```

The `data` block provides backward-compatible backtesting. When you run `flux backtest`, this block specifies the data source. The connector block is ignored in backtest mode and vice versa — giving you dual-mode capability from a single strategy file.

### Match Expression — Signal Routing (L175–L189)

```flux
match alert {
    AlertLevel.High(score) => {
        size = base_size * score
        if not in_position {
            OPEN(symbol, size)
        }
    }
    AlertLevel.Low => {
        if in_position {
            CLOSE(symbol)
        }
    }
    _ => {
        # No alert — hold current position
    }
}
```

The match expression destructures `AlertLevel` variants. The `High(score)` arm **binds** the score value and uses it to compute position size dynamically. `Low` and `_` (wildcard) arms handle exit and hold logic respectively. This pattern replaces nested if/else chains with exhaustive, type-safe branching.
