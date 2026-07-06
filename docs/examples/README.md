# Flux Examples

Complete, self-contained strategy files demonstrating common trading patterns. Each example passes `flux check` and can be run with `flux backtest`.

| File | Description |
|------|-------------|
| [`sma_crossover.flux`](./sma_crossover.flux) | Simple moving average crossover strategy (short vs long period SMA) |
| [`mean_reversion.flux`](./mean_reversion.flux) | Z-score mean reversion strategy with volatility filter |
| [`rsi_strategy.flux`](./rsi_strategy.flux) | RSI overbought/oversold entry and exit strategy |
| [`multi_indicator.flux`](./multi_indicator.flux) | Multi-indicator combination (SMA trend + RSI momentum) |
| [`portfolio.flux`](./portfolio.flux) | Minimum-variance portfolio construction using covariance optimization |
