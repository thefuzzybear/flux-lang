# Built-in Functions Reference

Flux provides built-in functions for math operations, statistical indicators, and portfolio construction. These functions are available globally without imports (except `sma` and `ema`, which require `from indicators import {sma, ema}`).

All built-in functions are type-checked at compile time. The type checker rejects calls with incorrect argument types or wrong argument counts before any code is executed.

---

## Math Functions (Tier 1)

Core mathematical operations. Each accepts numeric arguments (Int or Float) and returns a Float.

### Single-Argument Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `abs` | `abs(x: Numeric) -> Float` | Returns the absolute value of `x`. |
| `sqrt` | `sqrt(x: Numeric) -> Float` | Returns the square root of `x`. |
| `exp` | `exp(x: Numeric) -> Float` | Returns Euler's number raised to the power `x`. |
| `log` | `log(x: Numeric) -> Float` | Returns the natural logarithm of `x`. |
| `floor` | `floor(x: Numeric) -> Float` | Returns the largest integer less than or equal to `x`. |
| `ceil` | `ceil(x: Numeric) -> Float` | Returns the smallest integer greater than or equal to `x`. |
| `round` | `round(x: Numeric) -> Float` | Returns `x` rounded to the nearest integer. |
| `sign` | `sign(x: Numeric) -> Float` | Returns -1.0, 0.0, or 1.0 indicating the sign of `x`. |

### Multi-Argument Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `pow` | `pow(base: Numeric, exp: Numeric) -> Float` | Returns `base` raised to the power `exp`. |
| `min` | `min(a: Numeric, b: Numeric) -> Float` | Returns the smaller of `a` and `b`. |
| `max` | `max(a: Numeric, b: Numeric) -> Float` | Returns the larger of `a` and `b`. |

> **Note:** `Numeric` means either `Int` or `Float` is accepted. The return type is always `Float`.

### Usage Example

```flux
on bar {
    log_return = log(close / open)
    range = abs(high - low)
    capped = min(close, 200.0)
    growth = pow(1.05, 10)
}
```

---

## Statistical Indicators (Tier 2)

Rolling statistical functions for time-series analysis. These accept numeric arguments (Int or Float) and return a Float. Internally, they maintain rolling state across bars.

### Imported Indicators

The `sma` and `ema` functions must be imported before use:

```flux
from indicators import {sma, ema}
```

| Function | Signature | Description |
|----------|-----------|-------------|
| `sma` | `sma(value: Numeric, period: Numeric) -> Float` | Simple moving average of `value` over `period` bars. |
| `ema` | `ema(value: Numeric, period: Numeric) -> Float` | Exponential moving average of `value` over `period` bars. |

### Global Statistical Functions

These are available without imports:

| Function | Signature | Description |
|----------|-----------|-------------|
| `stddev` | `stddev(value: Numeric, period: Numeric) -> Float` | Rolling standard deviation of `value` over `period` bars. |
| `variance` | `variance(value: Numeric, period: Numeric) -> Float` | Rolling variance of `value` over `period` bars. |
| `zscore` | `zscore(value: Numeric, period: Numeric) -> Float` | Z-score of `value` relative to its rolling mean and standard deviation over `period` bars. |
| `rsi` | `rsi(value: Numeric, period: Numeric) -> Float` | Relative Strength Index of `value` over `period` bars (0–100 scale). |
| `atr` | `atr(high: Numeric, low: Numeric, close: Numeric, period: Numeric) -> Float` | Average True Range computed from high, low, and close over `period` bars. |
| `corr` | `corr(a: Numeric, b: Numeric, period: Numeric) -> Float` | Rolling Pearson correlation between series `a` and `b` over `period` bars. |
| `covariance` | `covariance(a: Numeric, b: Numeric, period: Numeric) -> Float` | Rolling covariance between series `a` and `b` over `period` bars. |

> **Note:** All indicator functions use `VariadicNumeric` parameter typing — they accept any number of numeric arguments. The semantic interpretation (which argument is "value" vs "period") is determined at runtime.

### Usage Example

```flux
from indicators import {sma, ema}

strategy MeanReversion {
    params {
        lookback = 20
        threshold = 2.0
    }

    state {
        bar_count = 0
    }

    on bar {
        bar_count = bar_count + 1
        avg = sma(close, lookback)
        fast_avg = ema(close, 10)
        z = zscore(close, lookback)
        vol = stddev(close, lookback)
        strength = rsi(close, 14)

        if z < 0.0 - threshold and not in_position {
            OPEN(symbol, 100.0)
        }
        if z > threshold and in_position {
            CLOSE(symbol)
        }
    }
}
```

---

## Portfolio Operations (Tier 3)

Functions for portfolio construction, risk analysis, and matrix operations. Unlike Tier 1 and Tier 2 functions, portfolio operations use fixed parameter types (`VecFloat`, `MatFloat`, `Int`, `Float`).

### Matrix Operations

| Function | Signature | Description |
|----------|-----------|-------------|
| `mat_mul` | `mat_mul(a: MatFloat, b: MatFloat) -> MatFloat` | Matrix multiplication of `a` and `b`. |
| `transpose` | `transpose(m: MatFloat) -> MatFloat` | Transpose of matrix `m`. |
| `inverse` | `inverse(m: MatFloat) -> MatFloat` | Inverse of matrix `m`. |
| `det` | `det(m: MatFloat) -> Float` | Determinant of matrix `m`. |

### Portfolio Construction

| Function | Signature | Description |
|----------|-----------|-------------|
| `cov_matrix` | `cov_matrix(returns: VecFloat, period: Int) -> MatFloat` | Computes the covariance matrix from asset returns over `period` bars. |
| `corr_matrix` | `corr_matrix(returns: VecFloat, period: Int) -> MatFloat` | Computes the correlation matrix from asset returns over `period` bars. |
| `min_variance_weights` | `min_variance_weights(cov: MatFloat, constraints: VecFloat) -> VecFloat` | Computes minimum-variance portfolio weights given a covariance matrix and constraint bounds. |
| `portfolio_var` | `portfolio_var(weights: VecFloat, cov: MatFloat) -> Float` | Computes portfolio variance given weight vector and covariance matrix. |
| `sharpe` | `sharpe(returns: VecFloat, rf_rate: Float) -> Float` | Computes the Sharpe ratio of `returns` using `rf_rate` as the risk-free rate. |

### Usage Example

```flux
strategy MinVariance {
    params {
        lookback = 60
        rf = 0.02
    }

    state {
        bar_count = 0
    }

    on bar {
        bar_count = bar_count + 1
        returns = [ret("AAPL"), ret("GOOG"), ret("MSFT")]
        cov = cov_matrix(returns, lookback)
        constraints = [0.0, 0.0, 0.0]
        weights = min_variance_weights(cov, constraints)
        risk = portfolio_var(weights, cov)
        ratio = sharpe(returns, rf)

        if ratio > 1.5 and not in_position {
            OPEN(symbol, 100.0)
        }
    }
}
```

---

## Data Accessor

| Function | Signature | Description |
|----------|-----------|-------------|
| `ret` | `ret(symbol: String) -> Float` | Returns the simple return for `symbol`, computed as `(current_close / previous_close) - 1.0`. Returns 0.0 when no previous close is available (first bar). |

### Usage Example

```flux
on bar {
    r = ret("AAPL")
    if r > 0.05 {
        OPEN("AAPL", 50.0)
    }
}
```

---

## Error Behavior

The Flux type checker validates all built-in function calls at compile time. Errors are reported with source location spans and descriptive messages.

### Wrong Argument Type

Functions with `VariadicNumeric` parameters (Tier 1 math and Tier 2 indicators) require all arguments to be numeric (`Int` or `Float`). Passing a non-numeric type produces a compile error:

```
error: 'sqrt' argument 1 must be numeric, found String
  --> strategy.flux:5:14
```

Functions with `Fixed` parameters (Tier 3 portfolio operations and `ret`) require exact type matches:

```
error: 'cov_matrix' argument 1 must be VecFloat, found Float
  --> strategy.flux:8:22
```

```
error: 'ret' argument 1 must be String, found Int
  --> strategy.flux:4:10
```

### Wrong Argument Count

Functions with fixed parameter lists reject calls with incorrect argument counts:

```
error: 'portfolio_var' expects 2 arguments, found 3
  --> strategy.flux:9:5
```

> **Note:** `VariadicNumeric` functions (Tier 1 and Tier 2) accept any number of numeric arguments at the type-checking level. Argument count validation for these functions (e.g., `pow` requires exactly 2 arguments) is enforced at runtime.

### Period Constraints (Runtime)

Statistical indicators that require a `period` argument enforce the following at runtime:

- Period must be a positive integer (≥ 1)
- If fewer bars have been observed than the specified period, the function returns 0.0 (insufficient data)

### Domain Errors (Runtime)

- `sqrt(x)` where `x < 0` returns `NaN`
- `log(x)` where `x <= 0` returns `NaN` or `-Infinity`
- `inverse(m)` where `m` is singular returns a matrix of `NaN` values

---

## Summary Table

| Category | Functions | Param Style | Available Without Import |
|----------|-----------|-------------|--------------------------|
| Math | abs, sqrt, exp, log, floor, ceil, round, sign, pow, min, max | VariadicNumeric | Yes |
| Indicators (imported) | sma, ema | VariadicNumeric | No — requires `from indicators import` |
| Indicators (global) | stddev, variance, zscore, rsi, atr, corr, covariance | VariadicNumeric | Yes |
| Matrix ops | mat_mul, transpose, inverse, det | Fixed | Yes |
| Portfolio | cov_matrix, corr_matrix, min_variance_weights, portfolio_var, sharpe | Fixed | Yes |
| Data accessor | ret | Fixed | Yes |
