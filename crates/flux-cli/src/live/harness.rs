//! LiveHarness — the central orchestrator for live strategy execution.
//!
//! Manages the tokio event loop, dispatches bars to subscribed strategies,
//! collects signals, and coordinates with the signal aggregator and
//! position tracker.

use std::path::PathBuf;
use std::time::Duration;

use flux_runtime::Signal;
use tokio::sync::mpsc;

use super::aggregator::SignalAggregator;
use super::connector::{ConnectorState, LiveBar, ReconnectPolicy};
use super::loader::StrategyModule;
use super::position::LivePositionTracker;
use super::state::{
    save_state, HarnessState, PositionState, SerializedPosition, SerializedValue, StrategyState,
};
use crate::interpreter::Value;

/// Errors that can occur during live harness operation.
#[derive(Debug, thiserror::Error)]
pub enum LiveError {
    /// All connectors have permanently failed — no data sources remain.
    #[error("all connectors failed: no data sources remain")]
    AllConnectorsFailed,
    /// State persistence error during shutdown.
    #[error("state persistence error: {0}")]
    StatePersistence(String),
    /// An I/O error occurred.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// The central orchestrator for live strategy execution.
///
/// Holds loaded strategy modules, the unified position tracker, signal
/// aggregator with risk constraints, and configuration for state
/// persistence, reconnection, and heartbeat output.
pub struct LiveHarness {
    /// Loaded strategy modules with their interpreters.
    pub strategies: Vec<StrategyModule>,
    /// Signal aggregator with portfolio-level risk constraints.
    pub aggregator: SignalAggregator,
    /// Unified position tracker shared across all strategies.
    pub tracker: LivePositionTracker,
    /// Optional path for state persistence across restarts.
    pub state_file: Option<PathBuf>,
    /// Reconnection policy for failed connectors.
    pub reconnect_policy: ReconnectPolicy,
    /// Interval between heartbeat status outputs.
    pub heartbeat_interval: Duration,
}

impl LiveHarness {
    /// Create a new `LiveHarness` with the given configuration.
    pub fn new(
        strategies: Vec<StrategyModule>,
        aggregator: SignalAggregator,
        tracker: LivePositionTracker,
        state_file: Option<PathBuf>,
        reconnect_policy: ReconnectPolicy,
        heartbeat_interval: Duration,
    ) -> Self {
        Self {
            strategies,
            aggregator,
            tracker,
            state_file,
            reconnect_policy,
            heartbeat_interval,
        }
    }

    /// Print a startup summary to stderr listing all loaded strategies,
    /// configured risk constraints, state file, and heartbeat interval.
    ///
    /// Called once when the harness starts to provide visibility into the
    /// running configuration (requirement 9.1).
    pub fn print_startup_summary(&self) {
        eprintln!("=== Flux Live Harness ===");
        eprintln!();

        // Loaded strategies
        eprintln!(
            "Strategies: {} loaded",
            self.strategies.len()
        );
        for strategy in &self.strategies {
            let symbols = if strategy.subscribed_symbols.is_empty() {
                "(no symbols)".to_string()
            } else {
                strategy.subscribed_symbols.join(", ")
            };
            eprintln!("  • {} [{}]", strategy.name, symbols);
        }
        eprintln!();

        // Risk constraints
        let constraints = self.aggregator.constraints();
        eprintln!("Risk constraints:");
        match constraints.max_position_size {
            Some(v) => eprintln!("  max_position_size: {:.2}", v),
            None => eprintln!("  max_position_size: unlimited"),
        }
        match constraints.max_exposure {
            Some(v) => eprintln!("  max_exposure: {:.2}", v),
            None => eprintln!("  max_exposure: unlimited"),
        }
        match constraints.max_positions {
            Some(v) => eprintln!("  max_positions: {}", v),
            None => eprintln!("  max_positions: unlimited"),
        }
        eprintln!();

        // State file
        match &self.state_file {
            Some(path) => eprintln!("State file: {}", path.display()),
            None => eprintln!("State file: (none)"),
        }

        // Heartbeat interval
        eprintln!("Heartbeat interval: {}s", self.heartbeat_interval.as_secs());
        eprintln!();
        eprintln!("Listening for bars...");
        eprintln!();
    }

    /// Dispatch a bar to all subscribed strategies, collect signals,
    /// run them through the aggregator, and process approved signals
    /// through the unified position tracker.
    ///
    /// The dispatch flow:
    /// 1. Route bar to strategies whose subscribed_symbols include the bar's symbol
    /// 2. Derive `in_position` from the unified tracker for each strategy
    /// 3. Execute each strategy's `on_bar` handler and collect signals
    /// 4. Pass all signals through the aggregator (risk constraints)
    /// 5. Process approved signals through the unified tracker
    /// 6. Mark to market for the bar's symbol
    ///
    /// Strategy runtime errors (including panics) are caught, logged,
    /// and the harness continues processing remaining strategies (requirement 2.8).
    pub fn dispatch_bar(&mut self, live_bar: &LiveBar) {
        let bar = &live_bar.bar;
        let mut all_signals: Vec<(String, Signal)> = Vec::new();

        for strategy in &mut self.strategies {
            if !strategy.subscribed_symbols.contains(&bar.symbol) {
                continue;
            }

            // Derive in_position from unified tracker for this strategy's symbols
            let in_position = self.tracker.in_position_for(&strategy.subscribed_symbols);

            // Set in_position on the interpreter before executing on_bar
            strategy.interpreter.in_position = in_position;

            // Execute on_bar — catch panics so a single strategy crash
            // doesn't take down the entire harness (requirement 2.8).
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                strategy.interpreter.on_bar(bar)
            }));

            let signals = match result {
                Ok(signals) => signals,
                Err(_) => {
                    eprintln!(
                        "  [ERROR] strategy '{}' panicked during on_bar for {} — skipping",
                        strategy.name, bar.symbol
                    );
                    continue;
                }
            };

            for signal in signals {
                let kind = signal_kind_str(&signal);
                let qty_display = signal_qty_display(&signal);
                eprintln!(
                    "  [SIGNAL] {} | {} | {} | qty: {}",
                    strategy.name,
                    bar.symbol,
                    kind,
                    qty_display,
                );
                all_signals.push((strategy.name.clone(), signal));
            }
        }

        // Aggregate and apply risk constraints
        let approved = self.aggregator.process(&all_signals, &self.tracker.inner);

        // Process approved signals through the unified tracker
        for (strategy_name, signal) in &approved {
            let price = bar.close;
            if let Some(fill) = self.tracker.process_signal(signal, price, 0, strategy_name) {
                eprintln!(
                    "  [FILL] {} | {:?} {} x {:.4} @ {:.2}",
                    strategy_name, fill.side, fill.symbol, fill.qty, fill.price
                );
            }
        }

        // Mark to market
        self.tracker.inner.mark_to_market(bar.close, &bar.symbol);
    }

    /// Run the main event loop using `tokio::select!`.
    ///
    /// Listens for bars from the provided `mpsc::Receiver`, a periodic heartbeat
    /// timer, and SIGINT (Ctrl+C). The loop exits when:
    /// - The bar channel closes (all connectors/senders dropped)
    /// - SIGINT is received (graceful shutdown)
    ///
    /// Connectors are started externally and send `LiveBar` values into the
    /// channel. This keeps the harness testable without real async connectors.
    ///
    /// The `connector_count` parameter indicates the number of connectors that
    /// were started. When the bar channel closes and `connector_count > 0`,
    /// the harness treats this as all connectors having permanently failed
    /// and returns `LiveError::AllConnectorsFailed`. When `connector_count == 0`
    /// (e.g., no connectors configured), it returns `Ok(())`.
    pub async fn run(
        &mut self,
        mut bar_rx: mpsc::Receiver<LiveBar>,
        connector_count: usize,
    ) -> Result<(), LiveError> {
        let mut heartbeat = tokio::time::interval(self.heartbeat_interval);
        let shutdown = tokio::signal::ctrl_c();
        tokio::pin!(shutdown);

        loop {
            tokio::select! {
                bar_msg = bar_rx.recv() => {
                    match bar_msg {
                        Some(live_bar) => {
                            self.dispatch_bar(&live_bar);
                        }
                        None => {
                            // Channel closed — all connectors/senders dropped.
                            // If connectors were configured, this means they all
                            // permanently failed (requirement 6.6).
                            if connector_count > 0 {
                                eprintln!(
                                    "[harness] all {} connector(s) disconnected — no data sources remain",
                                    connector_count
                                );
                                return Err(LiveError::AllConnectorsFailed);
                            }
                            eprintln!("[harness] bar channel closed, exiting event loop");
                            return Ok(());
                        }
                    }
                }
                _ = heartbeat.tick() => {
                    self.print_heartbeat();
                }
                _ = &mut shutdown => {
                    eprintln!("[harness] SIGINT received, shutting down...");
                    self.graceful_shutdown().await;
                    return Ok(());
                }
            }
        }
    }

    /// Print a heartbeat status line to stderr.
    ///
    /// Shows current equity, number of open positions, and a timestamp.
    /// Called periodically by the event loop at `heartbeat_interval`.
    fn print_heartbeat(&self) {
        let equity = self.tracker.inner.equity();
        let open_positions = self.tracker.inner.open_position_count();
        let realized_pnl = self.tracker.inner.realized_pnl();

        eprintln!(
            "[heartbeat] equity: {:.2} | open positions: {} | realized P&L: {:.2} | strategies: {}",
            equity,
            open_positions,
            realized_pnl,
            self.strategies.len(),
        );
    }

    /// Graceful shutdown sequence.
    ///
    /// 1. Log "shutting down..."
    /// 2. If `state_file` configured: serialize and persist state via `save_state()`
    /// 3. Log final portfolio summary (equity, realized P&L, open positions)
    /// 4. Log shutdown complete
    pub async fn graceful_shutdown(&self) {
        eprintln!("[harness] shutting down...");

        // Persist state if state_file is configured
        if let Some(ref path) = self.state_file {
            let state = self.build_harness_state();
            match save_state(&state, path) {
                Ok(()) => {
                    eprintln!("[harness] state persisted to {}", path.display());
                }
                Err(e) => {
                    eprintln!("[harness] error persisting state: {}", e);
                }
            }
        }

        // Log final portfolio summary
        let equity = self.tracker.inner.equity();
        let realized_pnl = self.tracker.inner.realized_pnl();
        let open_count = self.tracker.inner.open_position_count();

        eprintln!("[harness] === final portfolio summary ===");
        eprintln!(
            "[harness] equity: {:.2} | realized P&L: {:.2} | open positions: {}",
            equity, realized_pnl, open_count,
        );

        // Log per-position details
        for (symbol, position) in self.tracker.inner.positions() {
            if position.qty != 0.0 {
                eprintln!(
                    "[harness]   {} | qty: {:.2} | avg entry: {:.2} | unrealized P&L: {:.2}",
                    symbol, position.qty, position.avg_entry_price, position.unrealized_pnl,
                );
            }
        }

        eprintln!("[harness] shutdown complete");
    }

    /// Build a `HarnessState` from current tracker and strategy state for persistence.
    fn build_harness_state(&self) -> HarnessState {
        // Serialize positions
        let positions: Vec<SerializedPosition> = self
            .tracker
            .inner
            .positions()
            .values()
            .map(|p| SerializedPosition {
                symbol: p.symbol.clone(),
                qty: p.qty,
                avg_entry_price: p.avg_entry_price,
                realized_pnl: p.realized_pnl,
            })
            .collect();

        let last_prices: Vec<(String, f64)> = self
            .tracker
            .inner
            .last_prices()
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();

        let position_state = PositionState {
            initial_capital: self.tracker.inner.initial_capital(),
            positions,
            total_realized_pnl: self.tracker.inner.realized_pnl(),
            last_prices,
        };

        // Serialize per-strategy state
        let strategy_states: Vec<StrategyState> = self
            .strategies
            .iter()
            .map(|strategy| {
                let state_variables: Vec<(String, SerializedValue)> = strategy
                    .interpreter
                    .state
                    .iter()
                    .filter_map(|(name, value)| {
                        value_to_serialized(value).map(|sv| (name.clone(), sv))
                    })
                    .collect();

                let indicator_buffers: Vec<(String, Vec<f64>)> = strategy
                    .interpreter
                    .indicators
                    .iter()
                    .filter_map(|(key, entry)| {
                        extract_indicator_buffer(entry).map(|buf| (key.clone(), buf))
                    })
                    .collect();

                StrategyState {
                    name: strategy.name.clone(),
                    state_variables,
                    indicator_buffers,
                }
            })
            .collect();

        HarnessState {
            version: 1,
            positions: position_state,
            strategy_states,
        }
    }
}

// --- Observability helper functions ---

/// Return a human-readable signal type string.
fn signal_kind_str(signal: &Signal) -> &'static str {
    match signal {
        Signal::Open { .. } => "OPEN",
        Signal::Close { .. } => "CLOSE",
        Signal::CloseQty { .. } => "CLOSE_QTY",
    }
}

/// Return a display string for the signal's quantity.
fn signal_qty_display(signal: &Signal) -> String {
    match signal.qty() {
        Some(q) => format!("{:.4}", q),
        None => "full".to_string(),
    }
}

/// Log a connector state transition to stderr.
///
/// This is a public helper intended to be called by connector management code
/// (or externally) whenever a connector changes state. It logs the connector
/// identifier, previous state, and new state in a consistent format.
///
/// # Requirements
/// Fulfills requirement 9.6: log all connector state transitions.
pub fn log_connector_state_transition(
    connector_id: &str,
    from: ConnectorState,
    to: ConnectorState,
) {
    eprintln!(
        "  [CONNECTOR] {} | {} -> {}",
        connector_id,
        format_connector_state(from),
        format_connector_state(to),
    );
}

/// Format a `ConnectorState` variant into a human-readable string.
fn format_connector_state(state: ConnectorState) -> String {
    match state {
        ConnectorState::Connecting => "connecting".to_string(),
        ConnectorState::Connected => "connected".to_string(),
        ConnectorState::Disconnected => "disconnected".to_string(),
        ConnectorState::Reconnecting { attempt } => format!("reconnecting (attempt {})", attempt),
        ConnectorState::PermanentlyFailed => "permanently failed".to_string(),
    }
}

/// Convert an interpreter `Value` to a `SerializedValue` for state persistence.
///
/// Returns `None` for values that cannot be serialized (signals, null, matrices).
fn value_to_serialized(value: &Value) -> Option<SerializedValue> {
    match value {
        Value::Int(i) => Some(SerializedValue::Int(*i)),
        Value::Float(f) => Some(SerializedValue::Float(*f)),
        Value::Str(s) => Some(SerializedValue::Str(s.clone())),
        Value::Bool(b) => Some(SerializedValue::Bool(*b)),
        Value::List(items) => {
            let serialized: Vec<SerializedValue> = items
                .iter()
                .filter_map(value_to_serialized)
                .collect();
            Some(SerializedValue::List(serialized))
        }
        // VecFloat can be stored as a list of floats
        Value::VecFloat(v) => {
            let serialized: Vec<SerializedValue> =
                v.iter().map(|f| SerializedValue::Float(*f)).collect();
            Some(SerializedValue::List(serialized))
        }
        // Signals, Null, MatFloat, Struct, Enum, and HashMap are not persisted
        Value::Null | Value::Signal(_) | Value::MatFloat { .. } | Value::Struct { .. } | Value::Enum { .. } | Value::HashMap(_) => None,
    }
}

/// Extract the main buffer from an indicator state entry for persistence.
///
/// Returns the circular buffer contents for SMA and RollingStats indicators.
/// For EMA-style indicators that don't maintain a buffer, returns None.
fn extract_indicator_buffer(entry: &crate::interpreter::IndicatorStateEntry) -> Option<Vec<f64>> {
    use crate::interpreter::IndicatorStateEntry;
    match entry {
        IndicatorStateEntry::Sma { buffer, .. } => Some(buffer.clone()),
        IndicatorStateEntry::RollingStats { buffer, .. } => Some(buffer.clone()),
        IndicatorStateEntry::RollingPair { buffer_a, .. } => Some(buffer_a.clone()),
        // EMA, RSI, ATR don't have meaningful buffers to persist directly
        IndicatorStateEntry::Ema { prev_ema, .. } => {
            prev_ema.map(|v| vec![v])
        }
        IndicatorStateEntry::Rsi { avg_gain, avg_loss, .. } => {
            Some(vec![*avg_gain, *avg_loss])
        }
        IndicatorStateEntry::Atr { atr_value, .. } => {
            atr_value.map(|v| vec![v])
        }
        IndicatorStateEntry::RollingMatrix { window, .. } => {
            // Flatten the window into a single vec
            Some(window.iter().flatten().copied().collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::aggregator::RiskConstraints;
    use flux_runtime::BarContext;

    /// Helper to create a LiveBar for testing.
    fn make_live_bar(symbol: &str, close: f64, open: f64) -> LiveBar {
        LiveBar {
            bar: BarContext {
                close,
                open,
                high: close + 1.0,
                low: open - 1.0,
                volume: 1000.0,
                symbol: symbol.to_string(),
                in_position: false,
            },
            connector_id: "test-connector".to_string(),
            received_at: chrono::Utc::now(),
        }
    }

    /// Helper to create a minimal harness with no strategies.
    fn make_empty_harness() -> LiveHarness {
        LiveHarness::new(
            vec![],
            SignalAggregator::new(RiskConstraints::default()),
            LivePositionTracker::new(10_000.0),
            None,
            ReconnectPolicy::default(),
            Duration::from_secs(30),
        )
    }

    #[test]
    fn dispatch_bar_with_no_strategies_does_nothing() {
        let mut harness = make_empty_harness();
        let bar = make_live_bar("AAPL", 150.0, 148.0);

        // Should not panic
        harness.dispatch_bar(&bar);

        // Tracker should have no fills
        assert!(harness.tracker.fill_attribution.is_empty());
    }

    #[test]
    fn dispatch_bar_marks_to_market() {
        let mut harness = make_empty_harness();

        // Manually open a position via the tracker
        let signal = Signal::open("AAPL".to_string(), 100.0);
        harness
            .tracker
            .process_signal(&signal, 150.0, 0, "manual");

        // Dispatch a bar with a different close price
        let bar = make_live_bar("AAPL", 155.0, 150.0);
        harness.dispatch_bar(&bar);

        // Position should reflect the new mark-to-market price
        let position = harness.tracker.inner.position("AAPL").unwrap();
        // unrealized_pnl = (155 - 150) * 100 = 500
        assert!((position.unrealized_pnl - 500.0).abs() < 0.01);
    }

    #[test]
    fn new_harness_has_correct_fields() {
        let harness = LiveHarness::new(
            vec![],
            SignalAggregator::new(RiskConstraints::default()),
            LivePositionTracker::new(50_000.0),
            Some(PathBuf::from("/tmp/state.json")),
            ReconnectPolicy::default(),
            Duration::from_secs(60),
        );

        assert!(harness.strategies.is_empty());
        assert_eq!(harness.state_file, Some(PathBuf::from("/tmp/state.json")));
        assert_eq!(harness.heartbeat_interval, Duration::from_secs(60));
        assert_eq!(harness.reconnect_policy.max_attempts, 10);
    }

    #[tokio::test]
    async fn run_exits_when_channel_closes() {
        let mut harness = make_empty_harness();
        let (tx, rx) = mpsc::channel::<LiveBar>(16);

        // Send one bar then drop the sender to close the channel
        let bar = make_live_bar("AAPL", 150.0, 148.0);
        tx.send(bar).await.unwrap();
        drop(tx);

        // run() with connector_count=0 should process the bar and exit cleanly
        let result = harness.run(rx, 0).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn run_dispatches_bars_from_channel() {
        let mut harness = make_empty_harness();

        // Open a position so mark-to-market has something to update
        let signal = Signal::open("AAPL".to_string(), 100.0);
        harness.tracker.process_signal(&signal, 150.0, 0, "manual");

        let (tx, rx) = mpsc::channel::<LiveBar>(16);

        // Send bars then close
        let bar1 = make_live_bar("AAPL", 155.0, 150.0);
        let bar2 = make_live_bar("AAPL", 160.0, 155.0);
        tx.send(bar1).await.unwrap();
        tx.send(bar2).await.unwrap();
        drop(tx);

        let result = harness.run(rx, 0).await;
        assert!(result.is_ok());

        // Should have marked to market at 160.0
        let position = harness.tracker.inner.position("AAPL").unwrap();
        // unrealized_pnl = (160 - 150) * 100 = 1000
        assert!((position.unrealized_pnl - 1000.0).abs() < 0.01);
    }

    #[tokio::test]
    async fn run_exits_immediately_on_empty_closed_channel() {
        let mut harness = make_empty_harness();
        let (_tx, rx) = mpsc::channel::<LiveBar>(16);
        drop(_tx);

        let result = harness.run(rx, 0).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn run_returns_error_when_connectors_fail() {
        let mut harness = make_empty_harness();
        let (_tx, rx) = mpsc::channel::<LiveBar>(16);
        drop(_tx);

        // With connector_count > 0, channel closing = all connectors failed
        let result = harness.run(rx, 2).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("all connectors failed"),
            "expected AllConnectorsFailed error, got: {}",
            err
        );
    }

    #[test]
    fn print_heartbeat_does_not_panic() {
        let harness = make_empty_harness();
        // Should not panic even with no positions
        harness.print_heartbeat();
    }

    #[test]
    fn print_startup_summary_does_not_panic() {
        let harness = make_empty_harness();
        // Should not panic with no strategies
        harness.print_startup_summary();
    }

    #[test]
    fn print_startup_summary_with_constraints_does_not_panic() {
        let harness = LiveHarness::new(
            vec![],
            SignalAggregator::new(RiskConstraints {
                max_position_size: Some(500.0),
                max_exposure: Some(100_000.0),
                max_positions: Some(10),
            }),
            LivePositionTracker::new(50_000.0),
            Some(PathBuf::from("/tmp/test-state.json")),
            ReconnectPolicy::default(),
            Duration::from_secs(60),
        );
        harness.print_startup_summary();
    }

    #[test]
    fn log_connector_state_transition_does_not_panic() {
        use super::log_connector_state_transition;

        // Test all state transitions
        log_connector_state_transition(
            "ws-0",
            ConnectorState::Connecting,
            ConnectorState::Connected,
        );
        log_connector_state_transition(
            "ws-0",
            ConnectorState::Connected,
            ConnectorState::Disconnected,
        );
        log_connector_state_transition(
            "ws-0",
            ConnectorState::Disconnected,
            ConnectorState::Reconnecting { attempt: 1 },
        );
        log_connector_state_transition(
            "ws-0",
            ConnectorState::Reconnecting { attempt: 10 },
            ConnectorState::PermanentlyFailed,
        );
    }

    #[test]
    fn signal_helpers_format_correctly() {
        use super::{signal_kind_str, signal_qty_display};

        let open = Signal::open("AAPL".to_string(), 100.0);
        assert_eq!(signal_kind_str(&open), "OPEN");
        assert_eq!(signal_qty_display(&open), "100.0000");

        let close = Signal::close("AAPL".to_string());
        assert_eq!(signal_kind_str(&close), "CLOSE");
        assert_eq!(signal_qty_display(&close), "full");

        let close_qty = Signal::close_qty("AAPL".to_string(), 50.0);
        assert_eq!(signal_kind_str(&close_qty), "CLOSE_QTY");
        assert_eq!(signal_qty_display(&close_qty), "50.0000");
    }
}
