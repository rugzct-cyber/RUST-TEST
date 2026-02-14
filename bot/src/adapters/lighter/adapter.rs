//! Lighter Adapter Implementation
//!
//! Main LighterAdapter struct implementing ExchangeAdapter trait.
//! Read-only market data via WebSocket (public orderbooks).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use tokio::sync::{Mutex, RwLock};
use tokio_tungstenite::{
    connect_async,
    tungstenite::protocol::Message,
};

use crate::adapters::errors::{ExchangeError, ExchangeResult};
use crate::adapters::traits::ExchangeAdapter;
use crate::adapters::types::{
    create_http_client, next_subscription_id, ConnectionHealth, ConnectionState,
    Orderbook, OrderbookLevel, MAX_ORDERBOOK_DEPTH, STALE_THRESHOLD_MS,
};
use crate::core::channels::{AtomicBestPrices, OrderbookNotify, SharedBestPrices, SharedOrderbooks};

use super::config::LighterConfig;
use super::types::{MarketMapping, normalize_symbol_to_lighter};

/// Type alias for the WebSocket write sink
type WsSink = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    Message,
>;

// =============================================================================
// Lighter Adapter
// =============================================================================

/// Lighter exchange adapter implementing ExchangeAdapter trait (read-only market data)
pub struct LighterAdapter {
    /// Configuration
    config: LighterConfig,
    /// HTTP client for REST API calls
    http: reqwest::Client,
    /// WebSocket write half
    ws_sink: Arc<Mutex<Option<WsSink>>>,
    /// Connection health tracking
    pub(crate) health: ConnectionHealth,
    /// Subscribed symbols mapping: symbol → market_id
    subscriptions: HashMap<String, u8>,
    /// Market info: symbol → MarketMapping
    market_info: HashMap<String, MarketMapping>,
    /// Local orderbook cache (updated from sync_orderbooks)
    local_orderbooks: HashMap<String, Orderbook>,
    /// Shared orderbooks (written by WS reader, read by monitoring)
    shared_orderbooks: SharedOrderbooks,
    /// Shared best prices (atomic, lock-free)
    shared_best_prices: SharedBestPrices,
    /// Orderbook notification (event-driven monitoring)
    orderbook_notify: Option<OrderbookNotify>,
    /// Timestamp of last data received (for staleness check)
    last_data: Arc<AtomicU64>,
}

impl LighterAdapter {
    /// Create a new LighterAdapter with the given configuration
    pub fn new(config: LighterConfig) -> Self {
        let http = create_http_client("lighter");
        let health = ConnectionHealth::new();
        let last_data = Arc::clone(&health.last_data);

        Self {
            config,
            http,
            ws_sink: Arc::new(Mutex::new(None)),
            health,
            subscriptions: HashMap::new(),
            market_info: HashMap::new(),
            local_orderbooks: HashMap::new(),
            shared_orderbooks: Arc::new(RwLock::new(HashMap::new())),
            shared_best_prices: Arc::new(AtomicBestPrices::new()),
            orderbook_notify: None,
            last_data,
        }
    }

    /// Fetch exchange/market info from REST API (v1 orderBookDetails)
    async fn fetch_market_info(&mut self) -> ExchangeResult<()> {
        let url = format!("{}/api/v1/orderBookDetails", self.config.rest_url());
        let resp = self.http.get(&url).send().await.map_err(|e| {
            ExchangeError::ConnectionFailed(format!("Failed to fetch market info: {}", e))
        })?;
        let body: serde_json::Value = resp.json().await.map_err(|e| {
            ExchangeError::InvalidResponse(format!("Failed to parse market info: {}", e))
        })?;

        // Parse order_book_details array (v1 API format)
        if let Some(markets) = body["order_book_details"].as_array() {
            for m in markets {
                let symbol = m["symbol"].as_str().unwrap_or("").to_string();
                let market_id = m["market_id"].as_u64().unwrap_or(0) as u8;

                // v1 API provides decimal counts directly
                let price_precision = m["price_decimals"].as_u64().unwrap_or(2) as u8;
                let size_precision = m["size_decimals"].as_u64().unwrap_or(3) as u8;

                // Derive tick/step sizes from decimal counts: 10^(-decimals)
                let tick_size = 10f64.powi(-(price_precision as i32));
                let step_size = 10f64.powi(-(size_precision as i32));

                self.market_info.insert(
                    symbol.clone(),
                    MarketMapping {
                        symbol: symbol.clone(),
                        market_id,
                        tick_size,
                        step_size,
                        price_precision,
                        size_precision,
                    },
                );

                tracing::debug!(
                    exchange = "lighter",
                    symbol = %symbol,
                    market_id = market_id,
                    price_decimals = price_precision,
                    size_decimals = size_precision,
                    "Market registered"
                );
            }
        }

        tracing::info!(
            exchange = "lighter",
            market_count = self.market_info.len(),
            "Market info loaded"
        );
        Ok(())
    }

    /// Look up market_id for a symbol
    fn market_id_for(&self, symbol: &str) -> ExchangeResult<u8> {
        // Try direct match first
        if let Some(info) = self.market_info.get(symbol) {
            return Ok(info.market_id);
        }
        // Try normalized match (BTC-PERP → search for BTC in Lighter symbols)
        let base = normalize_symbol_to_lighter(symbol);
        for (lighter_sym, info) in &self.market_info {
            if lighter_sym.contains(&base) {
                return Ok(info.market_id);
            }
        }
        Err(ExchangeError::InvalidResponse(format!(
            "Unknown Lighter market: {}",
            symbol
        )))
    }

    /// Spawn the WebSocket reader loop as a background task
    fn spawn_reader(
        &mut self,
        reader: futures_util::stream::SplitStream<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
        >,
    ) {
        let shared_obs = Arc::clone(&self.shared_orderbooks);
        let shared_prices = Arc::clone(&self.shared_best_prices);
        let last_data = Arc::clone(&self.last_data);
        let reader_alive = Arc::clone(&self.health.reader_alive);
        let notify = self.orderbook_notify.clone();
        let market_info = self.market_info.clone();
        let ws_sink = Arc::clone(&self.ws_sink);

        reader_alive.store(true, Ordering::SeqCst);

        tokio::spawn(async move {
            Self::reader_loop(
                reader,
                shared_obs,
                shared_prices,
                notify,
                last_data,
                reader_alive,
                market_info,
                ws_sink,
            )
            .await;
        });
    }

    /// Background reader loop for WebSocket messages
    async fn reader_loop(
        mut reader: futures_util::stream::SplitStream<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
        >,
        shared_obs: SharedOrderbooks,
        shared_prices: SharedBestPrices,
        notify: Option<OrderbookNotify>,
        last_data: Arc<AtomicU64>,
        reader_alive: Arc<std::sync::atomic::AtomicBool>,
        market_info: HashMap<String, MarketMapping>,
        ws_sink: Arc<Mutex<Option<WsSink>>>,
    ) {
        // Build reverse mapping: market_id → symbol
        let id_to_symbol: HashMap<u8, String> = market_info
            .iter()
            .map(|(sym, m)| (m.market_id, sym.clone()))
            .collect();

        while let Some(msg_result) = reader.next().await {
            match msg_result {
                Ok(Message::Text(text)) => {
                    let now_ms = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0);
                    last_data.store(now_ms, Ordering::Relaxed);

                    // Parse the WS message
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) {
                        let msg_type = val["type"].as_str().unwrap_or("");

                        // Handle app-level ping/pong (Lighter sends {"type":"ping"})
                        if msg_type == "ping" {
                            let pong = serde_json::json!({"type": "pong"});
                            let mut sink_guard = ws_sink.lock().await;
                            if let Some(sink) = sink_guard.as_mut() {
                                let _ = sink.send(Message::Text(pong.to_string())).await;
                            }
                            continue;
                        } else if msg_type == "connected" {
                            tracing::info!(exchange = "lighter", "WS connected message received");
                            continue;
                        }

                        // Check for orderbook updates: channel = "order_book:{MARKET_INDEX}"
                        if msg_type == "update/order_book" || msg_type == "subscribed/order_book" {
                            if let Some(channel) = val["channel"].as_str() {
                                // Extract market_id from "order_book:{ID}"
                                let market_id = channel
                                    .split(':')
                                    .nth(1)
                                    .and_then(|s| s.parse::<u8>().ok())
                                    .unwrap_or(0);

                                if let Some(symbol) = id_to_symbol.get(&market_id) {
                                    if let Some(ob_data) = val.get("order_book") {
                                        let is_snapshot = msg_type == "subscribed/order_book";

                                        // Read existing OB state (or empty for snapshot)
                                        let mut bid_map: std::collections::BTreeMap<i64, f64>;
                                        let mut ask_map: std::collections::BTreeMap<i64, f64>;

                                        if is_snapshot {
                                            bid_map = std::collections::BTreeMap::new();
                                            ask_map = std::collections::BTreeMap::new();
                                        } else {
                                            // Read current state and convert to maps
                                            let obs = shared_obs.read().await;
                                            if let Some(existing) = obs.get(symbol) {
                                                bid_map = existing.bids.iter()
                                                    .map(|l| ((l.price * 1_000_000.0) as i64, l.quantity))
                                                    .collect();
                                                ask_map = existing.asks.iter()
                                                    .map(|l| ((l.price * 1_000_000.0) as i64, l.quantity))
                                                    .collect();
                                            } else {
                                                bid_map = std::collections::BTreeMap::new();
                                                ask_map = std::collections::BTreeMap::new();
                                            }
                                        }

                                        // Apply delta: size=0 removes, size>0 adds/updates
                                        let apply_delta = |map: &mut std::collections::BTreeMap<i64, f64>, arr: &serde_json::Value| {
                                            if let Some(levels) = arr.as_array() {
                                                for level in levels {
                                                    let price = level["price"]
                                                        .as_str()
                                                        .and_then(|s| s.parse::<f64>().ok())
                                                        .or_else(|| level["price"].as_f64());
                                                    let qty = level["size"]
                                                        .as_str()
                                                        .and_then(|s| s.parse::<f64>().ok())
                                                        .or_else(|| level["size"].as_f64());
                                                    if let (Some(p), Some(q)) = (price, qty) {
                                                        let key = (p * 1_000_000.0) as i64;
                                                        if q <= 0.0 {
                                                            map.remove(&key);
                                                        } else {
                                                            map.insert(key, q);
                                                        }
                                                    }
                                                }
                                            }
                                        };

                                        apply_delta(&mut bid_map, &ob_data["bids"]);
                                        apply_delta(&mut ask_map, &ob_data["asks"]);

                                        // Rebuild sorted OB: bids descending, asks ascending
                                        let bids: Vec<OrderbookLevel> = bid_map.iter().rev()
                                            .take(MAX_ORDERBOOK_DEPTH)
                                            .map(|(&k, &q)| OrderbookLevel::new(k as f64 / 1_000_000.0, q))
                                            .collect();
                                        let asks: Vec<OrderbookLevel> = ask_map.iter()
                                            .take(MAX_ORDERBOOK_DEPTH)
                                            .map(|(&k, &q)| OrderbookLevel::new(k as f64 / 1_000_000.0, q))
                                            .collect();

                                        let ob = Orderbook {
                                            bids,
                                            asks,
                                            timestamp: now_ms,
                                        };

                                        // Update shared best prices (atomic, lock-free)
                                        shared_prices.store(
                                            ob.best_bid().unwrap_or(0.0),
                                            ob.best_ask().unwrap_or(0.0),
                                        );

                                        // Update shared orderbook
                                        {
                                            let mut obs = shared_obs.write().await;
                                            obs.insert(symbol.clone(), ob);
                                        }

                                        // Notify monitoring
                                        if let Some(ref n) = &notify {
                                            n.notify_waiters();
                                        }
                                    }
                                }
                            }
                        } else if val.get("error").is_some() {
                            tracing::warn!(
                                exchange = "lighter",
                                msg = %text,
                                "WS error message"
                            );
                        }
                    }
                }
                Ok(Message::Ping(data)) => {
                    tracing::trace!(exchange = "lighter", "Received WS ping");
                    // Pong is automatically handled by tungstenite
                    let _ = data; // suppress unused warning
                }
                Ok(Message::Pong(_)) => {
                    let now_ms = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0);
                    last_data.store(now_ms, Ordering::Relaxed);
                }
                Ok(Message::Close(_)) => {
                    tracing::warn!(exchange = "lighter", "WebSocket closed by server");
                    break;
                }
                Err(e) => {
                    tracing::error!(exchange = "lighter", error = %e, "WebSocket read error");
                    break;
                }
                _ => {}
            }
        }

        // Signal reader death
        reader_alive.store(false, Ordering::SeqCst);
        tracing::warn!(exchange = "lighter", "Reader loop exited");
    }
}

// =============================================================================
// ExchangeAdapter Trait Implementation
// =============================================================================

#[async_trait::async_trait]
impl ExchangeAdapter for LighterAdapter {
    async fn connect(&mut self) -> ExchangeResult<()> {
        tracing::info!(exchange = "lighter", url = %self.config.rest_url(), "Connecting…");

        // 1. Fetch market info (needed for symbol → market_id mapping)
        self.fetch_market_info().await?;

        // 2. Connect WebSocket (public orderbook, no auth needed)
        let ws_url = self.config.ws_url();
        let (ws_stream, _) = connect_async(ws_url).await.map_err(|e| {
            ExchangeError::ConnectionFailed(format!("WebSocket connect failed: {}", e))
        })?;
        let (sink, reader) = ws_stream.split();

        {
            let mut sink_guard = self.ws_sink.lock().await;
            *sink_guard = Some(sink);
        }

        // 3. Spawn reader loop
        self.spawn_reader(reader);

        // 4. Mark connected
        {
            let mut state = self.health.state.write().await;
            *state = ConnectionState::Connected;
        }

        // Update last_data to now
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        self.last_data.store(now_ms, Ordering::Relaxed);

        tracing::info!(
            exchange = "lighter",
            markets = self.market_info.len(),
            "Connected successfully"
        );
        Ok(())
    }

    async fn disconnect(&mut self) -> ExchangeResult<()> {
        {
            let mut sink_guard = self.ws_sink.lock().await;
            if let Some(ref mut sink) = *sink_guard {
                let _ = sink.close().await;
            }
            *sink_guard = None;
        }

        let mut state = self.health.state.write().await;
        *state = ConnectionState::Disconnected;
        tracing::info!(exchange = "lighter", "Disconnected");
        Ok(())
    }

    async fn subscribe_orderbook(&mut self, symbol: &str) -> ExchangeResult<()> {
        let market_id = self.market_id_for(symbol)?;

        // Send WS subscribe message (Lighter format: type + channel)
        let msg = serde_json::json!({
            "type": "subscribe",
            "channel": format!("order_book/{}", market_id),
        });

        {
            let mut sink_guard = self.ws_sink.lock().await;
            if let Some(ref mut sink) = *sink_guard {
                sink.send(Message::Text(msg.to_string()))
                    .await
                    .map_err(|e| {
                        ExchangeError::ConnectionFailed(format!(
                            "Failed to subscribe orderbook: {}",
                            e
                        ))
                    })?;
            } else {
                return Err(ExchangeError::ConnectionFailed(
                    "WebSocket not connected".into(),
                ));
            }
        }

        self.subscriptions.insert(symbol.to_string(), market_id);

        // Initialize empty orderbook in shared storage
        {
            let mut obs = self.shared_orderbooks.write().await;
            obs.entry(symbol.to_string())
                .or_insert_with(Orderbook::new);
        }

        tracing::info!(
            exchange = "lighter",
            symbol = %symbol,
            market_id = market_id,
            "Subscribed to orderbook"
        );
        Ok(())
    }

    async fn unsubscribe_orderbook(&mut self, symbol: &str) -> ExchangeResult<()> {
        if let Some(market_id) = self.subscriptions.remove(symbol) {
            let sub_id = next_subscription_id();
            let msg = serde_json::json!({
                "method": "unsubscribe",
                "params": {
                    "channel": "orderbookdepth",
                    "market_id": market_id.to_string(),
                },
                "id": sub_id,
            });

            {
                let mut sink_guard = self.ws_sink.lock().await;
                if let Some(ref mut sink) = *sink_guard {
                    let _ = sink.send(Message::Text(msg.to_string())).await;
                }
            }

            // Remove from shared storage
            {
                let mut obs = self.shared_orderbooks.write().await;
                obs.remove(symbol);
            }

            tracing::info!(
                exchange = "lighter",
                symbol = %symbol,
                "Unsubscribed from orderbook"
            );
        }
        Ok(())
    }

    fn get_orderbook(&self, symbol: &str) -> Option<&Orderbook> {
        self.local_orderbooks.get(symbol)
    }

    fn is_connected(&self) -> bool {
        // Fast check: if reader died, not connected
        if !self.health.reader_alive.load(Ordering::Relaxed) {
            return false;
        }
        // Check ws_sink presence (try_lock since this is a sync fn)
        match self.ws_sink.try_lock() {
            Ok(guard) => guard.is_some(),
            Err(_) => true, // Lock contended = actively in use = connected
        }
    }

    fn is_stale(&self) -> bool {
        // If reader died, immediately stale
        if !self.health.reader_alive.load(Ordering::Relaxed) {
            return true;
        }

        let last = self.last_data.load(Ordering::Relaxed);
        if last == 0 {
            return false; // Not yet initialized
        }

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        now_ms.saturating_sub(last) > STALE_THRESHOLD_MS
    }

    async fn sync_orderbooks(&mut self) {
        let shared = self.shared_orderbooks.read().await;
        for (sym, ob) in shared.iter() {
            self.local_orderbooks.insert(sym.clone(), ob.clone());
        }
    }

    async fn reconnect(&mut self) -> ExchangeResult<()> {
        tracing::info!(exchange = "lighter", "Reconnecting…");

        // Save current subscriptions
        let subs: Vec<String> = self.subscriptions.keys().cloned().collect();

        // Disconnect
        self.disconnect().await?;

        // Reconnect
        self.connect().await?;

        // Re-subscribe
        for sym in subs {
            self.subscribe_orderbook(&sym).await?;
        }

        tracing::info!(exchange = "lighter", "Reconnected successfully");
        Ok(())
    }

    fn exchange_name(&self) -> &'static str {
        "lighter"
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
