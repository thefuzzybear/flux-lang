use std::collections::HashMap;

use crate::backtest::BacktestResult;
use crate::context::BarContext;
use crate::indicators::state::reset_indicator_state;
use crate::signal::Signal;
use crate::strategy::Strategy;

/// Whether a fill opened or closed a position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillSide {
    Open,
    Close,
}

/// Record of a simulated execution.
#[derive(Debug, Clone)]
pub struct Fill {
    /// Instrument symbol
    pub symbol: String,
    /// Quantity filled (always positive)
    pub qty: f64,
    /// Execution price (bar close)
    pub price: f64,
    /// Whether this fill opened or closed a position
    pub side: FillSide,
    /// 0-based bar index at which the fill occurred
    pub bar_index: usize,
}

/// Open holding state for one symbol.
#[derive(Debug, Clone)]
pub struct Position {
    /// Instrument symbol
    pub symbol: String,
    /// Current quantity held (positive = long)
    pub qty: f64,
    /// Volume-weighted average entry price
    pub avg_entry_price: f64,
    /// Current unrealized P&L (updated on mark-to-market)
    pub unrealized_pnl: f64,
    /// Cumulative realized P&L for this symbol
    pub realized_pnl: f64,
    /// Bar index when position was first opened
    pub open_bar: usize,
    /// Bar index of most recent fill or mark-to-market update
    pub last_update_bar: usize,
}

/// Snapshot of all positions and portfolio-level metrics.
#[derive(Debug, Clone)]
pub struct PortfolioState {
    /// All currently open positions
    pub positions: HashMap<String, Position>,
    /// Total realized P&L across all symbols
    pub realized_pnl: f64,
    /// Total unrealized P&L across all symbols
    pub unrealized_pnl: f64,
    /// Current equity (initial_capital + realized + unrealized)
    pub equity: f64,
    /// Sum of |qty * price| for all open positions
    pub gross_exposure: f64,
    /// Sum of qty * price for all open positions
    pub net_exposure: f64,
    /// Initial capital provided at construction
    pub initial_capital: f64,
    /// Number of open positions
    pub open_position_count: usize,
}

/// Enriched backtest result with portfolio state.
#[derive(Debug, Clone)]
pub struct TrackedBacktestResult {
    /// Raw signal pairs (same as BacktestResult) for backward compat
    pub signals: BacktestResult,
    /// Final portfolio state after all bars processed
    pub portfolio: PortfolioState,
    /// Complete fill history in chronological order
    pub fills: Vec<Fill>,
}

/// Stateful processor: signals → fills → positions → P&L.
#[derive(Debug)]
pub struct PositionTracker {
    pub(crate) initial_capital: f64,
    pub(crate) positions: HashMap<String, Position>,
    pub(crate) fills: Vec<Fill>,
    pub(crate) total_realized_pnl: f64,
    /// Most recent mark-to-market price per symbol
    pub(crate) last_prices: HashMap<String, f64>,
}

impl PositionTracker {
    /// Create a new tracker with the given initial capital.
    /// Panics if initial_capital < 0.0.
    pub fn new(initial_capital: f64) -> Self {
        assert!(
            initial_capital >= 0.0,
            "PositionTracker::new: initial_capital must be >= 0.0, got {}",
            initial_capital
        );
        Self {
            initial_capital,
            positions: HashMap::new(),
            fills: Vec::new(),
            total_realized_pnl: 0.0,
            last_prices: HashMap::new(),
        }
    }

    /// Process a single signal at the given fill price and bar index.
    /// Returns Some(Fill) if a fill was generated, None if the signal was ignored
    /// (e.g., Close/CloseQty with no open position).
    ///
    /// # Panics
    /// - If `price <= 0.0` (defensive: bar close should always be positive)
    /// - If a fill would result in `avg_entry_price <= 0.0` (practically unreachable with positive prices)
    pub fn process_signal(
        &mut self,
        signal: &Signal,
        price: f64,
        bar_index: usize,
    ) -> Option<Fill> {
        assert!(
            price > 0.0,
            "PositionTracker::process_signal: price must be > 0.0, got {}",
            price
        );

        match signal {
            Signal::Open { symbol, qty } => {
                let fill = Fill {
                    symbol: symbol.clone(),
                    qty: *qty,
                    price,
                    side: FillSide::Open,
                    bar_index,
                };

                if let Some(position) = self.positions.get_mut(symbol) {
                    let new_qty = position.qty + qty;
                    let new_avg = (position.qty * position.avg_entry_price + qty * price) / new_qty;
                    assert!(
                        new_avg > 0.0,
                        "PositionTracker::process_signal: avg_entry_price must be > 0.0, got {}",
                        new_avg
                    );
                    position.avg_entry_price = new_avg;
                    position.qty = new_qty;
                    position.last_update_bar = bar_index;
                } else {
                    let position = Position {
                        symbol: symbol.clone(),
                        qty: *qty,
                        avg_entry_price: price,
                        unrealized_pnl: 0.0,
                        realized_pnl: 0.0,
                        open_bar: bar_index,
                        last_update_bar: bar_index,
                    };
                    self.positions.insert(symbol.clone(), position);
                }

                self.last_prices.insert(symbol.clone(), price);
                self.fills.push(fill.clone());
                Some(fill)
            }

            Signal::Close { symbol } => {
                let position = self.positions.get_mut(symbol)?;
                let close_qty = position.qty;
                let realized = (price - position.avg_entry_price) * close_qty;

                let fill = Fill {
                    symbol: symbol.clone(),
                    qty: close_qty,
                    price,
                    side: FillSide::Close,
                    bar_index,
                };

                self.total_realized_pnl += realized;
                position.realized_pnl += realized;
                self.positions.remove(symbol);
                self.last_prices.insert(symbol.clone(), price);
                self.fills.push(fill.clone());
                Some(fill)
            }

            Signal::CloseQty { symbol, qty } => {
                let position = self.positions.get_mut(symbol)?;
                let actual_qty = qty.min(position.qty);
                let realized = (price - position.avg_entry_price) * actual_qty;

                let fill = Fill {
                    symbol: symbol.clone(),
                    qty: actual_qty,
                    price,
                    side: FillSide::Close,
                    bar_index,
                };

                self.total_realized_pnl += realized;
                position.realized_pnl += realized;
                position.qty -= actual_qty;
                position.last_update_bar = bar_index;

                if position.qty == 0.0 {
                    self.positions.remove(symbol);
                }

                self.last_prices.insert(symbol.clone(), price);
                self.fills.push(fill.clone());
                Some(fill)
            }
        }
    }

    /// Process a batch of signals for a single bar.
    /// Updates position state between each signal.
    pub fn process_signals(&mut self, signals: &[Signal], price: f64, bar_index: usize) -> Vec<Fill> {
        signals.iter()
            .filter_map(|signal| self.process_signal(signal, price, bar_index))
            .collect()
    }

    /// Update mark-to-market for a single position.
    /// Stores the price and updates unrealized P&L.
    pub fn mark_to_market(&mut self, price: f64, symbol: &str) {
        self.last_prices.insert(symbol.to_string(), price);
        if let Some(position) = self.positions.get_mut(symbol) {
            position.unrealized_pnl = (price - position.avg_entry_price) * position.qty;
        }
    }

    /// Mark all positions to market with given prices per symbol.
    pub fn mark_all_to_market(&mut self, prices: &HashMap<String, f64>) {
        for (symbol, &price) in prices {
            self.mark_to_market(price, symbol);
        }
    }

    /// Current equity: initial_capital + realized + unrealized.
    pub fn equity(&self) -> f64 {
        let total_unrealized: f64 = self.positions.values().map(|p| p.unrealized_pnl).sum();
        self.initial_capital + self.total_realized_pnl + total_unrealized
    }

    /// Gross exposure: sum(|qty * last_price|) across open positions.
    pub fn gross_exposure(&self) -> f64 {
        self.positions.values().map(|p| {
            let price = self.last_prices.get(&p.symbol).copied().unwrap_or(p.avg_entry_price);
            (p.qty * price).abs()
        }).sum()
    }

    /// Net exposure: sum(qty * last_price) across open positions.
    pub fn net_exposure(&self) -> f64 {
        self.positions.values().map(|p| {
            let price = self.last_prices.get(&p.symbol).copied().unwrap_or(p.avg_entry_price);
            p.qty * price
        }).sum()
    }

    /// Number of open positions with non-zero qty.
    pub fn open_position_count(&self) -> usize {
        self.positions.values().filter(|p| p.qty != 0.0).count()
    }

    /// Get a reference to a position by symbol.
    pub fn position(&self, symbol: &str) -> Option<&Position> {
        self.positions.get(symbol)
    }

    /// Get all open positions.
    pub fn positions(&self) -> &HashMap<String, Position> {
        &self.positions
    }

    /// Get all fills in chronological order.
    pub fn fills(&self) -> &[Fill] {
        &self.fills
    }

    /// Total realized P&L across all closed trades.
    pub fn realized_pnl(&self) -> f64 {
        self.total_realized_pnl
    }

    /// Total unrealized P&L across all open positions.
    pub fn unrealized_pnl(&self) -> f64 {
        self.positions.values().map(|p| p.unrealized_pnl).sum()
    }

    /// Initial capital provided at construction.
    pub fn initial_capital(&self) -> f64 {
        self.initial_capital
    }

    /// Last known mark-to-market prices per symbol.
    pub fn last_prices(&self) -> &HashMap<String, f64> {
        &self.last_prices
    }

    /// Take a snapshot of the current portfolio state.
    pub fn portfolio_state(&self) -> PortfolioState {
        PortfolioState {
            positions: self.positions.clone(),
            realized_pnl: self.total_realized_pnl,
            unrealized_pnl: self.unrealized_pnl(),
            equity: self.equity(),
            gross_exposure: self.gross_exposure(),
            net_exposure: self.net_exposure(),
            initial_capital: self.initial_capital,
            open_position_count: self.open_position_count(),
        }
    }
}


/// Run a backtest with full position tracking and P&L calculation.
///
/// This wraps the standard bar-iteration loop, adding fill simulation
/// and portfolio state tracking. The existing `run_backtest` function
/// remains unchanged for backward compatibility.
pub fn run_backtest_with_tracker(
    strategy: &mut dyn Strategy,
    bars: &[BarContext],
    initial_capital: f64,
) -> TrackedBacktestResult {
    reset_indicator_state();

    let mut tracker = PositionTracker::new(initial_capital);
    let mut results: BacktestResult = Vec::new();

    for (i, bar) in bars.iter().enumerate() {
        let signals = strategy.on_bar(bar);

        // Process signals through tracker
        tracker.process_signals(&signals, bar.close, i);

        // Collect raw signal pairs for BacktestResult compatibility
        for signal in signals {
            results.push((i, signal));
        }

        // Mark all open positions to market at bar close
        tracker.mark_to_market(bar.close, &bar.symbol);
    }

    TrackedBacktestResult {
        signals: results,
        portfolio: tracker.portfolio_state(),
        fills: tracker.fills.clone(),
    }
}


#[cfg(test)]
mod tests {
    use super::{FillSide, PositionTracker, Signal};
    use proptest::prelude::*;
    use std::collections::HashMap;

    /// Helper: generate an arbitrary Signal.
    /// Reused across all property tests in this module.
    fn arb_signal() -> impl Strategy<Value = Signal> {
        let open = ("[A-Z]{1,4}", 0.01..1000.0f64)
            .prop_map(|(symbol, qty)| Signal::open(symbol, qty));
        let close = "[A-Z]{1,4}".prop_map(|symbol| Signal::close(symbol));
        let close_qty = ("[A-Z]{1,4}", 0.01..1000.0f64)
            .prop_map(|(symbol, qty)| Signal::close_qty(symbol, qty));
        prop_oneof![open, close, close_qty]
    }

    // Feature: position-tracker, Property 1: Equity Invariant
    // **Validates: Requirements 6.1, 8.1**
    proptest! {
        #[test]
        fn prop_equity_invariant(
            initial_capital in 0.0..100_000.0f64,
            signals in proptest::collection::vec(arb_signal(), 0..30),
            prices in proptest::collection::vec(0.01..10000.0f64, 1..50),
        ) {
            let mut tracker = PositionTracker::new(initial_capital);

            for (i, signal) in signals.iter().enumerate() {
                let price = prices[i % prices.len()];
                tracker.process_signal(signal, price, i);

                // Mark to market with a fresh price
                let mark_price = prices[(i + 1) % prices.len()];
                tracker.mark_to_market(mark_price, signal.symbol());

                // Verify invariant after every operation
                let expected = tracker.initial_capital() + tracker.realized_pnl() + tracker.unrealized_pnl();
                let actual = tracker.equity();
                prop_assert!((actual - expected).abs() < 1e-9,
                    "Equity invariant violated: equity={}, expected={}", actual, expected);
            }
        }
    }

    // Feature: position-tracker, Property 4: No Fill When No Position
    // **Validates: Requirements 1.4, 5.5**
    proptest! {
        #[test]
        fn prop_no_fill_when_no_position(
            symbol in "[A-Z]{1,4}",
            qty in 0.01..1000.0f64,
            price in 0.01..10000.0f64,
            bar_index in 0..100usize,
        ) {
            let mut tracker = PositionTracker::new(10000.0);

            // Close signal for non-existent position should return None
            let close_signal = Signal::close(symbol.clone());
            let result = tracker.process_signal(&close_signal, price, bar_index);
            prop_assert!(result.is_none(), "Close should return None when no position exists");
            prop_assert!(tracker.fills().is_empty(), "No fills should be generated");

            // CloseQty signal for non-existent position should return None
            let close_qty_signal = Signal::close_qty(symbol.clone(), qty);
            let result = tracker.process_signal(&close_qty_signal, price, bar_index);
            prop_assert!(result.is_none(), "CloseQty should return None when no position exists");
            prop_assert!(tracker.fills().is_empty(), "No fills should be generated");
        }
    }

    // Feature: position-tracker, Property 3: Fill Creation Correctness
    // **Validates: Requirements 1.1, 1.2, 1.3, 1.5, 1.6, 5.2, 5.3, 7.4**
    proptest! {
        #[test]
        fn prop_fill_creation_correctness(
            symbol in "[A-Z]{1,4}",
            qty in 0.01..1000.0f64,
            price in 0.01..10000.0f64,
            bar_index in 0..100usize,
        ) {
            // Test Open signal fill correctness
            let mut tracker = PositionTracker::new(10000.0);
            let open_signal = Signal::open(symbol.clone(), qty);
            let fill = tracker.process_signal(&open_signal, price, bar_index).unwrap();

            prop_assert_eq!(&fill.symbol, &symbol);
            prop_assert_eq!(fill.qty, qty);
            prop_assert_eq!(fill.price, price);
            prop_assert_eq!(fill.side, FillSide::Open);
            prop_assert_eq!(fill.bar_index, bar_index);

            // Test Close signal fill correctness
            let close_signal = Signal::close(symbol.clone());
            let fill = tracker.process_signal(&close_signal, price, bar_index + 1).unwrap();

            prop_assert_eq!(&fill.symbol, &symbol);
            prop_assert_eq!(fill.qty, qty); // closes full position
            prop_assert_eq!(fill.price, price);
            prop_assert_eq!(fill.side, FillSide::Close);
            prop_assert_eq!(fill.bar_index, bar_index + 1);

            // Test CloseQty signal fill correctness (with clamping)
            let mut tracker2 = PositionTracker::new(10000.0);
            let open_signal = Signal::open(symbol.clone(), qty);
            tracker2.process_signal(&open_signal, price, 0);

            let close_qty_val = qty * 2.0; // intentionally larger than position
            let close_qty_signal = Signal::close_qty(symbol.clone(), close_qty_val);
            let fill = tracker2.process_signal(&close_qty_signal, price, 1).unwrap();

            prop_assert_eq!(&fill.symbol, &symbol);
            prop_assert_eq!(fill.qty, qty); // clamped to position qty
            prop_assert_eq!(fill.price, price);
            prop_assert_eq!(fill.side, FillSide::Close);
            prop_assert_eq!(fill.bar_index, 1);
        }
    }

    // Feature: position-tracker, Property 2: Quantity Conservation
    // **Validates: Requirements 8.2, 2.3, 2.4**
    proptest! {
        #[test]
        fn prop_quantity_conservation(
            initial_capital in 0.0..100_000.0f64,
            signals in proptest::collection::vec(arb_signal(), 1..30),
            price in 0.01..10000.0f64,
        ) {
            let mut tracker = PositionTracker::new(initial_capital);

            for (i, signal) in signals.iter().enumerate() {
                tracker.process_signal(signal, price, i);
            }

            // For each symbol, verify quantity conservation
            // sum(open fills qty) - sum(close fills qty) == position qty
            let mut open_qty_by_symbol: HashMap<String, f64> = HashMap::new();
            let mut close_qty_by_symbol: HashMap<String, f64> = HashMap::new();

            for fill in tracker.fills() {
                match fill.side {
                    FillSide::Open => {
                        *open_qty_by_symbol.entry(fill.symbol.clone()).or_insert(0.0) += fill.qty;
                    }
                    FillSide::Close => {
                        *close_qty_by_symbol.entry(fill.symbol.clone()).or_insert(0.0) += fill.qty;
                    }
                }
            }

            // Check all symbols that had any fills
            let all_symbols: std::collections::HashSet<&String> = open_qty_by_symbol.keys()
                .chain(close_qty_by_symbol.keys())
                .collect();

            for symbol in all_symbols {
                let total_opened = open_qty_by_symbol.get(symbol).copied().unwrap_or(0.0);
                let total_closed = close_qty_by_symbol.get(symbol).copied().unwrap_or(0.0);
                let current_qty = tracker.position(symbol).map(|p| p.qty).unwrap_or(0.0);

                let diff = (total_opened - total_closed) - current_qty;
                prop_assert!(diff.abs() < 1e-9,
                    "Quantity conservation failed for {}: opened={}, closed={}, position={}, diff={}",
                    symbol, total_opened, total_closed, current_qty, diff);
            }
        }
    }

    // Feature: position-tracker, Property 5: VWAP Average Entry Price
    // **Validates: Requirements 2.1, 2.2, 2.4, 3.5, 1.7**
    proptest! {
        #[test]
        fn prop_vwap_avg_entry_price(
            entries in proptest::collection::vec((0.01..1000.0f64, 0.01..10000.0f64), 2..10),
            partial_close_qty in 0.01..500.0f64,
        ) {
            let symbol = "TEST".to_string();
            let mut tracker = PositionTracker::new(10000.0);

            // Open multiple times at different prices
            let mut total_cost = 0.0f64;
            let mut total_qty = 0.0f64;

            for (i, (qty, price)) in entries.iter().enumerate() {
                let signal = Signal::open(symbol.clone(), *qty);
                tracker.process_signal(&signal, *price, i);
                total_cost += qty * price;
                total_qty += qty;
            }

            // Verify VWAP formula
            let expected_avg = total_cost / total_qty;
            let position = tracker.position(&symbol).unwrap();
            prop_assert!((position.avg_entry_price - expected_avg).abs() < 1e-9,
                "VWAP mismatch: got {}, expected {}", position.avg_entry_price, expected_avg);

            // Verify partial close preserves avg entry price
            let avg_before_close = position.avg_entry_price;
            let close_qty = partial_close_qty.min(total_qty * 0.5); // Close at most half
            if close_qty > 0.0 && close_qty < total_qty {
                let close_signal = Signal::close_qty(symbol.clone(), close_qty);
                tracker.process_signal(&close_signal, 5000.0, entries.len());

                if let Some(position) = tracker.position(&symbol) {
                    prop_assert!((position.avg_entry_price - avg_before_close).abs() < 1e-9,
                        "Partial close changed avg entry: got {}, expected {}",
                        position.avg_entry_price, avg_before_close);
                }
            }
        }
    }

    // Feature: position-tracker, Property 6: Unrealized PnL Formula
    // **Validates: Requirements 3.1, 3.3, 8.6**
    proptest! {
        #[test]
        fn prop_unrealized_pnl_formula(
            qty in 0.01..1000.0f64,
            entry_price in 0.01..10000.0f64,
            mark_price in 0.01..10000.0f64,
        ) {
            let symbol = "TEST".to_string();
            let mut tracker = PositionTracker::new(10000.0);

            // Open a position
            let signal = Signal::open(symbol.clone(), qty);
            tracker.process_signal(&signal, entry_price, 0);

            // Mark to market
            tracker.mark_to_market(mark_price, &symbol);

            // Verify unrealized PnL formula
            let expected_pnl = (mark_price - entry_price) * qty;
            let position = tracker.position(&symbol).unwrap();
            prop_assert!((position.unrealized_pnl - expected_pnl).abs() < 1e-9,
                "Unrealized PnL mismatch: got {}, expected {}", position.unrealized_pnl, expected_pnl);

            // Also verify the total unrealized_pnl accessor
            prop_assert!((tracker.unrealized_pnl() - expected_pnl).abs() < 1e-9,
                "Total unrealized PnL mismatch: got {}, expected {}", tracker.unrealized_pnl(), expected_pnl);
        }
    }

    // Feature: position-tracker, Property 9: Fill Chronological Ordering
    // **Validates: Requirements 5.1, 7.3, 1.8**
    proptest! {
        #[test]
        fn prop_fill_chronological_ordering(
            num_bars in 2..20usize,
            signals_per_bar in proptest::collection::vec(
                proptest::collection::vec(arb_signal(), 0..5),
                2..20
            ),
            price in 0.01..10000.0f64,
        ) {
            let mut tracker = PositionTracker::new(10000.0);
            let bars = num_bars.min(signals_per_bar.len());

            for bar_idx in 0..bars {
                tracker.process_signals(&signals_per_bar[bar_idx], price, bar_idx);
            }

            // Verify fills are in chronological order by bar_index
            let fills = tracker.fills();
            for i in 1..fills.len() {
                prop_assert!(fills[i].bar_index >= fills[i - 1].bar_index,
                    "Fill ordering violated at index {}: bar_index {} < {}",
                    i, fills[i].bar_index, fills[i - 1].bar_index);
            }
        }
    }

    // Feature: position-tracker, Property 8: Exposure Calculation
    // **Validates: Requirements 6.2, 6.3, 6.4, 6.5**
    proptest! {
        #[test]
        fn prop_exposure_calculation(
            qtys in proptest::collection::vec(0.01..1000.0f64, 1..5),
            prices in proptest::collection::vec(0.01..10000.0f64, 1..5),
            mark_prices in proptest::collection::vec(0.01..10000.0f64, 1..5),
        ) {
            let mut tracker = PositionTracker::new(10000.0);

            // When no positions are open, exposures should be 0.0
            prop_assert_eq!(tracker.gross_exposure(), 0.0);
            prop_assert_eq!(tracker.net_exposure(), 0.0);

            let symbols: Vec<String> = (0..qtys.len().min(prices.len()))
                .map(|i| format!("S{}", i))
                .collect();

            // Open positions for different symbols
            let num = symbols.len().min(qtys.len()).min(prices.len());
            for i in 0..num {
                let signal = Signal::open(symbols[i].clone(), qtys[i]);
                tracker.process_signal(&signal, prices[i], i);
            }

            // Mark all to market with new prices
            let mut mark_map = HashMap::new();
            for i in 0..num {
                let mark_price = mark_prices[i % mark_prices.len()];
                mark_map.insert(symbols[i].clone(), mark_price);
            }
            tracker.mark_all_to_market(&mark_map);

            // Calculate expected exposures
            let mut expected_gross = 0.0f64;
            let mut expected_net = 0.0f64;
            for i in 0..num {
                let mark_price = mark_map[&symbols[i]];
                expected_gross += (qtys[i] * mark_price).abs();
                expected_net += qtys[i] * mark_price;
            }

            let gross_tol = expected_gross.abs() * 1e-12 + 1e-9;
            prop_assert!((tracker.gross_exposure() - expected_gross).abs() < gross_tol,
                "Gross exposure mismatch: got {}, expected {}", tracker.gross_exposure(), expected_gross);
            let net_tol = expected_net.abs() * 1e-12 + 1e-9;
            prop_assert!((tracker.net_exposure() - expected_net).abs() < net_tol,
                "Net exposure mismatch: got {}, expected {}", tracker.net_exposure(), expected_net);
        }
    }

    // Feature: position-tracker, Property 10: Average Entry Price Positivity
    // **Validates: Requirements 8.4, 8.5**
    proptest! {
        #[test]
        fn prop_avg_entry_price_positivity(
            signals in proptest::collection::vec(arb_signal(), 1..30),
            price in 0.01..10000.0f64,
        ) {
            let mut tracker = PositionTracker::new(10000.0);

            for (i, signal) in signals.iter().enumerate() {
                tracker.process_signal(signal, price, i);

                // After every operation, verify all open positions have positive avg_entry_price
                for position in tracker.positions().values() {
                    prop_assert!(position.avg_entry_price > 0.0,
                        "avg_entry_price must be > 0.0 for {}, got {}",
                        position.symbol, position.avg_entry_price);
                    prop_assert!(position.qty > 0.0,
                        "Open position qty must be > 0.0 for {}, got {}",
                        position.symbol, position.qty);
                }
            }
        }
    }

    // Feature: position-tracker, Property 7: P&L Round-Trip Decomposition
    // **Validates: Requirements 4.1, 4.2, 4.3, 4.4, 8.3**
    proptest! {
        #[test]
        fn prop_pnl_round_trip_decomposition(
            entry_price in 0.01..10000.0f64,
            exit_price in 0.01..10000.0f64,
            mark_price in 0.01..10000.0f64,
            open_qty in 0.01..1000.0f64,
            close_fraction in 0.1..0.9f64,
        ) {
            let symbol = "TEST".to_string();
            let mut tracker = PositionTracker::new(10000.0);

            // Open a position
            let open_signal = Signal::open(symbol.clone(), open_qty);
            tracker.process_signal(&open_signal, entry_price, 0);

            // Partially close
            let close_qty = open_qty * close_fraction;
            let close_signal = Signal::close_qty(symbol.clone(), close_qty);
            tracker.process_signal(&close_signal, exit_price, 1);

            // Mark remaining to market
            tracker.mark_to_market(mark_price, &symbol);

            // Compute expected total PnL
            let realized_from_close = (exit_price - entry_price) * close_qty;
            let remaining_qty = open_qty - close_qty;
            let unrealized_from_open = (mark_price - entry_price) * remaining_qty;
            let expected_total_pnl = realized_from_close + unrealized_from_open;

            let actual_total_pnl = tracker.realized_pnl() + tracker.unrealized_pnl();

            prop_assert!((actual_total_pnl - expected_total_pnl).abs() < 1e-9,
                "P&L decomposition failed: actual={}, expected={}, realized={}, unrealized={}",
                actual_total_pnl, expected_total_pnl, tracker.realized_pnl(), tracker.unrealized_pnl());
        }
    }

    // --- Integration tests for run_backtest_with_tracker ---

    use crate::context::BarContext;
    use crate::strategy::Strategy as StrategyTrait;

    /// Helper: create a bar context for integration tests
    fn make_bar(symbol: &str, close: f64) -> BarContext {
        BarContext {
            close,
            open: close - 1.0,
            high: close + 1.0,
            low: close - 2.0,
            volume: 1000.0,
            symbol: symbol.to_string(),
            in_position: false,
        }
    }

    /// A simple strategy that opens on bar 0, closes on bar 2
    struct OpenCloseStrategy {
        bar_count: usize,
    }

    impl OpenCloseStrategy {
        fn new() -> Self {
            Self { bar_count: 0 }
        }
    }

    impl StrategyTrait for OpenCloseStrategy {
        fn on_bar(&mut self, ctx: &BarContext) -> Vec<Signal> {
            let signals = match self.bar_count {
                0 => vec![Signal::open(ctx.symbol.clone(), 100.0)],
                2 => vec![Signal::close(ctx.symbol.clone())],
                _ => vec![],
            };
            self.bar_count += 1;
            signals
        }
    }

    #[test]
    fn test_tracked_produces_same_signals_as_run_backtest() {
        use crate::backtest::run_backtest;

        let bars = vec![
            make_bar("AAPL", 100.0),
            make_bar("AAPL", 110.0),
            make_bar("AAPL", 120.0),
        ];

        let mut strategy1 = OpenCloseStrategy::new();
        let mut strategy2 = OpenCloseStrategy::new();

        let raw_result = run_backtest(&mut strategy1, &bars);
        let tracked_result = super::run_backtest_with_tracker(&mut strategy2, &bars, 10000.0);

        // Same number of signals
        assert_eq!(raw_result.len(), tracked_result.signals.len());

        // Same bar indices and signal types
        for (i, ((idx1, sig1), (idx2, sig2))) in
            raw_result.iter().zip(tracked_result.signals.iter()).enumerate()
        {
            assert_eq!(idx1, idx2, "Bar index differs at position {}", i);
            assert_eq!(
                sig1.symbol(),
                sig2.symbol(),
                "Symbol differs at position {}",
                i
            );
            assert_eq!(sig1.qty(), sig2.qty(), "Qty differs at position {}", i);
        }
    }

    #[test]
    fn test_tracked_backtest_pnl_calculation() {
        let bars = vec![
            make_bar("AAPL", 100.0), // bar 0: open at 100
            make_bar("AAPL", 110.0), // bar 1: hold
            make_bar("AAPL", 120.0), // bar 2: close at 120
        ];

        let mut strategy = OpenCloseStrategy::new();
        let result = super::run_backtest_with_tracker(&mut strategy, &bars, 10000.0);

        // Position was opened at 100, closed at 120, qty=100
        // Realized PnL = (120 - 100) * 100 = 2000
        assert!(
            (result.portfolio.realized_pnl - 2000.0).abs() < 1e-9,
            "Expected realized PnL of 2000.0, got {}",
            result.portfolio.realized_pnl
        );
        // No open positions after close
        assert_eq!(result.portfolio.open_position_count, 0);
        // Equity = initial + realized + unrealized = 10000 + 2000 + 0
        assert!(
            (result.portfolio.equity - 12000.0).abs() < 1e-9,
            "Expected equity of 12000.0, got {}",
            result.portfolio.equity
        );
        // 2 fills: one open, one close
        assert_eq!(result.fills.len(), 2);
        assert_eq!(result.fills[0].side, FillSide::Open);
        assert_eq!(result.fills[1].side, FillSide::Close);
    }

    #[test]
    fn test_tracked_backtest_determinism() {
        let bars = vec![
            make_bar("AAPL", 100.0),
            make_bar("AAPL", 110.0),
            make_bar("AAPL", 120.0),
        ];

        let mut strategy1 = OpenCloseStrategy::new();
        let mut strategy2 = OpenCloseStrategy::new();

        let result1 = super::run_backtest_with_tracker(&mut strategy1, &bars, 10000.0);
        let result2 = super::run_backtest_with_tracker(&mut strategy2, &bars, 10000.0);

        assert_eq!(result1.portfolio.equity, result2.portfolio.equity);
        assert_eq!(
            result1.portfolio.realized_pnl,
            result2.portfolio.realized_pnl
        );
        assert_eq!(
            result1.portfolio.unrealized_pnl,
            result2.portfolio.unrealized_pnl
        );
        assert_eq!(result1.fills.len(), result2.fills.len());
    }

    #[test]
    fn test_backward_compatibility_run_backtest_unchanged() {
        use crate::backtest::run_backtest;

        let bars = vec![make_bar("SPY", 400.0), make_bar("SPY", 410.0)];
        let mut strategy = OpenCloseStrategy::new();
        let result = run_backtest(&mut strategy, &bars);

        // run_backtest still works and returns BacktestResult (Vec<(usize, Signal)>)
        assert_eq!(result.len(), 1); // Only opens on bar 0
        assert_eq!(result[0].0, 0);
        assert_eq!(result[0].1.symbol(), "SPY");
    }
}
