//! Hyperliquid Adapter Implementation
//!
//! WebSocket adapter for Hyperliquid DEX using l2Book channel.
//! Read-only market data — public orderbook snapshots.
//!
//! Docs: https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/websocket/subscriptions

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

use super::config::HyperliquidConfig;
use super::types::{coin_to_symbol, get_hyperliquid_symbols, HyperliquidWsMessage};

// =============================================================================
// Helpers
// =============================================================================

fn current_time_ms() -> u64 {
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
// HyperliquidAdapter
// =============================================================================

/// Hyperliquid Exchange Adapter implementing ExchangeAdapter trait
pub struct HyperliquidAdapter {
    config: HyperliquidConfig,
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

impl HyperliquidAdapter {
    /// Create a new HyperliquidAdapter
    pub fn new(config: HyperliquidConfig) -> Self {
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
        tracing::info!("Connecting to Hyperliquid WebSocket: {}", url);
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

    async fn subscribe_to_l2books(&self) -> ExchangeResult<()> {
        let ws_sender = self
            .ws_sender
            .as_ref()
            .ok_or_else(|| ExchangeError::ConnectionFailed("WebSocket not connected".into()))?;

        let symbols = get_hyperliquid_symbols();

        for coin in &symbols {
            let msg = serde_json::json!({
                "method": "subscribe",
                "subscription": {
                    "type": "l2Book",
                    "coin": coin,
                },
            });

            {
                let mut sender = ws_sender.lock().await;
                sender
                    .send(Message::Text(msg.to_string()))
                    .await
                    .map_err(|e| ExchangeError::WebSocket(Box::new(e)))?;
            }

            // 30ms delay between subscriptions to stay within rate limits
            tokio::time::sleep(Duration::from_millis(30)).await;
        }

        tracing::info!(
            exchange = "hyperliquid",
            count = symbols.len(),
            "Subscribed to l2Book channels"
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
        tracing::info!("Hyperliquid message_reader_loop started");
        reader_alive.store(true, Ordering::Relaxed);

        while let Some(msg_result) = ws_receiver.next().await {
            last_data.store(current_time_ms(), Ordering::Relaxed);
            last_pong.store(current_time_ms(), Ordering::Relaxed);

            match msg_result {
                Ok(Message::Text(text)) => {
                    tracing::trace!("Raw WS message: {}", text);

                    match serde_json::from_str::<HyperliquidWsMessage>(&text) {
                        Ok(HyperliquidWsMessage::L2Book(book)) => {
                            let symbol = coin_to_symbol(&book.coin);

                            match book.to_orderbook() {
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
                                        "Hyperliquid orderbook updated"
                                    );
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        error = %e,
                                        coin = %book.coin,
                                        "Failed to parse Hyperliquid orderbook"
                                    );
                                }
                            }
                        }
                        Ok(HyperliquidWsMessage::Pong) => {
                            last_pong.store(current_time_ms(), Ordering::Relaxed);
                            tracing::trace!("Hyperliquid PONG received");
                        }
                        Ok(HyperliquidWsMessage::SubscriptionResponse(_)) => {
                            tracing::debug!("Hyperliquid subscription confirmed");
                        }
                        Err(_) => {
                            // Silently ignore unknown messages (e.g. error responses)
                            tracing::trace!(
                                message = %text,
                                "Hyperliquid: unknown message format"
                            );
                        }
                    }
                }
                Ok(Message::Close(_)) => {
                    tracing::info!("Hyperliquid WebSocket closed by server");
                    break;
                }
                Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => {}
                Ok(Message::Binary(data)) => {
                    if let Ok(text) = String::from_utf8(data) {
                        if let Ok(HyperliquidWsMessage::L2Book(book)) =
                            serde_json::from_str::<HyperliquidWsMessage>(&text)
                        {
                            let symbol = coin_to_symbol(&book.coin);
                            if let Ok(orderbook) = book.to_orderbook() {
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
                    }
                }
                Ok(Message::Frame(_)) => {}
                Err(e) => {
                    tracing::error!("Hyperliquid WebSocket error: {}", e);
                    break;
                }
            }
        }

        reader_alive.store(false, Ordering::Relaxed);
        tracing::warn!("Hyperliquid message reader loop ended");
    }

    // =========================================================================
    // Heartbeat (Ping every 5s, same as arbi-v5)
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

                let ping_msg = serde_json::json!({"method": "ping"});

                {
                    let mut sender = ws_sender.lock().await;
                    if let Err(e) = sender.send(Message::Text(ping_msg.to_string())).await {
                        tracing::warn!("Hyperliquid heartbeat: Failed to send ping - {}", e);
                        reader_alive.store(false, Ordering::Relaxed);
                        break;
                    }
                    tracing::trace!("Hyperliquid heartbeat: ping sent");
                }

                tokio::time::sleep(Duration::from_secs(5)).await;

                let last = last_pong.load(Ordering::Relaxed);
                let now = current_time_ms();
                let pong_age_ms = now.saturating_sub(last);
                if pong_age_ms > 30_000 {
                    tracing::warn!(
                        "Hyperliquid heartbeat: PONG stale ({}ms ago), marking dead",
                        pong_age_ms
                    );
                    reader_alive.store(false, Ordering::Relaxed);
                    break;
                }
            }

            tracing::debug!("Hyperliquid heartbeat task ended");
        });

        self.heartbeat_handle = Some(handle);
        tracing::info!("Hyperliquid: Heartbeat monitoring started (5s interval)");
    }
}

// =============================================================================
// ExchangeAdapter Implementation
// =============================================================================

#[async_trait]
impl ExchangeAdapter for HyperliquidAdapter {
    async fn connect(&mut self) -> ExchangeResult<()> {
        self.connect_websocket().await?;
        self.split_and_spawn_reader()?;
        self.spawn_heartbeat_task();
        self.subscribe_to_l2books().await?;

        self.connected = true;
        tracing::info!(exchange = "hyperliquid", "Hyperliquid WebSocket connected");

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
        tracing::info!("Hyperliquid: Initiating reconnection...");

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
                "Hyperliquid: Reconnect attempt {} of {}, waiting {}ms...",
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
                                "Hyperliquid: Failed to re-subscribe to {}: {}",
                                symbol,
                                e
                            );
                        }
                    }

                    tracing::info!(
                        "Hyperliquid: Reconnection complete ({} subscriptions restored)",
                        self.subscriptions.len()
                    );

                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!("Hyperliquid: Reconnect attempt {} failed: {}", attempt + 1, e);
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
        "hyperliquid"
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
        let config = HyperliquidConfig::default();
        let adapter = HyperliquidAdapter::new(config);
        assert!(!adapter.connected);
        assert_eq!(adapter.exchange_name(), "hyperliquid");
    }
}
