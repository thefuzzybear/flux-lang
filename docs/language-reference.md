# Flux Language Reference

This document is the complete syntax and semantics reference for the Flux programming language.

## Primitive Types

Flux has four primitive types:

### Int

Whole numbers (64-bit signed integer).

```flux
strategy Example {
    params {
        period = 20
        count = -5
    }
    state {
        bar_count = 0
    }
    on bar {
        bar_count = bar_count + 1
    }
}
```

**Supported operations:** arithmetic (`+`, `-`, `*`, `/`, `%`), comparison (`==`, `!=`, `<`, `<=`, `>`, `>=`).

### Float

Floating-point numbers (64-bit double precision). Int values are automatically promoted to Float when used in mixed expressions.

```flux
strategy Example {
    params {
        threshold = 2.5
        ratio = -0.75
    }
    state {
        total = 0.0
    }
    on bar {
        total = total + close
    }
}
```

**Supported operations:** arithmetic (`+`, `-`, `*`, `/`, `%`), comparison (`==`, `!=`, `<`, `<=`, `>`, `>=`).

### String

Text values enclosed in double quotes.

```flux
strategy Example {
    params {
        target_symbol = "AAPL"
    }
    state {
        last_signal = "none"
    }
    on bar {
        last_signal = "open"
        OPEN(target_symbol, 100.0)
    }
}
```

**Supported operations:** equality comparison (`==`, `!=`), used as arguments to signal and built-in functions.

### Bool

Boolean values `true` and `false`.

```flux
strategy Example {
    params {
        use_filter = true
    }
    state {
        triggered = false
    }
    on bar {
        triggered = close > 100.0
        if triggered and not in_position {
            OPEN(symbol, 50.0)
        }
    }
}
```

**Supported operations:** logical (`and`, `or`, `not`), equality comparison (`==`, `!=`).

## Collection Types

### VecFloat

A one-dimensional vector of Float values. Used for return series, weight vectors, and other sequences of numeric data.

```flux
strategy Example {
    params {
        rf_rate = 0.02
    }
    state {
        returns = []
    }
    on bar {
        r = ret(symbol)
        returns = returns
        ratio = sharpe(returns, rf_rate)
    }
}
```

**Construction:**
- Empty vector: `[]`
- Pre-populated: `[1.0, 2.0, 3.0]`

**Indexing:** `vec[index]` where `index` is an Int (0-based).

### MatFloat

A two-dimensional matrix of Float values. Used for covariance matrices, correlation matrices, and other tabular numeric data.

```flux
strategy Example {
    params {
        lookback = 60
    }
    state {
        returns = []
    }
    on bar {
        r = ret(symbol)
        returns = returns
        cov = cov_matrix(returns, lookback)
        weights = min_variance_weights(cov, returns)
    }
}
```

**Construction:** Produced by portfolio functions such as `cov_matrix()` and `corr_matrix()`.

**Operations:** Passed to functions like `min_variance_weights()`, `portfolio_var()`, `mat_mul()`, `transpose()`, `inverse()`, and `det()`.

## Strategy Structure

Every Flux source file defines a single strategy. A strategy consists of four sections in the following order:

1. **`strategy` block** — the top-level container, given a name
2. **`params` block** — configurable constants (immutable during execution)
3. **`state` block** — mutable variables that persist across bars
4. **`on bar` handler** — logic executed once per market data bar

```flux
from indicators import {sma, ema}

strategy MyCrossover {
    params {
        short_period = 10
        long_period = 30
        position_size = 100.0
    }
    state {
        bar_count = 0
    }
    on bar {
        bar_count = bar_count + 1
        short_avg = sma(close, short_period)
        long_avg = sma(close, long_period)
        if short_avg > long_avg and not in_position {
            OPEN(symbol, position_size)
        }
        if short_avg < long_avg and in_position {
            CLOSE(symbol)
        }
    }
}
```

### `params` Block

Declares named parameters with default values. Parameters are constants — they cannot be reassigned during execution.

```flux
params {
    period = 20
    threshold = 2.5
    enabled = true
}
```

### `state` Block

Declares variables with initial values that persist across bars. State variables can be reassigned in `on bar`.

```flux
state {
    bar_count = 0
    total_volume = 0.0
    returns = []
}
```

### `on bar` Handler

The event handler called once for each bar of market data. All trading logic lives here. Context variables and signal functions are available within this block.

```flux
on bar {
    bar_count = bar_count + 1
    avg = sma(close, period)
    if close > avg and not in_position {
        OPEN(symbol, 100.0)
    }
}
```

## Operators

### Precedence Table

Operators listed from highest precedence (binds tightest) to lowest:

| Precedence | Category   | Operators                    | Associativity |
|------------|------------|------------------------------|---------------|
| 7          | Unary      | `-` (negate), `not`          | Right         |
| 6          | Multiply   | `*`, `/`, `%`                | Left          |
| 5          | Add        | `+`, `-`                     | Left          |
| 4          | Relational | `<`, `<=`, `>`, `>=`         | Left          |
| 3          | Equality   | `==`, `!=`                   | Left          |
| 2          | Logical AND| `and`                        | Left          |
| 1          | Logical OR | `or`                         | Left          |

### Arithmetic Operators

Operate on Int and Float values. When mixing Int and Float, the result is Float.

```flux
strategy ArithExample {
    params {
        base = 10
    }
    state {
        result = 0.0
    }
    on bar {
        result = (close + base) * 2.0
        remainder = base % 3
        diff = high - low
    }
}
```

| Operator | Description    | Example         |
|----------|---------------|-----------------|
| `+`      | Addition      | `a + b`         |
| `-`      | Subtraction   | `a - b`         |
| `*`      | Multiplication| `a * b`         |
| `/`      | Division      | `a / b`         |
| `%`      | Modulo        | `a % b`         |

### Comparison Operators

Compare two values of the same type. Return a Bool.

```flux
strategy CompExample {
    params {
        threshold = 50.0
    }
    state {
        above = false
    }
    on bar {
        above = close > threshold
        at_target = close == threshold
        in_range = close >= 45.0 and close <= 55.0
    }
}
```

| Operator | Description           | Example       |
|----------|-----------------------|---------------|
| `==`     | Equal                 | `a == b`      |
| `!=`     | Not equal             | `a != b`      |
| `<`      | Less than             | `a < b`       |
| `<=`     | Less than or equal    | `a <= b`      |
| `>`      | Greater than          | `a > b`       |
| `>=`     | Greater than or equal | `a >= b`      |

### Logical Operators

Operate on Bool values.

```flux
strategy LogicExample {
    params {
        use_volume_filter = true
    }
    state {
        signal = false
    }
    on bar {
        price_ok = close > 100.0
        volume_ok = volume > 1000000.0
        signal = price_ok and volume_ok or not use_volume_filter
    }
}
```

| Operator | Description  | Example         |
|----------|-------------|-----------------|
| `and`    | Logical AND | `a and b`       |
| `or`     | Logical OR  | `a or b`        |
| `not`    | Logical NOT | `not a`         |

## Control Flow

### if / elif / else

Conditional branching. The condition must evaluate to a Bool.

```flux
strategy IfExample {
    params {
        upper = 70.0
        lower = 30.0
    }
    state {
        zone = "neutral"
    }
    on bar {
        if close > upper {
            zone = "overbought"
            if in_position {
                CLOSE(symbol)
            }
        } elif close < lower {
            zone = "oversold"
            if not in_position {
                OPEN(symbol, 100.0)
            }
        } else {
            zone = "neutral"
        }
    }
}
```

**Syntax:**
```
if <condition> {
    <body>
} elif <condition> {
    <body>
} else {
    <body>
}
```

The `elif` and `else` clauses are optional. Multiple `elif` branches are allowed.

### for Loops

Iterate over a list.

```flux
strategy ForExample {
    params {
        weights_count = 3
    }
    state {
        values = [1.0, 2.0, 3.0]
        total = 0.0
    }
    on bar {
        total = 0.0
        for v in values {
            total = total + v
        }
    }
}
```

**Syntax:**
```
for <variable> in <iterable> {
    <body>
}
```

The iterable must be a list. The loop variable is scoped to the loop body.

### while Loops

Repeat while a condition is true.

```flux
strategy WhileExample {
    params {
        target = 100.0
    }
    state {
        accumulator = 0.0
        count = 0
    }
    on bar {
        accumulator = 0.0
        count = 0
        while accumulator < target and count < 10 {
            accumulator = accumulator + close
            count = count + 1
        }
    }
}
```

**Syntax:**
```
while <condition> {
    <body>
}
```

**Constraint:** While loops are limited to a maximum of **10,000 iterations** per execution. Exceeding this limit produces a runtime error.

## Variable Assignment and Mutability

### Initial Assignment

Variables are declared with initial values in `params` and `state` blocks. The type is inferred from the initial value.

```flux
params {
    period = 20        # Int
    threshold = 2.5    # Float
    name = "AAPL"      # String
    active = true      # Bool
}
state {
    counter = 0        # Int, mutable across bars
    prices = []        # VecFloat (empty initially)
}
```

### Reassignment in `on bar`

Variables declared in `state` can be reassigned in `on bar`. New local variables can also be introduced in `on bar` — they exist only for the current bar.

```flux
strategy MutabilityExample {
    params {
        multiplier = 2.0
    }
    state {
        running_total = 0.0
    }
    on bar {
        # Reassign state variable (persists to next bar)
        running_total = running_total + close

        # Local variable (exists only this bar)
        scaled = close * multiplier
        avg = running_total / 10.0
    }
}
```

**Rules:**
- `params` variables are **immutable** — they cannot be reassigned.
- `state` variables are **mutable** — they retain their value between bars.
- Local variables in `on bar` are created on first assignment and discarded at the end of the bar.

## Signal Functions

Signal functions dispatch trading signals. They are available inside `on bar`.

### OPEN(symbol, qty)

Opens a new position.

| Parameter | Type   | Description                        |
|-----------|--------|------------------------------------|
| `symbol`  | String | The asset symbol to trade          |
| `qty`     | Float  | The quantity to open               |

**Returns:** Signal

```flux
on bar {
    if close > sma(close, 20) and not in_position {
        OPEN(symbol, 100.0)
    }
}
```

### CLOSE(symbol)

Closes an entire position.

| Parameter | Type   | Description                        |
|-----------|--------|------------------------------------|
| `symbol`  | String | The asset symbol to close          |

**Returns:** Signal

```flux
on bar {
    if close < sma(close, 20) and in_position {
        CLOSE(symbol)
    }
}
```

### CLOSE_QTY(symbol, qty)

Closes a partial position. Equivalent to calling `CLOSE(symbol, qty)` with two arguments.

| Parameter | Type   | Description                        |
|-----------|--------|------------------------------------|
| `symbol`  | String | The asset symbol to partially close|
| `qty`     | Float  | The quantity to close              |

**Returns:** Signal

```flux
on bar {
    if close < sma(close, 50) and in_position {
        # Both forms are equivalent:
        CLOSE_QTY(symbol, 50.0)
        CLOSE(symbol, 50.0)
    }
}
```

## Import Syntax

Flux supports importing functions from modules using the `from ... import` syntax. Imported functions become available throughout the strategy.

```flux
from indicators import {sma, ema}
from stats import {zscore, stddev}

strategy ImportExample {
    params {
        period = 20
    }
    state {
        bar_count = 0
    }
    on bar {
        bar_count = bar_count + 1
        fast = ema(close, 10)
        slow = sma(close, period)
    }
}
```

**Syntax:**
```
from <module_path> import {<name1>, <name2>, ...}
```

- `<module_path>` — the module name (e.g., `indicators`, `stats`)
- `<name1>, <name2>` — comma-separated list of function names enclosed in braces

Imported functions accept a variable number of numeric arguments and return Float.

## Context Variables

The following variables are automatically available inside `on bar`. They represent the current bar's market data and position state.

| Variable       | Type   | Description                                         |
|----------------|--------|-----------------------------------------------------|
| `close`        | Float  | Closing price of the current bar                    |
| `open`         | Float  | Opening price of the current bar                    |
| `high`         | Float  | Highest price during the current bar                |
| `low`          | Float  | Lowest price during the current bar                 |
| `volume`       | Float  | Trading volume for the current bar                  |
| `symbol`       | String | The asset symbol for the current bar                |
| `in_position`  | Bool   | Whether a position is currently open for this symbol|

```flux
strategy ContextExample {
    params {
        min_volume = 500000.0
    }
    state {
        bar_count = 0
    }
    on bar {
        bar_count = bar_count + 1
        range = high - low
        mid = (high + low) / 2.0
        if volume > min_volume and not in_position {
            OPEN(symbol, 100.0)
        }
    }
}
```

## Constraints

| Constraint                    | Limit  | Behavior on violation                          |
|-------------------------------|--------|------------------------------------------------|
| While loop max iterations     | 10,000 | Runtime error: "while loop exceeded maximum iterations" |
