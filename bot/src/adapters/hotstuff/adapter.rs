//! HotStuff Adapter — JSON-RPC 2.0, ticker channel per symbol

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

use super::config::HotstuffConfig;
use super::types::{get_hotstuff_markets, instrument_to_symbol, HotstuffJsonRpcMessage};

fn current_time_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

type WsStream = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
type WsWriter = SplitSink<WsStream, Message>;
type WsReader = SplitStream<WsStream>;

pub struct HotstuffAdapter {
    config: HotstuffConfig,
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

impl HotstuffAdapter {
    pub fn new(config: HotstuffConfig) -> Self {
        Self {
            config, ws_stream: None, ws_sender: None,
            reader_handle: None, heartbeat_handle: None,
            connected: false, subscriptions: Vec::new(),
            orderbooks: HashMap::new(),
            shared_orderbooks: Arc::new(RwLock::new(HashMap::new())),
            shared_best_prices: Arc::new(AtomicBestPrices::new()),
            orderbook_notify: None, connection_health: ConnectionHealth::default(),
        }
    }

    async fn connect_websocket(&mut self) -> ExchangeResult<()> {
        let url = self.config.ws_url();
        tracing::info!("Connecting to HotStuff WebSocket: {}", url);
        let ws_stream = crate::adapters::shared::connect_tls(url).await?;
        self.ws_stream = Some(Mutex::new(ws_stream));
        Ok(())
    }

    fn split_and_spawn_reader(&mut self) -> ExchangeResult<()> {
        let ws_stream_mutex = self.ws_stream.take().ok_or_else(|| ExchangeError::ConnectionFailed("No WS stream".into()))?;
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

    async fn subscribe_to_tickers(&self) -> ExchangeResult<()> {
        let ws_sender = self.ws_sender.as_ref().ok_or_else(|| ExchangeError::ConnectionFailed("WS not connected".into()))?;
        let markets = get_hotstuff_markets();
        for (i, (instrument, _)) in markets.iter().enumerate() {
            let msg = serde_json::json!({
                "jsonrpc": "2.0",
                "id": i + 1,
                "method": "subscribe",
                "params": { "channel": "ticker", "symbol": instrument }
            });
            {
                let mut sender = ws_sender.lock().await;
                sender.send(Message::Text(msg.to_string())).await.map_err(|e| ExchangeError::WebSocket(Box::new(e)))?;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        tracing::info!(exchange = "hotstuff", count = markets.len(), "Subscribed to ticker channels");
        Ok(())
    }

    async fn message_reader_loop(
        mut ws_receiver: WsReader,
        shared_orderbooks: SharedOrderbooks,
        shared_best_prices: SharedBestPrices,
        orderbook_notify: Option<OrderbookNotify>,
        last_pong: Arc<AtomicU64>,
        last_data: Arc<AtomicU64>,
        reader_alive: Arc<AtomicBool>,
    ) {
        tracing::info!("HotStuff message_reader_loop started");
        reader_alive.store(true, Ordering::Relaxed);
        let mut msg_count: u64 = 0;

        while let Some(msg_result) = ws_receiver.next().await {
            last_data.store(current_time_ms(), Ordering::Relaxed);
            last_pong.store(current_time_ms(), Ordering::Relaxed);

            match msg_result {
                Ok(Message::Text(text)) => {
                    msg_count += 1;
                    if msg_count <= 3 {
                        tracing::info!(exchange = "hotstuff", msg_count, raw = %text.chars().take(300).collect::<String>(), "RAW WS message");
                    }
                    if let Ok(msg) = serde_json::from_str::<HotstuffJsonRpcMessage>(&text) {
                        if msg.result.is_some() { continue; }
                        if let Some(params) = msg.params {
                            if let Some(data) = params.data {
                                let instrument = match data.instrument_name { Some(i) => i, None => continue };
                                if let Some(symbol) = instrument_to_symbol(&instrument) {
                                    let bid: f64 = match data.best_bid_price.and_then(|s| s.parse().ok()) { Some(v) => v, None => continue };
                                    let ask: f64 = match data.best_ask_price.and_then(|s| s.parse().ok()) { Some(v) => v, None => continue };
                                    if bid > 0.0 && ask > 0.0 {
                                        let orderbook = Orderbook {
                                            bids: vec![OrderbookLevel::new(bid, 0.0)],
                                            asks: vec![OrderbookLevel::new(ask, 0.0)],
                                            timestamp: current_time_ms(),
                                        };
                                        shared_best_prices.store(bid, ask);
                                        if let Some(ref n) = orderbook_notify { n.notify_waiters(); }
                                        let mut books = shared_orderbooks.write().await;
                                        books.insert(symbol.to_string(), orderbook);
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(Message::Close(_)) => { tracing::info!("HotStuff WS closed"); break; }
                Ok(_) => {}
                Err(e) => { tracing::error!("HotStuff WS error: {}", e); break; }
            }
        }
        reader_alive.store(false, Ordering::Relaxed);
        tracing::warn!("HotStuff message reader loop ended");
    }

    fn spawn_heartbeat_task(&mut self) {
        let ws_sender = match &self.ws_sender { Some(s) => Arc::clone(s), None => return };
        let last_pong = Arc::clone(&self.connection_health.last_pong);
        let reader_alive = Arc::clone(&self.connection_health.reader_alive);
        last_pong.store(current_time_ms(), Ordering::Relaxed);

        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(15));
            interval.tick().await;
            loop {
                interval.tick().await;
                let ping = serde_json::json!({"id": 0, "method": "public/heartbeat"});
                {
                    let mut sender = ws_sender.lock().await;
                    if let Err(e) = sender.send(Message::Text(ping.to_string())).await {
                        tracing::warn!("HotStuff heartbeat failed: {}", e);
                        reader_alive.store(false, Ordering::Relaxed);
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
                let age = current_time_ms().saturating_sub(last_pong.load(Ordering::Relaxed));
                if age > 60_000 { reader_alive.store(false, Ordering::Relaxed); break; }
            }
        });
        self.heartbeat_handle = Some(handle);
    }
}

#[async_trait]
impl ExchangeAdapter for HotstuffAdapter {
    async fn connect(&mut self) -> ExchangeResult<()> {
        self.connect_websocket().await?;
        self.split_and_spawn_reader()?;
        self.spawn_heartbeat_task();
        self.subscribe_to_tickers().await?;
        self.connected = true;
        tracing::info!(exchange = "hotstuff", "HotStuff WebSocket connected");
        Ok(())
    }

    async fn disconnect(&mut self) -> ExchangeResult<()> {
        { let mut state = self.connection_health.state.write().await; *state = ConnectionState::Disconnected; }
        if let Some(h) = self.reader_handle.take() { h.abort(); }
        if let Some(h) = self.heartbeat_handle.take() { h.abort(); }
        if let Some(s) = self.ws_sender.take() { let mut s = s.lock().await; let _ = s.close().await; }
        if let Some(w) = self.ws_stream.take() { let mut w = w.lock().await; let _ = w.close(None).await; }
        self.connected = false; self.subscriptions.clear();
        self.connection_health.last_pong.store(0, Ordering::Relaxed);
        self.connection_health.last_data.store(0, Ordering::Relaxed);
        self.connection_health.reader_alive.store(false, Ordering::Relaxed);
        let mut books = self.shared_orderbooks.write().await; books.clear(); self.orderbooks.clear();
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
        self.orderbooks.remove(symbol); Ok(())
    }

    fn get_orderbook(&self, symbol: &str) -> Option<&Orderbook> { self.orderbooks.get(symbol) }
    fn is_connected(&self) -> bool { self.connected }

    fn is_stale(&self) -> bool {
        if !self.connected { return true; }
        if !self.connection_health.reader_alive.load(Ordering::Relaxed) { return true; }
        let last_data = self.connection_health.last_data.load(Ordering::Relaxed);
        if last_data == 0 { return false; }
        use crate::adapters::types::STALE_THRESHOLD_MS;
        current_time_ms().saturating_sub(last_data) > STALE_THRESHOLD_MS
    }

    async fn sync_orderbooks(&mut self) { let books = self.shared_orderbooks.read().await; self.orderbooks = books.clone(); }

    async fn reconnect(&mut self) -> ExchangeResult<()> {
        tracing::info!("HotStuff: Reconnecting...");
        { let mut state = self.connection_health.state.write().await; *state = ConnectionState::Reconnecting; }
        let saved = self.subscriptions.clone(); self.disconnect().await?;
        for attempt in 0..3u32 {
            let backoff = std::cmp::min(500 * (1u64 << attempt), 5000);
            tokio::time::sleep(Duration::from_millis(backoff)).await;
            if self.connect().await.is_ok() {
                { let mut state = self.connection_health.state.write().await; *state = ConnectionState::Connected; }
                for s in &saved { let _ = self.subscribe_orderbook(s).await; }
                return Ok(());
            }
        }
        { let mut state = self.connection_health.state.write().await; *state = ConnectionState::Disconnected; }
        Err(ExchangeError::ConnectionFailed("HotStuff reconnection failed".into()))
    }

    fn exchange_name(&self) -> &'static str { "hotstuff" }
    fn get_shared_orderbooks(&self) -> SharedOrderbooks { Arc::clone(&self.shared_orderbooks) }
    fn get_shared_best_prices(&self) -> SharedBestPrices { Arc::clone(&self.shared_best_prices) }
    fn set_orderbook_notify(&mut self, notify: OrderbookNotify) { self.orderbook_notify = Some(notify); }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_adapter_not_connected_initially() {
        let adapter = HotstuffAdapter::new(HotstuffConfig::default());
        assert!(!adapter.connected);
        assert_eq!(adapter.exchange_name(), "hotstuff");
    }
}
