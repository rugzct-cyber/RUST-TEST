//! Nord Adapter — incremental deltas stream

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

use super::config::NordConfig;
use super::types::{get_nord_markets, nord_symbol_to_canonical, NordWsMessage};

fn current_time_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

type WsStream = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
type WsWriter = SplitSink<WsStream, Message>;
type WsReader = SplitStream<WsStream>;

pub struct NordWsAdapter {
    config: NordConfig,
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

impl NordWsAdapter {
    pub fn new(config: NordConfig) -> Self {
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
        tracing::info!("Connecting to Nord WebSocket: {}", url);
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

    async fn subscribe_to_deltas(&self) -> ExchangeResult<()> {
        let ws_sender = self.ws_sender.as_ref().ok_or_else(|| ExchangeError::ConnectionFailed("WS not connected".into()))?;
        let markets = get_nord_markets();
        // Nord has a stream limit; subscribe to top markets only
        for (symbol, _) in markets.iter().take(5) {
            let msg = serde_json::json!({"type": "subscribe", "channel": "deltas", "symbol": symbol});
            let mut sender = ws_sender.lock().await;
            sender.send(Message::Text(msg.to_string())).await.map_err(|e| ExchangeError::WebSocket(Box::new(e)))?;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        tracing::info!(exchange = "nord", "Subscribed to deltas");
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
        tracing::info!("Nord message_reader_loop started");
        reader_alive.store(true, Ordering::Relaxed);
        let mut msg_count: u64 = 0;
        while let Some(msg_result) = ws_receiver.next().await {
            last_data.store(current_time_ms(), Ordering::Relaxed);
            last_pong.store(current_time_ms(), Ordering::Relaxed);

            match msg_result {
                Ok(Message::Text(text)) => {
                    msg_count += 1;
                    if msg_count <= 3 {
                        tracing::info!(exchange = "nord", msg_count, raw = %text.chars().take(300).collect::<String>(), "RAW WS message");
                    }
                    if let Ok(msg) = serde_json::from_str::<NordWsMessage>(&text) {
                        if let Some(data) = msg.delta {
                            let nord_symbol = match &data.market_symbol { Some(s) => s.clone(), None => continue };
                            if let Some(symbol) = nord_symbol_to_canonical(&nord_symbol) {
                                // Find best bid (highest) and best ask (lowest) from non-zero entries
                                let bid = data.bids.iter()
                                    .filter(|(_, qty)| *qty > 0.0)
                                    .map(|(price, _)| *price)
                                    .fold(f64::NEG_INFINITY, f64::max);
                                let ask = data.asks.iter()
                                    .filter(|(_, qty)| *qty > 0.0)
                                    .map(|(price, _)| *price)
                                    .fold(f64::INFINITY, f64::min);
                                if bid > 0.0 && ask < f64::INFINITY && bid.is_finite() && ask.is_finite() {
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
                Ok(Message::Close(_)) => break,
                Ok(_) => {}
                Err(e) => { tracing::error!("Nord WS error: {}", e); break; }
            }
        }
        reader_alive.store(false, Ordering::Relaxed);
    }

    fn spawn_heartbeat_task(&mut self) {
        let ws_sender = match &self.ws_sender { Some(s) => Arc::clone(s), None => return };
        let last_pong = Arc::clone(&self.connection_health.last_pong);
        let reader_alive = Arc::clone(&self.connection_health.reader_alive);
        last_pong.store(current_time_ms(), Ordering::Relaxed);

        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(20));
            interval.tick().await;
            loop {
                interval.tick().await;
                {
                    let mut sender = ws_sender.lock().await;
                    if let Err(_) = sender.send(Message::Ping(vec![])).await { reader_alive.store(false, Ordering::Relaxed); break; }
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
impl ExchangeAdapter for NordWsAdapter {
    async fn connect(&mut self) -> ExchangeResult<()> {
        self.connect_websocket().await?;
        self.split_and_spawn_reader()?;
        self.spawn_heartbeat_task();
        self.subscribe_to_deltas().await?;
        self.connected = true;
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
        let saved = self.subscriptions.clone(); self.disconnect().await?;
        for attempt in 0..3u32 {
            tokio::time::sleep(Duration::from_millis(std::cmp::min(500 * (1u64 << attempt), 5000))).await;
            if self.connect().await.is_ok() { for s in &saved { let _ = self.subscribe_orderbook(s).await; } return Ok(()); }
        }
        Err(ExchangeError::ConnectionFailed("Nord reconnection failed".into()))
    }
    fn exchange_name(&self) -> &'static str { "nord" }
    fn get_shared_orderbooks(&self) -> SharedOrderbooks { Arc::clone(&self.shared_orderbooks) }
    fn get_shared_best_prices(&self) -> SharedBestPrices { Arc::clone(&self.shared_best_prices) }
    fn set_orderbook_notify(&mut self, notify: OrderbookNotify) { self.orderbook_notify = Some(notify); }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_adapter_basics() {
        let a = NordWsAdapter::new(NordConfig::default());
        assert!(!a.connected);
        assert_eq!(a.exchange_name(), "nord");
    }
}
