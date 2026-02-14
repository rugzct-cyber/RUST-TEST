//! GRVT Adapter Implementation
//!
//! WebSocket adapter for GRVT DEX using v1.mini.s mini ticker stream.
//! JSON-RPC 2.0 protocol, subscribes to best bid/ask per instrument.

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
use crate::adapters::types::{ConnectionHealth, ConnectionState, Orderbook, OrderbookLevel};
use crate::core::channels::{AtomicBestPrices, OrderbookNotify, SharedBestPrices, SharedOrderbooks};

use super::config::GrvtConfig;
use super::types::{get_grvt_markets, instrument_to_symbol, GrvtTickerMessage};

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
// GrvtAdapter
// =============================================================================

pub struct GrvtAdapter {
    config: GrvtConfig,
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

impl GrvtAdapter {
    pub fn new(config: GrvtConfig) -> Self {
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

    async fn connect_websocket(&mut self) -> ExchangeResult<()> {
        let url = self.config.ws_url();
        tracing::info!("Connecting to GRVT WebSocket: {}", url);
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
    // Subscriptions — JSON-RPC 2.0 subscribe to v1.mini.s
    // =========================================================================

    async fn subscribe_to_tickers(&self) -> ExchangeResult<()> {
        let ws_sender = self
            .ws_sender
            .as_ref()
            .ok_or_else(|| ExchangeError::ConnectionFailed("WebSocket not connected".into()))?;

        let markets = get_grvt_markets();
        // Build selectors: ["BTC_USDT_Perp@500", "ETH_USDT_Perp@500", ...]
        let selectors: Vec<String> = markets
            .iter()
            .map(|(inst, _)| format!("{}@500", inst))
            .collect();

        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "subscribe",
            "params": {
                "stream": "v1.mini.s",
                "selectors": selectors,
            }
        });

        {
            let mut sender = ws_sender.lock().await;
            sender
                .send(Message::Text(msg.to_string()))
                .await
                .map_err(|e| ExchangeError::WebSocket(Box::new(e)))?;
        }

        tracing::info!(
            exchange = "grvt",
            count = selectors.len(),
            "Subscribed to v1.mini.s tickers"
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
        tracing::info!("GRVT message_reader_loop started");
        reader_alive.store(true, Ordering::Relaxed);
        let mut msg_count: u64 = 0;

        while let Some(msg_result) = ws_receiver.next().await {
            last_data.store(current_time_ms(), Ordering::Relaxed);
            last_pong.store(current_time_ms(), Ordering::Relaxed);

            match msg_result {
                Ok(Message::Text(text)) => {
                    msg_count += 1;
                    if msg_count <= 3 {
                        tracing::info!(exchange = "grvt", msg_count, raw = %text.chars().take(300).collect::<String>(), "RAW WS message");
                    }

                    if let Ok(msg) = serde_json::from_str::<GrvtTickerMessage>(&text) {
                        // Subscription confirmation (has result, no params)
                        if msg.result.is_some() {
                            tracing::debug!("GRVT subscription confirmed (id={:?})", msg.id);
                            continue;
                        }

                        // Ticker data
                        if let Some(params) = msg.params {
                            let data = params.data;
                            if let Some(symbol) = instrument_to_symbol(&data.instrument) {
                                let bid: f64 = match data.best_bid_price.parse() {
                                    Ok(v) => v,
                                    Err(_) => continue,
                                };
                                let ask: f64 = match data.best_ask_price.parse() {
                                    Ok(v) => v,
                                    Err(_) => continue,
                                };

                                if bid > 0.0 && ask > 0.0 {
                                    let orderbook = Orderbook {
                                        bids: vec![OrderbookLevel::new(bid, 0.0)],
                                        asks: vec![OrderbookLevel::new(ask, 0.0)],
                                        timestamp: current_time_ms(),
                                    };

                                    shared_best_prices.store(bid, ask);
                                    if let Some(ref n) = orderbook_notify {
                                        n.notify_waiters();
                                    }
                                    let mut books = shared_orderbooks.write().await;
                                    books.insert(symbol, orderbook);
                                }
                            }
                        }
                    }
                }
                Ok(Message::Close(_)) => {
                    tracing::info!("GRVT WebSocket closed by server");
                    break;
                }
                Ok(Message::Ping(_)) | Ok(Message::Pong(_)) | Ok(Message::Frame(_)) => {}
                Ok(Message::Binary(_)) => {}
                Err(e) => {
                    tracing::error!("GRVT WebSocket error: {}", e);
                    break;
                }
            }
        }

        reader_alive.store(false, Ordering::Relaxed);
        tracing::warn!("GRVT message reader loop ended");
    }

    // =========================================================================
    // Heartbeat
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
            let mut interval = tokio::time::interval(Duration::from_secs(15));
            interval.tick().await;

            loop {
                interval.tick().await;

                // GRVT doesn't have explicit ping; send a no-op subscribe to keep alive
                let ping_msg = serde_json::json!({"jsonrpc": "2.0", "id": 0, "method": "ping", "params": {}});

                {
                    let mut sender = ws_sender.lock().await;
                    if let Err(e) = sender.send(Message::Text(ping_msg.to_string())).await {
                        tracing::warn!("GRVT heartbeat: Failed to send ping - {}", e);
                        reader_alive.store(false, Ordering::Relaxed);
                        break;
                    }
                }

                tokio::time::sleep(Duration::from_secs(5)).await;

                let last = last_pong.load(Ordering::Relaxed);
                let now = current_time_ms();
                let pong_age_ms = now.saturating_sub(last);
                if pong_age_ms > 60_000 {
                    tracing::warn!("GRVT heartbeat: stale ({}ms), marking dead", pong_age_ms);
                    reader_alive.store(false, Ordering::Relaxed);
                    break;
                }
            }

            tracing::debug!("GRVT heartbeat task ended");
        });

        self.heartbeat_handle = Some(handle);
    }
}

// =============================================================================
// ExchangeAdapter Implementation
// =============================================================================

#[async_trait]
impl ExchangeAdapter for GrvtAdapter {
    async fn connect(&mut self) -> ExchangeResult<()> {
        self.connect_websocket().await?;
        self.split_and_spawn_reader()?;
        self.spawn_heartbeat_task();
        self.subscribe_to_tickers().await?;

        self.connected = true;
        tracing::info!(exchange = "grvt", "GRVT WebSocket connected");
        Ok(())
    }

    async fn disconnect(&mut self) -> ExchangeResult<()> {
        {
            let mut state = self.connection_health.state.write().await;
            *state = ConnectionState::Disconnected;
        }
        if let Some(handle) = self.reader_handle.take() { handle.abort(); }
        if let Some(handle) = self.heartbeat_handle.take() { handle.abort(); }
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
        self.connection_health.reader_alive.store(false, Ordering::Relaxed);
        let mut books = self.shared_orderbooks.write().await;
        books.clear();
        self.orderbooks.clear();
        Ok(())
    }

    async fn subscribe_orderbook(&mut self, symbol: &str) -> ExchangeResult<()> {
        if !self.connected { return Err(ExchangeError::ConnectionFailed("Not connected".into())); }
        self.subscriptions.push(symbol.to_string());
        { let mut books = self.shared_orderbooks.write().await; books.insert(symbol.to_string(), Orderbook::default()); }
        Ok(())
    }

    async fn unsubscribe_orderbook(&mut self, symbol: &str) -> ExchangeResult<()> {
        if !self.connected { return Err(ExchangeError::ConnectionFailed("Not connected".into())); }
        self.subscriptions.retain(|s| s != symbol);
        { let mut books = self.shared_orderbooks.write().await; books.remove(symbol); }
        self.orderbooks.remove(symbol);
        Ok(())
    }

    fn get_orderbook(&self, symbol: &str) -> Option<&Orderbook> { self.orderbooks.get(symbol) }
    fn is_connected(&self) -> bool { self.connected }

    fn is_stale(&self) -> bool {
        if !self.connected { return true; }
        if !self.connection_health.reader_alive.load(Ordering::Relaxed) { return true; }
        let last_data = self.connection_health.last_data.load(Ordering::Relaxed);
        if last_data == 0 { return false; }
        let now = current_time_ms();
        use crate::adapters::types::STALE_THRESHOLD_MS;
        now.saturating_sub(last_data) > STALE_THRESHOLD_MS
    }

    async fn sync_orderbooks(&mut self) {
        let books = self.shared_orderbooks.read().await;
        self.orderbooks = books.clone();
    }

    async fn reconnect(&mut self) -> ExchangeResult<()> {
        tracing::info!("GRVT: Initiating reconnection...");
        { let mut state = self.connection_health.state.write().await; *state = ConnectionState::Reconnecting; }
        let saved = self.subscriptions.clone();
        self.disconnect().await?;

        for attempt in 0..3u32 {
            let backoff_ms = std::cmp::min(500 * (1u64 << attempt), 5000);
            tracing::info!("GRVT: Reconnect attempt {}/3, waiting {}ms...", attempt + 1, backoff_ms);
            tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
            { let mut state = self.connection_health.state.write().await; *state = ConnectionState::Reconnecting; }
            match self.connect().await {
                Ok(()) => {
                    { let mut state = self.connection_health.state.write().await; *state = ConnectionState::Connected; }
                    for symbol in &saved {
                        let _ = self.subscribe_orderbook(symbol).await;
                    }
                    tracing::info!("GRVT: Reconnection complete");
                    return Ok(());
                }
                Err(e) => tracing::warn!("GRVT: Reconnect attempt {} failed: {}", attempt + 1, e),
            }
        }

        { let mut state = self.connection_health.state.write().await; *state = ConnectionState::Disconnected; }
        Err(ExchangeError::ConnectionFailed("GRVT reconnection failed after max attempts".into()))
    }

    fn exchange_name(&self) -> &'static str { "grvt" }
    fn get_shared_orderbooks(&self) -> SharedOrderbooks { Arc::clone(&self.shared_orderbooks) }
    fn get_shared_best_prices(&self) -> SharedBestPrices { Arc::clone(&self.shared_best_prices) }
    fn set_orderbook_notify(&mut self, notify: OrderbookNotify) { self.orderbook_notify = Some(notify); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adapter_not_connected_initially() {
        let adapter = GrvtAdapter::new(GrvtConfig::default());
        assert!(!adapter.connected);
        assert_eq!(adapter.exchange_name(), "grvt");
    }
}
