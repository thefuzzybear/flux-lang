# std/engine/metrics.flux — Standardized performance metrics computation
#
# Provides metrics computation for backtest results: Sharpe ratio,
# max drawdown, win rate, profit factor, and trade P&L analysis.
# Works with Fill data from any engine fidelity level.

from engine::types import {Fill, OrderSide, PositionState}

# --- Structs ---

struct Metrics {
    sharpe_ratio: f64,
    max_drawdown_pct: f64,
    win_rate: f64,
    profit_factor: f64,
    avg_trade_pnl: f64,
    total_trades: int,
    total_pnl: f64
}

# --- Public API ---

# Compute all performance metrics from fill history and equity curve.
# Returns a Metrics struct with Sharpe ratio, max drawdown, win rate,
# profit factor, average trade P&L, total trades, and total P&L.
fn compute_metrics(fills: list, equity_curve: list) -> Metrics {
    # Edge case: no fills means no trades
    if fills.len() == 0 {
        return Metrics {
            sharpe_ratio = 0.0,
            max_drawdown_pct = 0.0,
            win_rate = 0.0,
            profit_factor = 0.0,
            avg_trade_pnl = 0.0,
            total_trades = 0,
            total_pnl = 0.0
        }
    }

    # Compute daily returns from equity curve
    returns = []
    i = 1
    while i < equity_curve.len() {
        prev = equity_curve[i - 1]
        curr = equity_curve[i]
        if prev > 0.0 {
            daily_ret = (curr - prev) / prev
            returns.push(daily_ret)
        }
        i = i + 1
    }

    # Compute Sharpe ratio from daily returns
    sharpe_val = compute_sharpe(returns)

    # Compute max drawdown from equity curve
    max_dd = compute_max_drawdown(equity_curve)

    # Compute round-trip trade P&Ls by pairing buys with sells
    trade_pnls = compute_trade_pnls(fills)
    total_trades = trade_pnls.len()

    # Aggregate trade statistics
    total_pnl = 0.0
    wins = 0
    gross_profit = 0.0
    gross_loss = 0.0

    for pnl in trade_pnls {
        total_pnl = total_pnl + pnl
        if pnl > 0.0 {
            wins = wins + 1
            gross_profit = gross_profit + pnl
        } else {
            gross_loss = gross_loss + abs(pnl)
        }
    }

    # Win rate: proportion of profitable trades
    win_rate = 0.0
    if total_trades > 0 {
        win_rate = wins * 1.0 / total_trades
    }

    # Profit factor: gross profit / gross loss
    # No losers → use 999999.0 as positive infinity sentinel
    profit_factor = 999999.0
    if gross_loss > 0.0 {
        profit_factor = gross_profit / gross_loss
    }

    # Average trade P&L
    avg_pnl = 0.0
    if total_trades > 0 {
        avg_pnl = total_pnl / total_trades
    }

    return Metrics {
        sharpe_ratio = sharpe_val,
        max_drawdown_pct = max_dd,
        win_rate = win_rate,
        profit_factor = profit_factor,
        avg_trade_pnl = avg_pnl,
        total_trades = total_trades,
        total_pnl = total_pnl
    }
}

# --- Helper Functions ---

# Compute annualized Sharpe ratio from a list of daily returns.
# Formula: (mean / stddev) * sqrt(252)
# Returns 0.0 if fewer than 2 returns or stddev is zero.
fn compute_sharpe(returns: list) -> f64 {
    if returns.len() < 2 {
        return 0.0
    }

    # Compute mean of returns
    sum = 0.0
    for r in returns {
        sum = sum + r
    }
    mean = sum / returns.len()

    # Compute sample standard deviation
    sq_sum = 0.0
    for r in returns {
        diff = r - mean
        sq_sum = sq_sum + diff * diff
    }
    var_val = sq_sum / (returns.len() - 1)
    stdev = sqrt(var_val)

    if stdev == 0.0 {
        return 0.0
    }

    # sqrt(252) ≈ 15.8745 — annualization factor
    return (mean / stdev) * 15.8745
}

# Compute maximum drawdown as a fraction (0.0 to 1.0) from equity curve.
# Drawdown = (peak - current) / peak for each point.
# Returns the largest drawdown observed.
fn compute_max_drawdown(equity_curve: list) -> f64 {
    if equity_curve.len() == 0 {
        return 0.0
    }

    peak = equity_curve[0]
    max_dd = 0.0

    for equity in equity_curve {
        if equity > peak {
            peak = equity
        }
        if peak > 0.0 {
            dd = (peak - equity) / peak
            if dd > max_dd {
                max_dd = dd
            }
        }
    }

    return max_dd
}

# Pair buy fills with subsequent sell fills for the same symbol
# to compute round-trip trade P&L values.
# P&L = (sell_price - buy_entry_price) * sell_qty
# Returns a list of f64 values (one per completed round-trip trade).
fn compute_trade_pnls(fills: list) -> list {
    pnls = []
    # Track open positions per symbol: symbol -> {price, qty}
    open_positions = HashMap.new()

    for fill in fills {
        match fill.side {
            OrderSide.Buy => {
                # Accumulate buy fills into open position for this symbol
                if open_positions.contains_key(fill.symbol) {
                    pos = open_positions.get(fill.symbol)
                    old_qty = pos.qty
                    old_price = pos.avg_entry_price
                    new_qty = old_qty + fill.qty
                    new_avg = (old_price * old_qty + fill.price * fill.qty) / new_qty
                    open_positions.insert(fill.symbol, PositionState {
                        symbol = fill.symbol,
                        qty = new_qty,
                        avg_entry_price = new_avg,
                        unrealized_pnl = 0.0,
                        realized_pnl = 0.0
                    })
                } else {
                    open_positions.insert(fill.symbol, PositionState {
                        symbol = fill.symbol,
                        qty = fill.qty,
                        avg_entry_price = fill.price,
                        unrealized_pnl = 0.0,
                        realized_pnl = 0.0
                    })
                }
            }
            OrderSide.Sell => {
                # Close (fully or partially) the open position
                if open_positions.contains_key(fill.symbol) {
                    pos = open_positions.get(fill.symbol)
                    pnl = (fill.price - pos.avg_entry_price) * fill.qty
                    pnls.push(pnl)
                    remaining_qty = pos.qty - fill.qty
                    if remaining_qty <= 0.0 {
                        open_positions.remove(fill.symbol)
                    } else {
                        open_positions.insert(fill.symbol, PositionState {
                            symbol = fill.symbol,
                            qty = remaining_qty,
                            avg_entry_price = pos.avg_entry_price,
                            unrealized_pnl = 0.0,
                            realized_pnl = 0.0
                        })
                    }
                }
            }
        }
    }

    return pnls
}
