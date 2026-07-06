# CSV Data Format

This document specifies the CSV format required by `flux backtest --data <file>`.

## Required Columns

Every CSV file must contain a header row with the following seven columns:

| Column      | Data Type | Description                                      |
|-------------|-----------|--------------------------------------------------|
| `timestamp` | String    | Date or datetime identifying the bar (e.g. `2024-01-02`) |
| `symbol`    | String    | Ticker or asset identifier (e.g. `AAPL`)         |
| `open`      | Numeric   | Opening price for the bar                        |
| `high`      | Numeric   | Highest price during the bar                     |
| `low`       | Numeric   | Lowest price during the bar                      |
| `close`     | Numeric   | Closing price for the bar                        |
| `volume`    | Numeric   | Trading volume during the bar                    |

Numeric columns accept integer or decimal values (e.g. `186.20`, `1200000`).

## Case-Insensitive Column Names

Column names in the header are matched **case-insensitively**. All of the following headers are equivalent:

```csv
timestamp,symbol,open,high,low,close,volume
Timestamp,Symbol,Open,High,Low,Close,Volume
TIMESTAMP,SYMBOL,OPEN,HIGH,LOW,CLOSE,VOLUME
TimeStamp,SYMBOL,open,HIGH,low,CLOSE,Volume
```

## Any-Order Columns

Columns may appear in any order. The parser locates each required column by name, not by position. For example, this is valid:

```csv
volume,close,low,high,open,symbol,timestamp
1000000,153.0,149.0,155.0,150.0,AAPL,2024-01-01
```

## Multi-Asset Format

To backtest strategies across multiple assets, include rows for each symbol at the same timestamp grouped consecutively. The `symbol` column distinguishes which asset each row belongs to.

```csv
timestamp,symbol,open,high,low,close,volume
2024-01-02,AAPL,185.50,186.75,185.10,186.20,1200000
2024-01-02,MSFT,375.00,377.50,374.25,376.80,800000
2024-01-03,AAPL,186.20,187.00,185.80,186.50,1100000
2024-01-03,MSFT,376.80,378.00,376.00,377.20,750000
```

Rows sharing the same timestamp are processed together as a single bar event, with the strategy's `on bar` handler invoked once per symbol in that group.

## Complete Example

Below is a valid CSV file with data for a single asset over five trading days:

```csv
timestamp,symbol,open,high,low,close,volume
2024-01-02,AAPL,185.50,186.75,185.10,186.20,1200000
2024-01-03,AAPL,186.20,187.00,185.80,186.50,1100000
2024-01-04,AAPL,186.50,186.90,184.75,185.00,1350000
2024-01-05,AAPL,185.00,185.50,183.20,183.80,1500000
2024-01-08,AAPL,183.80,185.00,183.50,184.90,1250000
```

## Error Behavior for Missing Columns

If any of the seven required columns are absent from the header row, the CLI reports an error listing the missing column names and exits with a non-zero status. The error message follows this format:

```
missing required columns: ["high", "low", "close", "volume"]
```

All missing columns are reported together in a single error — the parser does not stop at the first missing column.

## Extra Columns

Any columns beyond the seven required ones are silently ignored. You can include additional data (e.g. `adjusted_close`, `dividends`, `splits`) without affecting parsing:

```csv
timestamp,symbol,open,high,low,close,volume,adjusted_close,dividends
2024-01-02,AAPL,185.50,186.75,185.10,186.20,1200000,186.20,0.00
2024-01-03,AAPL,186.20,187.00,185.80,186.50,1100000,186.50,0.00
```

The parser reads only the required columns and discards the rest.
