//! Paradex Adapter Implementation
//!
//! Main ParadexAdapter struct implementing ExchangeAdapter trait.
//! Read-only market data via WebSocket (public orderbooks).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{Mutex, RwLock};
use tokio_tungstenite::{tungstenite::protocol::Message, MaybeTlsStream, WebSocketStream};

use crate::adapters::errors::{ExchangeError, ExchangeResult};
use crate::adapters::traits::ExchangeAdapter;
use crate::adapters::types::{
    create_http_client, next_subscription_id, Orderbook,
};

// Import from our sub-modules
use super::config::ParadexConfig;
use super::types::ParadexWsMessage;

/// Get current time in milliseconds
fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Type alias for the WebSocket stream
type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

// next_subscription_id() imported from crate::adapters::types (shared counter)

// =============================================================================
// WebSocket Stream Types
// =============================================================================

/// Type alias for WebSocket sender (write half)
type WsSink = SplitSink<WsStream, Message>;

/// Type alias for WebSocket receiver (read half)
type WsReader = SplitStream<WsStream>;

/// Shared orderbook storage for concurrent access (lock-free monitoring)
pub use crate::core::channels::SharedOrderbooks;
/// Lock-free atomic best prices for hot-path monitoring
use crate::core::channels::SharedBestPrices;
use crate::core::channels::AtomicBestPrices;
/// Event-driven orderbook notification (Axe 5)
use crate::core::channels::OrderbookNotify;

// =============================================================================
// Paradex Adapter
// =============================================================================

/// Paradex exchange adapter implementing ExchangeAdapter trait (read-only market data)
pub struct ParadexAdapter {
    /// Configuration
    config: ParadexConfig,
    /// HTTP client for REST API
    http_client: reqwest::Client,
    /// WebSocket stream (replaced by split after connect)
    ws_stream: Option<Mutex<WsStream>>,
    /// WebSocket sender (write half, used after connection established)
    ws_sender: Option<Arc<Mutex<WsSink>>>,
    /// Connection status
    connected: bool,
    /// Shared orderbooks (thread-safe for background reader)
    shared_orderbooks: SharedOrderbooks,
    /// Atomic best prices for lock-free hot-path monitoring
    shared_best_prices: SharedBestPrices,
    /// Local reference for get_orderbook (synced from shared)
    orderbooks: HashMap<String, Orderbook>,
    /// Active subscriptions by symbol
    subscriptions: Vec<String>,
    /// Pending subscription IDs for confirmation tracking
    pending_subscriptions: HashMap<u64, String>,
    /// Handle to message reader task (for cleanup)
    reader_handle: Option<tokio::task::JoinHandle<()>>,
    /// Connection health tracking
    pub(crate) connection_health: crate::adapters::types::ConnectionHealth,
    /// Handle to heartbeat task (for cleanup)
    heartbeat_handle: Option<tokio::task::JoinHandle<()>>,

    /// USD/USDC rate cache for price conversion (Pyth integration)
    usdc_rate_cache: Option<Arc<crate::core::UsdcRateCache>>,
    /// Orderbook update notification (Axe 5 event-driven monitoring)
    orderbook_notify: Option<OrderbookNotify>,
}

impl ParadexAdapter {
    /// Create a new ParadexAdapter with the given configuration
    pub fn new(config: ParadexConfig) -> Self {
        Self {
            config,
            http_client: create_http_client("Paradex"),
            ws_stream: None,
            ws_sender: None,
            connected: false,
            shared_orderbooks: Arc::new(RwLock::new(HashMap::new())),
            shared_best_prices: Arc::new(AtomicBestPrices::new()),
            orderbooks: HashMap::new(),
            subscriptions: Vec::new(),
            pending_subscriptions: HashMap::new(),
            reader_handle: None,
            connection_health: crate::adapters::types::ConnectionHealth::new(),
            heartbeat_handle: None,

            usdc_rate_cache: None,
            orderbook_notify: None,
        }
    }

    /// Set the USDC rate cache for USDâ†’USDC price conversion
    ///
    /// This enables automatic conversion of orderbook prices from USD to USDC.
    /// Call this after creating the adapter and before connecting.
    pub fn set_usdc_rate_cache(&mut self, cache: Arc<crate::core::UsdcRateCache>) {
        self.usdc_rate_cache = Some(cache);
    }

    /// Get shared orderbooks for lock-free monitoring
    ///
    /// Returns Arc<RwLock<...>> that can be read directly without acquiring
    /// the adapter's Mutex. This enables high-frequency orderbook polling
    /// without blocking execution.
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

    /// Connect to WebSocket endpoint
    async fn connect_websocket(&mut self) -> ExchangeResult<()> {
        let url = self.config.ws_base_url();
        let ws_stream = crate::adapters::shared::connect_tls(url).await?;
        self.ws_stream = Some(Mutex::new(ws_stream));
        Ok(())
    }

    /// Split WebSocket stream and spawn background message reader
    fn split_and_spawn_reader(&mut self) -> ExchangeResult<()> {
        // Take the ws_stream and split it
        let ws_stream_mutex = self.ws_stream.take().ok_or_else(|| {
            ExchangeError::ConnectionFailed("No WebSocket stream to split".into())
        })?;

        // Get the stream out of the mutex
        let ws_stream = ws_stream_mutex.into_inner();

        // Split into sender and receiver
        let (ws_sender, ws_receiver) = ws_stream.split();

        // Store sender in Arc<Mutex> for thread-safe access
        self.ws_sender = Some(Arc::new(Mutex::new(ws_sender)));

        // Clone Arc references for background tasks
        let shared_orderbooks = Arc::clone(&self.shared_orderbooks);
        let shared_best_prices = Arc::clone(&self.shared_best_prices);
        let last_data = Arc::clone(&self.connection_health.last_data);
        let reader_alive = Arc::clone(&self.connection_health.reader_alive);
        let usdc_rate_cache = self.usdc_rate_cache.clone();
        let orderbook_notify = self.orderbook_notify.clone();

        // Initialize last_data to now so we don't immediately appear stale
        last_data.store(current_time_ms(), Ordering::Relaxed);

        // Spawn background reader with shared orderbooks, health tracking, and USDC rate
        let handle = tokio::spawn(async move {
            Self::message_reader_loop(ws_receiver, shared_orderbooks, shared_best_prices, orderbook_notify, last_data, reader_alive, usdc_rate_cache)
                .await;
        });

        self.reader_handle = Some(handle);
        Ok(())
    }

    /// Background message reader loop
    /// Processes incoming WebSocket messages and updates orderbooks
    /// Also updates connection health timestamps
    ///
    /// If `usdc_rate_cache` is provided, orderbook prices are converted from USD to USDC
    async fn message_reader_loop(
        mut ws_receiver: WsReader,
        shared_orderbooks: SharedOrderbooks,
        shared_best_prices: SharedBestPrices,
        orderbook_notify: Option<OrderbookNotify>,
        last_data: Arc<AtomicU64>,
        reader_alive: Arc<AtomicBool>,
        usdc_rate_cache: Option<Arc<crate::core::UsdcRateCache>>,
    ) {
        reader_alive.store(true, Ordering::Relaxed);
        tracing::info!("Paradex message_reader_loop started");
        while let Some(msg_result) = ws_receiver.next().await {
            // Update last_data timestamp for any message received
            last_data.store(current_time_ms(), Ordering::Relaxed);

            match msg_result {
                Ok(Message::Text(text)) => {
                    // Log raw message at trace level
                    tracing::trace!("Paradex raw WS message: {}", text);

                    // Parse JSON once (avoid double parsing)
                    let json: serde_json::Value = match serde_json::from_str(&text) {
                        Ok(v) => v,
                        Err(e) => {
                            tracing::debug!("Paradex WS: failed to parse JSON: {}", e);
                            continue;
                        }
                    };



                    // Convert pre-parsed JSON to typed message (no re-parsing)
                    match serde_json::from_value::<ParadexWsMessage>(json) {
                        Ok(msg) => {
                            match msg {
                                ParadexWsMessage::SubscriptionNotification(notif) => {
                                    // JSON-RPC subscription notification with orderbook data
                                    let symbol = notif.params.data.market.clone();

                                    tracing::debug!(
                                        symbol = %symbol,
                                        channel = %notif.params.channel,
                                        levels = notif.params.data.inserts.len(),
                                        "Paradex subscription orderbook update received"
                                    );

                                    // Convert to orderbook (with USDâ†’USDC conversion if rate available)
                                    let usdc_rate = usdc_rate_cache.as_ref().map(|c| c.get_rate());
                                    match notif.params.data.to_orderbook(usdc_rate) {
                                        Ok(orderbook) => {
                                            // Write atomic best prices FIRST (lock-free hot path)
                                            shared_best_prices.store(
                                                orderbook.best_bid().unwrap_or(0.0),
                                                orderbook.best_ask().unwrap_or(0.0),
                                            );
                                            if let Some(ref n) = orderbook_notify { n.notify_waiters(); }
                                            // Update shared orderbook (acquire lock briefly)
                                            let mut books = shared_orderbooks.write().await;
                                            books.insert(symbol.clone(), orderbook);
                                            tracing::trace!(symbol = %symbol, "Paradex orderbook updated from subscription");
                                        }
                                        Err(e) => {
                                            tracing::warn!(error = %e, "Failed to parse Paradex orderbook from subscription");
                                        }
                                    }
                                }
                                ParadexWsMessage::Orderbook(orderbook_msg) => {
                                    // Direct orderbook message format (legacy/fallback)
                                    let symbol = orderbook_msg.data.market.clone();

                                    tracing::debug!(
                                        symbol = %symbol,
                                        levels = orderbook_msg.data.inserts.len(),
                                        "Paradex orderbook update received (direct format)"
                                    );

                                    // Convert to orderbook (with USDâ†’USDC conversion if rate available)
                                    let usdc_rate = usdc_rate_cache.as_ref().map(|c| c.get_rate());
                                    match orderbook_msg.data.to_orderbook(usdc_rate) {
                                        Ok(orderbook) => {
                                            // Write atomic best prices FIRST (lock-free hot path)
                                            shared_best_prices.store(
                                                orderbook.best_bid().unwrap_or(0.0),
                                                orderbook.best_ask().unwrap_or(0.0),
                                            );
                                            if let Some(ref n) = orderbook_notify { n.notify_waiters(); }
                                            // Update shared orderbook (acquire lock briefly)
                                            let mut books = shared_orderbooks.write().await;
                                            books.insert(symbol.clone(), orderbook);
                                            tracing::trace!(symbol = %symbol, "Paradex orderbook updated in shared storage");
                                        }
                                        Err(e) => {
                                            tracing::warn!(error = %e, "Failed to parse Paradex orderbook data");
                                        }
                                    }
                                }
                                ParadexWsMessage::JsonRpc(rpc_resp) => {
                                    // JSON-RPC response - subscription confirmation, etc.
                                    if let Some(err) = rpc_resp.error {
                                        tracing::warn!(
                                            "JSON-RPC error {}: {}",
                                            err.code,
                                            err.message
                                        );
                                    } else {
                                        tracing::debug!("JSON-RPC response: id={}", rpc_resp.id);
                                    }
                                }
                            }
                        }
                        Err(parse_err) => {
                            // Log full message when parsing fails
                            tracing::warn!(
                                error = %parse_err,
                                message = %text,
                                "Paradex message parse failed"
                            );
                        }
                    }
                }
                Ok(Message::Close(_)) => {
                    tracing::info!("Paradex WebSocket closed by server");
                    break;
                }
                Ok(Message::Ping(data)) => {
                    // Respond to pings (handled by tungstenite automatically)
                    tracing::trace!("Ping received: {:?}", data);
                }
                Ok(_) => {
                    // Binary or other messages - ignore
                }
                Err(e) => {
                    tracing::error!("Paradex WebSocket error: {}", e);
                    break;
                }
            }
        }
        reader_alive.store(false, Ordering::Relaxed);
        tracing::warn!("Paradex message reader loop ended â€” reader_alive set to false");
    }

    /// Send a subscribe request for a symbol's orderbook
    async fn send_subscribe_request(&self, symbol: &str) -> ExchangeResult<u64> {
        let ws_sender = self
            .ws_sender
            .as_ref()
            .ok_or_else(|| ExchangeError::ConnectionFailed("WebSocket not connected".into()))?;

        let sub_id = next_subscription_id();
        // Paradex orderbook channel format: order_book.{symbol}.snapshot@15@100ms
        let channel = format!("order_book.{}.snapshot@15@100ms", symbol);

        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "subscribe",
            "params": {
                "channel": channel
            },
            "id": sub_id
        });

        let mut sender = ws_sender.lock().await;
        sender
            .send(Message::Text(msg.to_string()))
            .await
            .map_err(|e| ExchangeError::WebSocket(Box::new(e)))?;

        Ok(sub_id)
    }

    /// Send an unsubscribe request for a symbol's orderbook
    async fn send_unsubscribe_request(&self, symbol: &str) -> ExchangeResult<u64> {
        let ws_sender = self
            .ws_sender
            .as_ref()
            .ok_or_else(|| ExchangeError::ConnectionFailed("WebSocket not connected".into()))?;

        let unsub_id = next_subscription_id();
        let channel = format!("order_book.{}.snapshot@15@100ms", symbol);

        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "unsubscribe",
            "params": {
                "channel": channel
            },
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
    ///
    /// Paradex uses native WebSocket PING/PONG which tokio-tungstenite handles automatically.
    /// This task monitors the last_data timestamp to detect stale connections.
    /// If data is stale for more than STALE_THRESHOLD_MS, sets `reader_alive = false`
    /// so that `is_stale()` detects the dead connection.
    fn spawn_heartbeat_task(&mut self) {
        let last_data = Arc::clone(&self.connection_health.last_data);
        let reader_alive = Arc::clone(&self.connection_health.reader_alive);

        // Initialize last_data to now so we don't immediately appear stale
        last_data.store(current_time_ms(), Ordering::Relaxed);

        let handle = tokio::spawn(async move {
            // Check every 30 seconds as per NFR20
            let mut interval = tokio::time::interval(Duration::from_secs(
                crate::adapters::types::WS_PING_INTERVAL_SECS,
            ));
            // Skip the first immediate tick
            interval.tick().await;

            loop {
                interval.tick().await;

                let last = last_data.load(Ordering::Relaxed);
                let now = current_time_ms();
                let age_ms = now.saturating_sub(last);

                if age_ms > crate::adapters::types::STALE_THRESHOLD_MS {
                    tracing::warn!(
                        "Paradex heartbeat: data stale for {}ms (threshold {}ms) â€” signaling dead connection",
                        age_ms,
                        crate::adapters::types::STALE_THRESHOLD_MS
                    );
                    reader_alive.store(false, Ordering::Relaxed);
                    break;
                }

                tracing::trace!(
                    "Paradex heartbeat: last data was {}ms ago",
                    age_ms
                );
            }
        });

        self.heartbeat_handle = Some(handle);
        tracing::info!("Paradex: Heartbeat monitoring started (30s interval)");
    }

    /// Warm up HTTP connection pool by making a lightweight request
    ///
    /// This establishes TCP/TLS connections upfront to avoid handshake latency
    /// on the first real request. Uses GET /system/time as it's lightweight.
    async fn warm_up_http(&self) -> ExchangeResult<()> {
        let url = format!("{}/system/time", self.config.rest_base_url());
        let start = std::time::Instant::now();

        let response =
            self.http_client.get(&url).send().await.map_err(|e| {
                ExchangeError::ConnectionFailed(format!("HTTP warm-up failed: {}", e))
            })?;

        let elapsed = start.elapsed();

        if response.status().is_success() {
            tracing::info!(
                phase = "init",
                exchange = "paradex",
                latency_ms = %elapsed.as_millis(),
                "HTTP connection pool warmed up"
            );
        } else {
            tracing::warn!(
                phase = "init",
                exchange = "paradex",
                status = %response.status(),
                latency_ms = %elapsed.as_millis(),
                "HTTP warm-up returned non-success status"
            );
        }

        Ok(())
    }
}

// =============================================================================
// ExchangeAdapter Trait Implementation
// =============================================================================

#[async_trait]
impl ExchangeAdapter for ParadexAdapter {
    /// Connect to Paradex: WebSocket connection (public channels only)
    async fn connect(&mut self) -> ExchangeResult<()> {
        tracing::info!("Connecting to Paradex (public channels)...");

        // Step 1: Connect WebSocket
        self.connect_websocket().await?;
        tracing::info!(exchange = "paradex", "Paradex WebSocket connected");

        // Step 2: Split stream and spawn reader
        self.split_and_spawn_reader()?;

        // Step 3: Start heartbeat monitoring
        self.spawn_heartbeat_task();

        // Step 4: Warm up HTTP connection pool (establish TCP/TLS upfront)
        if let Err(e) = self.warm_up_http().await {
            tracing::warn!("HTTP warm-up failed (non-fatal): {}", e);
        }

        self.connected = true;
        tracing::info!("Paradex adapter fully connected");

        Ok(())
    }

    /// Disconnect from Paradex
    async fn disconnect(&mut self) -> ExchangeResult<()> {
        tracing::info!("Disconnecting from Paradex...");

        // Set state to Disconnected
        {
            let mut state = self.connection_health.state.write().await;
            *state = crate::adapters::types::ConnectionState::Disconnected;
        }

        // Cancel reader task
        if let Some(handle) = self.reader_handle.take() {
            handle.abort();
        }

        // Abort heartbeat task if running
        if let Some(handle) = self.heartbeat_handle.take() {
            handle.abort();
        }

        // Close WebSocket sender
        if let Some(ws_sender) = self.ws_sender.take() {
            let mut sender = ws_sender.lock().await;
            let _ = sender.close().await;
        }

        // Close raw WebSocket stream to prevent lingering TCP connections
        if let Some(ws) = self.ws_stream.take() {
            let mut stream = ws.lock().await;
            let _ = stream.close(None).await;
        }

        // Clear state
        self.connected = false;
        self.subscriptions.clear();
        self.orderbooks.clear();

        // Reset connection health
        self.connection_health.last_pong.store(0, Ordering::Relaxed);
        self.connection_health.last_data.store(0, Ordering::Relaxed);
        self.connection_health.reader_alive.store(false, Ordering::Relaxed);

        // Clear shared orderbooks
        let mut books = self.shared_orderbooks.write().await;
        books.clear();

        tracing::info!("Paradex adapter disconnected");
        Ok(())
    }

    /// Subscribe to orderbook updates for a trading symbol
    async fn subscribe_orderbook(&mut self, symbol: &str) -> ExchangeResult<()> {
        if !self.connected {
            return Err(ExchangeError::ConnectionFailed("Not connected".into()));
        }

        let sub_id = self.send_subscribe_request(symbol).await?;
        self.pending_subscriptions
            .insert(sub_id, symbol.to_string());
        self.subscriptions.push(symbol.to_string());

        tracing::info!(
            "Subscribed to Paradex orderbook: {} (id={})",
            symbol,
            sub_id
        );
        Ok(())
    }

    /// Unsubscribe from orderbook updates
    async fn unsubscribe_orderbook(&mut self, symbol: &str) -> ExchangeResult<()> {
        if !self.connected {
            return Err(ExchangeError::ConnectionFailed("Not connected".into()));
        }

        let unsub_id = self.send_unsubscribe_request(symbol).await?;
        self.subscriptions.retain(|s| s != symbol);

        // Remove from local orderbooks
        self.orderbooks.remove(symbol);

        // Remove from shared orderbooks
        let mut books = self.shared_orderbooks.write().await;
        books.remove(symbol);

        tracing::info!(
            "Unsubscribed from Paradex orderbook: {} (id={})",
            symbol,
            unsub_id
        );
        Ok(())
    }

    /// Get cached orderbook for a symbol
    ///
    /// NOTE: This method synchronously reads from the local cache.
    /// Use `get_orderbook_async()` to read from the shared orderbooks updated by the background reader.
    fn get_orderbook(&self, symbol: &str) -> Option<&Orderbook> {
        self.orderbooks.get(symbol)
    }

    /// Check if adapter is connected
    fn is_connected(&self) -> bool {
        self.connected
    }

    fn is_stale(&self) -> bool {
        if !self.connected {
            return true;
        }

        // Reader loop died â†’ connection is dead (S-1/S-2 fix)
        if !self.connection_health.reader_alive.load(Ordering::Relaxed) {
            return true;
        }


        let last_data = self.connection_health.last_data.load(Ordering::Relaxed);
        if last_data == 0 {
            // No data ever received - check if we just connected
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
        use crate::adapters::types::ConnectionState;

        tracing::info!("Paradex: Initiating reconnection...");

        // Set state to Reconnecting
        {
            let mut state = self.connection_health.state.write().await;
            *state = ConnectionState::Reconnecting;
        }

        // Store current subscriptions before disconnecting
        let saved_subscriptions = self.subscriptions.clone();

        // Disconnect (cleans up resources but we'll override state)
        self.disconnect().await?;

        // Exponential backoff retry loop
        // Delays: 500ms, 1000ms, 2000ms, cap at 5000ms. Max 3 attempts.
        const MAX_RECONNECT_ATTEMPTS: u32 = 3;
        let mut last_error: Option<ExchangeError> = None;

        for attempt in 0..MAX_RECONNECT_ATTEMPTS {
            // Exponential backoff: 500ms * 2^attempt, capped at 5000ms
            let backoff_ms = std::cmp::min(500 * (1u64 << attempt), 5000);
            tracing::info!(
                "Paradex: Reconnect attempt {} of {}, waiting {}ms...",
                attempt + 1,
                MAX_RECONNECT_ATTEMPTS,
                backoff_ms
            );

            tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;

            // Update state to Reconnecting for each attempt
            {
                let mut state = self.connection_health.state.write().await;
                *state = ConnectionState::Reconnecting;
            }

            // Try to reconnect
            match self.connect().await {
                Ok(()) => {
                    // Success! Set state to Connected
                    {
                        let mut state = self.connection_health.state.write().await;
                        *state = ConnectionState::Connected;
                    }

                    // Re-subscribe to all previously subscribed symbols
                    for symbol in &saved_subscriptions {
                        tracing::info!("Paradex: Re-subscribing to {}", symbol);
                        if let Err(e) = self.subscribe_orderbook(symbol).await {
                            tracing::warn!("Paradex: Failed to re-subscribe to {}: {}", symbol, e);
                        }
                    }

                    tracing::info!(
                        "Paradex: Reconnection complete with {} subscriptions restored",
                        self.subscriptions.len()
                    );

                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!("Paradex: Reconnect attempt {} failed: {}", attempt + 1, e);
                    last_error = Some(e);
                }
            }
        }

        // All attempts failed - set state to Disconnected
        {
            let mut state = self.connection_health.state.write().await;
            *state = ConnectionState::Disconnected;
        }

        Err(last_error.unwrap_or_else(|| {
            ExchangeError::ConnectionFailed("Reconnection failed after max attempts".into())
        }))
    }

    /// CR-11 fix: Query real fill info (price + fee) from Paradex REST API

    /// Get exchange name
    fn exchange_name(&self) -> &'static str {
        "paradex"
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
// Additional ParadexAdapter Methods (not part of ExchangeAdapter trait)
// =============================================================================

impl ParadexAdapter {
    /// Get orderbook from shared storage (async, reads from background reader updates)
    pub async fn get_orderbook_async(&self, symbol: &str) -> Option<Orderbook> {
        let books = self.shared_orderbooks.read().await;
        books.get(symbol).cloned()
    }

}

// =============================================================================
// Unit Tests (Minimal - essential adapter tests only)
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Test ParadexAdapter construction
    #[test]
    fn test_paradex_adapter_new() {
        let config = ParadexConfig::default();
        let adapter = ParadexAdapter::new(config);

        assert!(!adapter.is_connected());
        assert_eq!(adapter.exchange_name(), "paradex");
    }

    /// Test exchange name returns "paradex"
    #[test]
    fn test_exchange_name() {
        let config = ParadexConfig::default();
        let adapter = ParadexAdapter::new(config);
        assert_eq!(adapter.exchange_name(), "paradex");
    }

    /// Test adapter is not connected initially
    #[test]
    fn test_adapter_not_connected_initially() {
        let config = ParadexConfig::default();
        let adapter = ParadexAdapter::new(config);
        assert!(!adapter.is_connected());
    }

    /// Test warm_up_http() functionality
    /// Unit test for connection warm-up functionality
    /// Note: warm_up_http() uses the HTTP client which works independently of WS connection
    #[tokio::test]
    async fn test_warm_up_http_makes_request() {
        let config = ParadexConfig::default();
        let adapter = ParadexAdapter::new(config);

        // warm_up_http makes a GET request to /system/time
        // This should succeed even without WebSocket connection established
        // because the HTTP client is configured independently
        let result = adapter.warm_up_http().await;

        // Should succeed - HTTP client can reach Paradex
        assert!(
            result.is_ok(),
            "warm_up_http should succeed with default config: {:?}",
            result
        );
    }
}
