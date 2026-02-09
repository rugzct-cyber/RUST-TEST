//! Lighter Adapter Implementation
//!
//! Main LighterAdapter struct implementing ExchangeAdapter trait.
//! Uses Schnorr/Poseidon2/Goldilocks signing via vendored lighter-crypto crates.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use tokio::sync::RwLock;
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
    LighterPositionData, LighterSendTxResponse,
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
    /// WebSocket write half (for sending subscribe/unsubscribe)
    ws_sink: Option<WsSink>,
    /// Connection health tracking
    health: ConnectionHealth,
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
            ws_sink: None,
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

    /// Convert floating-point quantity to Lighter integer representation
    fn quantity_to_int(&self, symbol: &str, quantity: f64) -> i64 {
        if let Some(info) = self.market_info.get(symbol) {
            let factor = 10f64.powi(info.size_precision as i32);
            (quantity * factor).round() as i64
        } else {
            // Fallback: assume 3 decimal places
            (quantity * 1000.0).round() as i64
        }
    }

    /// Map our TimeInForce to the Lighter integer value
    fn tif_to_lighter(tif: &TimeInForce) -> u8 {
        match tif {
            TimeInForce::Ioc => 2, // IOC
            TimeInForce::Gtc => 1, // GoodTillTime
            TimeInForce::Fok => 3, // FillOrKill
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
                        // Check for orderbook updates
                        if let Some(channel) = val["channel"].as_str() {
                            if channel == "orderbookdepth" || channel == "orderbook" {
                                if let Some(data) = val.get("data") {
                                    let market_id = data["market_id"]
                                        .as_u64()
                                        .or_else(|| data["market_id"].as_str().and_then(|s| s.parse().ok()))
                                        .unwrap_or(0) as u8;

                                    if let Some(symbol) = id_to_symbol.get(&market_id) {
                                        // Parse bids and asks
                                        let parse_side = |key: &str| -> Vec<OrderbookLevel> {
                                            data[key]
                                                .as_array()
                                                .map(|arr| {
                                                    arr.iter()
                                                        .filter_map(|level| {
                                                            let price = level[0]
                                                                .as_str()
                                                                .and_then(|s| s.parse::<f64>().ok())
                                                                .or_else(|| level[0].as_f64())?;
                                                            let qty = level[1]
                                                                .as_str()
                                                                .and_then(|s| s.parse::<f64>().ok())
                                                                .or_else(|| level[1].as_f64())?;
                                                            Some(OrderbookLevel::new(price, qty))
                                                        })
                                                        .take(MAX_ORDERBOOK_DEPTH)
                                                        .collect()
                                                })
                                                .unwrap_or_default()
                                        };

                                        let ob = Orderbook {
                                            bids: parse_side("bids"),
                                            asks: parse_side("asks"),
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
                                        if let Some(ref n) = notify {
                                            n.notify_waiters();
                                        }
                                    }
                                }
                            }
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

    /// POST a signed transaction to the sendTx endpoint
    async fn send_tx(
        &self,
        tx_type: u32,
        tx_info_json: &str,
    ) -> ExchangeResult<LighterSendTxResponse> {
        let url = format!("{}/api/v1/sendTx", self.config.rest_url());

        let form = [
            ("tx_type", tx_type.to_string()),
            ("tx_info", tx_info_json.to_string()),
        ];

        let resp = self.http.post(&url).form(&form).send().await.map_err(|e| {
            ExchangeError::ConnectionFailed(format!("sendTx request failed: {}", e))
        })?;

        let status = resp.status();
        let body = resp.text().await.map_err(|e| {
            ExchangeError::InvalidResponse(format!("Failed to read sendTx response: {}", e))
        })?;

        if !status.is_success() {
            return Err(ExchangeError::OrderRejected(format!(
                "sendTx HTTP {} - {}",
                status, body
            )));
        }

        serde_json::from_str(&body).map_err(|e| {
            ExchangeError::InvalidResponse(format!(
                "Failed to parse sendTx response: {} (body: {})",
                e, body
            ))
        })
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

        // 3. Create auth token for WS
        let auth_token = self.signer.create_auth_token(AUTH_TOKEN_LIFETIME_SECS)?;

        // 4. Connect WebSocket
        let ws_url = self.config.ws_url();
        let (ws_stream, _) = connect_async(ws_url).await.map_err(|e| {
            ExchangeError::ConnectionFailed(format!("WebSocket connect failed: {}", e))
        })?;
        let (mut sink, reader) = ws_stream.split();

        // 5. Authenticate WS
        let auth_msg = serde_json::json!({
            "method": "auth",
            "params": {
                "token": auth_token,
            }
        });
        sink.send(Message::Text(auth_msg.to_string()))
            .await
            .map_err(|e| {
                ExchangeError::AuthenticationFailed(format!("WS auth send failed: {}", e))
            })?;

        self.ws_sink = Some(sink);

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
        if let Some(ref mut sink) = self.ws_sink {
            let _ = sink.close().await;
        }
        self.ws_sink = None;

        let mut state = self.health.state.write().await;
        *state = ConnectionState::Disconnected;
        tracing::info!(exchange = "lighter", "Disconnected");
        Ok(())
    }

    async fn subscribe_orderbook(&mut self, symbol: &str) -> ExchangeResult<()> {
        let market_id = self.market_id_for(symbol)?;

        // Send WS subscribe message
        let sub_id = next_subscription_id();
        let msg = serde_json::json!({
            "method": "subscribe",
            "params": {
                "channel": "orderbookdepth",
                "market_id": market_id.to_string(),
            },
            "id": sub_id,
        });

        if let Some(ref mut sink) = self.ws_sink {
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

            if let Some(ref mut sink) = self.ws_sink {
                let _ = sink.send(Message::Text(msg.to_string())).await;
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

        // Send to API
        let resp = self.send_tx(14, &tx_info_json).await;

        match resp {
            Ok(tx_resp) => {
                if let Some(err) = tx_resp.error {
                    // Check for nonce/signature errors — rollback nonce
                    if err.contains("nonce") || err.contains("signature") {
                        self.signer.rollback_nonce();
                    }
                    return Err(ExchangeError::OrderRejected(err));
                }

                let order_id = tx_resp
                    .order_index
                    .map(|i| i.to_string())
                    .unwrap_or_else(|| "unknown".to_string());

                Ok(OrderResponse {
                    order_id,
                    client_order_id: order.client_order_id,
                    status: OrderStatus::Pending, // Lighter doesn't instantly confirm fills
                    filled_quantity: 0.0,
                    avg_price: None,
                })
            }
            Err(e) => {
                self.signer.rollback_nonce();
                Err(e)
            }
        }
    }

    async fn cancel_order(&self, order_id: &str) -> ExchangeResult<()> {
        // order_id is the Lighter order index (numeric)
        let order_index: i64 = order_id.parse().map_err(|_| {
            ExchangeError::InvalidOrder(format!("Invalid order ID: {}", order_id))
        })?;

        // We need the market_id — for now use 0 if not known
        // In production, the caller should provide context or we look up from state
        let market_id: u8 = 0; // TODO: look up from open orders state

        let nonce = self.signer.next_nonce();

        let tx_info_json = self.signer.sign_cancel_order(market_id, order_index, nonce)?;

        let resp = self.send_tx(15, &tx_info_json).await;

        match resp {
            Ok(tx_resp) => {
                if let Some(err) = tx_resp.error {
                    if err.contains("nonce") || err.contains("signature") {
                        self.signer.rollback_nonce();
                    }
                    return Err(ExchangeError::OrderRejected(err));
                }
                Ok(())
            }
            Err(e) => {
                self.signer.rollback_nonce();
                Err(e)
            }
        }
    }

    fn get_orderbook(&self, symbol: &str) -> Option<&Orderbook> {
        self.local_orderbooks.get(symbol)
    }

    fn is_connected(&self) -> bool {
        // Fast check: if reader died, not connected
        if !self.health.reader_alive.load(Ordering::Relaxed) {
            return false;
        }
        // Check ws_sink presence
        self.ws_sink.is_some()
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
        let url = format!(
            "{}/api/v1/positions?account_index={}",
            self.config.rest_url(),
            self.signer.account_index()
        );

        let resp = self.http.get(&url).send().await.map_err(|e| {
            ExchangeError::ConnectionFailed(format!("Failed to fetch positions: {}", e))
        })?;
        let body: serde_json::Value = resp.json().await.map_err(|e| {
            ExchangeError::InvalidResponse(format!("Failed to parse positions response: {}", e))
        })?;

        // Find matching position
        let _base = normalize_symbol_to_lighter(symbol);
        if let Some(positions) = body["positions"].as_array() {
            for pos in positions {
                let pos_data: Result<LighterPositionData, _> =
                    serde_json::from_value::<LighterPositionData>(pos.clone());
                if let Ok(data) = pos_data {
                    // Match by market_id or symbol
                    if let Some(mid) = data.market_id {
                        if let Ok(expected_mid) = self.market_id_for(symbol) {
                            if mid != expected_mid {
                                continue;
                            }
                        }
                    }

                    let size = data
                        .size
                        .as_deref()
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(0.0);

                    if size.abs() < 1e-12 {
                        continue; // No position
                    }

                    let side = data
                        .side
                        .as_deref()
                        .map(|s| s.to_lowercase())
                        .unwrap_or_else(|| {
                            if size > 0.0 {
                                "long".to_string()
                            } else {
                                "short".to_string()
                            }
                        });

                    return Ok(Some(PositionInfo {
                        symbol: symbol.to_string(),
                        quantity: size.abs(),
                        side,
                        entry_price: data
                            .entry_price
                            .as_deref()
                            .and_then(|s| s.parse::<f64>().ok())
                            .unwrap_or(0.0),
                        mark_price: data
                            .mark_price
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
        _symbol: &str,
        _order_id: &str,
    ) -> ExchangeResult<Option<FillInfo>> {
        // TODO: Implement when we understand Lighter's fill/trade API
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
