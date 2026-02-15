//! Nado Adapter — WS subscription on /v1/subscribe with permessage-deflate
//!
//! Uses `yawc` crate with `Options::default()` for native permessage-deflate
//! decompression, plus `.with_request()` to inject the required
//! `Sec-WebSocket-Extensions: permessage-deflate` HTTP header on the upgrade.
//!
//! Nado's gateway REQUIRES this header (returns 403 without it) AND sends
//! compressed frames (reserved bits set), so we need both the header AND
//! the decompression support.
//!
//! Subscribes to `best_bid_offer` streams for real-time BBO data.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::stream::StreamExt;
use futures_util::SinkExt;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use crate::adapters::errors::{ExchangeError, ExchangeResult};
use crate::adapters::traits::ExchangeAdapter;
use crate::adapters::types::{ConnectionHealth, ConnectionState, Orderbook, OrderbookLevel};
use crate::core::channels::{AtomicBestPrices, OrderbookNotify, SharedBestPrices, SharedOrderbooks};

use super::config::NadoConfig;
use super::types::{
    get_nado_markets, parse_nado_price, product_id_to_symbol,
    NadoBboEvent, NadoSubscribeMsg,
};

fn current_time_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub struct NadoAdapter {
    config: NadoConfig,
    reader_handle: Option<JoinHandle<()>>,
    connected: bool,
    subscriptions: Vec<String>,
    orderbooks: HashMap<String, Orderbook>,
    shared_orderbooks: SharedOrderbooks,
    shared_best_prices: SharedBestPrices,
    orderbook_notify: Option<OrderbookNotify>,
    connection_health: ConnectionHealth,
    /// Timestamp (ms) when connect() was last called — used for grace period in is_stale()
    connect_time_ms: u64,
}

impl NadoAdapter {
    pub fn new(config: NadoConfig) -> Self {
        Self {
            config,
            reader_handle: None,
            connected: false,
            subscriptions: Vec::new(),
            orderbooks: HashMap::new(),
            shared_orderbooks: Arc::new(RwLock::new(HashMap::new())),
            shared_best_prices: Arc::new(AtomicBestPrices::new()),
            orderbook_notify: None,
            connection_health: ConnectionHealth::default(),
            connect_time_ms: 0,
        }
    }

    fn spawn_ws_task(&mut self) -> ExchangeResult<()> {
        let url = self.config.ws_url().to_string();
        let shared_orderbooks = Arc::clone(&self.shared_orderbooks);
        let shared_best_prices = Arc::clone(&self.shared_best_prices);
        let orderbook_notify = self.orderbook_notify.clone();
        let last_pong = Arc::clone(&self.connection_health.last_pong);
        let last_data = Arc::clone(&self.connection_health.last_data);
        let reader_alive = Arc::clone(&self.connection_health.reader_alive);

        let handle = tokio::spawn(async move {
            Self::ws_subscribe_loop(
                url, shared_orderbooks, shared_best_prices, orderbook_notify,
                last_pong, last_data, reader_alive,
            ).await;
        });
        self.reader_handle = Some(handle);
        Ok(())
    }

    /// Connects via yawc with:
    ///   - Options with low_latency_compression → native permessage-deflate decompression
    ///   - .with_request() → explicit Sec-WebSocket-Extensions header for gateway
    /// Then subscribes to best_bid_offer and reads the push stream.
    async fn ws_subscribe_loop(
        url: String,
        shared_orderbooks: SharedOrderbooks,
        shared_best_prices: SharedBestPrices,
        orderbook_notify: Option<OrderbookNotify>,
        last_pong: Arc<AtomicU64>,
        last_data: Arc<AtomicU64>,
        reader_alive: Arc<AtomicBool>,
    ) {
        tracing::info!(exchange = "nado", "Connecting to Nado WS: {}", url);

        let url_parsed: url::Url = match url.parse() {
            Ok(u) => u,
            Err(e) => { tracing::error!(exchange = "nado", error = %e, "Invalid URL"); return; }
        };

        // Build custom HTTP request with the required Sec-WebSocket-Extensions header.
        // Nado's gateway checks for this header and returns 403 without it.
        let http_builder = yawc::HttpRequestBuilder::new()
            .header("Sec-WebSocket-Extensions", "permessage-deflate");

        // Enable compression so yawc:
        //   1) Sends the Sec-WebSocket-Extensions: permessage-deflate header (gateway requires it)
        //   2) Decompresses the compressed frames Nado sends back
        let options = yawc::Options::default().with_low_latency_compression();

        let mut ws = match yawc::WebSocket::connect(url_parsed)
            .with_options(options)
            .with_request(http_builder)
            .await
        {
            Ok(ws) => ws,
            Err(e) => {
                tracing::error!(exchange = "nado", error = %e, "Connection failed (yawc+deflate+header)");
                return;
            }
        };

        tracing::info!(exchange = "nado", "Connected with yawc (deflate decompression + explicit header)");
        reader_alive.store(true, Ordering::Relaxed);
        last_data.store(current_time_ms(), Ordering::Relaxed);

        // Subscribe to best_bid_offer for each market (BBO only)
        let markets = get_nado_markets();
        for (i, (product_id, sym)) in markets.iter().enumerate() {
            let sub_msg = NadoSubscribeMsg::best_bid_offer(*product_id, (i + 1) as u32);
            let json = match serde_json::to_string(&sub_msg) {
                Ok(j) => j,
                Err(e) => { tracing::error!(exchange = "nado", error = %e, "Serialize failed"); continue; }
            };
            tracing::info!(exchange = "nado", symbol = sym, product_id, "Subscribing to best_bid_offer");
            let frame = yawc::Frame::text(json);
            if let Err(e) = ws.send(frame).await {
                tracing::error!(exchange = "nado", error = %e, "Subscribe send failed");
                reader_alive.store(false, Ordering::Relaxed);
                return;
            }
        }

        // Read push events
        let mut msg_count: u64 = 0;
        let mut last_ping = current_time_ms();

        while let Some(frame) = ws.next().await {
            let now = current_time_ms();
            last_data.store(now, Ordering::Relaxed);
            last_pong.store(now, Ordering::Relaxed);

            // Send ping every 25 seconds (Nado docs: keep-alive every 30s)
            if now.saturating_sub(last_ping) > 25_000 {
                if let Err(e) = ws.send(yawc::Frame::ping(vec![])).await {
                    tracing::warn!(exchange = "nado", error = %e, "Ping send failed");
                    break;
                }
                last_ping = now;
            }

            let text = match std::str::from_utf8(frame.payload()) {
                Ok(t) => t,
                Err(_) => continue,
            };

            match frame.opcode() {
                yawc::frame::OpCode::Text => {
                    msg_count += 1;
                    if msg_count <= 10 {
                        tracing::info!(exchange = "nado", msg_count, raw = %text.chars().take(300).collect::<String>(), "RAW WS message");
                    }

                    // Parse best_bid_offer events (BBO only)
                    if let Ok(evt) = serde_json::from_str::<NadoBboEvent>(text) {
                        if evt.event_type.as_deref() == Some("best_bid_offer") {
                            if let Some(pid) = evt.product_id {
                                if let Some(symbol) = product_id_to_symbol(pid) {
                                    let bid = evt.bid_price.as_deref().and_then(parse_nado_price);
                                    let ask = evt.ask_price.as_deref().and_then(parse_nado_price);
                                    if let (Some(b), Some(a)) = (bid, ask) {
                                        if b > 0.0 && a > 0.0 {
                                            let orderbook = Orderbook {
                                                bids: vec![OrderbookLevel::new(b, 0.0)],
                                                asks: vec![OrderbookLevel::new(a, 0.0)],
                                                timestamp: now,
                                            };
                                            shared_best_prices.store(b, a);
                                            if let Some(ref n) = orderbook_notify { n.notify_waiters(); }
                                            let mut books = shared_orderbooks.write().await;
                                            books.insert(symbol.to_string(), orderbook);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                yawc::frame::OpCode::Ping => {
                    // yawc auto-responds with pong
                }
                _ => {}
            }
        }

        tracing::warn!(exchange = "nado", msg_count, "WS stream ended");
        reader_alive.store(false, Ordering::Relaxed);
    }
}

#[async_trait]
impl ExchangeAdapter for NadoAdapter {
    async fn connect(&mut self) -> ExchangeResult<()> {
        self.connect_time_ms = current_time_ms();
        self.spawn_ws_task()?;

        // Wait for reader_alive to become true (yawc connected) with a 10s timeout
        // instead of a blind 500ms sleep. This prevents the race condition where the
        // manager's health check triggers reconnect before yawc finishes connecting.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if self.connection_health.reader_alive.load(Ordering::Relaxed) {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                tracing::warn!(exchange = "nado", "Connection timed out waiting for reader_alive (10s)");
                // Don't abort — the task may still connect; just proceed
                break;
            }
        }

        self.connected = true;
        tracing::info!(exchange = "nado", "Nado adapter started (yawc + deflate + explicit header)");
        Ok(())
    }

    async fn disconnect(&mut self) -> ExchangeResult<()> {
        { let mut state = self.connection_health.state.write().await; *state = ConnectionState::Disconnected; }
        if let Some(h) = self.reader_handle.take() { h.abort(); }
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
        if !self.connection_health.reader_alive.load(Ordering::Relaxed) {
            // Grace period: allow up to 15s after connect() before declaring stale.
            // This prevents the manager from triggering reconnect while yawc is
            // still establishing the WS connection.
            let age_since_connect = current_time_ms().saturating_sub(self.connect_time_ms);
            if age_since_connect < 15_000 {
                return false; // Still within grace period
            }
            return true;
        }
        let last_data = self.connection_health.last_data.load(Ordering::Relaxed);
        if last_data == 0 { return false; }
        use crate::adapters::types::STALE_THRESHOLD_MS;
        current_time_ms().saturating_sub(last_data) > STALE_THRESHOLD_MS
    }

    async fn sync_orderbooks(&mut self) {
        let books = self.shared_orderbooks.read().await;
        self.orderbooks = books.clone();
    }

    async fn reconnect(&mut self) -> ExchangeResult<()> {
        let saved = self.subscriptions.clone();
        self.disconnect().await?;
        for attempt in 0..3u32 {
            tokio::time::sleep(Duration::from_millis(std::cmp::min(500 * (1u64 << attempt), 5000))).await;
            if self.connect().await.is_ok() {
                for s in &saved { let _ = self.subscribe_orderbook(s).await; }
                return Ok(());
            }
        }
        Err(ExchangeError::ConnectionFailed("Nado reconnection failed".into()))
    }

    fn exchange_name(&self) -> &'static str { "nado" }
    fn get_shared_orderbooks(&self) -> SharedOrderbooks { Arc::clone(&self.shared_orderbooks) }
    fn get_shared_best_prices(&self) -> SharedBestPrices { Arc::clone(&self.shared_best_prices) }
    fn set_orderbook_notify(&mut self, notify: OrderbookNotify) { self.orderbook_notify = Some(notify); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adapter_basics() {
        let a = NadoAdapter::new(NadoConfig::default());
        assert!(!a.connected);
        assert_eq!(a.exchange_name(), "nado");
    }
}
