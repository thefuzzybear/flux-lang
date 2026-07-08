//! Live trading harness module.
//!
//! Provides the runtime infrastructure for `flux live` — a persistent,
//! event-driven loop that connects compiled strategies to live market data
//! via pluggable connectors, aggregates signals through portfolio-level risk
//! constraints, and persists state across restarts.

pub mod connector;
pub mod harness;
pub mod aggregator;
pub mod position;
pub mod state;
pub mod loader;
pub mod reconnect;
pub mod replay_connector;
pub mod ws_connector;
pub mod poll_connector;
