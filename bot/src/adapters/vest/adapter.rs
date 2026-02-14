//! Vest Adapter Implementation
//!
//! Main VestAdapter struct implementing ExchangeAdapter trait.
//! Read-only market data via WebSocket (public orderbooks).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;

use crate::adapters::errors::{ExchangeError, ExchangeResult};
use crate::adapters::traits::ExchangeAdapter;
use crate::adapters::types::{
    next_subscription_id, ConnectionHealth, ConnectionState, Orderbook,
};

// Import from sub-modules
use super::config::VestConfig;
use super::types::VestWsMessage;

/// Get current time in milliseconds
fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// =============================================================================
// Constants
// =============================================================================

/// Timeout for PING/PONG validation in seconds
const PING_TIMEOUT_SECS: u64 = 5;

// =============================================================================
// WebSocket Type Aliases
// =============================================================================

pub(crate) type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
pub(crate) type WsWriter = SplitSink<WsStream, Message>;
pub(crate) type WsReader = SplitStream<WsStream>;
/// Thread-safe shared orderbooks storage for lock-free monitoring
pub use crate::core::channels::SharedOrderbooks;
/// Lock-free atomic best prices for hot-path monitoring
use crate::core::channels::SharedBestPrices;
use crate::core::channels::AtomicBestPrices;
/// Event-driven orderbook notification (Axe 5)
use crate::core::channels::OrderbookNotify;

// =============================================================================
// VestAdapter Implementation
// =============================================================================

/// Vest Exchange Adapter implementing ExchangeAdapter trait
pub struct VestAdapter {
    pub(crate) config: VestConfig,
    pub(crate) ws_stream: Option<Mutex<WsStream>>,
    pub(crate) ws_sender: Option<Arc<Mutex<WsWriter>>>,
    pub(crate) reader_handle: Option<JoinHandle<()>>,
    pub(crate) heartbeat_handle: Option<JoinHandle<()>>,
    pub(crate) connected: bool,
    pub(crate) subscriptions: Vec<String>,
    pub(crate) pending_subscriptions: HashMap<u64, String>,
    pub(crate) orderbooks: HashMap<String, Orderbook>,
    pub(crate) shared_orderbooks: SharedOrderbooks,
    /// Atomic best prices for lock-free hot-path monitoring
    pub(crate) shared_best_prices: SharedBestPrices,
    /// Orderbook update notification (Axe 5 event-driven monitoring)
    pub(crate) orderbook_notify: Option<OrderbookNotify>,
    pub(crate) connection_health: ConnectionHealth,
}

impl VestAdapter {
    /// Create a new VestAdapter with the given configuration
    pub fn new(config: VestConfig) -> Self {
        Self {
            config,
            ws_stream: None,
            ws_sender: None,
            reader_handle: None,
            heartbeat_handle: None,
            connected: false,
            subscriptions: Vec::new(),
            pending_subscriptions: HashMap::new(),
            orderbooks: HashMap::new(),
            shared_orderbooks: Arc::new(RwLock::new(HashMap::new())),
            shared_best_prices: Arc::new(AtomicBestPrices::new()),
            orderbook_notify: None,
            connection_health: ConnectionHealth::default(),
        }
    }

    /// Build public WebSocket URL (for public channels like orderbook)
    pub fn build_public_ws_url(&self) -> String {
        format!(
            "{}?version=1.0&xwebsocketserver=restserver{}",
            self.config.ws_base_url(),
            self.config.account_group
        )
    }

    /// Get shared orderbooks for lock-free monitoring
    pub fn get_shared_orderbooks(&self) -> SharedOrderbooks {
        Arc::clone(&self.shared_orderbooks)
    }

    /// Get shared atomic best prices for lock-free hot-path monitoring
    pub fn get_shared_best_prices(&self) -> SharedBestPrices {
        Arc::clone(&self.shared_best_prices)
    }

    /// Set the shared orderbook notification (Axe 5 event-driven monitoring)
    pub fn set_orderbook_notify(&mut self, notify: OrderbookNotify) {
        self.orderbook_notify = Some(notify);
    }

    // =========================================================================
    // WebSocket Connection Management
    // =========================================================================

    /// Connect to WebSocket and validate with PING/PONG
    async fn connect_websocket(&mut self) -> ExchangeResult<()> {
        let url = self.build_public_ws_url();
        tracing::info!("Connecting to Vest public WebSocket: {}", url);
        let ws_stream = crate::adapters::shared::connect_tls(&url).await?;
        self.ws_stream = Some(Mutex::new(ws_stream));
        Ok(())
    }

    /// Send PING and validate PONG response with timeout
    async fn validate_connection(&self) -> ExchangeResult<()> {
        let ws = self
            .ws_stream
            .as_ref()
            .ok_or_else(|| ExchangeError::ConnectionFailed("No WebSocket connection".into()))?;

        let mut stream = ws.lock().await;

        let ping_msg = serde_json::json!({
            "method": "PING",
            "params": [],
            "id": 0
        });

        stream
            .send(Message::Text(ping_msg.to_string()))
            .await
            .map_err(|e| ExchangeError::WebSocket(Box::new(e)))?;

        let pong_timeout = Duration::from_secs(PING_TIMEOUT_SECS);
        let pong_result = timeout(pong_timeout, stream.next())
            .await
            .map_err(|_| ExchangeError::NetworkTimeout(PING_TIMEOUT_SECS * 1000))?;

        match pong_result {
            Some(msg_result) => {
                let msg = msg_result.map_err(|e| ExchangeError::WebSocket(Box::new(e)))?;

                if let Message::Text(text) = msg {
                    let response: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
                        ExchangeError::InvalidResponse(format!("Invalid PONG: {}", e))
                    })?;

                    if response.get("data").and_then(|d| d.as_str()) != Some("PONG") {
                        return Err(ExchangeError::ConnectionFailed(format!(
                            "Expected PONG, got: {}",
                            text
                        )));
                    }
                }
            }
            None => {
                return Err(ExchangeError::ConnectionFailed(
                    "No response to PING".into(),
                ));
            }
        }

        Ok(())
    }

    /// Split WebSocket stream and spawn background message reader
    fn split_and_spawn_reader(&mut self) -> ExchangeResult<()> {
        let ws_stream_mutex = self.ws_stream.take().ok_or_else(|| {
            ExchangeError::ConnectionFailed("No WebSocket stream to split".into())
        })?;

        let ws_stream = ws_stream_mutex.into_inner();
        let (ws_sender, ws_receiver) = ws_stream.split();

        self.ws_sender = Some(Arc::new(Mutex::new(ws_sender)));

        let shared_orderbooks = Arc::clone(&self.shared_orderbooks);
        let shared_best_prices = Arc::clone(&self.shared_best_prices);
        let orderbook_notify = self.orderbook_notify.clone();
        let last_pong = Arc::clone(&self.connection_health.last_pong);
        let last_data = Arc::clone(&self.connection_health.last_data);
        let reader_alive = Arc::clone(&self.connection_health.reader_alive);

        last_data.store(current_time_ms(), Ordering::Relaxed);

        let handle = tokio::spawn(async move {
            Self::message_reader_loop(ws_receiver, shared_orderbooks, shared_best_prices, orderbook_notify, last_pong, last_data, reader_alive).await;
        });

        self.reader_handle = Some(handle);
        Ok(())
    }

    /// Background message reader loop
    ///
    /// Sets `reader_alive` to `true` on entry and `false` on exit, so that
    /// `is_stale()` can detect a dead connection immediately.
    async fn message_reader_loop(
        mut ws_receiver: WsReader,
        shared_orderbooks: SharedOrderbooks,
        shared_best_prices: SharedBestPrices,
        orderbook_notify: Option<OrderbookNotify>,
        last_pong: Arc<AtomicU64>,
        last_data: Arc<AtomicU64>,
        reader_alive: Arc<AtomicBool>,
    ) {
        tracing::info!("Vest message_reader_loop started");
        reader_alive.store(true, Ordering::Relaxed);

        while let Some(msg_result) = ws_receiver.next().await {
            last_data.store(current_time_ms(), Ordering::Relaxed);
            // Any WS message proves the connection is alive -- reset PONG staleness timer
            last_pong.store(current_time_ms(), Ordering::Relaxed);

            match msg_result {
                Ok(Message::Text(text)) => {
                    tracing::trace!("Raw WS message: {}", text);

                    match serde_json::from_str::<VestWsMessage>(&text) {
                        Ok(msg) => match msg {
                            VestWsMessage::Depth(depth_msg) => {
                                let symbol = depth_msg
                                    .channel
                                    .strip_suffix("@depth")
                                    .unwrap_or(&depth_msg.channel)
                                    .to_string();

                                tracing::debug!(
                                    symbol = %symbol,
                                    bids = depth_msg.data.bids.len(),
                                    asks = depth_msg.data.asks.len(),
                                    "Vest depth update received"
                                );

                                match depth_msg.data.to_orderbook() {
                                    Ok(orderbook) => {
                                        // Write atomic best prices FIRST (lock-free hot path)
                                        shared_best_prices.store(
                                            orderbook.best_bid().unwrap_or(0.0),
                                            orderbook.best_ask().unwrap_or(0.0),
                                        );
                                        if let Some(ref n) = orderbook_notify { n.notify_waiters(); }
                                        let mut books = shared_orderbooks.write().await;
                                        books.insert(symbol.clone(), orderbook);
                                        tracing::trace!(symbol = %symbol, "Orderbook updated in shared storage");
                                    }
                                    Err(e) => {
                                        tracing::warn!(error = %e, "Failed to parse orderbook data");
                                    }
                                }
                            }
                            VestWsMessage::Subscription(sub_resp) => {
                                tracing::debug!("Subscription confirmed: id={}", sub_resp.id);
                            }
                            VestWsMessage::Pong { .. } => {
                                last_pong.store(current_time_ms(), Ordering::Relaxed);
                                tracing::debug!("Vest PONG received, updating last_pong timestamp");
                            }
                        },
                        Err(parse_err) => {
                            tracing::warn!(
                                error = %parse_err,
                                message = %text,
                                "Failed to parse WS message - unknown format"
                            );
                        }
                    }
                }
                Ok(Message::Close(_)) => {
                    tracing::info!("WebSocket closed by server");
                    break;
                }
                Ok(Message::Ping(data)) => {
                    tracing::debug!("Ping received: {:?}", data);
                }
                Ok(Message::Pong(data)) => {
                    tracing::debug!("Pong WS frame received: {:?}", data);
                }
                Ok(Message::Binary(data)) => match String::from_utf8(data.clone()) {
                    Ok(text) => {
                        tracing::debug!("Binary->Text: {}", text);
                        if let Ok(VestWsMessage::Depth(depth_msg)) =
                            serde_json::from_str::<VestWsMessage>(&text)
                        {
                            let symbol = depth_msg
                                .channel
                                .strip_suffix("@depth")
                                .unwrap_or(&depth_msg.channel)
                                .to_string();

                            if let Ok(orderbook) = depth_msg.data.to_orderbook() {
                                shared_best_prices.store(
                                    orderbook.best_bid().unwrap_or(0.0),
                                    orderbook.best_ask().unwrap_or(0.0),
                                );
                                if let Some(ref n) = orderbook_notify { n.notify_waiters(); }
                                let mut books = shared_orderbooks.write().await;
                                books.insert(symbol, orderbook);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::debug!("Binary message not UTF-8: {} ({} bytes)", e, data.len());
                    }
                },
                Ok(Message::Frame(_)) => {
                    tracing::trace!("Raw frame received");
                }
                Err(e) => {
                    tracing::error!("WebSocket error: {}", e);
                    break;
                }
            }
        }

        reader_alive.store(false, Ordering::Relaxed);
        tracing::warn!("Vest message reader loop ended -- reader_alive set to false");
    }

    /// Send a SUBSCRIBE request for a symbol's orderbook
    async fn send_subscribe_request(&self, symbol: &str) -> ExchangeResult<u64> {
        let ws_sender = self
            .ws_sender
            .as_ref()
            .ok_or_else(|| ExchangeError::ConnectionFailed("WebSocket not connected".into()))?;

        let sub_id = next_subscription_id();
        let channel = format!("{}@depth", symbol);

        let msg = serde_json::json!({
            "method": "SUBSCRIBE",
            "params": [channel],
            "id": sub_id
        });

        let mut sender = ws_sender.lock().await;
        sender
            .send(Message::Text(msg.to_string()))
            .await
            .map_err(|e| ExchangeError::WebSocket(Box::new(e)))?;

        Ok(sub_id)
    }

    /// Send an UNSUBSCRIBE request for a symbol's orderbook
    async fn send_unsubscribe_request(&self, symbol: &str) -> ExchangeResult<u64> {
        let ws_sender = self
            .ws_sender
            .as_ref()
            .ok_or_else(|| ExchangeError::ConnectionFailed("WebSocket not connected".into()))?;

        let unsub_id = next_subscription_id();
        let channel = format!("{}@depth", symbol);

        let msg = serde_json::json!({
            "method": "UNSUBSCRIBE",
            "params": [channel],
            "id": unsub_id
        });

        let mut sender = ws_sender.lock().await;
        sender
            .send(Message::Text(msg.to_string()))
            .await
            .map_err(|e| ExchangeError::WebSocket(Box::new(e)))?;

        Ok(unsub_id)
    }

    /// Spawn heartbeat monitoring task
    fn spawn_heartbeat_task(&mut self) {
        let ws_sender = match &self.ws_sender {
            Some(sender) => Arc::clone(sender),
            None => return,
        };

        let last_pong = Arc::clone(&self.connection_health.last_pong);
        let reader_alive = Arc::clone(&self.connection_health.reader_alive);
        last_pong.store(current_time_ms(), Ordering::Relaxed);

        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(
                crate::adapters::types::WS_PING_INTERVAL_SECS,
            ));
            interval.tick().await;

            loop {
                interval.tick().await;

                let ping_msg = serde_json::json!({
                    "method": "PING",
                    "params": [],
                    "id": 0
                });

                {
                    let mut sender = ws_sender.lock().await;
                    if let Err(e) = sender.send(Message::Text(ping_msg.to_string())).await {
                        tracing::warn!("Vest heartbeat: Failed to send PING - {}", e);
                        reader_alive.store(false, Ordering::Relaxed);
                        break;
                    }
                    tracing::trace!("Vest heartbeat: PING sent");
                }

                tokio::time::sleep(Duration::from_secs(5)).await;

                // Check if PONG is stale - connection likely dead
                let last = last_pong.load(Ordering::Relaxed);
                let now = current_time_ms();
                let pong_age_ms = now.saturating_sub(last);
                if pong_age_ms > 30_000 {
                    tracing::warn!(
                        "Vest heartbeat: PONG stale ({}ms ago) - connection likely dead, setting reader_alive=false",
                        pong_age_ms
                    );
                    reader_alive.store(false, Ordering::Relaxed);
                    break;
                }
                tracing::trace!("Vest heartbeat: last PONG was {}ms ago", pong_age_ms);
            }

            tracing::debug!("Vest heartbeat task ended");
        });

        self.heartbeat_handle = Some(handle);
        tracing::info!("Vest: Heartbeat monitoring started (30s interval)");
    }
}

// =============================================================================
// ExchangeAdapter Implementation
// =============================================================================

#[async_trait]
impl ExchangeAdapter for VestAdapter {
    async fn connect(&mut self) -> ExchangeResult<()> {
        self.connect_websocket().await?;
        self.validate_connection().await?;
        self.split_and_spawn_reader()?;
        self.spawn_heartbeat_task();

        self.connected = true;
        tracing::info!(exchange = "vest", "Vest WebSocket connected");

        Ok(())
    }

    async fn disconnect(&mut self) -> ExchangeResult<()> {
        {
            let mut state = self.connection_health.state.write().await;
            *state = ConnectionState::Disconnected;
        }

        if let Some(handle) = self.reader_handle.take() {
            handle.abort();
        }

        if let Some(handle) = self.heartbeat_handle.take() {
            handle.abort();
        }

        if let Some(ws_sender) = self.ws_sender.take() {
            let mut sender = ws_sender.lock().await;
            let _ = sender.close().await;
        }

        if let Some(ws) = self.ws_stream.take() {
            let mut stream = ws.lock().await;
            let _ = stream.close(None).await;
        }

        self.connected = false;
        self.subscriptions.clear();
        self.pending_subscriptions.clear();

        self.connection_health.last_pong.store(0, Ordering::Relaxed);
        self.connection_health.last_data.store(0, Ordering::Relaxed);
        self.connection_health.reader_alive.store(false, Ordering::Relaxed);

        let mut books = self.shared_orderbooks.write().await;
        books.clear();
        self.orderbooks.clear();

        Ok(())
    }

    async fn subscribe_orderbook(&mut self, symbol: &str) -> ExchangeResult<()> {
        if !self.connected {
            return Err(ExchangeError::ConnectionFailed("Not connected".into()));
        }

        let sub_id = self.send_subscribe_request(symbol).await?;
        self.pending_subscriptions
            .insert(sub_id, symbol.to_string());
        self.subscriptions.push(symbol.to_string());

        {
            let mut books = self.shared_orderbooks.write().await;
            books.insert(symbol.to_string(), Orderbook::default());
        }

        Ok(())
    }

    async fn unsubscribe_orderbook(&mut self, symbol: &str) -> ExchangeResult<()> {
        if !self.connected {
            return Err(ExchangeError::ConnectionFailed("Not connected".into()));
        }

        let _ = self.send_unsubscribe_request(symbol).await?;
        self.subscriptions.retain(|s| s != symbol);

        {
            let mut books = self.shared_orderbooks.write().await;
            books.remove(symbol);
        }
        self.orderbooks.remove(symbol);

        Ok(())
    }

    fn get_orderbook(&self, symbol: &str) -> Option<&Orderbook> {
        self.orderbooks.get(symbol)
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    fn is_stale(&self) -> bool {
        if !self.connected {
            return true;
        }

        // Reader loop died -> connection is dead (S-1/S-2 fix)
        if !self.connection_health.reader_alive.load(Ordering::Relaxed) {
            return true;
        }

        let last_data = self.connection_health.last_data.load(Ordering::Relaxed);
        if last_data == 0 {
            return false;
        }

        let now = current_time_ms();
        use crate::adapters::types::STALE_THRESHOLD_MS;

        now.saturating_sub(last_data) > STALE_THRESHOLD_MS
    }

    async fn sync_orderbooks(&mut self) {
        let books = self.shared_orderbooks.read().await;
        self.orderbooks = books.clone();
    }

    async fn reconnect(&mut self) -> ExchangeResult<()> {
        tracing::info!("Vest: Initiating reconnection...");

        {
            let mut state = self.connection_health.state.write().await;
            *state = ConnectionState::Reconnecting;
        }

        let saved_subscriptions = self.subscriptions.clone();
        self.disconnect().await?;

        const MAX_RECONNECT_ATTEMPTS: u32 = 3;
        let mut last_error: Option<ExchangeError> = None;

        for attempt in 0..MAX_RECONNECT_ATTEMPTS {
            let backoff_ms = std::cmp::min(500 * (1u64 << attempt), 5000);
            tracing::info!(
                "Vest: Reconnect attempt {} of {}, waiting {}ms...",
                attempt + 1,
                MAX_RECONNECT_ATTEMPTS,
                backoff_ms
            );

            tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;

            {
                let mut state = self.connection_health.state.write().await;
                *state = ConnectionState::Reconnecting;
            }

            match self.connect().await {
                Ok(()) => {
                    {
                        let mut state = self.connection_health.state.write().await;
                        *state = ConnectionState::Connected;
                    }

                    for symbol in &saved_subscriptions {
                        tracing::info!("Vest: Re-subscribing to {}", symbol);
                        if let Err(e) = self.subscribe_orderbook(symbol).await {
                            tracing::warn!("Vest: Failed to re-subscribe to {}: {}", symbol, e);
                        }
                    }

                    tracing::info!(
                        "Vest: Reconnection complete with {} subscriptions restored",
                        self.subscriptions.len()
                    );

                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!("Vest: Reconnect attempt {} failed: {}", attempt + 1, e);
                    last_error = Some(e);
                }
            }
        }

        {
            let mut state = self.connection_health.state.write().await;
            *state = ConnectionState::Disconnected;
        }

        Err(last_error.unwrap_or_else(|| {
            ExchangeError::ConnectionFailed("Reconnection failed after max attempts".into())
        }))
    }

    fn exchange_name(&self) -> &'static str {
        "vest"
    }

    fn get_shared_orderbooks(&self) -> crate::core::channels::SharedOrderbooks {
        Arc::clone(&self.shared_orderbooks)
    }

    fn get_shared_best_prices(&self) -> crate::core::channels::SharedBestPrices {
        Arc::clone(&self.shared_best_prices)
    }

    fn set_orderbook_notify(&mut self, notify: crate::core::channels::OrderbookNotify) {
        self.orderbook_notify = Some(notify);
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::super::config::VestConfig;
    use super::*;

    #[test]
    fn test_adapter_not_connected_initially() {
        let config = VestConfig::default();
        let adapter = VestAdapter::new(config);
        assert!(!adapter.connected);
        assert_eq!(adapter.exchange_name(), "vest");
    }
}
