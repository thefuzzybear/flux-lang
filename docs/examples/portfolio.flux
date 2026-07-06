# Minimum-Variance Portfolio Construction Strategy
#
# This strategy demonstrates portfolio construction using modern portfolio
# theory. Instead of picking a single stock, it allocates capital across
# multiple assets to minimize overall portfolio risk (variance).
#
# Key concepts:
#   - ret(symbol) returns the simple return for a given asset:
#     (current_close / previous_close) - 1.0
#   - VecFloat literals [a, b, c] collect multiple values into a vector
#   - cov_matrix(returns, period) estimates the covariance matrix from
#     a vector of asset returns over a rolling window
#   - min_variance_weights(cov, constraints) solves for the portfolio
#     weights that minimize variance, subject to lower-bound constraints
#   - portfolio_var(weights, cov) computes the portfolio variance given
#     weights and the covariance matrix
#   - sharpe(returns, rf_rate) computes the Sharpe ratio: a measure of
#     risk-adjusted return (higher is better)
#
# Logic:
#   - Each bar, collect simple returns for three assets
#   - Estimate the covariance matrix over a lookback window
#   - Solve for minimum-variance weights (all weights >= 0)
#   - Compute portfolio risk and Sharpe ratio
#   - Enter when risk-adjusted performance is attractive (Sharpe > 1.0)
#   - Exit when risk-adjusted performance deteriorates (Sharpe < 0.5)

strategy MinVariancePortfolio {
    # Parameters for the portfolio optimizer
    params {
        lookback = 60          # Rolling window for covariance estimation
        rf_rate = 0.02         # Annualized risk-free rate for Sharpe ratio
        position_size = 100.0  # Capital allocated when entering
    }

    # State persists across bars
    state {
        bar_count = 0
    }

    on bar {
        # Count bars processed — useful for ensuring enough history
        bar_count = bar_count + 1

        # Collect simple returns for each asset in our universe.
        # ret(symbol) computes (current_close / prev_close) - 1.0
        # The result is a VecFloat — a vector of floating-point values.
        returns = [ret("AAPL"), ret("GOOG"), ret("MSFT")]

        # Estimate the covariance matrix from the return series.
        # cov_matrix uses a rolling window of `lookback` bars to compute
        # pairwise covariances between all assets in the vector.
        cov = cov_matrix(returns, lookback)

        # Define lower-bound constraints for the optimizer.
        # Each value corresponds to the minimum weight for that asset.
        # Setting all to 0.0 means no short-selling is allowed.
        constraints = [0.0, 0.0, 0.0]

        # Solve for the minimum-variance portfolio weights.
        # These weights sum to 1.0 and minimize total portfolio variance
        # subject to the constraints above.
        weights = min_variance_weights(cov, constraints)

        # Compute the portfolio variance — a scalar measure of risk.
        # Lower variance means the portfolio is better diversified.
        risk = portfolio_var(weights, cov)

        # Compute the Sharpe ratio for risk-adjusted performance.
        # Sharpe = (portfolio return - rf_rate) / portfolio volatility.
        # A ratio above 1.0 indicates attractive risk-adjusted returns.
        ratio = sharpe(returns, rf_rate)

        # Entry: Sharpe ratio exceeds 1.0 — risk-adjusted performance
        # is strong enough to justify holding a position.
        if ratio > 1.0 and not in_position {
            OPEN(symbol, position_size)
        }

        # Exit: Sharpe ratio drops below 0.5 — performance no longer
        # justifies the risk, so we close and wait for better conditions.
        if ratio < 0.5 and in_position {
            CLOSE(symbol)
        }
    }
}
