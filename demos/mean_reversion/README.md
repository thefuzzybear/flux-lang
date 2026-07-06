# Mean Reversion Demo — AAPL Daily

A z-score mean reversion strategy backtested on 1 year of AAPL daily data.

## Strategy

The strategy computes a rolling z-score over a 20-day lookback window:
- **Entry:** When z-score drops below -2.0 (price is 2σ below mean) and volatility is above the minimum threshold
- **Exit:** When z-score rises above +2.0 (price has reverted above mean)

## Running

```bash
# From the project root:
flux backtest demos/mean_reversion/strategy.flux \
  --data demos/mean_reversion/aapl_daily.csv \
  --capital 10000
```

Or with cargo:
```bash
cargo run -p flux-cli -- backtest demos/mean_reversion/strategy.flux \
  --data demos/mean_reversion/aapl_daily.csv \
  --capital 10000
```

## Refreshing Data

To download fresh AAPL data from Yahoo Finance:

```bash
pip install yfinance
python3 -c "
import yfinance as yf
import csv

df = yf.Ticker('AAPL').history(period='1y', interval='1d')
rows = [{'timestamp': d.strftime('%Y-%m-%d'), 'symbol': 'AAPL',
         'open': f'{r[\"Open\"]:.2f}', 'high': f'{r[\"High\"]:.2f}',
         'low': f'{r[\"Low\"]:.2f}', 'close': f'{r[\"Close\"]:.2f}',
         'volume': int(r['Volume'])} for d, r in df.iterrows()]

with open('demos/mean_reversion/aapl_daily.csv', 'w', newline='') as f:
    w = csv.DictWriter(f, fieldnames=['timestamp','symbol','open','high','low','close','volume'])
    w.writeheader()
    w.writerows(rows)
print(f'{len(rows)} rows written')
"
```

## Results

With default parameters (lookback=20, threshold=2.0, $10k capital, 100 shares per trade):

```
4 round-trip trades over ~1 year
Total Return: 52.80%
Win rate: 4/4 (100%)
```

## Tuning

Edit `strategy.flux` params to experiment:
- `lookback` — rolling window for z-score (try 10, 30, 50)
- `threshold` — entry/exit z-score threshold (try 1.5, 2.5, 3.0)
- `position_size` — shares per trade
- `min_volatility` — minimum stddev to enter (filters dead markets)
