//! Risk limits module — stateful circuit-breaker layer for the live trading harness.
//!
//! Provides session-aware risk controls including daily/weekly P&L tracking,
//! drawdown monitoring, per-product position limits, notional exposure caps,
//! and correlation warnings. Sits upstream of the `SignalAggregator`.

use std::collections::HashMap;

use chrono::{Datelike, DateTime, NaiveTime};
use chrono_tz::Tz;
use flux_runtime::Signal;

use crate::live::market_calendar::MarketCalendar;
use crate::live::product_registry::ProductRegistry;

/// Configuration for the risk limits module.
/// Designed to be deserializable from TOML.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct RiskLimitsConfig {
    /// Maximum daily loss before flattening (negative value, e.g., -15_000.0).
    pub max_daily_loss: f64,
    /// Maximum weekly loss before flattening (negative value, e.g., -30_000.0).
    pub max_weekly_loss: f64,
    /// Maximum contracts per product symbol.
    pub max_position_per_product: u32,
    /// Maximum total notional exposure across all positions.
    pub max_total_notional: f64,
    /// Maximum drawdown from equity peak as a fraction (e.g., 0.08 for 8%).
    pub max_drawdown_pct: f64,
    /// Number of products that, if all long simultaneously, triggers a
    /// correlation warning (not a kill). E.g., 4 = warn if all 4 long.
    pub correlation_warning_threshold: usize,
    /// Starting equity for drawdown tracking.
    pub initial_equity: f64,
}

impl RiskLimitsConfig {
    /// Validate all configuration preconditions.
    ///
    /// Returns `Ok(())` if valid, or `Err(String)` with a descriptive message
    /// for the first invalid field encountered.
    pub fn validate(&self) -> Result<(), String> {
        if self.max_daily_loss >= 0.0 {
            return Err("max_daily_loss must be negative".to_string());
        }
        if self.max_weekly_loss >= 0.0 {
            return Err("max_weekly_loss must be negative".to_string());
        }
        if self.max_drawdown_pct <= 0.0 || self.max_drawdown_pct >= 1.0 {
            return Err(
                "max_drawdown_pct must be between 0.0 and 1.0 (exclusive)".to_string(),
            );
        }
        if self.max_position_per_product == 0 {
            return Err("max_position_per_product must be positive".to_string());
        }
        if self.max_total_notional <= 0.0 {
            return Err("max_total_notional must be positive".to_string());
        }
        if self.initial_equity <= 0.0 {
            return Err("initial_equity must be positive".to_string());
        }
        Ok(())
    }
}

/// Decision returned by the risk limits check.
#[derive(Debug, Clone, PartialEq)]
pub enum RiskDecision {
    /// Signal is allowed to proceed to the aggregator.
    Allow,
    /// Signal is rejected; trading continues for other signals.
    Reject { reason: RejectionReason },
    /// All positions must be flattened and the system halted.
    FlattenAll { reason: HaltReason },
}

/// Reason for halting the system.
#[derive(Debug, Clone, PartialEq)]
pub enum HaltReason {
    DailyLoss { pnl: f64, limit: f64 },
    WeeklyLoss { pnl: f64, limit: f64 },
    MaxDrawdown { drawdown_pct: f64, limit: f64 },
    /// Broker has been disconnected for more than 5 minutes.
    BrokerDisconnectionTimeout,
}

/// Reason for rejecting a signal.
#[derive(Debug, Clone, PartialEq)]
pub enum RejectionReason {
    PositionLimitExceeded {
        symbol: String,
        current: u32,
        requested: u32,
        limit: u32,
    },
    NotionalLimitExceeded {
        current_notional: f64,
        additional: f64,
        limit: f64,
    },
    UnknownSymbol {
        symbol: String,
    },
    MarginExceeded {
        symbol: String,
        required: f64,
        available: f64,
    },
    SystemHalted { reason: HaltReason },
}

/// Alert events emitted on limit triggers.
/// The harness forwards these to notification backends.
#[derive(Debug, Clone)]
pub enum AlertEvent {
    DailyLossBreached { pnl: f64, limit: f64 },
    WeeklyLossBreached { pnl: f64, limit: f64 },
    DrawdownBreached { drawdown_pct: f64, limit: f64 },
    PositionLimitRejected { symbol: String, current: u32, limit: u32 },
    NotionalLimitRejected { current: f64, limit: f64 },
    UnknownSymbolRejected { symbol: String },
    MarginExceededRejected { symbol: String, required: f64, available: f64 },
    CorrelationWarning { long_count: usize, symbols: Vec<String> },
    SystemHalted { reason: HaltReason },
    /// A broker order was rejected (e.g., insufficient margin, invalid contract).
    OrderRejected { order_id: String, reason: String },
    /// Broker has been disconnected beyond the 5-minute threshold.
    BrokerDisconnected { duration_secs: u64 },
    /// Position reconciliation detected a mismatch between broker and local state.
    PositionMismatch { symbol: String, local_qty: f64, broker_qty: f64 },
}

/// Snapshot of current portfolio state passed to check_signal().
/// Assembled by the harness from the LivePositionTracker.
pub struct PortfolioState {
    /// Per-symbol position quantities (signed: positive = long, negative = short).
    pub positions: HashMap<String, f64>,
    /// Per-symbol last known prices (for notional calculation).
    pub prices: HashMap<String, f64>,
    /// Current timestamp of the bar being processed.
    pub timestamp: DateTime<Tz>,
    /// Available margin for new positions.
    pub available_margin: f64,
}

/// The stateful risk limits engine.
pub struct RiskLimits {
    config: RiskLimitsConfig,
    /// Product registry for symbol lookups.
    registry: ProductRegistry,
    /// Market calendar for session awareness.
    calendar: MarketCalendar,
    /// Accumulated daily P&L (realized + unrealized at last mark).
    daily_pnl: f64,
    /// Accumulated weekly P&L (realized + unrealized at last mark).
    weekly_pnl: f64,
    /// Peak equity observed (high-water mark for drawdown).
    equity_peak: f64,
    /// Current equity.
    current_equity: f64,
    /// Per-symbol current position sizes (absolute qty held).
    position_sizes: HashMap<String, u32>,
    /// Current total notional across all positions.
    total_notional: f64,
    /// Whether the system is halted.
    halted: bool,
    /// Reason for halt, if any.
    halt_reason: Option<HaltReason>,
    /// Timestamp of last daily reset.
    last_daily_reset: DateTime<Tz>,
    /// Timestamp of last weekly reset.
    last_weekly_reset: DateTime<Tz>,
    /// Realized P&L accumulated today (for daily limit).
    realized_pnl_today: f64,
    /// Realized P&L accumulated this week (for weekly limit).
    realized_pnl_week: f64,
    /// Total realized P&L since inception.
    realized_pnl_total: f64,
}

/// Extract symbol and quantity from a Signal.
/// For Close signals, returns qty of 0.0 (they don't have meaningful qty for risk checks).
fn signal_symbol_qty(signal: &Signal) -> (String, f64) {
    match signal {
        Signal::Open { symbol, qty } => (symbol.clone(), *qty),
        Signal::Short { symbol, qty } => (symbol.clone(), *qty),
        Signal::Close { symbol } => (symbol.clone(), 0.0),
        Signal::CloseQty { symbol, qty } => (symbol.clone(), *qty),
    }
}

impl RiskLimits {
    /// Create a new RiskLimits engine from validated config.
    ///
    /// Calls `config.validate()` and returns an error if invalid.
    /// Initializes all accumulators to zero/clean state.
    pub fn new(config: RiskLimitsConfig, registry: ProductRegistry, calendar: MarketCalendar) -> Result<Self, String> {
        config.validate()?;

        let epoch_eastern = chrono::DateTime::UNIX_EPOCH.with_timezone(&chrono_tz::US::Eastern);

        Ok(Self {
            equity_peak: config.initial_equity,
            current_equity: config.initial_equity,
            config,
            registry,
            calendar,
            daily_pnl: 0.0,
            weekly_pnl: 0.0,
            position_sizes: HashMap::new(),
            total_notional: 0.0,
            halted: false,
            halt_reason: None,
            last_daily_reset: epoch_eastern,
            last_weekly_reset: epoch_eastern,
            realized_pnl_today: 0.0,
            realized_pnl_week: 0.0,
            realized_pnl_total: 0.0,
        })
    }

    /// Reset daily/weekly P&L accumulators when session boundaries are crossed.
    ///
    /// Skips all resets on non-trading days (holidays and weekends).
    /// Daily reset: triggered when timestamp >= today's session open and last_daily_reset < today's session open.
    /// Weekly reset: triggered when timestamp >= this week's Monday session open and last_weekly_reset < Monday session open.
    ///
    /// Daily loss halts are auto-cleared on daily reset. Weekly/drawdown halts are NOT auto-cleared.
    pub fn maybe_reset_session(&mut self, timestamp: DateTime<Tz>) {
        let eastern = chrono_tz::US::Eastern;
        let date = timestamp.with_timezone(&eastern).date_naive();

        // Skip all resets on non-trading days (holidays and weekends)
        if !self.calendar.is_trading_day(date) {
            return;
        }

        // Use calendar-provided session open for reset boundary
        let session_open_time = match self.calendar.session_times_for_date("CME", date) {
            Ok((open, _close)) => open,
            Err(_) => NaiveTime::from_hms_opt(9, 30, 0).unwrap(), // fallback
        };

        // Convert timestamp to Eastern for session boundary calculations
        let ts_eastern = timestamp.with_timezone(&eastern);

        // Compute today's session open in the exchange timezone
        let today_open = ts_eastern
            .date_naive()
            .and_time(session_open_time)
            .and_local_timezone(eastern)
            .unwrap();

        // Daily reset: if we've crossed session open since last reset
        if ts_eastern >= today_open && self.last_daily_reset < today_open {
            self.daily_pnl = 0.0;
            self.realized_pnl_today = 0.0;
            self.last_daily_reset = today_open;

            // Auto-clear daily loss halt only
            if matches!(self.halt_reason, Some(HaltReason::DailyLoss { .. })) {
                self.halted = false;
                self.halt_reason = None;
            }
        }

        // Weekly reset: compute this week's Monday session open
        let days_since_monday = ts_eastern.weekday().num_days_from_monday();
        let monday_date =
            ts_eastern.date_naive() - chrono::Duration::days(days_since_monday as i64);
        let monday_open = monday_date
            .and_time(session_open_time)
            .and_local_timezone(eastern)
            .unwrap();

        if ts_eastern >= monday_open && self.last_weekly_reset < monday_open {
            self.weekly_pnl = 0.0;
            self.realized_pnl_week = 0.0;
            self.last_weekly_reset = monday_open;
            // NOTE: Weekly halt is NOT auto-cleared — requires manual config edit
        }
    }

    /// Check for correlation warning — all products long simultaneously.
    /// Returns a warning alert if long count >= threshold. Non-blocking.
    pub fn check_correlation_warning(&self, state: &PortfolioState) -> Option<AlertEvent> {
        let long_symbols: Vec<String> = state
            .positions
            .iter()
            .filter(|(_, &qty)| qty > 0.0)
            .map(|(sym, _)| sym.clone())
            .collect();

        if long_symbols.len() >= self.config.correlation_warning_threshold {
            Some(AlertEvent::CorrelationWarning {
                long_count: long_symbols.len(),
                symbols: long_symbols,
            })
        } else {
            None
        }
    }

    /// Gate a signal through all risk checks.
    ///
    /// Returns a decision (Allow/Reject) and any alert events generated.
    /// CLOSE signals are never blocked. When halted, only CLOSE signals pass.
    pub fn check_signal(
        &mut self,
        signal: &Signal,
        state: &PortfolioState,
    ) -> (RiskDecision, Vec<AlertEvent>) {
        let mut alerts = Vec::new();

        // Step 1: Session boundary check (stub — just calls maybe_reset_session)
        self.maybe_reset_session(state.timestamp);

        // Step 2: If halted, allow CLOSE/CloseQty, reject all others
        if self.halted {
            match signal {
                Signal::Close { .. } | Signal::CloseQty { .. } => {
                    return (RiskDecision::Allow, alerts);
                }
                _ => {
                    return (
                        RiskDecision::Reject {
                            reason: RejectionReason::SystemHalted {
                                reason: self.halt_reason.clone().unwrap(),
                            },
                        },
                        alerts,
                    );
                }
            }
        }

        // Step 3: CLOSE signals always pass
        match signal {
            Signal::Close { .. } | Signal::CloseQty { .. } => {
                return (RiskDecision::Allow, alerts);
            }
            _ => {}
        }

        // Step 4: Extract symbol and qty, then check unknown symbol
        let (symbol, qty) = signal_symbol_qty(signal);

        // Step 4a: Unknown symbol check
        // Try direct lookup first, then fall back to stripping generic symbol suffix
        // (e.g., "ES=F" → "ES", "NQ=2" → "NQ") for futures roll manager symbols.
        let spec = match self.registry.get(&symbol) {
            Some(spec) => spec,
            None => {
                // Try stripping generic symbol suffix (everything after '=')
                let root = symbol.split('=').next().unwrap_or(&symbol);
                match self.registry.get(root) {
                    Some(spec) => spec,
                    None => {
                        alerts.push(AlertEvent::UnknownSymbolRejected {
                            symbol: symbol.clone(),
                        });
                        return (
                            RiskDecision::Reject {
                                reason: RejectionReason::UnknownSymbol { symbol },
                            },
                            alerts,
                        );
                    }
                }
            }
        };

        // Step 4b: Margin pre-check
        let required_margin = qty * spec.margin_initial;
        if required_margin > state.available_margin {
            alerts.push(AlertEvent::MarginExceededRejected {
                symbol: symbol.clone(),
                required: required_margin,
                available: state.available_margin,
            });
            return (
                RiskDecision::Reject {
                    reason: RejectionReason::MarginExceeded {
                        symbol,
                        required: required_margin,
                        available: state.available_margin,
                    },
                },
                alerts,
            );
        }

        // Step 5: Per-product position limit
        let current_pos = self.position_sizes.get(&symbol).copied().unwrap_or(0);
        let requested = current_pos + qty as u32;
        if requested > self.config.max_position_per_product {
            alerts.push(AlertEvent::PositionLimitRejected {
                symbol: symbol.clone(),
                current: current_pos,
                limit: self.config.max_position_per_product,
            });
            return (
                RiskDecision::Reject {
                    reason: RejectionReason::PositionLimitExceeded {
                        symbol,
                        current: current_pos,
                        requested,
                        limit: self.config.max_position_per_product,
                    },
                },
                alerts,
            );
        }

        // Step 6: Total notional limit (multiplier-aware)
        let price = state.prices.get(&symbol).copied().unwrap_or(0.0);
        let additional_notional = qty * price * spec.multiplier;
        if self.total_notional + additional_notional > self.config.max_total_notional {
            alerts.push(AlertEvent::NotionalLimitRejected {
                current: self.total_notional,
                limit: self.config.max_total_notional,
            });
            return (
                RiskDecision::Reject {
                    reason: RejectionReason::NotionalLimitExceeded {
                        current_notional: self.total_notional,
                        additional: additional_notional,
                        limit: self.config.max_total_notional,
                    },
                },
                alerts,
            );
        }

        // Step 7: Correlation warning (non-blocking)
        if let Some(warning) = self.check_correlation_warning(state) {
            alerts.push(warning);
        }

        (RiskDecision::Allow, alerts)
    }

    /// Compute unrealized P&L from current positions and market prices.
    ///
    /// `total_notional` tracks the cost basis (sum of qty × entry_price at fill time).
    /// Current market value = sum of abs(qty) × current_price for each position.
    /// Unrealized P&L = current_market_value - total_notional (cost basis).
    fn compute_unrealized_pnl(&self, state: &PortfolioState) -> f64 {
        let current_market_value: f64 = state
            .positions
            .iter()
            .map(|(symbol, &qty)| {
                let price = state.prices.get(symbol).copied().unwrap_or(0.0);
                qty.abs() * price
            })
            .sum();

        current_market_value - self.total_notional
    }

    /// Record a fill to keep internal position tracking accurate.
    ///
    /// Called after a fill is confirmed by the `LivePositionTracker`.
    ///
    /// For opening fills (Open/Short): increments `position_sizes` and adds to `total_notional`.
    /// For closing fills (Close/CloseQty): decrements `position_sizes`, reduces `total_notional`,
    /// and records realized P&L.
    ///
    /// `realized_pnl` is the P&L from this fill (0.0 for opening fills, non-zero for closing fills).
    pub fn record_fill(&mut self, signal: &Signal, fill_price: f64, qty: f64, realized_pnl: f64) {
        let (symbol, _) = signal_symbol_qty(signal);

        // Look up multiplier (default to 1.0 if not found — shouldn't happen post-check)
        let multiplier = self
            .registry
            .get(&symbol)
            .map(|spec| spec.multiplier)
            .unwrap_or(1.0);

        match signal {
            Signal::Open { .. } | Signal::Short { .. } => {
                // Opening fill: increase position and cost basis
                let current = self.position_sizes.get(&symbol).copied().unwrap_or(0);
                self.position_sizes.insert(symbol, current + qty as u32);
                self.total_notional += qty * fill_price * multiplier;
            }
            Signal::Close { .. } | Signal::CloseQty { .. } => {
                // Closing fill: decrease position and adjust cost basis
                let current = self.position_sizes.get(&symbol).copied().unwrap_or(0);
                let decrease = (qty as u32).min(current);
                let new_pos = current.saturating_sub(decrease);
                if new_pos == 0 {
                    self.position_sizes.remove(&symbol);
                } else {
                    self.position_sizes.insert(symbol, new_pos);
                }

                // Reduce cost basis proportionally
                self.total_notional =
                    (self.total_notional - qty * fill_price * multiplier).max(0.0);

                // Record realized P&L
                self.realized_pnl_today += realized_pnl;
                self.realized_pnl_week += realized_pnl;
                self.realized_pnl_total += realized_pnl;
            }
        }
    }

    /// Mark-to-market: recompute P&L and check daily/weekly/drawdown limits.
    ///
    /// Called once per bar after all signals have been processed.
    /// Returns `Some(FlattenAll)` if a P&L/drawdown threshold is breached, otherwise `None`.
    pub fn mark_to_market(
        &mut self,
        state: &PortfolioState,
    ) -> (Option<RiskDecision>, Vec<AlertEvent>) {
        let mut alerts = Vec::new();

        // If already halted, don't re-trigger
        if self.halted {
            return (None, alerts);
        }

        // Recompute current equity
        let unrealized_pnl = self.compute_unrealized_pnl(state);
        self.current_equity = self.config.initial_equity + self.realized_pnl_total + unrealized_pnl;
        self.daily_pnl = self.realized_pnl_today + unrealized_pnl;
        self.weekly_pnl = self.realized_pnl_week + unrealized_pnl;

        // Update equity peak (high-water mark)
        if self.current_equity > self.equity_peak {
            self.equity_peak = self.current_equity;
        }

        // Check daily loss limit
        if self.daily_pnl <= self.config.max_daily_loss {
            let reason = HaltReason::DailyLoss {
                pnl: self.daily_pnl,
                limit: self.config.max_daily_loss,
            };
            self.halted = true;
            self.halt_reason = Some(reason.clone());
            alerts.push(AlertEvent::DailyLossBreached {
                pnl: self.daily_pnl,
                limit: self.config.max_daily_loss,
            });
            alerts.push(AlertEvent::SystemHalted {
                reason: reason.clone(),
            });
            return (Some(RiskDecision::FlattenAll { reason }), alerts);
        }

        // Check weekly loss limit
        if self.weekly_pnl <= self.config.max_weekly_loss {
            let reason = HaltReason::WeeklyLoss {
                pnl: self.weekly_pnl,
                limit: self.config.max_weekly_loss,
            };
            self.halted = true;
            self.halt_reason = Some(reason.clone());
            alerts.push(AlertEvent::WeeklyLossBreached {
                pnl: self.weekly_pnl,
                limit: self.config.max_weekly_loss,
            });
            alerts.push(AlertEvent::SystemHalted {
                reason: reason.clone(),
            });
            return (Some(RiskDecision::FlattenAll { reason }), alerts);
        }

        // Check drawdown from peak
        if self.equity_peak > 0.0 {
            let drawdown_pct = (self.equity_peak - self.current_equity) / self.equity_peak;
            if drawdown_pct >= self.config.max_drawdown_pct {
                let reason = HaltReason::MaxDrawdown {
                    drawdown_pct,
                    limit: self.config.max_drawdown_pct,
                };
                self.halted = true;
                self.halt_reason = Some(reason.clone());
                alerts.push(AlertEvent::DrawdownBreached {
                    drawdown_pct,
                    limit: self.config.max_drawdown_pct,
                });
                alerts.push(AlertEvent::SystemHalted {
                    reason: reason.clone(),
                });
                return (Some(RiskDecision::FlattenAll { reason }), alerts);
            }
        }

        (None, alerts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use chrono_tz::US::Eastern;
    use crate::live::market_calendar::MarketCalendar;
    use crate::live::product_registry::ProductRegistry;

    fn default_calendar() -> MarketCalendar {
        let toml_str = r#"
[[session]]
exchange = "CME"
open = "09:30"
close = "16:00"
timezone = "US/Eastern"
"#;
        MarketCalendar::from_toml(toml_str).unwrap()
    }

    fn valid_config() -> RiskLimitsConfig {
        RiskLimitsConfig {
            max_daily_loss: -15_000.0,
            max_weekly_loss: -30_000.0,
            max_position_per_product: 10,
            max_total_notional: 3_000_000.0,
            max_drawdown_pct: 0.08,
            correlation_warning_threshold: 4,
            initial_equity: 500_000.0,
        }
    }

    fn empty_registry() -> ProductRegistry {
        ProductRegistry::from_entries(&[])
    }

    /// Registry with common test symbols (multiplier=1.0, tick_size=0.01, margin=1000.0).
    fn test_registry() -> ProductRegistry {
        use crate::live::account_config::ProductEntry;
        let entries = vec![
            ProductEntry { name: "AAPL".to_string(), multiplier: 1.0, tick_size: 0.01, margin: 1000.0 },
            ProductEntry { name: "MSFT".to_string(), multiplier: 1.0, tick_size: 0.01, margin: 1000.0 },
            ProductEntry { name: "GOOG".to_string(), multiplier: 1.0, tick_size: 0.01, margin: 1000.0 },
            ProductEntry { name: "AMZN".to_string(), multiplier: 1.0, tick_size: 0.01, margin: 1000.0 },
            ProductEntry { name: "TSLA".to_string(), multiplier: 1.0, tick_size: 0.01, margin: 1000.0 },
            ProductEntry { name: "ES".to_string(), multiplier: 50.0, tick_size: 0.25, margin: 15000.0 },
        ];
        ProductRegistry::from_entries(&entries)
    }

    fn test_timestamp() -> DateTime<Tz> {
        chrono::Utc
            .with_ymd_and_hms(2025, 7, 15, 14, 0, 0)
            .unwrap()
            .with_timezone(&Eastern)
            .with_timezone(&chrono_tz::US::Eastern)
    }

    fn empty_portfolio_state() -> PortfolioState {
        PortfolioState {
            positions: HashMap::new(),
            prices: HashMap::new(),
            timestamp: test_timestamp(),
            available_margin: f64::MAX,
        }
    }

    // --- Config validation tests (from task 1) ---

    #[test]
    fn test_valid_config_passes() {
        assert!(valid_config().validate().is_ok());
    }

    #[test]
    fn test_max_daily_loss_must_be_negative() {
        let mut config = valid_config();
        config.max_daily_loss = 0.0;
        assert_eq!(config.validate().unwrap_err(), "max_daily_loss must be negative");
        config.max_daily_loss = 100.0;
        assert_eq!(config.validate().unwrap_err(), "max_daily_loss must be negative");
    }

    #[test]
    fn test_max_weekly_loss_must_be_negative() {
        let mut config = valid_config();
        config.max_weekly_loss = 0.0;
        assert_eq!(config.validate().unwrap_err(), "max_weekly_loss must be negative");
        config.max_weekly_loss = 50.0;
        assert_eq!(config.validate().unwrap_err(), "max_weekly_loss must be negative");
    }

    #[test]
    fn test_max_drawdown_pct_range() {
        let mut config = valid_config();
        config.max_drawdown_pct = 0.0;
        assert_eq!(
            config.validate().unwrap_err(),
            "max_drawdown_pct must be between 0.0 and 1.0 (exclusive)"
        );
        config.max_drawdown_pct = -0.1;
        assert_eq!(
            config.validate().unwrap_err(),
            "max_drawdown_pct must be between 0.0 and 1.0 (exclusive)"
        );
        config.max_drawdown_pct = 1.0;
        assert_eq!(
            config.validate().unwrap_err(),
            "max_drawdown_pct must be between 0.0 and 1.0 (exclusive)"
        );
        config.max_drawdown_pct = 1.5;
        assert_eq!(
            config.validate().unwrap_err(),
            "max_drawdown_pct must be between 0.0 and 1.0 (exclusive)"
        );
    }

    #[test]
    fn test_max_position_per_product_must_be_positive() {
        let mut config = valid_config();
        config.max_position_per_product = 0;
        assert_eq!(config.validate().unwrap_err(), "max_position_per_product must be positive");
    }

    #[test]
    fn test_max_total_notional_must_be_positive() {
        let mut config = valid_config();
        config.max_total_notional = 0.0;
        assert_eq!(config.validate().unwrap_err(), "max_total_notional must be positive");
        config.max_total_notional = -100.0;
        assert_eq!(config.validate().unwrap_err(), "max_total_notional must be positive");
    }

    #[test]
    fn test_initial_equity_must_be_positive() {
        let mut config = valid_config();
        config.initial_equity = 0.0;
        assert_eq!(config.validate().unwrap_err(), "initial_equity must be positive");
        config.initial_equity = -1000.0;
        assert_eq!(config.validate().unwrap_err(), "initial_equity must be positive");
    }

    // --- Task 2.1: RiskLimits::new tests ---

    #[test]
    fn test_new_initializes_clean_state() {
        let rl = RiskLimits::new(valid_config(), empty_registry(), default_calendar()).unwrap();
        assert_eq!(rl.daily_pnl, 0.0);
        assert_eq!(rl.weekly_pnl, 0.0);
        assert_eq!(rl.equity_peak, 500_000.0);
        assert_eq!(rl.current_equity, 500_000.0);
        assert!(rl.position_sizes.is_empty());
        assert_eq!(rl.total_notional, 0.0);
        assert!(!rl.halted);
        assert!(rl.halt_reason.is_none());
        assert_eq!(rl.realized_pnl_today, 0.0);
        assert_eq!(rl.realized_pnl_week, 0.0);
        assert_eq!(rl.realized_pnl_total, 0.0);
    }

    #[test]
    fn test_new_rejects_invalid_config() {
        let mut config = valid_config();
        config.max_daily_loss = 100.0;
        assert!(RiskLimits::new(config, empty_registry(), default_calendar()).is_err());
    }

    // --- Task 2.2: check_signal tests ---

    #[test]
    fn test_close_signals_pass_when_halted() {
        let mut rl = RiskLimits::new(valid_config(), empty_registry(), default_calendar()).unwrap();
        rl.halted = true;
        rl.halt_reason = Some(HaltReason::DailyLoss {
            pnl: -16_000.0,
            limit: -15_000.0,
        });

        let state = empty_portfolio_state();

        // Close should pass
        let signal = Signal::close("AAPL".to_string());
        let (decision, _alerts) = rl.check_signal(&signal, &state);
        assert_eq!(decision, RiskDecision::Allow);

        // CloseQty should pass
        let signal = Signal::close_qty("AAPL".to_string(), 5.0);
        let (decision, _alerts) = rl.check_signal(&signal, &state);
        assert_eq!(decision, RiskDecision::Allow);
    }

    #[test]
    fn test_open_signals_rejected_when_halted() {
        let mut rl = RiskLimits::new(valid_config(), empty_registry(), default_calendar()).unwrap();
        rl.halted = true;
        rl.halt_reason = Some(HaltReason::DailyLoss {
            pnl: -16_000.0,
            limit: -15_000.0,
        });
        // Set last_daily_reset to today's session open so maybe_reset_session doesn't clear the halt
        let today_open = Eastern.with_ymd_and_hms(2025, 7, 15, 9, 30, 0).unwrap();
        rl.last_daily_reset = today_open;

        let state = empty_portfolio_state();

        let signal = Signal::open("AAPL".to_string(), 5.0);
        let (decision, _alerts) = rl.check_signal(&signal, &state);
        assert_eq!(
            decision,
            RiskDecision::Reject {
                reason: RejectionReason::SystemHalted {
                    reason: HaltReason::DailyLoss {
                        pnl: -16_000.0,
                        limit: -15_000.0,
                    },
                },
            }
        );

        // Short should also be rejected
        let signal = Signal::short("MSFT".to_string(), 3.0);
        let (decision, _alerts) = rl.check_signal(&signal, &state);
        assert!(matches!(
            decision,
            RiskDecision::Reject {
                reason: RejectionReason::SystemHalted { .. }
            }
        ));
    }

    #[test]
    fn test_position_limit_rejection() {
        let mut rl = RiskLimits::new(valid_config(), test_registry(), default_calendar()).unwrap();
        // Set current position to 8 for AAPL (limit is 10)
        rl.position_sizes.insert("AAPL".to_string(), 8);

        let state = empty_portfolio_state();

        // Request 3 more → 8 + 3 = 11 > 10 → reject
        let signal = Signal::open("AAPL".to_string(), 3.0);
        let (decision, alerts) = rl.check_signal(&signal, &state);
        assert_eq!(
            decision,
            RiskDecision::Reject {
                reason: RejectionReason::PositionLimitExceeded {
                    symbol: "AAPL".to_string(),
                    current: 8,
                    requested: 11,
                    limit: 10,
                },
            }
        );
        assert_eq!(alerts.len(), 1);
        assert!(matches!(alerts[0], AlertEvent::PositionLimitRejected { .. }));
    }

    #[test]
    fn test_notional_limit_rejection() {
        let mut rl = RiskLimits::new(valid_config(), test_registry(), default_calendar()).unwrap();
        // Set total notional close to limit (3_000_000)
        rl.total_notional = 2_900_000.0;

        let mut state = empty_portfolio_state();
        state.prices.insert("ES".to_string(), 50_000.0);

        // Request 5 contracts × $50,000 × 50 (multiplier) = $12,500,000
        // 2_900_000 + 12_500_000 > 3_000_000 → reject
        let signal = Signal::open("ES".to_string(), 5.0);
        let (decision, alerts) = rl.check_signal(&signal, &state);
        assert_eq!(
            decision,
            RiskDecision::Reject {
                reason: RejectionReason::NotionalLimitExceeded {
                    current_notional: 2_900_000.0,
                    additional: 12_500_000.0,
                    limit: 3_000_000.0,
                },
            }
        );
        assert_eq!(alerts.len(), 1);
        assert!(matches!(alerts[0], AlertEvent::NotionalLimitRejected { .. }));
    }

    #[test]
    fn test_allows_signal_within_limits() {
        let mut rl = RiskLimits::new(valid_config(), test_registry(), default_calendar()).unwrap();

        let mut state = empty_portfolio_state();
        state.prices.insert("AAPL".to_string(), 150.0);

        // 5 contracts × $150 × 1.0 (multiplier) = $750 notional, well within limits
        let signal = Signal::open("AAPL".to_string(), 5.0);
        let (decision, alerts) = rl.check_signal(&signal, &state);
        assert_eq!(decision, RiskDecision::Allow);
        assert!(alerts.is_empty());
    }

    #[test]
    fn test_correlation_warning_is_non_blocking() {
        let mut rl = RiskLimits::new(valid_config(), test_registry(), default_calendar()).unwrap();

        let mut state = empty_portfolio_state();
        // Set up 4 long positions (threshold is 4)
        state.positions.insert("AAPL".to_string(), 10.0);
        state.positions.insert("MSFT".to_string(), 5.0);
        state.positions.insert("GOOG".to_string(), 3.0);
        state.positions.insert("AMZN".to_string(), 7.0);
        state.prices.insert("TSLA".to_string(), 200.0);

        // Open a new position — should be allowed but with correlation warning
        let signal = Signal::open("TSLA".to_string(), 2.0);
        let (decision, alerts) = rl.check_signal(&signal, &state);
        assert_eq!(decision, RiskDecision::Allow);
        assert_eq!(alerts.len(), 1);
        assert!(matches!(alerts[0], AlertEvent::CorrelationWarning { .. }));
    }

    // --- Task 2.3: signal_symbol_qty tests ---

    #[test]
    fn test_signal_symbol_qty_open() {
        let signal = Signal::open("AAPL".to_string(), 10.0);
        let (sym, qty) = signal_symbol_qty(&signal);
        assert_eq!(sym, "AAPL");
        assert_eq!(qty, 10.0);
    }

    #[test]
    fn test_signal_symbol_qty_short() {
        let signal = Signal::short("MSFT".to_string(), 5.0);
        let (sym, qty) = signal_symbol_qty(&signal);
        assert_eq!(sym, "MSFT");
        assert_eq!(qty, 5.0);
    }

    #[test]
    fn test_signal_symbol_qty_close() {
        let signal = Signal::close("GOOG".to_string());
        let (sym, qty) = signal_symbol_qty(&signal);
        assert_eq!(sym, "GOOG");
        assert_eq!(qty, 0.0);
    }

    #[test]
    fn test_signal_symbol_qty_close_qty() {
        let signal = Signal::close_qty("AMZN".to_string(), 3.0);
        let (sym, qty) = signal_symbol_qty(&signal);
        assert_eq!(sym, "AMZN");
        assert_eq!(qty, 3.0);
    }

    // --- Task 2.4: check_correlation_warning tests ---

    #[test]
    fn test_correlation_warning_below_threshold() {
        let rl = RiskLimits::new(valid_config(), empty_registry(), default_calendar()).unwrap();

        let mut state = empty_portfolio_state();
        // Only 3 long positions (threshold is 4) → no warning
        state.positions.insert("AAPL".to_string(), 10.0);
        state.positions.insert("MSFT".to_string(), 5.0);
        state.positions.insert("GOOG".to_string(), 3.0);

        assert!(rl.check_correlation_warning(&state).is_none());
    }

    #[test]
    fn test_correlation_warning_at_threshold() {
        let rl = RiskLimits::new(valid_config(), empty_registry(), default_calendar()).unwrap();

        let mut state = empty_portfolio_state();
        // Exactly 4 long positions (threshold is 4) → warning
        state.positions.insert("AAPL".to_string(), 10.0);
        state.positions.insert("MSFT".to_string(), 5.0);
        state.positions.insert("GOOG".to_string(), 3.0);
        state.positions.insert("AMZN".to_string(), 7.0);

        let warning = rl.check_correlation_warning(&state);
        assert!(warning.is_some());
        if let Some(AlertEvent::CorrelationWarning { long_count, symbols }) = warning {
            assert_eq!(long_count, 4);
            assert_eq!(symbols.len(), 4);
        } else {
            panic!("Expected CorrelationWarning");
        }
    }

    #[test]
    fn test_correlation_warning_ignores_short_positions() {
        let rl = RiskLimits::new(valid_config(), empty_registry(), default_calendar()).unwrap();

        let mut state = empty_portfolio_state();
        // 3 long + 1 short → only 3 long → no warning
        state.positions.insert("AAPL".to_string(), 10.0);
        state.positions.insert("MSFT".to_string(), 5.0);
        state.positions.insert("GOOG".to_string(), 3.0);
        state.positions.insert("AMZN".to_string(), -7.0); // short

        assert!(rl.check_correlation_warning(&state).is_none());
    }

    // --- Task 3: mark_to_market and compute_unrealized_pnl tests ---

    #[test]
    fn test_mark_to_market_no_breach() {
        let mut rl = RiskLimits::new(valid_config(), empty_registry(), default_calendar()).unwrap();
        // Small position, no breach
        rl.total_notional = 10_000.0;
        let mut state = empty_portfolio_state();
        state.positions.insert("AAPL".to_string(), 10.0);
        state.prices.insert("AAPL".to_string(), 1050.0); // value = 10_500, unrealized = 500

        let (decision, alerts) = rl.mark_to_market(&state);
        assert!(decision.is_none());
        assert!(alerts.is_empty());
        assert_eq!(rl.current_equity, 500_000.0 + 500.0); // initial + unrealized
        assert_eq!(rl.equity_peak, 500_500.0); // new high
    }

    #[test]
    fn test_mark_to_market_daily_loss_breach() {
        let mut rl = RiskLimits::new(valid_config(), empty_registry(), default_calendar()).unwrap();
        // Simulate a big unrealized loss
        rl.total_notional = 1_000_000.0; // cost basis
        rl.realized_pnl_today = -5_000.0; // already lost 5k realized today

        let mut state = empty_portfolio_state();
        // Position worth much less than cost basis
        state.positions.insert("ES".to_string(), 10.0);
        state.prices.insert("ES".to_string(), 98_500.0); // value = 985_000, unrealized = -15_000
        // daily_pnl = realized_today (-5000) + unrealized (-15000) = -20_000 < -15_000 limit

        let (decision, alerts) = rl.mark_to_market(&state);
        assert!(decision.is_some());
        assert!(matches!(
            decision.unwrap(),
            RiskDecision::FlattenAll {
                reason: HaltReason::DailyLoss { .. }
            }
        ));
        assert!(rl.halted);
        assert_eq!(alerts.len(), 2); // DailyLossBreached + SystemHalted
    }

    #[test]
    fn test_mark_to_market_drawdown_breach() {
        let mut rl = RiskLimits::new(valid_config(), empty_registry(), default_calendar()).unwrap();
        // Set equity peak higher (like it grew to 550k then fell)
        rl.equity_peak = 550_000.0;
        // We need drawdown >= 8% without tripping daily loss (-15k) first.
        // 550_000 * 0.08 = 44_000 drawdown threshold → equity must be <= 506_000
        // Use realized_pnl_total = 9_500 so equity = 500k + 9_500 + unrealized
        // Unrealized = -4_500 → equity = 505_000
        // drawdown = (550_000 - 505_000) / 550_000 = 0.0818 > 0.08 ✓
        // daily_pnl = 0 + (-4_500) = -4_500 > -15_000 (no daily breach) ✓
        rl.realized_pnl_total = 9_500.0;
        rl.total_notional = 50_000.0;

        let mut state = empty_portfolio_state();
        state.positions.insert("NQ".to_string(), 10.0);
        state.prices.insert("NQ".to_string(), 4550.0); // value = 45_500
        // unrealized = 45_500 - 50_000 = -4_500
        // current_equity = 500_000 + 9_500 + (-4_500) = 505_000
        // drawdown = (550_000 - 505_000) / 550_000 = 0.0818 > 0.08

        let (decision, _alerts) = rl.mark_to_market(&state);
        assert!(decision.is_some());
        assert!(matches!(
            decision.unwrap(),
            RiskDecision::FlattenAll {
                reason: HaltReason::MaxDrawdown { .. }
            }
        ));
        assert!(rl.halted);
    }

    #[test]
    fn test_mark_to_market_equity_peak_only_increases() {
        let mut rl = RiskLimits::new(valid_config(), empty_registry(), default_calendar()).unwrap();
        rl.total_notional = 10_000.0;

        // First mark: profit
        let mut state = empty_portfolio_state();
        state.positions.insert("AAPL".to_string(), 10.0);
        state.prices.insert("AAPL".to_string(), 1200.0); // value = 12_000, unrealized = +2000
        let _ = rl.mark_to_market(&state);
        assert_eq!(rl.equity_peak, 502_000.0);

        // Second mark: loss (but peak stays at previous high)
        state.prices.insert("AAPL".to_string(), 900.0); // value = 9_000, unrealized = -1000
        let _ = rl.mark_to_market(&state);
        assert_eq!(rl.equity_peak, 502_000.0); // Should NOT decrease
        assert_eq!(rl.current_equity, 499_000.0); // 500k + (-1000)
    }

    #[test]
    fn test_mark_to_market_halted_returns_none() {
        let mut rl = RiskLimits::new(valid_config(), empty_registry(), default_calendar()).unwrap();
        rl.halted = true;
        rl.halt_reason = Some(HaltReason::DailyLoss {
            pnl: -20_000.0,
            limit: -15_000.0,
        });

        let state = empty_portfolio_state();
        let (decision, alerts) = rl.mark_to_market(&state);
        assert!(decision.is_none());
        assert!(alerts.is_empty());
    }

    // --- Task 4.1: maybe_reset_session tests ---

    #[test]
    fn test_daily_reset_crosses_session_boundary() {
        let mut rl = RiskLimits::new(valid_config(), empty_registry(), default_calendar()).unwrap();
        rl.daily_pnl = -5_000.0;
        rl.realized_pnl_today = -5_000.0;
        // last_daily_reset is at UNIX_EPOCH, so any 09:30 ET should trigger

        // Tuesday July 15, 2025 at 10:00 AM ET (after session open)
        let ts = Eastern.with_ymd_and_hms(2025, 7, 15, 10, 0, 0).unwrap();

        rl.maybe_reset_session(ts.with_timezone(&chrono_tz::US::Eastern));

        assert_eq!(rl.daily_pnl, 0.0);
        assert_eq!(rl.realized_pnl_today, 0.0);
    }

    #[test]
    fn test_daily_reset_clears_daily_halt() {
        let mut rl = RiskLimits::new(valid_config(), empty_registry(), default_calendar()).unwrap();
        rl.halted = true;
        rl.halt_reason = Some(HaltReason::DailyLoss {
            pnl: -16_000.0,
            limit: -15_000.0,
        });

        let ts = Eastern.with_ymd_and_hms(2025, 7, 15, 10, 0, 0).unwrap();
        rl.maybe_reset_session(ts.with_timezone(&chrono_tz::US::Eastern));

        assert!(!rl.halted);
        assert!(rl.halt_reason.is_none());
    }

    #[test]
    fn test_daily_reset_does_not_clear_weekly_halt() {
        let mut rl = RiskLimits::new(valid_config(), empty_registry(), default_calendar()).unwrap();
        rl.halted = true;
        rl.halt_reason = Some(HaltReason::WeeklyLoss {
            pnl: -35_000.0,
            limit: -30_000.0,
        });

        let ts = Eastern.with_ymd_and_hms(2025, 7, 15, 10, 0, 0).unwrap();
        rl.maybe_reset_session(ts.with_timezone(&chrono_tz::US::Eastern));

        assert!(rl.halted); // Weekly halt NOT cleared by daily reset
        assert!(matches!(
            rl.halt_reason,
            Some(HaltReason::WeeklyLoss { .. })
        ));
    }

    #[test]
    fn test_weekly_reset_on_monday() {
        let mut rl = RiskLimits::new(valid_config(), empty_registry(), default_calendar()).unwrap();
        rl.weekly_pnl = -10_000.0;
        rl.realized_pnl_week = -10_000.0;

        // Monday July 14, 2025 at 10:00 AM ET
        let ts = Eastern.with_ymd_and_hms(2025, 7, 14, 10, 0, 0).unwrap();
        rl.maybe_reset_session(ts.with_timezone(&chrono_tz::US::Eastern));

        assert_eq!(rl.weekly_pnl, 0.0);
        assert_eq!(rl.realized_pnl_week, 0.0);
    }

    #[test]
    fn test_no_reset_before_session_open() {
        let mut rl = RiskLimits::new(valid_config(), empty_registry(), default_calendar()).unwrap();
        rl.daily_pnl = -5_000.0;
        rl.realized_pnl_today = -5_000.0;
        // Set last_daily_reset to yesterday's session open so we don't trigger from EPOCH
        let yesterday_open = Eastern.with_ymd_and_hms(2025, 7, 14, 9, 30, 0).unwrap();
        rl.last_daily_reset = yesterday_open;

        // Now set timestamp to 9:00 AM (before session open)
        let ts = Eastern.with_ymd_and_hms(2025, 7, 15, 9, 0, 0).unwrap();
        rl.maybe_reset_session(ts.with_timezone(&chrono_tz::US::Eastern));

        // Should NOT reset — we haven't crossed today's 09:30
        assert_eq!(rl.daily_pnl, -5_000.0);
        assert_eq!(rl.realized_pnl_today, -5_000.0);
    }

    #[test]
    fn test_daily_reset_does_not_clear_drawdown_halt() {
        let mut rl = RiskLimits::new(valid_config(), empty_registry(), default_calendar()).unwrap();
        rl.halted = true;
        rl.halt_reason = Some(HaltReason::MaxDrawdown {
            drawdown_pct: 0.09,
            limit: 0.08,
        });

        let ts = Eastern.with_ymd_and_hms(2025, 7, 15, 10, 0, 0).unwrap();
        rl.maybe_reset_session(ts.with_timezone(&chrono_tz::US::Eastern));

        // Drawdown halt NOT cleared by daily reset
        assert!(rl.halted);
        assert!(matches!(
            rl.halt_reason,
            Some(HaltReason::MaxDrawdown { .. })
        ));
    }

    #[test]
    fn test_weekly_reset_midweek_does_not_reset_weekly() {
        let mut rl = RiskLimits::new(valid_config(), empty_registry(), default_calendar()).unwrap();
        rl.weekly_pnl = -10_000.0;
        rl.realized_pnl_week = -10_000.0;
        // Set last_weekly_reset to this week's Monday open (already triggered this week)
        let monday_open = Eastern.with_ymd_and_hms(2025, 7, 14, 9, 30, 0).unwrap();
        rl.last_weekly_reset = monday_open;

        // Tuesday at 10:00 — same week, already reset on Monday
        let ts = Eastern.with_ymd_and_hms(2025, 7, 15, 10, 0, 0).unwrap();
        rl.maybe_reset_session(ts.with_timezone(&chrono_tz::US::Eastern));

        // Weekly should NOT reset again mid-week
        assert_eq!(rl.weekly_pnl, -10_000.0);
        assert_eq!(rl.realized_pnl_week, -10_000.0);
    }

    #[test]
    fn test_monday_resets_both_daily_and_weekly() {
        let mut rl = RiskLimits::new(valid_config(), empty_registry(), default_calendar()).unwrap();
        rl.daily_pnl = -5_000.0;
        rl.realized_pnl_today = -5_000.0;
        rl.weekly_pnl = -20_000.0;
        rl.realized_pnl_week = -20_000.0;
        // Set last resets to previous Friday
        let friday_open = Eastern.with_ymd_and_hms(2025, 7, 11, 9, 30, 0).unwrap();
        rl.last_daily_reset = friday_open;
        rl.last_weekly_reset = Eastern.with_ymd_and_hms(2025, 7, 7, 9, 30, 0).unwrap(); // Previous Monday

        // Monday July 14 at 10:00 AM ET
        let ts = Eastern.with_ymd_and_hms(2025, 7, 14, 10, 0, 0).unwrap();
        rl.maybe_reset_session(ts.with_timezone(&chrono_tz::US::Eastern));

        // Both daily and weekly should reset
        assert_eq!(rl.daily_pnl, 0.0);
        assert_eq!(rl.realized_pnl_today, 0.0);
        assert_eq!(rl.weekly_pnl, 0.0);
        assert_eq!(rl.realized_pnl_week, 0.0);
    }

    // --- Task 6.1: record_fill tests ---

    #[test]
    fn test_record_fill_open_increases_position() {
        let mut rl = RiskLimits::new(valid_config(), empty_registry(), default_calendar()).unwrap();

        let signal = Signal::open("ES".to_string(), 3.0);
        rl.record_fill(&signal, 5000.0, 3.0, 0.0);

        assert_eq!(rl.position_sizes.get("ES"), Some(&3));
        assert_eq!(rl.total_notional, 15_000.0);
    }

    #[test]
    fn test_record_fill_short_increases_position() {
        let mut rl = RiskLimits::new(valid_config(), empty_registry(), default_calendar()).unwrap();

        let signal = Signal::short("NQ".to_string(), 2.0);
        rl.record_fill(&signal, 18_000.0, 2.0, 0.0);

        assert_eq!(rl.position_sizes.get("NQ"), Some(&2));
        assert_eq!(rl.total_notional, 36_000.0);
    }

    #[test]
    fn test_record_fill_close_decreases_position() {
        let mut rl = RiskLimits::new(valid_config(), empty_registry(), default_calendar()).unwrap();
        rl.position_sizes.insert("ES".to_string(), 5);
        rl.total_notional = 25_000.0;

        let signal = Signal::close_qty("ES".to_string(), 2.0);
        rl.record_fill(&signal, 5100.0, 2.0, 200.0); // $200 profit

        assert_eq!(rl.position_sizes.get("ES"), Some(&3));
        assert_eq!(rl.total_notional, 14_800.0); // 25000 - 2*5100 = 14800
        assert_eq!(rl.realized_pnl_today, 200.0);
        assert_eq!(rl.realized_pnl_week, 200.0);
        assert_eq!(rl.realized_pnl_total, 200.0);
    }

    #[test]
    fn test_record_fill_close_full_position() {
        let mut rl = RiskLimits::new(valid_config(), empty_registry(), default_calendar()).unwrap();
        rl.position_sizes.insert("AAPL".to_string(), 10);
        rl.total_notional = 15_000.0;

        let signal = Signal::close("AAPL".to_string());
        // Close full position — qty should be the full position size
        rl.record_fill(&signal, 160.0, 10.0, 500.0);

        assert!(rl.position_sizes.get("AAPL").is_none()); // Removed entirely
        // total_notional: 15000 - 10*160 = 13400
        assert_eq!(rl.total_notional, 13_400.0);
        assert_eq!(rl.realized_pnl_total, 500.0);
    }

    #[test]
    fn test_record_fill_accumulates_realized_pnl() {
        let mut rl = RiskLimits::new(valid_config(), empty_registry(), default_calendar()).unwrap();
        rl.position_sizes.insert("ES".to_string(), 5);
        rl.total_notional = 25_000.0;

        // First close: +$300
        let signal = Signal::close_qty("ES".to_string(), 2.0);
        rl.record_fill(&signal, 5100.0, 2.0, 300.0);

        // Second close: -$100
        let signal2 = Signal::close_qty("ES".to_string(), 1.0);
        rl.record_fill(&signal2, 4900.0, 1.0, -100.0);

        assert_eq!(rl.realized_pnl_today, 200.0); // 300 + (-100)
        assert_eq!(rl.realized_pnl_week, 200.0);
        assert_eq!(rl.realized_pnl_total, 200.0);
    }

    #[test]
    fn test_record_fill_open_accumulates_positions() {
        let mut rl = RiskLimits::new(valid_config(), empty_registry(), default_calendar()).unwrap();

        // First open: 3 contracts
        let signal = Signal::open("ES".to_string(), 3.0);
        rl.record_fill(&signal, 5000.0, 3.0, 0.0);

        // Second open: 2 more contracts
        let signal2 = Signal::open("ES".to_string(), 2.0);
        rl.record_fill(&signal2, 5050.0, 2.0, 0.0);

        assert_eq!(rl.position_sizes.get("ES"), Some(&5));
        assert_eq!(rl.total_notional, 25_100.0); // 3*5000 + 2*5050
    }

    #[test]
    fn test_record_fill_close_clamps_notional_to_zero() {
        let mut rl = RiskLimits::new(valid_config(), empty_registry(), default_calendar()).unwrap();
        rl.position_sizes.insert("AAPL".to_string(), 2);
        rl.total_notional = 100.0; // Very small notional

        // Close at a much higher price — notional would go negative without clamping
        let signal = Signal::close("AAPL".to_string());
        rl.record_fill(&signal, 200.0, 2.0, 50.0);

        assert_eq!(rl.total_notional, 0.0); // Clamped to 0, not -300
        assert!(rl.position_sizes.get("AAPL").is_none());
    }

    #[test]
    fn test_record_fill_close_more_than_position() {
        let mut rl = RiskLimits::new(valid_config(), empty_registry(), default_calendar()).unwrap();
        rl.position_sizes.insert("AAPL".to_string(), 3);
        rl.total_notional = 3000.0;

        // Try to close 5, but only have 3 — should saturate at 0
        let signal = Signal::close_qty("AAPL".to_string(), 5.0);
        rl.record_fill(&signal, 100.0, 5.0, 0.0);

        assert!(rl.position_sizes.get("AAPL").is_none()); // Removed (clamped to 0)
    }

    // --- Task 5.2: Unknown symbol rejection tests ---

    #[test]
    fn test_open_unknown_symbol_rejected() {
        let mut rl = RiskLimits::new(valid_config(), test_registry(), default_calendar()).unwrap();
        let state = empty_portfolio_state();

        let signal = Signal::open("UNKNOWN".to_string(), 1.0);
        let (decision, alerts) = rl.check_signal(&signal, &state);

        assert_eq!(
            decision,
            RiskDecision::Reject {
                reason: RejectionReason::UnknownSymbol {
                    symbol: "UNKNOWN".to_string(),
                },
            }
        );
        assert_eq!(alerts.len(), 1);
        assert!(matches!(
            &alerts[0],
            AlertEvent::UnknownSymbolRejected { symbol } if symbol == "UNKNOWN"
        ));
    }

    #[test]
    fn test_short_unknown_symbol_rejected() {
        let mut rl = RiskLimits::new(valid_config(), test_registry(), default_calendar()).unwrap();
        let state = empty_portfolio_state();

        let signal = Signal::short("NOPE".to_string(), 2.0);
        let (decision, alerts) = rl.check_signal(&signal, &state);

        assert_eq!(
            decision,
            RiskDecision::Reject {
                reason: RejectionReason::UnknownSymbol {
                    symbol: "NOPE".to_string(),
                },
            }
        );
        assert_eq!(alerts.len(), 1);
        assert!(matches!(
            &alerts[0],
            AlertEvent::UnknownSymbolRejected { symbol } if symbol == "NOPE"
        ));
    }

    #[test]
    fn test_unknown_symbol_alert_event_emitted() {
        let mut rl = RiskLimits::new(valid_config(), test_registry(), default_calendar()).unwrap();
        let state = empty_portfolio_state();

        let signal = Signal::open("XYZ".to_string(), 1.0);
        let (_decision, alerts) = rl.check_signal(&signal, &state);

        assert_eq!(alerts.len(), 1);
        match &alerts[0] {
            AlertEvent::UnknownSymbolRejected { symbol } => {
                assert_eq!(symbol, "XYZ");
            }
            other => panic!("Expected UnknownSymbolRejected, got {:?}", other),
        }
    }

    #[test]
    fn test_known_symbols_pass_unknown_check() {
        let mut rl = RiskLimits::new(valid_config(), test_registry(), default_calendar()).unwrap();
        let mut state = empty_portfolio_state();
        state.prices.insert("AAPL".to_string(), 150.0);
        state.prices.insert("ES".to_string(), 5000.0);

        // AAPL is in the registry — should not reject as UnknownSymbol
        let signal = Signal::open("AAPL".to_string(), 1.0);
        let (decision, _alerts) = rl.check_signal(&signal, &state);
        assert_eq!(decision, RiskDecision::Allow);

        // ES is in the registry — should not reject as UnknownSymbol
        let signal = Signal::short("ES".to_string(), 1.0);
        let (decision, _alerts) = rl.check_signal(&signal, &state);
        assert_eq!(decision, RiskDecision::Allow);
    }

    // --- Task 5.3: Margin pre-check tests ---

    #[test]
    fn test_open_exceeding_margin_rejected() {
        let mut rl = RiskLimits::new(valid_config(), test_registry(), default_calendar()).unwrap();
        let mut state = empty_portfolio_state();
        state.available_margin = 500.0; // Only $500 available
        state.prices.insert("AAPL".to_string(), 150.0);

        // AAPL margin_initial = 1000.0, qty=1 → required = 1*1000 = 1000 > 500
        let signal = Signal::open("AAPL".to_string(), 1.0);
        let (decision, alerts) = rl.check_signal(&signal, &state);

        assert_eq!(
            decision,
            RiskDecision::Reject {
                reason: RejectionReason::MarginExceeded {
                    symbol: "AAPL".to_string(),
                    required: 1000.0,
                    available: 500.0,
                },
            }
        );
        assert_eq!(alerts.len(), 1);
        assert!(matches!(&alerts[0], AlertEvent::MarginExceededRejected { .. }));
    }

    #[test]
    fn test_short_exceeding_margin_rejected() {
        let mut rl = RiskLimits::new(valid_config(), test_registry(), default_calendar()).unwrap();
        let mut state = empty_portfolio_state();
        state.available_margin = 10_000.0; // $10k available
        state.prices.insert("ES".to_string(), 5000.0);

        // ES margin_initial = 15000.0, qty=1 → required = 1*15000 = 15000 > 10000
        let signal = Signal::short("ES".to_string(), 1.0);
        let (decision, alerts) = rl.check_signal(&signal, &state);

        assert_eq!(
            decision,
            RiskDecision::Reject {
                reason: RejectionReason::MarginExceeded {
                    symbol: "ES".to_string(),
                    required: 15000.0,
                    available: 10_000.0,
                },
            }
        );
        assert_eq!(alerts.len(), 1);
        assert!(matches!(&alerts[0], AlertEvent::MarginExceededRejected { .. }));
    }

    #[test]
    fn test_signal_within_margin_passes() {
        let mut rl = RiskLimits::new(valid_config(), test_registry(), default_calendar()).unwrap();
        let mut state = empty_portfolio_state();
        state.available_margin = 5000.0; // $5k available
        state.prices.insert("AAPL".to_string(), 150.0);

        // AAPL margin_initial = 1000.0, qty=2 → required = 2*1000 = 2000 <= 5000
        let signal = Signal::open("AAPL".to_string(), 2.0);
        let (decision, _alerts) = rl.check_signal(&signal, &state);

        assert_eq!(decision, RiskDecision::Allow);
    }

    #[test]
    fn test_margin_alert_contains_correct_values() {
        let mut rl = RiskLimits::new(valid_config(), test_registry(), default_calendar()).unwrap();
        let mut state = empty_portfolio_state();
        state.available_margin = 2500.0;
        state.prices.insert("MSFT".to_string(), 400.0);

        // MSFT margin_initial = 1000.0, qty=3 → required = 3*1000 = 3000 > 2500
        let signal = Signal::open("MSFT".to_string(), 3.0);
        let (_decision, alerts) = rl.check_signal(&signal, &state);

        assert_eq!(alerts.len(), 1);
        match &alerts[0] {
            AlertEvent::MarginExceededRejected {
                symbol,
                required,
                available,
            } => {
                assert_eq!(symbol, "MSFT");
                assert_eq!(*required, 3000.0);
                assert_eq!(*available, 2500.0);
            }
            other => panic!("Expected MarginExceededRejected, got {:?}", other),
        }
    }

    // --- Task 5.4: Multiplier-aware notional tests ---

    #[test]
    fn test_notional_uses_multiplier() {
        let mut rl = RiskLimits::new(valid_config(), test_registry(), default_calendar()).unwrap();
        let mut state = empty_portfolio_state();
        state.prices.insert("ES".to_string(), 5000.0);

        // ES multiplier = 50.0
        // Set total_notional close to limit so we can verify multiplier is used
        // additional_notional = qty(1) × price(5000) × multiplier(50) = 250_000
        // If multiplier wasn't used, it would be 1 × 5000 = 5000 (well within limit)
        rl.total_notional = 2_800_000.0;

        // 2_800_000 + 250_000 = 3_050_000 > 3_000_000 limit → reject
        let signal = Signal::open("ES".to_string(), 1.0);
        let (decision, _alerts) = rl.check_signal(&signal, &state);

        assert_eq!(
            decision,
            RiskDecision::Reject {
                reason: RejectionReason::NotionalLimitExceeded {
                    current_notional: 2_800_000.0,
                    additional: 250_000.0, // qty × price × multiplier = 1 × 5000 × 50
                    limit: 3_000_000.0,
                },
            }
        );
    }

    #[test]
    fn test_record_fill_updates_notional_with_multiplier() {
        let mut rl = RiskLimits::new(valid_config(), test_registry(), default_calendar()).unwrap();

        // ES multiplier = 50.0
        let signal = Signal::open("ES".to_string(), 2.0);
        rl.record_fill(&signal, 5000.0, 2.0, 0.0);

        // total_notional = qty(2) × fill_price(5000) × multiplier(50) = 500_000
        assert_eq!(rl.total_notional, 500_000.0);

        // AAPL multiplier = 1.0
        let signal2 = Signal::open("AAPL".to_string(), 10.0);
        rl.record_fill(&signal2, 150.0, 10.0, 0.0);

        // total_notional += qty(10) × fill_price(150) × multiplier(1) = 1_500
        assert_eq!(rl.total_notional, 501_500.0);
    }

    // --- Task 5.5: Check ordering (unknown → margin → position) ---

    #[test]
    fn test_check_ordering_margin_before_position() {
        let mut rl = RiskLimits::new(valid_config(), test_registry(), default_calendar()).unwrap();

        // Set position at the limit for AAPL (max_position_per_product = 10)
        rl.position_sizes.insert("AAPL".to_string(), 10);

        let mut state = empty_portfolio_state();
        state.available_margin = 500.0; // Only $500, AAPL requires 1000 per qty
        state.prices.insert("AAPL".to_string(), 150.0);

        // This signal would fail BOTH:
        // - Margin check: qty(1) × margin(1000) = 1000 > 500 available
        // - Position limit: 10 + 1 = 11 > 10 limit
        // Should reject with MarginExceeded (checked first), NOT PositionLimitExceeded
        let signal = Signal::open("AAPL".to_string(), 1.0);
        let (decision, _alerts) = rl.check_signal(&signal, &state);

        assert_eq!(
            decision,
            RiskDecision::Reject {
                reason: RejectionReason::MarginExceeded {
                    symbol: "AAPL".to_string(),
                    required: 1000.0,
                    available: 500.0,
                },
            }
        );
    }

    // --- Task 2.3: Calendar-aware session reset tests ---

    /// Calendar with 2026-01-01 as a holiday (New Year's Day).
    fn calendar_with_holiday() -> MarketCalendar {
        let toml_str = r#"
[[session]]
exchange = "CME"
open = "09:30"
close = "16:00"
timezone = "US/Eastern"

[holidays_2026]
dates = ["2026-01-01"]
"#;
        MarketCalendar::from_toml(toml_str).unwrap()
    }

    /// Calendar with CME open at a custom time.
    fn calendar_with_open(open_time: &str) -> MarketCalendar {
        let toml_str = format!(
            r#"
[[session]]
exchange = "CME"
open = "{}"
close = "16:00"
timezone = "US/Eastern"
"#,
            open_time
        );
        MarketCalendar::from_toml(&toml_str).unwrap()
    }

    #[test]
    fn test_maybe_reset_session_holiday_no_reset() {
        let mut rl =
            RiskLimits::new(valid_config(), empty_registry(), calendar_with_holiday()).unwrap();
        rl.daily_pnl = -5_000.0;
        rl.realized_pnl_today = -5_000.0;

        // 2026-01-01 is a holiday (Thursday) — no reset should occur
        let ts = Eastern.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap();
        rl.maybe_reset_session(ts.with_timezone(&chrono_tz::US::Eastern));

        assert_eq!(rl.daily_pnl, -5_000.0);
        assert_eq!(rl.realized_pnl_today, -5_000.0);
    }

    #[test]
    fn test_maybe_reset_session_weekend_no_reset() {
        let mut rl = RiskLimits::new(valid_config(), empty_registry(), default_calendar()).unwrap();
        rl.daily_pnl = -3_000.0;
        rl.realized_pnl_today = -3_000.0;
        rl.weekly_pnl = -3_000.0;
        rl.realized_pnl_week = -3_000.0;

        // 2026-01-03 is a Saturday — no reset should occur
        let ts = Eastern.with_ymd_and_hms(2026, 1, 3, 10, 0, 0).unwrap();
        rl.maybe_reset_session(ts.with_timezone(&chrono_tz::US::Eastern));

        assert_eq!(rl.daily_pnl, -3_000.0);
        assert_eq!(rl.realized_pnl_today, -3_000.0);
        assert_eq!(rl.weekly_pnl, -3_000.0);
        assert_eq!(rl.realized_pnl_week, -3_000.0);
    }

    #[test]
    fn test_maybe_reset_session_trading_day_resets() {
        let mut rl = RiskLimits::new(valid_config(), empty_registry(), default_calendar()).unwrap();
        rl.daily_pnl = -5_000.0;
        rl.realized_pnl_today = -5_000.0;

        // 2026-01-05 is a Monday (trading day), 10:00 ET is after session open (09:30)
        let ts = Eastern.with_ymd_and_hms(2026, 1, 5, 10, 0, 0).unwrap();
        rl.maybe_reset_session(ts.with_timezone(&chrono_tz::US::Eastern));

        assert_eq!(rl.daily_pnl, 0.0);
        assert_eq!(rl.realized_pnl_today, 0.0);
    }

    #[test]
    fn test_maybe_reset_session_uses_calendar_open_time() {
        // Calendar with CME open at 10:00 instead of 09:30
        let mut rl =
            RiskLimits::new(valid_config(), empty_registry(), calendar_with_open("10:00")).unwrap();
        rl.daily_pnl = -5_000.0;
        rl.realized_pnl_today = -5_000.0;

        // 2026-01-05 is a Monday (trading day).
        // At 09:45 ET — before the calendar open of 10:00, no reset should occur
        let ts_before = Eastern.with_ymd_and_hms(2026, 1, 5, 9, 45, 0).unwrap();
        rl.maybe_reset_session(ts_before.with_timezone(&chrono_tz::US::Eastern));

        assert_eq!(rl.daily_pnl, -5_000.0);
        assert_eq!(rl.realized_pnl_today, -5_000.0);

        // At 10:15 ET — after the calendar open of 10:00, reset should occur
        let ts_after = Eastern.with_ymd_and_hms(2026, 1, 5, 10, 15, 0).unwrap();
        rl.maybe_reset_session(ts_after.with_timezone(&chrono_tz::US::Eastern));

        assert_eq!(rl.daily_pnl, 0.0);
        assert_eq!(rl.realized_pnl_today, 0.0);
    }
}
