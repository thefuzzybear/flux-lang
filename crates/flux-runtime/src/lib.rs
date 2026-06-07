//! Flux Runtime Library
//!
//! Provides the execution environment for compiled Flux strategies:
//! - `Strategy` trait for the strategy execution contract
//! - `BarContext` for market data access
//! - `Signal` for trade signal emission
//! - `sma` / `ema` indicator functions with per-call-site state
//! - `run_backtest` for backtesting strategies against historical data

mod strategy;
mod context;
mod signal;
pub mod indicators;
mod backtest;

pub use strategy::Strategy;
pub use context::BarContext;
pub use signal::Signal;
pub use indicators::{sma, ema};
pub use backtest::{run_backtest, BacktestResult};
