account {
    name = "test_integration"
    broker = "ibkr"
    account_id = "DU99999"
    mode = "paper"
}

gateway {
    host = "127.0.0.1"
    port = 4002
}

data {
    source = "ibkr"
    symbols = ["ES", "NQ"]
    interval = "1d"
}

database {
    url = "postgres://localhost/test"
    schema = "test"
}

risk {
    max_daily_loss = -5000.0
    max_weekly_loss = -10000.0
    max_position_per_product = 5
    max_total_notional = 1000000.0
    max_drawdown_pct = 0.05
    correlation_warning_threshold = 3
    initial_equity = 100000.0
}

products {
    ES = { multiplier = 50.0, tick_size = 0.25, margin = 15840.0 }
}

strategies {
    alpha = { path = "alpha/strategy.flux", allocation = 1.0, priority = 1 }
}
