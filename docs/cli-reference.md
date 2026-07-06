# CLI Reference

The Flux CLI provides commands for checking, building, backtesting, and scaffolding Flux strategy projects. All commands follow the pattern `flux <command> [arguments] [flags]`.

## Exit Codes

All Flux commands use a consistent set of exit codes:

| Code | Meaning |
|------|---------|
| 0    | Success |
| 1    | Compilation or runtime error (e.g., type errors, missing files) |
| 2    | Invalid usage (e.g., missing required arguments, unknown flags) |

---

## `flux check`

Run the Flux compiler front-end (lexer → parser → type checker) on a source file. Reports any errors with source-span annotations. Does not generate code.

### Usage

```
flux check <file>
```

### Arguments

| Argument | Required | Description |
|----------|----------|-------------|
| `file`   | Yes      | Path to the `.flux` source file to check |

### Exit Codes

- **0** — The file is valid; no lexer, parser, or type errors found.
- **1** — One or more compilation errors were detected.
- **2** — Invalid command-line usage (e.g., no file argument provided).

### Example

```bash
$ flux check strategies/sma_crossover.flux
strategies/sma_crossover.flux: ok
```

When errors are present, diagnostics are printed to stderr with source locations:

```bash
$ flux check strategies/broken.flux
error[strategies/broken.flux:3:12]: expected Bool, found Int
  |
3 |     if close + 1 {
  |            ^^^^^
```

---

## `flux build`

Compile a Flux source file through the full pipeline (lexer → parser → type checker → code generator) and emit generated Rust source code.

### Usage

```
flux build <file> [--output <path>]
```

### Arguments

| Argument | Required | Description |
|----------|----------|-------------|
| `file`   | Yes      | Path to the `.flux` source file to compile |

### Flags

| Flag              | Required | Default | Description |
|-------------------|----------|---------|-------------|
| `--output <path>` | No       | stdout  | Write generated Rust code to the specified file path. When omitted, generated code is printed to stdout. |

### Exit Codes

- **0** — Compilation succeeded; generated code was emitted.
- **1** — Compilation error (lexer, parser, type, or codegen error).
- **2** — Invalid command-line usage.

### Example

Print generated Rust code to stdout:

```bash
$ flux build strategies/sma_crossover.flux
use flux_runtime::prelude::*;

pub struct SmaCrossover {
    period: i64,
    bar_count: i64,
}

impl Strategy for SmaCrossover {
    fn on_bar(&mut self, ctx: &BarContext) -> Vec<Signal> {
        // ... generated implementation
    }
}
```

Write output to a file:

```bash
$ flux build strategies/sma_crossover.flux --output generated/sma.rs
```

---

## `flux backtest`

Compile and interpret a Flux strategy against historical CSV data, simulating trades through the PositionTracker. Produces signal logs, fill records, and a portfolio summary.

### Usage

```
flux backtest <file> --data <csv> [--capital <N>]
```

### Arguments

| Argument | Required | Description |
|----------|----------|-------------|
| `file`   | Yes      | Path to the `.flux` strategy source file |

### Flags

| Flag             | Required | Default  | Description |
|------------------|----------|----------|-------------|
| `--data <csv>`   | Yes      | —        | Path to the CSV data file containing OHLCV bars |
| `--capital <N>`  | No       | 10000.0  | Initial capital for portfolio tracking |

### Exit Codes

- **0** — Backtest completed successfully.
- **1** — Compilation error or runtime failure (e.g., CSV parse error).
- **2** — Invalid command-line usage (e.g., missing `--data` flag).

### Output Sections

The backtest output contains four sections:

1. **Signals** — Raw signals emitted by the strategy per bar (Open, Close, CloseQty)
2. **Fills** — Executed trades with side (BUY/SELL), symbol, quantity, and fill price
3. **Portfolio Summary** — Final equity, P&L breakdown, return percentage, and exposure
4. **Summary** — Signal count totals by type

### Example

```bash
$ flux backtest strategies/sma_crossover.flux --data data/sample.csv --capital 10000
--- Signals ---
  5 Open AAPL 100
  8 Close AAPL

--- Fills ---
  Bar    5 |  BUY | AAPL     100.00 @     187.30
  Bar    8 | SELL | AAPL     100.00 @     186.00

--- Portfolio Summary ---
  Initial Capital:      10000.00
  Final Equity:          9870.00
  Realized P&L:          -130.00
  Unrealized P&L:           0.00
  Total Return:          -1.30%
  Open Positions:              0
  Gross Exposure:           0.00
  Net Exposure:             0.00
  Total Fills:                 2

--- Summary ---
Total signals: 2
Open: 1
Close: 1
CloseQty: 0
```

---

## `flux init`

Scaffold a new Flux project with a standard directory structure, example strategy, and sample data.

### Usage

```
flux init [name]
```

### Arguments

| Argument | Required | Description |
|----------|----------|-------------|
| `name`   | No       | Project name. When omitted, uses the current directory name. Must contain only alphanumeric characters, hyphens, and underscores (max 64 characters). |

### Exit Codes

- **0** — Project created successfully.
- **1** — Error (e.g., directory not empty, invalid name).
- **2** — Invalid command-line usage.

### Generated Project Structure

```
my-strategy/
├── flux.toml              # Project manifest
├── strategies/
│   └── example.flux       # Example SMA crossover strategy
├── data/
│   └── sample.csv         # Sample OHLCV data (10 rows, AAPL)
├── README.md              # Quick-start instructions
└── .gitignore             # Ignores build artifacts and large data
```

### Example

Create a new project in a subdirectory:

```bash
$ flux init my-strategy
Created Flux project 'my-strategy' at /home/user/my-strategy
```

Initialize in the current (empty) directory:

```bash
$ mkdir my-project && cd my-project
$ flux init
Created Flux project 'my-project' at /home/user/my-project
```

---

## Error Handling

When a command receives invalid arguments (missing required flags, unknown options, or malformed values), the CLI exits with code **2** and prints a usage error message to stderr:

```bash
$ flux backtest strategies/sma.flux
error: the following required arguments were not provided:
  --data <csv>

Usage: flux backtest <file> --data <csv> [--capital <N>]

For more information, try '--help'.
```

Compilation errors (exit code 1) include source-span annotations pointing to the exact location of the problem:

```bash
$ flux check bad.flux
error[bad.flux:7:5]: undefined variable 'closee' (did you mean 'close'?)
  |
7 |     avg = sma(closee, period)
  |               ^^^^^^
```
