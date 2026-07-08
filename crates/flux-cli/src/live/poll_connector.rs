//! Polling connector for fetching OHLCV data at configurable intervals via HTTP.
//!
//! Periodically makes HTTP GET requests to a configured URL, parses the JSON
//! response into `BarContext` values, and sends them as `LiveBar` events over
//! the mpsc channel. Uses `tokio::time::interval` for scheduling polls.
//!
//! Expected JSON response format:
//! ```json
//! {
//!   "bars": [
//!     {
//!       "symbol": "AAPL",
//!       "open": 150.0,
//!       "high": 152.0,
//!       "low": 149.0,
//!       "close": 151.0,
//!       "volume": 1000000.0
//!     }
//!   ]
//! }
//! ```

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use flux_runtime::BarContext;

use super::connector::{Connector, ConnectorError, ConnectorState, LiveBar};

/// A connector that polls an HTTP endpoint at a fixed interval for OHLCV data.
///
/// Each poll makes an HTTP GET request to the configured URL, parses the JSON
/// response into one or more `BarContext` values, and sends them over the
/// channel as `LiveBar` events.
///
/// If a request fails (network error, parse error), the connector logs a
/// warning and continues polling — it does not disconnect.
pub struct PollingConnector {
    /// Human-readable identifier for this connector instance.
    id: String,
    /// HTTP endpoint URL to fetch OHLCV data from.
    url: String,
    /// Interval between poll requests.
    interval: Duration,
    /// Current connection state.
    state: ConnectorState,
    /// Symbols this connector is subscribed to (used for URL construction or filtering).
    symbols: Vec<String>,
    /// Handle to the spawned polling task (if connected).
    task_handle: Option<JoinHandle<()>>,
}

/// Expected JSON response structure from the polling endpoint.
#[derive(Debug, Deserialize)]
struct PollResponse {
    bars: Vec<BarData>,
}

/// A single bar entry in the JSON response.
#[derive(Debug, Deserialize)]
struct BarData {
    symbol: String,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
}

impl PollingConnector {
    /// Create a new polling connector.
    ///
    /// # Arguments
    /// - `id` — Human-readable identifier for observability
    /// - `url` — HTTP endpoint URL to poll for OHLCV data
    /// - `interval` — Duration between poll requests
    pub fn new(id: impl Into<String>, url: impl Into<String>, interval: Duration) -> Self {
        Self {
            id: id.into(),
            url: url.into(),
            interval,
            state: ConnectorState::Disconnected,
            symbols: Vec::new(),
            task_handle: None,
        }
    }

    /// Build the request URL, optionally incorporating subscribed symbols.
    ///
    /// If the base URL contains `{symbols}`, it will be replaced with a
    /// comma-separated list of subscribed symbols. Otherwise returns the
    /// base URL unchanged.
    fn build_url(&self) -> String {
        if self.symbols.is_empty() {
            return self.url.clone();
        }

        let symbols_param = self.symbols.join(",");
        self.url.replace("{symbols}", &symbols_param)
    }
}

/// Parse a JSON response body into a vec of `BarContext` values.
///
/// Filters bars by the subscribed symbols list. If the symbols list is
/// empty, all bars are included.
fn parse_poll_response(
    body: &str,
    subscribed_symbols: &[String],
) -> Result<Vec<BarContext>, ConnectorError> {
    let response: PollResponse =
        serde_json::from_str(body).map_err(|e| ConnectorError::ParseError(e.to_string()))?;

    let bars = response
        .bars
        .into_iter()
        .filter(|bar_data| {
            subscribed_symbols.is_empty() || subscribed_symbols.contains(&bar_data.symbol)
        })
        .map(|bar_data| BarContext {
            symbol: bar_data.symbol,
            open: bar_data.open,
            high: bar_data.high,
            low: bar_data.low,
            close: bar_data.close,
            volume: bar_data.volume,
            in_position: false, // Set by the harness before dispatch
        })
        .collect();

    Ok(bars)
}

#[async_trait]
impl Connector for PollingConnector {
    fn id(&self) -> &str {
        &self.id
    }

    fn state(&self) -> ConnectorState {
        self.state
    }

    async fn connect(
        &mut self,
        symbols: &[String],
        tx: mpsc::Sender<LiveBar>,
    ) -> Result<(), ConnectorError> {
        self.state = ConnectorState::Connecting;
        self.symbols = symbols.to_vec();

        let connector_id = self.id.clone();
        let url = self.build_url();
        let interval_duration = self.interval;
        let subscribed_symbols = self.symbols.clone();

        // Build an async reqwest client for the polling task.
        let client = reqwest::Client::new();

        // Spawn a tokio task that polls at the configured interval.
        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval_duration);

            loop {
                ticker.tick().await;

                // Make the HTTP GET request.
                let response = match client.get(&url).send().await {
                    Ok(resp) => resp,
                    Err(e) => {
                        eprintln!(
                            "  [{}] poll request failed: {} (will retry next interval)",
                            connector_id, e
                        );
                        continue;
                    }
                };

                // Read the response body.
                let body = match response.text().await {
                    Ok(text) => text,
                    Err(e) => {
                        eprintln!(
                            "  [{}] failed to read response body: {} (will retry next interval)",
                            connector_id, e
                        );
                        continue;
                    }
                };

                // Parse JSON into BarContext values.
                let bars = match parse_poll_response(&body, &subscribed_symbols) {
                    Ok(bars) => bars,
                    Err(e) => {
                        eprintln!(
                            "  [{}] failed to parse response: {} (will retry next interval)",
                            connector_id, e
                        );
                        continue;
                    }
                };

                // Send each bar over the channel.
                for bar in bars {
                    let live_bar = LiveBar {
                        bar,
                        connector_id: connector_id.clone(),
                        received_at: chrono::Utc::now(),
                    };

                    // If the receiver has dropped, stop polling.
                    if tx.send(live_bar).await.is_err() {
                        return;
                    }
                }
            }
        });

        self.task_handle = Some(handle);
        self.state = ConnectorState::Connected;

        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), ConnectorError> {
        // Abort the polling task if it's still running.
        if let Some(handle) = self.task_handle.take() {
            handle.abort();
        }

        self.state = ConnectorState::Disconnected;
        Ok(())
    }

    async fn subscribe(&mut self, symbols: &[String]) -> Result<(), ConnectorError> {
        // Store the symbols list for use in URL construction or filtering.
        // New symbols are merged with existing subscriptions.
        for symbol in symbols {
            if !self.symbols.contains(symbol) {
                self.symbols.push(symbol.clone());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_poll_response_valid() {
        let json = r#"{
            "bars": [
                {"symbol": "AAPL", "open": 150.0, "high": 152.0, "low": 149.0, "close": 151.0, "volume": 1000000.0},
                {"symbol": "MSFT", "open": 300.0, "high": 305.0, "low": 298.0, "close": 302.0, "volume": 500000.0}
            ]
        }"#;

        let bars = parse_poll_response(json, &[]).unwrap();
        assert_eq!(bars.len(), 2);
        assert_eq!(bars[0].symbol, "AAPL");
        assert_eq!(bars[0].open, 150.0);
        assert_eq!(bars[0].high, 152.0);
        assert_eq!(bars[0].low, 149.0);
        assert_eq!(bars[0].close, 151.0);
        assert_eq!(bars[0].volume, 1_000_000.0);
        assert!(!bars[0].in_position);
        assert_eq!(bars[1].symbol, "MSFT");
    }

    #[test]
    fn test_parse_poll_response_filtered_by_symbols() {
        let json = r#"{
            "bars": [
                {"symbol": "AAPL", "open": 150.0, "high": 152.0, "low": 149.0, "close": 151.0, "volume": 1000000.0},
                {"symbol": "MSFT", "open": 300.0, "high": 305.0, "low": 298.0, "close": 302.0, "volume": 500000.0},
                {"symbol": "GOOG", "open": 140.0, "high": 142.0, "low": 139.0, "close": 141.0, "volume": 800000.0}
            ]
        }"#;

        let subscribed = vec!["AAPL".to_string(), "GOOG".to_string()];
        let bars = parse_poll_response(json, &subscribed).unwrap();
        assert_eq!(bars.len(), 2);
        assert_eq!(bars[0].symbol, "AAPL");
        assert_eq!(bars[1].symbol, "GOOG");
    }

    #[test]
    fn test_parse_poll_response_empty_bars() {
        let json = r#"{"bars": []}"#;
        let bars = parse_poll_response(json, &[]).unwrap();
        assert!(bars.is_empty());
    }

    #[test]
    fn test_parse_poll_response_invalid_json() {
        let json = "not valid json";
        let result = parse_poll_response(json, &[]);
        assert!(result.is_err());
        match result.unwrap_err() {
            ConnectorError::ParseError(_) => {}
            other => panic!("expected ParseError, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_poll_response_missing_fields() {
        let json = r#"{"bars": [{"symbol": "AAPL"}]}"#;
        let result = parse_poll_response(json, &[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_url_no_placeholder() {
        let connector = PollingConnector::new("test", "https://api.example.com/bars", Duration::from_secs(5));
        assert_eq!(connector.build_url(), "https://api.example.com/bars");
    }

    #[test]
    fn test_build_url_with_symbols_placeholder() {
        let mut connector = PollingConnector::new(
            "test",
            "https://api.example.com/bars?symbols={symbols}",
            Duration::from_secs(5),
        );
        connector.symbols = vec!["AAPL".to_string(), "MSFT".to_string()];
        assert_eq!(
            connector.build_url(),
            "https://api.example.com/bars?symbols=AAPL,MSFT"
        );
    }

    #[test]
    fn test_new_connector_defaults() {
        let connector = PollingConnector::new("poll-1", "https://api.example.com", Duration::from_secs(10));
        assert_eq!(connector.id(), "poll-1");
        assert_eq!(connector.state(), ConnectorState::Disconnected);
        assert!(connector.symbols.is_empty());
        assert!(connector.task_handle.is_none());
    }

    #[tokio::test]
    async fn test_subscribe_merges_symbols() {
        let mut connector = PollingConnector::new("test", "https://api.example.com", Duration::from_secs(5));

        connector.subscribe(&["AAPL".to_string(), "MSFT".to_string()]).await.unwrap();
        assert_eq!(connector.symbols, vec!["AAPL", "MSFT"]);

        // Adding duplicates should not create duplicates.
        connector.subscribe(&["MSFT".to_string(), "GOOG".to_string()]).await.unwrap();
        assert_eq!(connector.symbols, vec!["AAPL", "MSFT", "GOOG"]);
    }

    #[tokio::test]
    async fn test_disconnect_resets_state() {
        let mut connector = PollingConnector::new("test", "https://api.example.com", Duration::from_secs(5));
        // Simulate connected state.
        connector.state = ConnectorState::Connected;
        connector.task_handle = Some(tokio::spawn(async {}));

        connector.disconnect().await.unwrap();
        assert_eq!(connector.state(), ConnectorState::Disconnected);
        assert!(connector.task_handle.is_none());
    }
}
