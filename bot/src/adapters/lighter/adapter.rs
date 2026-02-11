//! Lighter Adapter Implementation
//!
//! Main LighterAdapter struct implementing ExchangeAdapter trait.
//! Uses Schnorr/Poseidon2/Goldilocks signing via vendored lighter-crypto crates.

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
    create_http_client, next_subscription_id, ConnectionHealth, ConnectionState, FillInfo,
    Orderbook, OrderRequest, OrderResponse, OrderSide, OrderStatus, OrderbookLevel,
    PositionInfo, TimeInForce, MAX_ORDERBOOK_DEPTH, STALE_THRESHOLD_MS,
};
use crate::core::channels::{AtomicBestPrices, OrderbookNotify, SharedBestPrices, SharedOrderbooks};

use super::config::LighterConfig;
use super::signing::LighterSigner;
use super::types::{
    LighterPositionData,
    MarketMapping, normalize_symbol_to_lighter,
};

/// Type alias for the WebSocket write sink
type WsSink = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    Message,
>;

// =============================================================================
// Constants
// =============================================================================

/// Timeout for WebSocket authentication (5 seconds)
const _AUTH_TIMEOUT_SECS: u64 = 5;
/// Auth token lifetime for WebSocket (10 minutes)
const AUTH_TOKEN_LIFETIME_SECS: i64 = 600;

// =============================================================================
// Lighter Adapter
// =============================================================================

/// Lighter exchange adapter implementing ExchangeAdapter trait
pub struct LighterAdapter {
    /// Configuration
    config: LighterConfig,
    /// Transaction signer
    signer: LighterSigner,
    /// HTTP client for REST API calls
    http: reqwest::Client,
    /// WebSocket write half (Mutex-wrapped for shared &self access from place_order/cancel_order)
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
        let signer = LighterSigner::new(
            &config.private_key,
            config.account_index,
            config.api_key_index,
            config.chain_id(),
        )
        .expect("Failed to initialize Lighter signer");

        let http = create_http_client("lighter");
        let health = ConnectionHealth::new();
        let last_data = Arc::clone(&health.last_data);

        Self {
            config,
            signer,
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

    /// Get shared orderbooks for lock-free monitoring
    pub fn get_shared_orderbooks(&self) -> SharedOrderbooks {
        Arc::clone(&self.shared_orderbooks)
    }

    /// Get shared atomic best prices for lock-free hot-path monitoring
    pub fn get_shared_best_prices(&self) -> SharedBestPrices {
        Arc::clone(&self.shared_best_prices)
    }

    /// Set the shared orderbook notification
    pub fn set_orderbook_notify(&mut self, notify: OrderbookNotify) {
        self.orderbook_notify = Some(notify);
    }

    /// Fetch nonce from Lighter API and initialize local nonce cache
    async fn init_nonce(&self) -> ExchangeResult<()> {
        let url = format!(
            "{}/api/v1/nextNonce?account_index={}&api_key_index={}",
            self.config.rest_url(),
            self.signer.account_index(),
            self.config.api_key_index,
        );
        let resp = self.http.get(&url).send().await.map_err(|e| {
            ExchangeError::ConnectionFailed(format!("Failed to fetch nonce: {}", e))
        })?;
        let body: serde_json::Value = resp.json().await.map_err(|e| {
            ExchangeError::InvalidResponse(format!("Failed to parse nonce response: {}", e))
        })?;

        let nonce = body["nonce"]
            .as_i64()
            .ok_or_else(|| ExchangeError::InvalidResponse("Missing nonce in response".into()))?;

        self.signer.set_nonce(nonce);
        tracing::info!(
            exchange = "lighter",
            nonce = nonce,
            "Nonce initialized from API"
        );
        Ok(())
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
        Err(ExchangeError::InvalidOrder(format!(
            "Unknown Lighter market: {}",
            symbol
        )))
    }

    /// Convert floating-point price to Lighter integer representation
    fn price_to_int(&self, symbol: &str, price: f64) -> u32 {
        if let Some(info) = self.market_info.get(symbol) {
            let factor = 10f64.powi(info.price_precision as i32);
            (price * factor).round() as u32
        } else {
            // Fallback: assume 2 decimal places
            (price * 100.0).round() as u32
        }
    }

    /// Convert floating-point quantity to Lighter integer representation.
    ///
    /// IMPORTANT: We must truncate to the coarser precision first (Vest's) so that
    /// both exchanges trade exactly the same quantity.  Vest formats BTC at 4 dp,
    /// ETH at 3 dp, SOL at 2 dp — if we skip this step, Lighter's finer precision
    /// (BTC=5dp) can produce a different rounded value.
    fn quantity_to_int(&self, symbol: &str, quantity: f64) -> i64 {
        // Step 1: truncate to the coarser exchange precision (Vest's rounding)
        let coarse_decimals: u32 = if symbol.starts_with("BTC") {
            4
        } else if symbol.starts_with("ETH") {
            3
        } else {
            2 // SOL and others
        };
        let coarse_factor = 10f64.powi(coarse_decimals as i32);
        let truncated_qty = (quantity * coarse_factor).round() / coarse_factor;

        // Step 2: convert to Lighter integer using its own size_precision
        if let Some(info) = self.market_info.get(symbol) {
            let factor = 10f64.powi(info.size_precision as i32);
            let result = (truncated_qty * factor).round() as i64;
            tracing::info!(
                exchange = "lighter",
                symbol = %symbol,
                raw_qty = %format!("{:.8}", quantity),
                truncated_qty = %format!("{:.8}", truncated_qty),
                size_precision = info.size_precision,
                base_amount = result,
                "quantity_to_int conversion (coarse-aligned)"
            );
            result
        } else {
            let result = (truncated_qty * 1000.0).round() as i64;
            tracing::warn!(
                exchange = "lighter",
                symbol = %symbol,
                raw_qty = %format!("{:.8}", quantity),
                base_amount = result,
                "quantity_to_int FALLBACK (symbol not in market_info!)"
            );
            result
        }
    }

    /// Map our TimeInForce to the Lighter integer value
    /// Lighter SDK: IOC=0, GoodTillTime=1, PostOnly=2
    fn tif_to_lighter(tif: &TimeInForce) -> u8 {
        match tif {
            TimeInForce::Ioc => 0, // ImmediateOrCancel
            TimeInForce::Gtc => 1, // GoodTillTime
            TimeInForce::Fok => 0, // FillOrKill → treated as IOC
        }
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
                        // Handle sendtx responses (fire-and-forget ACK/error)
                        } else if msg_type.contains("sendtx") || msg_type.contains("error") {
                            if let Some(err) = val["error"].as_str() {
                                tracing::warn!(
                                    exchange = "lighter",
                                    error = %err,
                                    "sendtx WS response error"
                                );
                            } else if val.get("error").is_some() {
                                tracing::warn!(
                                    exchange = "lighter",
                                    msg = %text,
                                    "WS error message"
                                );
                            } else {
                                tracing::debug!(
                                    exchange = "lighter",
                                    response = %text,
                                    "sendtx WS response"
                                );
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

    /// Send a signed transaction via WebSocket (jsonapi/sendtx)
    ///
    /// Format per official docs: https://apidocs.lighter.xyz/docs/websocket-reference#send-tx
    /// tx_info is sent as a nested JSON object (not a string).
    async fn send_tx(
        &self,
        tx_type: u32,
        tx_info_json: &str,
    ) -> ExchangeResult<()> {
        // Parse tx_info from string to JSON object so it's nested, not double-encoded
        let tx_info_obj: serde_json::Value = serde_json::from_str(tx_info_json)
            .map_err(|e| ExchangeError::InvalidOrder(format!("tx_info parse error: {}", e)))?;

        let ws_msg = serde_json::json!({
            "type": "jsonapi/sendtx",
            "data": {
                "tx_type": tx_type,
                "tx_info": tx_info_obj,
            }
        });

        tracing::debug!(
            exchange = "lighter",
            tx_type = tx_type,
            msg = %ws_msg,
            "Sending WS sendTx"
        );

        let mut sink_guard = self.ws_sink.lock().await;
        let sink = sink_guard.as_mut().ok_or_else(|| {
            ExchangeError::ConnectionFailed("WebSocket not connected".into())
        })?;

        sink.send(Message::Text(ws_msg.to_string())).await.map_err(|e| {
            ExchangeError::ConnectionFailed(format!("sendTx WS send failed: {}", e))
        })?;

        Ok(())
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

        // 2. Fetch nonce from API
        self.init_nonce().await?;

        // 3. Create auth token for WS (stored for authenticated subscriptions)
        let _auth_token = self.signer.create_auth_token(AUTH_TOKEN_LIFETIME_SECS)?;

        // 4. Connect WebSocket
        // Note: Lighter WS has no auth handshake — auth is per-subscribe for
        // private channels. sendTx uses REST with self-authenticated signatures.
        let ws_url = self.config.ws_url();
        let (ws_stream, _) = connect_async(ws_url).await.map_err(|e| {
            ExchangeError::ConnectionFailed(format!("WebSocket connect failed: {}", e))
        })?;
        let (sink, reader) = ws_stream.split();

        {
            let mut sink_guard = self.ws_sink.lock().await;
            *sink_guard = Some(sink);
        }

        // 6. Spawn reader loop
        self.spawn_reader(reader);

        // 7. Mark connected
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

    async fn place_order(&self, order: OrderRequest) -> ExchangeResult<OrderResponse> {
        // Validate
        if let Some(err) = order.validate() {
            return Err(ExchangeError::InvalidOrder(err.to_string()));
        }

        let market_id = self.market_id_for(&order.symbol)?;
        let price = order.price.unwrap_or(0.0);
        let price_int = self.price_to_int(&order.symbol, price);
        let base_amount = self.quantity_to_int(&order.symbol, order.quantity);
        let is_ask = order.side == OrderSide::Sell;
        let tif = Self::tif_to_lighter(&order.time_in_force);

        // Lighter order type: 0 = Limit
        let order_type: u8 = 0;

        // Get nonce (optimistic)
        let nonce = self.signer.next_nonce();

        tracing::debug!(
            exchange = "lighter",
            symbol = %order.symbol,
            side = ?order.side,
            price = price,
            price_int = price_int,
            quantity = order.quantity,
            base_amount = base_amount,
            nonce = nonce,
            "Placing order"
        );

        // Sign the transaction
        let tx_info_json = self.signer.sign_create_order(
            market_id,
            0, // client_order_index (auto-assigned)
            base_amount,
            price_int,
            is_ask,
            order_type,
            tif,
            order.reduce_only,
            0, // trigger_price (no trigger for limit)
            nonce,
        )?;

        // Send via WebSocket (fire-and-forget — response arrives asynchronously)
        self.send_tx(14, &tx_info_json).await.map_err(|e| {
            self.signer.rollback_nonce();
            e
        })?;

        Ok(OrderResponse {
            order_id: "pending".to_string(),
            client_order_id: order.client_order_id,
            status: OrderStatus::Pending,
            filled_quantity: 0.0,
            avg_price: None,
        })
    }

    async fn cancel_order(&self, order_id: &str) -> ExchangeResult<()> {
        // order_id is the Lighter order index (numeric)
        let order_index: i64 = order_id.parse().map_err(|_| {
            ExchangeError::InvalidOrder(format!("Invalid order ID: {}", order_id))
        })?;

        // Look up market_id from active subscriptions
        let market_id: u8 = self
            .subscriptions
            .values()
            .next()
            .copied()
            .ok_or_else(|| {
                ExchangeError::InvalidOrder(
                    "No subscribed market — cannot determine market_id for cancel".into(),
                )
            })?;

        let nonce = self.signer.next_nonce();

        let tx_info_json = self.signer.sign_cancel_order(market_id, order_index, nonce)?;

        // Send via WebSocket (fire-and-forget)
        self.send_tx(15, &tx_info_json).await.map_err(|e| {
            self.signer.rollback_nonce();
            e
        })
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

    async fn get_position(&self, symbol: &str) -> ExchangeResult<Option<PositionInfo>> {
        let account_index = self.signer.account_index();
        let url = format!(
            "{}/api/v1/account?by=index&value={}",
            self.config.rest_url(),
            account_index
        );

        let resp = self.http.get(&url).send().await.map_err(|e| {
            ExchangeError::ConnectionFailed(format!("Failed to fetch account: {}", e))
        })?;
        let body: serde_json::Value = resp.json().await.map_err(|e| {
            ExchangeError::InvalidResponse(format!("Failed to parse account response: {}", e))
        })?;


        // Positions are nested inside accounts[0].positions in the REST response
        let positions_val = if body["accounts"][0]["positions"].is_array() {
            &body["accounts"][0]["positions"]
        } else if body["positions"].is_array() {
            &body["positions"]
        } else {
            tracing::warn!(
                exchange = "lighter",
                "get_position: no 'positions' array found in response"
            );
            return Ok(None);
        };

        if let Some(positions) = positions_val.as_array() {
            for pos in positions {
                let pos_data: Result<LighterPositionData, _> =
                    serde_json::from_value::<LighterPositionData>(pos.clone());
                if let Ok(data) = pos_data {
                    // Match by market_id
                    if let Some(mid) = data.market_id {
                        if let Ok(expected_mid) = self.market_id_for(symbol) {
                            if mid != expected_mid {
                                continue;
                            }
                        }
                    }

                    let size = data
                        .position
                        .as_deref()
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(0.0);

                    if size.abs() < 1e-12 {
                        continue; // No position
                    }

                    // sign: 1 = long, -1 = short
                    let side = match data.sign {
                        Some(1) => "long".to_string(),
                        Some(-1) => "short".to_string(),
                        _ => {
                            if size > 0.0 {
                                "long".to_string()
                            } else {
                                "short".to_string()
                            }
                        }
                    };

                    let entry_price = data
                        .avg_entry_price
                        .as_deref()
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(0.0);

                    tracing::debug!(
                        exchange = "lighter",
                        symbol = %symbol,
                        position_size = %size,
                        side = %side,
                        entry_price = %entry_price,
                        "Parsed position from account endpoint"
                    );

                    return Ok(Some(PositionInfo {
                        symbol: symbol.to_string(),
                        quantity: size.abs(),
                        side,
                        entry_price,
                        mark_price: data
                            .liquidation_price
                            .as_deref()
                            .and_then(|s| s.parse::<f64>().ok()),
                        unrealized_pnl: data
                            .unrealized_pnl
                            .as_deref()
                            .and_then(|s| s.parse::<f64>().ok())
                            .unwrap_or(0.0),
                    }));
                }
            }
        }

        Ok(None) // No position found
    }

    async fn get_fill_info(
        &self,
        symbol: &str,
        order_id: &str,
    ) -> ExchangeResult<Option<FillInfo>> {
        // Query recent trades from REST API
        let account_index = self.signer.account_index();
        let market_id = self.market_id_for(symbol).unwrap_or(0);
        let is_pending = order_id == "pending";
        let order_index: i64 = if is_pending {
            -1
        } else {
            order_id.parse().unwrap_or(-1)
        };

        // Non-pending but unparseable order_id → nothing to look up
        if !is_pending && order_index < 0 {
            return Ok(None);
        }

        // Retry loop: WS fire-and-forget trades may take a moment to appear in REST API
        const MAX_ATTEMPTS: u32 = 3;
        const RETRY_DELAY_MS: u64 = 500;

        for attempt in 1..=MAX_ATTEMPTS {
            let url = format!(
                "{}/api/v1/trades?account_index={}&order_book_id={}&limit=5",
                self.config.rest_url(),
                account_index,
                market_id,
            );

            let resp = self.http.get(&url).send().await.map_err(|e| {
                ExchangeError::ConnectionFailed(format!("Failed to fetch trades: {}", e))
            })?;
            let body: serde_json::Value = resp.json().await.map_err(|e| {
                ExchangeError::InvalidResponse(format!("Failed to parse trades: {}", e))
            })?;

            if let Some(trades) = body["trades"].as_array() {
                if is_pending {
                    // No order_id to match — take the MOST RECENT trade for this account+market.
                    // Safe because get_fill_info is called immediately after close_position
                    // and the bot only holds one position at a time.
                    if let Some(trade) = trades.first() {
                        let fill_price = trade["price"]
                            .as_str()
                            .and_then(|s| s.parse::<f64>().ok())
                            .or_else(|| trade["price"].as_f64())
                            .unwrap_or(0.0);

                        if fill_price > 0.0 {
                            let fee = trade["taker_fee"]
                                .as_i64()
                                .map(|f| f as f64 / 1_000_000.0);

                            tracing::info!(
                                symbol = %symbol,
                                fill_price = %fill_price,
                                fee = ?fee,
                                attempt,
                                "Lighter: Retrieved fill price from most recent trade (pending order)"
                            );

                            return Ok(Some(FillInfo {
                                fill_price,
                                realized_pnl: None,
                                fee,
                            }));
                        }
                    }
                } else {
                    // Match by ask_id/bid_id (existing logic for known order IDs)
                    for trade in trades {
                        let ask_id = trade["ask_id"].as_i64().unwrap_or(-1);
                        let bid_id = trade["bid_id"].as_i64().unwrap_or(-1);

                        if ask_id == order_index || bid_id == order_index {
                            let fill_price = trade["price"]
                                .as_str()
                                .and_then(|s| s.parse::<f64>().ok())
                                .or_else(|| trade["price"].as_f64())
                                .unwrap_or(0.0);

                            let fee = trade["taker_fee"]
                                .as_i64()
                                .map(|f| f as f64 / 1_000_000.0);

                            return Ok(Some(FillInfo {
                                fill_price,
                                realized_pnl: None,
                                fee,
                            }));
                        }
                    }
                }
            }

            // Trade not found yet — retry after delay
            if attempt < MAX_ATTEMPTS {
                tracing::debug!(
                    symbol = %symbol,
                    order_id = %order_id,
                    attempt,
                    "Lighter: Trade not found yet, retrying..."
                );
                tokio::time::sleep(std::time::Duration::from_millis(RETRY_DELAY_MS)).await;
            }
        }

        tracing::warn!(
            symbol = %symbol,
            order_id = %order_id,
            attempts = MAX_ATTEMPTS,
            "Lighter: Fill info not found after all retries"
        );
        Ok(None)
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
