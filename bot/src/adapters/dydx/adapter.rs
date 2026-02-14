//! dYdX Adapter Implementation
//!
//! WebSocket adapter for dYdX v4 Indexer using v4_orderbook channel.
//! Read-only market data — public orderbook snapshots and updates.
//!
//! Docs: https://docs.dydx.xyz/indexer-client/websockets

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

use crate::adapters::errors::{ExchangeError, ExchangeResult};
use crate::adapters::traits::ExchangeAdapter;
use crate::adapters::types::{ConnectionHealth, ConnectionState, Orderbook};
use crate::core::channels::{AtomicBestPrices, OrderbookNotify, SharedBestPrices, SharedOrderbooks};

use super::config::DydxConfig;
use super::types::{coin_to_market, get_dydx_symbols, DydxWsMessage};

// =============================================================================
// Helpers
// =============================================================================

pub(crate) fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// =============================================================================
// WebSocket Type Aliases
// =============================================================================

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
type WsWriter = SplitSink<WsStream, Message>;
type WsReader = SplitStream<WsStream>;

// =============================================================================
// DydxAdapter
// =============================================================================

/// dYdX v4 Exchange Adapter implementing ExchangeAdapter trait
pub struct DydxAdapter {
    config: DydxConfig,
    ws_stream: Option<Mutex<WsStream>>,
    ws_sender: Option<Arc<Mutex<WsWriter>>>,
    reader_handle: Option<JoinHandle<()>>,
    heartbeat_handle: Option<JoinHandle<()>>,
    connected: bool,
    subscriptions: Vec<String>,
    orderbooks: HashMap<String, Orderbook>,
    shared_orderbooks: SharedOrderbooks,
    shared_best_prices: SharedBestPrices,
    orderbook_notify: Option<OrderbookNotify>,
    connection_health: ConnectionHealth,
}

impl DydxAdapter {
    /// Create a new DydxAdapter
    pub fn new(config: DydxConfig) -> Self {
        Self {
            config,
            ws_stream: None,
            ws_sender: None,
            reader_handle: None,
            heartbeat_handle: None,
            connected: false,
            subscriptions: Vec::new(),
            orderbooks: HashMap::new(),
            shared_orderbooks: Arc::new(RwLock::new(HashMap::new())),
            shared_best_prices: Arc::new(AtomicBestPrices::new()),
            orderbook_notify: None,
            connection_health: ConnectionHealth::default(),
        }
    }

    // =========================================================================
    // WebSocket Connection
    // =========================================================================

    async fn connect_websocket(&mut self) -> ExchangeResult<()> {
        let url = self.config.ws_url();
        tracing::info!("Connecting to dYdX WebSocket: {}", url);
        let ws_stream = crate::adapters::shared::connect_tls(url).await?;
        self.ws_stream = Some(Mutex::new(ws_stream));
        Ok(())
    }

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
        // Set reader_alive BEFORE spawn to prevent race with monitoring loop
        reader_alive.store(true, Ordering::Relaxed);

        let handle = tokio::spawn(async move {
            Self::message_reader_loop(
                ws_receiver,
                shared_orderbooks,
                shared_best_prices,
                orderbook_notify,
                last_pong,
                last_data,
                reader_alive,
            )
            .await;
        });

        self.reader_handle = Some(handle);
        Ok(())
    }

    // =========================================================================
    // Subscriptions
    // =========================================================================

    async fn subscribe_to_orderbooks(&self) -> ExchangeResult<()> {
        let ws_sender = self
            .ws_sender
            .as_ref()
            .ok_or_else(|| ExchangeError::ConnectionFailed("WebSocket not connected".into()))?;

        let symbols = get_dydx_symbols();

        for coin in &symbols {
            let market = coin_to_market(coin);
            let msg = serde_json::json!({
                "type": "subscribe",
                "channel": "v4_orderbook",
                "id": market,
                "batched": false,
            });

            {
                let mut sender = ws_sender.lock().await;
                sender
                    .send(Message::Text(msg.to_string()))
                    .await
                    .map_err(|e| ExchangeError::WebSocket(Box::new(e)))?;
            }

            // 50ms delay between subscriptions to stay within rate limits
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        tracing::info!(
            exchange = "dydx",
            count = symbols.len(),
            "Subscribed to v4_orderbook channels"
        );

        Ok(())
    }

    // =========================================================================
    // Background Reader Loop
    // =========================================================================

    async fn message_reader_loop(
        mut ws_receiver: WsReader,
        shared_orderbooks: SharedOrderbooks,
        shared_best_prices: SharedBestPrices,
        orderbook_notify: Option<OrderbookNotify>,
        last_pong: Arc<AtomicU64>,
        last_data: Arc<AtomicU64>,
        reader_alive: Arc<AtomicBool>,
    ) {
        tracing::info!("dYdX message_reader_loop started");

        const READ_TIMEOUT: Duration = Duration::from_secs(60);

        loop {
            let msg_result = match tokio::time::timeout(READ_TIMEOUT, ws_receiver.next()).await {
                Ok(Some(msg)) => msg,
                Ok(None) => {
                    tracing::info!("dYdX WebSocket stream ended");
                    break;
                }
                Err(_elapsed) => {
                    tracing::warn!(
                        "dYdX: no WS message received in {}s, assuming dead connection",
                        READ_TIMEOUT.as_secs()
                    );
                    break;
                }
            };

            last_data.store(current_time_ms(), Ordering::Relaxed);
            last_pong.store(current_time_ms(), Ordering::Relaxed);

            match msg_result {
                Ok(Message::Text(text)) => {
                    tracing::trace!("dYdX raw WS message: {}", text);

                    match serde_json::from_str::<DydxWsMessage>(&text) {
                        Ok(DydxWsMessage::Subscribed { id, contents, .. }) => {
                            let symbol = id.unwrap_or_default();
                            tracing::debug!(
                                exchange = "dydx",
                                symbol = %symbol,
                                "Subscription confirmed with snapshot"
                            );

                            match contents.to_orderbook() {
                                Ok(orderbook) => {
                                    shared_best_prices.store(
                                        orderbook.best_bid().unwrap_or(0.0),
                                        orderbook.best_ask().unwrap_or(0.0),
                                    );
                                    if let Some(ref n) = orderbook_notify {
                                        n.notify_waiters();
                                    }
                                    let mut books = shared_orderbooks.write().await;
                                    books.insert(symbol, orderbook);
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        error = %e,
                                        "Failed to parse dYdX initial orderbook"
                                    );
                                }
                            }
                        }
                        Ok(DydxWsMessage::ChannelData { id, contents, .. }) => {
                            let symbol = id.unwrap_or_default();

                            match contents.to_orderbook() {
                                Ok(orderbook) => {
                                    shared_best_prices.store(
                                        orderbook.best_bid().unwrap_or(0.0),
                                        orderbook.best_ask().unwrap_or(0.0),
                                    );
                                    if let Some(ref n) = orderbook_notify {
                                        n.notify_waiters();
                                    }
                                    let mut books = shared_orderbooks.write().await;
                                    books.insert(symbol.clone(), orderbook);
                                    tracing::trace!(
                                        symbol = %symbol,
                                        "dYdX orderbook updated"
                                    );
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        error = %e,
                                        symbol = %symbol,
                                        "Failed to parse dYdX orderbook update"
                                    );
                                }
                            }
                        }
                        Ok(DydxWsMessage::Connected { .. }) => {
                            tracing::info!("dYdX: connected to Indexer WebSocket");
                        }
                        Ok(DydxWsMessage::Unsubscribed { .. }) => {
                            tracing::debug!("dYdX: unsubscription confirmed");
                        }
                        Ok(DydxWsMessage::Error { message }) => {
                            tracing::warn!(
                                "dYdX WebSocket error: {}",
                                message.unwrap_or_else(|| "unknown".to_string())
                            );
                        }
                        Err(_) => {
                            tracing::trace!(
                                message = %text,
                                "dYdX: unknown message format"
                            );
                        }
                    }
                }
                Ok(Message::Close(_)) => {
                    tracing::info!("dYdX WebSocket closed by server");
                    break;
                }
                Ok(Message::Ping(data)) => {
                    // Respond to server pings with pongs
                    tracing::trace!("dYdX: Ping received, responding with Pong");
                    // Note: tokio-tungstenite auto-responds to pings
                    let _ = data;
                }
                Ok(Message::Pong(_)) => {}
                Ok(Message::Binary(data)) => {
                    if let Ok(text) = String::from_utf8(data) {
                        if let Ok(msg) = serde_json::from_str::<DydxWsMessage>(&text) {
                            match msg {
                                DydxWsMessage::ChannelData { id, contents, .. }
                                | DydxWsMessage::Subscribed { id, contents, .. } => {
                                    let symbol = id.unwrap_or_default();
                                    if let Ok(orderbook) = contents.to_orderbook() {
                                        shared_best_prices.store(
                                            orderbook.best_bid().unwrap_or(0.0),
                                            orderbook.best_ask().unwrap_or(0.0),
                                        );
                                        if let Some(ref n) = orderbook_notify {
                                            n.notify_waiters();
                                        }
                                        let mut books = shared_orderbooks.write().await;
                                        books.insert(symbol, orderbook);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                Ok(Message::Frame(_)) => {}
                Err(e) => {
                    tracing::error!("dYdX WebSocket error: {}", e);
                    break;
                }
            }
        }

        reader_alive.store(false, Ordering::Relaxed);
        tracing::warn!("dYdX message reader loop ended");
    }

    // =========================================================================
    // Heartbeat (WebSocket ping every 5s)
    // =========================================================================

    fn spawn_heartbeat_task(&mut self) {
        let ws_sender = match &self.ws_sender {
            Some(sender) => Arc::clone(sender),
            None => return,
        };

        let last_pong = Arc::clone(&self.connection_health.last_pong);
        let reader_alive = Arc::clone(&self.connection_health.reader_alive);
        last_pong.store(current_time_ms(), Ordering::Relaxed);

        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            interval.tick().await; // skip first immediate tick

            loop {
                interval.tick().await;

                // Send a WebSocket-level ping frame
                {
                    let mut sender = ws_sender.lock().await;
                    if let Err(e) = sender.send(Message::Ping(vec![])).await {
                        tracing::warn!("dYdX heartbeat: Failed to send ping - {}", e);
                        reader_alive.store(false, Ordering::Relaxed);
                        break;
                    }
                    tracing::trace!("dYdX heartbeat: ping sent");
                }

                tokio::time::sleep(Duration::from_secs(5)).await;

                let last = last_pong.load(Ordering::Relaxed);
                let now = current_time_ms();
                let pong_age_ms = now.saturating_sub(last);
                if pong_age_ms > 30_000 {
                    tracing::warn!(
                        "dYdX heartbeat: PONG stale ({}ms ago), marking dead",
                        pong_age_ms
                    );
                    reader_alive.store(false, Ordering::Relaxed);
                    break;
                }
            }

            tracing::debug!("dYdX heartbeat task ended");
        });

        self.heartbeat_handle = Some(handle);
        tracing::info!("dYdX: Heartbeat monitoring started (5s interval)");
    }
}

// =============================================================================
// ExchangeAdapter Implementation
// =============================================================================

#[async_trait]
impl ExchangeAdapter for DydxAdapter {
    async fn connect(&mut self) -> ExchangeResult<()> {
        self.connect_websocket().await?;
        self.split_and_spawn_reader()?;
        self.spawn_heartbeat_task();
        self.subscribe_to_orderbooks().await?;

        self.connected = true;
        tracing::info!(exchange = "dydx", "dYdX WebSocket connected");

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

        self.connection_health.last_pong.store(0, Ordering::Relaxed);
        self.connection_health.last_data.store(0, Ordering::Relaxed);
        self.connection_health
            .reader_alive
            .store(false, Ordering::Relaxed);

        let mut books = self.shared_orderbooks.write().await;
        books.clear();
        self.orderbooks.clear();

        Ok(())
    }

    async fn subscribe_orderbook(&mut self, symbol: &str) -> ExchangeResult<()> {
        if !self.connected {
            return Err(ExchangeError::ConnectionFailed("Not connected".into()));
        }

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
        tracing::info!("dYdX: Initiating reconnection...");

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
                "dYdX: Reconnect attempt {} of {}, waiting {}ms...",
                attempt + 1,
                MAX_RECONNECT_ATTEMPTS,
                backoff_ms
            );

            tokio::time::sleep(Duration::from_millis(backoff_ms)).await;

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
                        if let Err(e) = self.subscribe_orderbook(symbol).await {
                            tracing::warn!(
                                "dYdX: Failed to re-subscribe to {}: {}",
                                symbol,
                                e
                            );
                        }
                    }

                    tracing::info!(
                        "dYdX: Reconnection complete ({} subscriptions restored)",
                        self.subscriptions.len()
                    );

                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!("dYdX: Reconnect attempt {} failed: {}", attempt + 1, e);
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
        "dydx"
    }

    fn get_shared_orderbooks(&self) -> SharedOrderbooks {
        Arc::clone(&self.shared_orderbooks)
    }

    fn get_shared_best_prices(&self) -> SharedBestPrices {
        Arc::clone(&self.shared_best_prices)
    }

    fn set_orderbook_notify(&mut self, notify: OrderbookNotify) {
        self.orderbook_notify = Some(notify);
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adapter_not_connected_initially() {
        let config = DydxConfig::default();
        let adapter = DydxAdapter::new(config);
        assert!(!adapter.connected);
        assert_eq!(adapter.exchange_name(), "dydx");
    }
}
