//! WebSocket connector for live market data streaming.
//!
//! Connects to a configurable WebSocket endpoint, receives messages,
//! and parses them into `BarContext` values using a user-provided parser
//! function. Supports heartbeat/keepalive handling and proper state
//! transitions.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

use flux_runtime::BarContext;

use super::connector::{Connector, ConnectorError, ConnectorState, LiveBar};

/// Interval between keepalive Ping frames (30 seconds).
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);

/// A connector that streams bars from a WebSocket endpoint.
///
/// Connects to a WebSocket URL, receives text messages, and parses them
/// into `BarContext` values using a configurable parser function. This
/// allows different endpoints (e.g., Binance, Alpaca, custom) to use
/// different message formats while sharing the same connector logic.
///
/// # Heartbeat/Keepalive
///
/// `tokio-tungstenite` automatically responds to incoming Ping frames
/// with Pong frames. Additionally, this connector sends periodic Ping
/// frames to detect stale connections.
pub struct WebSocketConnector {
    /// Human-readable identifier for this connector instance.
    id: String,
    /// WebSocket endpoint URL (e.g., `wss://stream.example.com/v1`).
    url: String,
    /// Current connection state.
    state: ConnectorState,
    /// Parser function: converts raw WebSocket text message → BarContext.
    /// Stored as Arc to allow sharing with the spawned reader task.
    #[allow(clippy::type_complexity)]
    parser: Arc<dyn Fn(&str) -> Result<BarContext, ConnectorError> + Send + Sync>,
    /// Handle to the spawned WebSocket reader task (if connected).
    task_handle: Option<JoinHandle<()>>,
    /// Channel for sending messages to the WebSocket writer task
    /// (used by `subscribe()` and keepalive pings).
    write_tx: Option<mpsc::Sender<Message>>,
}

impl WebSocketConnector {
    /// Create a new WebSocket connector.
    ///
    /// # Arguments
    /// - `id` — Human-readable identifier for observability
    /// - `url` — WebSocket endpoint URL (e.g., `wss://stream.example.com`)
    /// - `parser` — Function that parses raw text messages into `BarContext`
    #[allow(clippy::type_complexity)]
    pub fn new(
        id: impl Into<String>,
        url: impl Into<String>,
        parser: Box<dyn Fn(&str) -> Result<BarContext, ConnectorError> + Send + Sync>,
    ) -> Self {
        Self {
            id: id.into(),
            url: url.into(),
            state: ConnectorState::Disconnected,
            parser: Arc::from(parser),
            task_handle: None,
            write_tx: None,
        }
    }
}

#[async_trait]
impl Connector for WebSocketConnector {
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

        // Establish WebSocket connection.
        let (ws_stream, _response) =
            tokio_tungstenite::connect_async(&self.url)
                .await
                .map_err(|e| {
                    self.state = ConnectorState::Disconnected;
                    ConnectorError::ConnectionFailed(format!(
                        "WebSocket connection to '{}' failed: {}",
                        self.url, e
                    ))
                })?;

        let (mut ws_write, mut ws_read) = ws_stream.split();

        // Internal channel for outbound messages (subscribe, keepalive pings).
        let (write_tx, mut write_rx) = mpsc::channel::<Message>(32);
        self.write_tx = Some(write_tx.clone());

        // Send initial subscription message if symbols are provided.
        if !symbols.is_empty() {
            let subscribe_msg = serde_json::json!({
                "type": "subscribe",
                "symbols": symbols,
            });
            ws_write
                .send(Message::Text(subscribe_msg.to_string()))
                .await
                .map_err(|e| {
                    self.state = ConnectorState::Disconnected;
                    ConnectorError::SubscriptionFailed(format!(
                        "failed to send subscription message: {}",
                        e
                    ))
                })?;
        }

        let connector_id = self.id.clone();
        let parser = Arc::clone(&self.parser);

        // Spawn a task that:
        // 1. Reads incoming WebSocket messages and parses them into bars
        // 2. Writes outbound messages (subscribe requests, keepalive pings)
        // 3. Sends periodic keepalive pings to detect stale connections
        let handle = tokio::spawn(async move {
            let mut keepalive = tokio::time::interval(KEEPALIVE_INTERVAL);
            // Skip the initial immediate tick.
            keepalive.tick().await;

            loop {
                tokio::select! {
                    // Incoming message from WebSocket.
                    msg = ws_read.next() => {
                        match msg {
                            Some(Ok(Message::Text(text))) => {
                                match parser(&text) {
                                    Ok(bar) => {
                                        let live_bar = LiveBar {
                                            bar,
                                            connector_id: connector_id.clone(),
                                            received_at: chrono::Utc::now(),
                                        };
                                        if tx.send(live_bar).await.is_err() {
                                            // Receiver dropped, stop the task.
                                            eprintln!("  [{}] bar channel closed, stopping", connector_id);
                                            break;
                                        }
                                    }
                                    Err(e) => {
                                        // Parse error — log and continue, don't disconnect.
                                        eprintln!("  [{}] parse error: {}", connector_id, e);
                                    }
                                }
                            }
                            Some(Ok(Message::Ping(_))) => {
                                // tungstenite handles Pong automatically at the protocol level.
                                // No action needed here.
                            }
                            Some(Ok(Message::Pong(_))) => {
                                // Keepalive response received — connection is healthy.
                            }
                            Some(Ok(Message::Close(_))) => {
                                eprintln!("  [{}] server sent close frame", connector_id);
                                break;
                            }
                            Some(Ok(Message::Binary(_))) => {
                                // Binary messages are ignored for now.
                            }
                            Some(Ok(Message::Frame(_))) => {
                                // Raw frames — ignore.
                            }
                            Some(Err(e)) => {
                                eprintln!("  [{}] WebSocket error: {}", connector_id, e);
                                break;
                            }
                            None => {
                                // Stream ended.
                                eprintln!("  [{}] WebSocket stream ended", connector_id);
                                break;
                            }
                        }
                    }
                    // Outbound message to send over WebSocket.
                    Some(msg) = write_rx.recv() => {
                        if ws_write.send(msg).await.is_err() {
                            eprintln!("  [{}] failed to send message, connection lost", connector_id);
                            break;
                        }
                    }
                    // Periodic keepalive ping.
                    _ = keepalive.tick() => {
                        if ws_write.send(Message::Ping(vec![])).await.is_err() {
                            eprintln!("  [{}] keepalive ping failed, connection lost", connector_id);
                            break;
                        }
                    }
                }
            }
        });

        self.task_handle = Some(handle);
        self.state = ConnectorState::Connected;

        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), ConnectorError> {
        // Abort the reader/writer task if it's still running.
        if let Some(handle) = self.task_handle.take() {
            handle.abort();
        }
        self.write_tx = None;
        self.state = ConnectorState::Disconnected;
        Ok(())
    }

    async fn subscribe(&mut self, symbols: &[String]) -> Result<(), ConnectorError> {
        let write_tx = self.write_tx.as_ref().ok_or_else(|| {
            ConnectorError::SubscriptionFailed("not connected".to_string())
        })?;

        // Send a JSON subscription message as a default format.
        // The exact format depends on the endpoint; this provides a
        // reasonable default: {"type": "subscribe", "symbols": [...]}
        let subscribe_msg = serde_json::json!({
            "type": "subscribe",
            "symbols": symbols,
        });

        write_tx
            .send(Message::Text(subscribe_msg.to_string()))
            .await
            .map_err(|e| {
                ConnectorError::SubscriptionFailed(format!(
                    "failed to send subscription message: {}",
                    e
                ))
            })?;

        Ok(())
    }
}
