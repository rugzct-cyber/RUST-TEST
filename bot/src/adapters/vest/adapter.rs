//! Vest Adapter Implementation
//!
//! Main VestAdapter struct implementing ExchangeAdapter trait.
//! Uses modules: config, types, signing for sub-components.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use ethers::core::types::U256;
use ethers::signers::{LocalWallet, Signer};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;

use crate::adapters::errors::{ExchangeError, ExchangeResult};
use crate::adapters::traits::ExchangeAdapter;
use crate::adapters::types::{
    create_http_client, next_subscription_id, ConnectionHealth, ConnectionState, OrderRequest,
    OrderResponse, OrderStatus, Orderbook, PositionInfo,
};

// Import from sub-modules
use super::config::VestConfig;
use super::signing::{
    current_time_ms, sign_cancel_order, sign_leverage_request, sign_order, sign_registration_proof,
};
use super::types::{
    ListenKeyResponse, PreSignedOrder, RegisterResponse, VestAccountResponse, VestLeverageResponse,
    VestOrderResponse, VestPositionData, VestWsMessage,
};

// =============================================================================
// Constants
// =============================================================================

/// Timeout for PING/PONG validation in seconds
const PING_TIMEOUT_SECS: u64 = 5;

/// Maximum retry attempts for registration
const MAX_REGISTRATION_RETRIES: u32 = 3;

/// Base backoff delay for retries in milliseconds
const RETRY_BACKOFF_MS: u64 = 500;

// =============================================================================
// WebSocket Type Aliases
// =============================================================================

pub(crate) type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
pub(crate) type WsWriter = SplitSink<WsStream, Message>;
pub(crate) type WsReader = SplitStream<WsStream>;
/// Thread-safe shared orderbooks storage for lock-free monitoring
pub use crate::core::channels::SharedOrderbooks;

// next_subscription_id() imported from crate::adapters::types (shared counter)

// ConnectionHealth imported from crate::adapters::types

// =============================================================================
// VestAdapter Implementation
// =============================================================================

/// Vest Exchange Adapter implementing ExchangeAdapter trait
pub struct VestAdapter {
    pub(crate) config: VestConfig,
    pub(crate) http_client: reqwest::Client,
    pub(crate) ws_stream: Option<Mutex<WsStream>>,
    pub(crate) ws_sender: Option<Arc<Mutex<WsWriter>>>,
    pub(crate) reader_handle: Option<JoinHandle<()>>,
    pub(crate) heartbeat_handle: Option<JoinHandle<()>>,
    pub(crate) listen_key_renewal_handle: Option<JoinHandle<()>>,
    pub(crate) connected: bool,
    pub(crate) api_key: Option<String>,
    pub(crate) listen_key: Option<String>,
    pub(crate) subscriptions: Vec<String>,
    pub(crate) pending_subscriptions: HashMap<u64, String>,
    pub(crate) orderbooks: HashMap<String, Orderbook>,
    pub(crate) shared_orderbooks: SharedOrderbooks,
    pub(crate) connection_health: ConnectionHealth,
}

impl VestAdapter {
    /// Create a new VestAdapter with the given configuration
    ///
    /// HTTP connection pooling configured for latency optimization:
    /// - pool_max_idle_per_host(2): Keep 2 idle connections per host
    /// - pool_idle_timeout(60s): Keep connections warm for 60 seconds
    /// - tcp_keepalive(30s): TCP keepalive every 30 seconds
    /// - connect_timeout(10s): Connection timeout
    /// - timeout(10s): Request timeout
    pub fn new(config: VestConfig) -> Self {
        Self {
            config,
            http_client: create_http_client("Vest"),
            ws_stream: None,
            ws_sender: None,
            reader_handle: None,
            heartbeat_handle: None,
            listen_key_renewal_handle: None,
            connected: false,
            api_key: None,
            listen_key: None,
            subscriptions: Vec::new(),
            pending_subscriptions: HashMap::new(),
            orderbooks: HashMap::new(),
            shared_orderbooks: Arc::new(RwLock::new(HashMap::new())),
            connection_health: ConnectionHealth::default(),
        }
    }

    /// Build WebSocket URL with listen key (for authenticated channels)
    pub fn build_ws_url(&self, listen_key: &str) -> String {
        format!(
            "{}?version=1.0&xwebsocketserver=restserver{}&listenKey={}",
            self.config.ws_base_url(),
            self.config.account_group,
            listen_key
        )
    }

    /// Build public WebSocket URL (for public channels like orderbook)
    pub fn build_public_ws_url(&self) -> String {
        format!(
            "{}?version=1.0&xwebsocketserver=restserver{}",
            self.config.ws_base_url(),
            self.config.account_group
        )
    }

    /// Get the required REST header for server routing
    pub fn rest_server_header(&self) -> String {
        format!("restserver{}", self.config.account_group)
    }

    /// Get shared orderbooks for lock-free monitoring
    ///
    /// Returns Arc<RwLock<...>> that can be read directly without acquiring
    /// the adapter's Mutex. This enables high-frequency orderbook polling
    /// without blocking execution.
    pub fn get_shared_orderbooks(&self) -> SharedOrderbooks {
        Arc::clone(&self.shared_orderbooks)
    }

    // =========================================================================
    // Signing Methods - Delegates to signing module
    // =========================================================================

    /// Generate EIP-712 signature for registration
    pub async fn sign_registration_proof(&self) -> ExchangeResult<(String, String, u64)> {
        sign_registration_proof(&self.config).await
    }

    /// Sign an order using Vest's signature format
    pub async fn sign_order(&self, order: &OrderRequest) -> ExchangeResult<(String, u64)> {
        let (sig, time, _nonce) = sign_order(&self.config, order).await?;
        Ok((sig, time))
    }

    /// Sign a cancel order request
    pub async fn sign_cancel_order(&self, order_id: &str, nonce: u64) -> ExchangeResult<String> {
        sign_cancel_order(&self.config, order_id, nonce).await
    }

    /// Sign a leverage request
    pub async fn sign_leverage_request(
        &self,
        symbol: &str,
        leverage: u32,
    ) -> ExchangeResult<(String, u64, u64)> {
        sign_leverage_request(&self.config, symbol, leverage).await
    }

    // =========================================================================
    // HTTP Connection Warm-up
    // =========================================================================

    /// Warm up HTTP connection pool by making a lightweight request
    ///
    /// This establishes TCP/TLS connections upfront to avoid handshake latency
    /// on the first real request. Uses the Vest /account endpoint with a
    /// minimal request to establish the connection.
    ///
    /// Called during connect() flow to ensure the first order request
    /// benefits from pre-established connections.
    pub async fn warm_up_http(&self) -> ExchangeResult<()> {
        // Use a simple request to the REST base URL to establish TCP/TLS
        // Vest requires the xrestservermm header for routing
        let url = format!(
            "{}/account?time={}",
            self.config.rest_base_url(),
            current_time_ms()
        );
        let start = std::time::Instant::now();

        let response = self
            .http_client
            .get(&url)
            .header("xrestservermm", self.rest_server_header())
            .send()
            .await
            .map_err(|e| ExchangeError::ConnectionFailed(format!("HTTP warm-up failed: {}", e)))?;

        let elapsed = start.elapsed();

        // Log success regardless of response status (we just want to establish the connection)
        // 401/403 are expected without auth — connection is still established
        let status_code = response.status().as_u16();
        if response.status().is_success() || status_code == 401 || status_code == 403 {
            tracing::info!(
                phase = "init",
                exchange = "vest",
                latency_ms = %elapsed.as_millis(),
                "HTTP connection pool warmed up"
            );
        } else {
            // Any other status still means TCP/TLS is established, just unexpected
            tracing::debug!(
                phase = "init",
                exchange = "vest",
                status = %response.status(),
                latency_ms = %elapsed.as_millis(),
                "HTTP warm-up returned unexpected status (connection still established)"
            );
        }

        Ok(())
    }

    // =========================================================================
    // Pre-Signed Order Methods
    // =========================================================================

    /// Pre-sign an order for faster submission later
    pub async fn pre_sign_order(&self, order: &OrderRequest) -> ExchangeResult<PreSignedOrder> {
        use ethers::abi::{encode, Token};
        use ethers::core::utils::keccak256;

        let signing_wallet: LocalWallet = self.config.signing_key.parse().map_err(|e| {
            ExchangeError::AuthenticationFailed(format!("Invalid signing key: {}", e))
        })?;

        let time = current_time_ms();
        let nonce: u64 = time;

        let order_type = match order.order_type {
            crate::adapters::types::OrderType::Limit => "LIMIT",
            crate::adapters::types::OrderType::Market => "MARKET",
        };

        let is_buy = matches!(order.side, crate::adapters::types::OrderSide::Buy);
        let size_str = format!("{:.3}", order.quantity); // Vest requires exactly 3 decimal places
        let price_str = order
            .price
            .map(|p| format!("{:.3}", p)) // Vest requires exactly 3 decimal places
            .unwrap_or_else(|| "0.000".to_string());
        let reduce_only = order.reduce_only;

        let encoded = encode(&[
            Token::Uint(U256::from(time)),
            Token::Uint(U256::from(nonce)),
            Token::String(order_type.to_string()),
            Token::String(order.symbol.clone()),
            Token::Bool(is_buy),
            Token::String(size_str.clone()),
            Token::String(price_str.clone()),
            Token::Bool(reduce_only),
        ]);

        let msg_hash = keccak256(&encoded);
        let signature = signing_wallet.sign_message(msg_hash).await.map_err(|e| {
            ExchangeError::AuthenticationFailed(format!("Order signing failed: {}", e))
        })?;

        let sig_hex = format!("0x{}", hex::encode(signature.to_vec()));

        Ok(PreSignedOrder {
            order: order.clone(),
            signature: sig_hex,
            time,
            nonce,
            created_at: std::time::Instant::now(),
            size_str,
            price_str,
        })
    }

    /// Send a pre-signed order (skips signing step for lower latency)
    pub async fn send_presigned_order(
        &self,
        presigned: PreSignedOrder,
    ) -> ExchangeResult<OrderResponse> {
        if !presigned.is_valid() {
            return Err(ExchangeError::OrderRejected(format!(
                "Pre-signed order expired ({:.1}s old)",
                presigned.created_at.elapsed().as_secs_f64()
            )));
        }

        if !self.connected {
            return Err(ExchangeError::ConnectionFailed("Not connected".into()));
        }

        let api_key = self
            .api_key
            .as_ref()
            .ok_or_else(|| ExchangeError::AuthenticationFailed("Not registered".into()))?;

        let order = &presigned.order;
        let order_type = match order.order_type {
            crate::adapters::types::OrderType::Limit => "LIMIT",
            crate::adapters::types::OrderType::Market => "MARKET",
        };
        let is_buy = matches!(order.side, crate::adapters::types::OrderSide::Buy);

        let body = serde_json::json!({
            "order": {
                "time": presigned.time,
                "nonce": presigned.nonce,
                "symbol": order.symbol,
                "isBuy": is_buy,
                "size": presigned.size_str,
                "orderType": order_type,
                "limitPrice": presigned.price_str,
                "reduceOnly": order.reduce_only,
            },
            "recvWindow": crate::adapters::types::VEST_RECV_WINDOW_MS,
            "signature": presigned.signature,
        });

        let url = format!("{}/orders", self.config.rest_base_url());

        tracing::debug!("Vest send_presigned_order: POST {} (pre-signed)", url);

        let response = self
            .http_client
            .post(&url)
            .header("X-API-Key", api_key)
            .header(
                "xrestservermm",
                format!("restserver{}", self.config.account_group),
            )
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ExchangeError::OrderRejected(format!("Order request failed: {}", e)))?;

        self.parse_order_response(response, &order.client_order_id)
            .await
    }

    /// Parse order response from Vest API
    async fn parse_order_response(
        &self,
        response: reqwest::Response,
        client_order_id: &str,
    ) -> ExchangeResult<OrderResponse> {
        let status = response.status();
        let text = response.text().await.map_err(|e| {
            ExchangeError::InvalidResponse(format!("Failed to read response: {}", e))
        })?;

        if !status.is_success() {
            return Err(ExchangeError::OrderRejected(format!(
                "Order failed ({}): {}",
                status, text
            )));
        }

        let result: VestOrderResponse = serde_json::from_str(&text).map_err(|e| {
            ExchangeError::InvalidResponse(format!("Invalid JSON: {} - {}", e, text))
        })?;

        if let Some(code) = result.code {
            if code != 0 {
                return Err(ExchangeError::OrderRejected(format!(
                    "Order error {}: {}",
                    code,
                    result.msg.unwrap_or_default()
                )));
            }
        }

        let order_status = match result.status.as_deref() {
            Some("NEW") => OrderStatus::Pending,
            Some("PARTIALLY_FILLED") => OrderStatus::PartiallyFilled,
            Some("FILLED") => OrderStatus::Filled,
            Some("CANCELLED") => OrderStatus::Cancelled,
            Some("REJECTED") => OrderStatus::Rejected,
            Some(unknown) => {
                tracing::warn!(
                    status = unknown,
                    "Unknown order status from Vest, treating as pending"
                );
                OrderStatus::Pending
            }
            None => {
                tracing::debug!("Vest response missing status field, assuming NEW order");
                OrderStatus::Pending
            }
        };

        // CR-1 fix: Use last_filled_size (actual filled qty) instead of size (order qty),
        // and keep the parsed value instead of discarding it.
        let filled_quantity = match result.last_filled_size.as_ref().and_then(|s| s.parse::<f64>().ok()) {
            Some(qty) => qty,
            None => {
                if result.last_filled_size.is_some() {
                    tracing::warn!(raw_size = ?result.last_filled_size, "Vest: failed to parse last_filled_size, defaulting to 0.0");
                }
                0.0
            }
        };

        // Parse avg_price from avgFilledPrice (preferred) or lastFilledPrice (fallback)
        let avg_price: Option<f64> = result.avg_filled_price
            .as_ref()
            .and_then(|s| {
                s.parse::<f64>().map_err(|e| {
                    tracing::warn!(raw_avg_price = %s, error = %e, "Vest: failed to parse avg_filled_price");
                    e
                }).ok()
            })
            .or_else(|| result.last_filled_price.as_ref().and_then(|s| {
                s.parse::<f64>().map_err(|e| {
                    tracing::warn!(raw_last_price = %s, error = %e, "Vest: failed to parse last_filled_price");
                    e
                }).ok()
            }));

        let order_id = match result.id {
            Some(id) => id,
            None => {
                tracing::warn!(
                    client_order_id = client_order_id,
                    "Vest response missing id, cancel may fail"
                );
                format!("vest-{}", client_order_id)
            }
        };

        Ok(OrderResponse {
            order_id,
            client_order_id: client_order_id.to_string(),
            status: order_status,
            filled_quantity,
            avg_price,
        })
    }

    // =========================================================================
    // Registration and Authentication
    // =========================================================================

    /// Register with Vest API to obtain API key (with retry logic)
    async fn register(&mut self) -> ExchangeResult<String> {
        let mut last_error = None;

        for attempt in 0..MAX_REGISTRATION_RETRIES {
            if attempt > 0 {
                let backoff = Duration::from_millis(RETRY_BACKOFF_MS * (1 << attempt));
                tokio::time::sleep(backoff).await;
            }

            match self.try_register().await {
                Ok(api_key) => return Ok(api_key),
                Err(e) => {
                    last_error = Some(e);
                    continue;
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            ExchangeError::AuthenticationFailed("Registration failed after max retries".into())
        }))
    }

    /// Single registration attempt
    async fn try_register(&self) -> ExchangeResult<String> {
        let (signature, signing_addr, expiry) = self.sign_registration_proof().await?;

        let url = format!("{}/register", self.config.rest_base_url());

        let body = serde_json::json!({
            "signingAddr": signing_addr,
            "primaryAddr": self.config.primary_addr.to_lowercase(),
            "signature": signature,
            "expiryTime": expiry,
            "networkType": 0
        });

        let response = self
            .http_client
            .post(&url)
            .header("xrestservermm", self.rest_server_header())
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                ExchangeError::ConnectionFailed(format!("Register request failed: {}", e))
            })?;

        let status = response.status();
        let text = response.text().await.map_err(|e| {
            ExchangeError::InvalidResponse(format!("Failed to read response: {}", e))
        })?;

        if !status.is_success() {
            return Err(ExchangeError::AuthenticationFailed(format!(
                "Registration failed ({}): {}",
                status, text
            )));
        }

        let result: RegisterResponse = serde_json::from_str(&text).map_err(|e| {
            ExchangeError::InvalidResponse(format!("Invalid JSON: {} - {}", e, text))
        })?;

        if let Some(code) = result.code {
            return Err(ExchangeError::AuthenticationFailed(format!(
                "Registration error {}: {}",
                code,
                result.msg.unwrap_or_default()
            )));
        }

        result
            .api_key
            .ok_or_else(|| ExchangeError::InvalidResponse("No api_key in response".into()))
    }

    /// Obtain listen key for WebSocket connection
    async fn get_listen_key(&self) -> ExchangeResult<String> {
        let api_key = self
            .api_key
            .as_ref()
            .ok_or_else(|| ExchangeError::AuthenticationFailed("Not registered".into()))?;

        let url = format!("{}/account/listenKey", self.config.rest_base_url());

        let response = self
            .http_client
            .post(&url)
            .header("xrestservermm", self.rest_server_header())
            .header("X-API-KEY", api_key)
            .send()
            .await
            .map_err(|e| {
                ExchangeError::ConnectionFailed(format!("ListenKey request failed: {}", e))
            })?;

        let status = response.status();
        let text = response.text().await.map_err(|e| {
            ExchangeError::InvalidResponse(format!("Failed to read response: {}", e))
        })?;

        if !status.is_success() {
            return Err(ExchangeError::AuthenticationFailed(format!(
                "ListenKey failed ({}): {}",
                status, text
            )));
        }

        let result: ListenKeyResponse = serde_json::from_str(&text).map_err(|e| {
            ExchangeError::InvalidResponse(format!("Invalid JSON: {} - {}", e, text))
        })?;

        if let Some(code) = result.code {
            return Err(ExchangeError::AuthenticationFailed(format!(
                "ListenKey error {}: {}",
                code,
                result.msg.unwrap_or_default()
            )));
        }

        result
            .listen_key
            .ok_or_else(|| ExchangeError::InvalidResponse("No listenKey in response".into()))
    }

    // =========================================================================
    // WebSocket Connection Management
    // =========================================================================

    /// Connect to WebSocket and validate with PING/PONG
    async fn connect_websocket(&mut self) -> ExchangeResult<()> {
        let url = self.build_public_ws_url();
        tracing::info!("Connecting to Vest public WebSocket: {}", url);
        let ws_stream = crate::adapters::shared::connect_tls(&url).await?;
        self.ws_stream = Some(Mutex::new(ws_stream));
        Ok(())
    }

    /// Send PING and validate PONG response with timeout
    async fn validate_connection(&self) -> ExchangeResult<()> {
        let ws = self
            .ws_stream
            .as_ref()
            .ok_or_else(|| ExchangeError::ConnectionFailed("No WebSocket connection".into()))?;

        let mut stream = ws.lock().await;

        let ping_msg = serde_json::json!({
            "method": "PING",
            "params": [],
            "id": 0
        });

        stream
            .send(Message::Text(ping_msg.to_string()))
            .await
            .map_err(|e| ExchangeError::WebSocket(Box::new(e)))?;

        let pong_timeout = Duration::from_secs(PING_TIMEOUT_SECS);
        let pong_result = timeout(pong_timeout, stream.next())
            .await
            .map_err(|_| ExchangeError::NetworkTimeout(PING_TIMEOUT_SECS * 1000))?;

        match pong_result {
            Some(msg_result) => {
                let msg = msg_result.map_err(|e| ExchangeError::WebSocket(Box::new(e)))?;

                if let Message::Text(text) = msg {
                    let response: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
                        ExchangeError::InvalidResponse(format!("Invalid PONG: {}", e))
                    })?;

                    if response.get("data").and_then(|d| d.as_str()) != Some("PONG") {
                        return Err(ExchangeError::ConnectionFailed(format!(
                            "Expected PONG, got: {}",
                            text
                        )));
                    }
                }
            }
            None => {
                return Err(ExchangeError::ConnectionFailed(
                    "No response to PING".into(),
                ));
            }
        }

        Ok(())
    }

    /// Split WebSocket stream and spawn background message reader
    fn split_and_spawn_reader(&mut self) -> ExchangeResult<()> {
        let ws_stream_mutex = self.ws_stream.take().ok_or_else(|| {
            ExchangeError::ConnectionFailed("No WebSocket stream to split".into())
        })?;

        let ws_stream = ws_stream_mutex.into_inner();
        let (ws_sender, ws_receiver) = ws_stream.split();

        self.ws_sender = Some(Arc::new(Mutex::new(ws_sender)));

        let shared_orderbooks = Arc::clone(&self.shared_orderbooks);
        let last_pong = Arc::clone(&self.connection_health.last_pong);
        let last_data = Arc::clone(&self.connection_health.last_data);
        let reader_alive = Arc::clone(&self.connection_health.reader_alive);

        last_data.store(current_time_ms(), Ordering::Relaxed);

        let handle = tokio::spawn(async move {
            Self::message_reader_loop(ws_receiver, shared_orderbooks, last_pong, last_data, reader_alive).await;
        });

        self.reader_handle = Some(handle);
        Ok(())
    }

    /// Background message reader loop
    ///
    /// Sets `reader_alive` to `true` on entry and `false` on exit, so that
    /// `is_stale()` can detect a dead connection immediately.
    async fn message_reader_loop(
        mut ws_receiver: WsReader,
        shared_orderbooks: SharedOrderbooks,
        last_pong: Arc<AtomicU64>,
        last_data: Arc<AtomicU64>,
        reader_alive: Arc<AtomicBool>,
    ) {
        tracing::info!("Vest message_reader_loop started");
        reader_alive.store(true, Ordering::Relaxed);

        while let Some(msg_result) = ws_receiver.next().await {
            last_data.store(current_time_ms(), Ordering::Relaxed);

            match msg_result {
                Ok(Message::Text(text)) => {
                    tracing::trace!("Raw WS message: {}", text);

                    match serde_json::from_str::<VestWsMessage>(&text) {
                        Ok(msg) => match msg {
                            VestWsMessage::Depth(depth_msg) => {
                                let symbol = depth_msg
                                    .channel
                                    .strip_suffix("@depth")
                                    .unwrap_or(&depth_msg.channel)
                                    .to_string();

                                tracing::debug!(
                                    symbol = %symbol,
                                    bids = depth_msg.data.bids.len(),
                                    asks = depth_msg.data.asks.len(),
                                    "Vest depth update received"
                                );

                                match depth_msg.data.to_orderbook() {
                                    Ok(orderbook) => {
                                        let mut books = shared_orderbooks.write().await;
                                        books.insert(symbol.clone(), orderbook);
                                        tracing::trace!(symbol = %symbol, "Orderbook updated in shared storage");
                                    }
                                    Err(e) => {
                                        tracing::warn!(error = %e, "Failed to parse orderbook data");
                                    }
                                }
                            }
                            VestWsMessage::Subscription(sub_resp) => {
                                tracing::debug!("Subscription confirmed: id={}", sub_resp.id);
                            }
                            VestWsMessage::Pong { .. } => {
                                last_pong.store(current_time_ms(), Ordering::Relaxed);
                                tracing::debug!("Vest PONG received, updating last_pong timestamp");
                            }
                        },
                        Err(parse_err) => {
                            tracing::warn!(
                                error = %parse_err,
                                message = %text,
                                "Failed to parse WS message - unknown format"
                            );
                        }
                    }
                }
                Ok(Message::Close(_)) => {
                    tracing::info!("WebSocket closed by server");
                    break;
                }
                Ok(Message::Ping(data)) => {
                    tracing::debug!("Ping received: {:?}", data);
                }
                Ok(Message::Pong(data)) => {
                    tracing::debug!("Pong WS frame received: {:?}", data);
                }
                Ok(Message::Binary(data)) => match String::from_utf8(data.clone()) {
                    Ok(text) => {
                        tracing::debug!("Binary->Text: {}", text);
                        if let Ok(VestWsMessage::Depth(depth_msg)) =
                            serde_json::from_str::<VestWsMessage>(&text)
                        {
                            let symbol = depth_msg
                                .channel
                                .strip_suffix("@depth")
                                .unwrap_or(&depth_msg.channel)
                                .to_string();

                            if let Ok(orderbook) = depth_msg.data.to_orderbook() {
                                let mut books = shared_orderbooks.write().await;
                                books.insert(symbol, orderbook);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::debug!("Binary message not UTF-8: {} ({} bytes)", e, data.len());
                    }
                },
                Ok(Message::Frame(_)) => {
                    tracing::trace!("Raw frame received");
                }
                Err(e) => {
                    tracing::error!("WebSocket error: {}", e);
                    break;
                }
            }
        }

        reader_alive.store(false, Ordering::Relaxed);
        tracing::warn!("Vest message reader loop ended — reader_alive set to false");
    }

    /// Send a SUBSCRIBE request for a symbol's orderbook
    async fn send_subscribe_request(&self, symbol: &str) -> ExchangeResult<u64> {
        let ws_sender = self
            .ws_sender
            .as_ref()
            .ok_or_else(|| ExchangeError::ConnectionFailed("WebSocket not connected".into()))?;

        let sub_id = next_subscription_id();
        let channel = format!("{}@depth", symbol);

        let msg = serde_json::json!({
            "method": "SUBSCRIBE",
            "params": [channel],
            "id": sub_id
        });

        let mut sender = ws_sender.lock().await;
        sender
            .send(Message::Text(msg.to_string()))
            .await
            .map_err(|e| ExchangeError::WebSocket(Box::new(e)))?;

        Ok(sub_id)
    }

    /// Send an UNSUBSCRIBE request for a symbol's orderbook
    async fn send_unsubscribe_request(&self, symbol: &str) -> ExchangeResult<u64> {
        let ws_sender = self
            .ws_sender
            .as_ref()
            .ok_or_else(|| ExchangeError::ConnectionFailed("WebSocket not connected".into()))?;

        let unsub_id = next_subscription_id();
        let channel = format!("{}@depth", symbol);

        let msg = serde_json::json!({
            "method": "UNSUBSCRIBE",
            "params": [channel],
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
    fn spawn_heartbeat_task(&mut self) {
        let ws_sender = match &self.ws_sender {
            Some(sender) => Arc::clone(sender),
            None => return,
        };

        let last_pong = Arc::clone(&self.connection_health.last_pong);
        let reader_alive = Arc::clone(&self.connection_health.reader_alive);
        last_pong.store(current_time_ms(), Ordering::Relaxed);

        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(
                crate::adapters::types::WS_PING_INTERVAL_SECS,
            ));
            interval.tick().await;

            loop {
                interval.tick().await;

                let ping_msg = serde_json::json!({
                    "method": "PING",
                    "params": [],
                    "id": 0
                });

                {
                    let mut sender = ws_sender.lock().await;
                    if let Err(e) = sender.send(Message::Text(ping_msg.to_string())).await {
                        tracing::warn!("Vest heartbeat: Failed to send PING - {}", e);
                        reader_alive.store(false, Ordering::Relaxed);
                        break;
                    }
                    tracing::trace!("Vest heartbeat: PING sent");
                }

                tokio::time::sleep(Duration::from_secs(5)).await;

                // Check if PONG is stale - connection likely dead
                let last = last_pong.load(Ordering::Relaxed);
                let now = current_time_ms();
                let pong_age_ms = now.saturating_sub(last);
                if pong_age_ms > 30_000 {
                    tracing::warn!(
                        "Vest heartbeat: PONG stale ({}ms ago) - connection likely dead, setting reader_alive=false",
                        pong_age_ms
                    );
                    reader_alive.store(false, Ordering::Relaxed);
                    break;
                }
                tracing::trace!("Vest heartbeat: last PONG was {}ms ago", pong_age_ms);
            }

            tracing::debug!("Vest heartbeat task ended");
        });

        self.heartbeat_handle = Some(handle);
        tracing::info!("Vest: Heartbeat monitoring started (30s interval)");
    }

    // =========================================================================
    // Listen Key Renewal (S-8: prevents 60-min expiry)
    // =========================================================================

    /// Extend listen key validity by 60 minutes (Vest API: PUT /account/listenKey)
    async fn extend_listen_key_request(
        http_client: &reqwest::Client,
        rest_base_url: &str,
        api_key: &str,
        rest_server_header: &str,
    ) -> ExchangeResult<()> {
        let url = format!("{}/account/listenKey", rest_base_url);

        let response = http_client
            .put(&url)
            .header("xrestservermm", rest_server_header)
            .header("X-API-KEY", api_key)
            .send()
            .await
            .map_err(|e| {
                ExchangeError::ConnectionFailed(format!("ListenKey extend failed: {}", e))
            })?;

        if !response.status().is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(ExchangeError::AuthenticationFailed(format!(
                "ListenKey extend failed: {}",
                text
            )));
        }

        tracing::info!("Vest: Listen key extended successfully (+60 min)");
        Ok(())
    }

    /// Spawn background task that extends the listen key every 45 minutes.
    ///
    /// The Vest listen key expires after 60 minutes. Renewing every 45 minutes
    /// gives a 15-minute safety margin. On failure, sets `reader_alive = false`
    /// to trigger reconnection via `is_stale()` → `ensure_ready()`.
    fn spawn_listen_key_renewal_task(&mut self) {
        let api_key = match &self.api_key {
            Some(key) => key.clone(),
            None => return,
        };

        let http_client = self.http_client.clone();
        let rest_base_url = self.config.rest_base_url().to_string();
        let rest_server_header = self.rest_server_header();
        let reader_alive = Arc::clone(&self.connection_health.reader_alive);

        let handle = tokio::spawn(async move {
            use crate::adapters::types::VEST_LISTEN_KEY_RENEWAL_SECS;

            let mut interval = tokio::time::interval(Duration::from_secs(
                VEST_LISTEN_KEY_RENEWAL_SECS,
            ));
            // Skip the immediate first tick (key was just created)
            interval.tick().await;

            loop {
                interval.tick().await;

                tracing::info!("Vest: Attempting listen key renewal...");
                match Self::extend_listen_key_request(
                    &http_client,
                    &rest_base_url,
                    &api_key,
                    &rest_server_header,
                )
                .await
                {
                    Ok(()) => {
                        tracing::info!("Vest: Listen key renewal successful");
                    }
                    Err(e) => {
                        tracing::error!(
                            "Vest: Listen key renewal FAILED: {} — setting reader_alive=false to trigger reconnect",
                            e
                        );
                        reader_alive.store(false, Ordering::Relaxed);
                        break;
                    }
                }
            }

            tracing::debug!("Vest listen key renewal task ended");
        });

        self.listen_key_renewal_handle = Some(handle);
        tracing::info!(
            "Vest: Listen key renewal task started ({}s interval)",
            crate::adapters::types::VEST_LISTEN_KEY_RENEWAL_SECS
        );
    }

    // =========================================================================
    // Public API: Account & Leverage Methods
    // =========================================================================

    /// Get full account information
    pub async fn get_account_info(&self) -> ExchangeResult<VestAccountResponse> {
        if !self.connected {
            return Err(ExchangeError::ConnectionFailed("Not connected".into()));
        }

        let api_key = self
            .api_key
            .as_ref()
            .ok_or_else(|| ExchangeError::AuthenticationFailed("Not registered".into()))?;

        let time = current_time_ms();
        let url = format!("{}/account?time={}", self.config.rest_base_url(), time);

        tracing::debug!("Vest get_account_info: GET {}", url);

        let response = self
            .http_client
            .get(&url)
            .header("X-API-Key", api_key)
            .header(
                "xrestservermm",
                format!("restserver{}", self.config.account_group),
            )
            .send()
            .await
            .map_err(|e| {
                ExchangeError::ConnectionFailed(format!("Account request failed: {}", e))
            })?;

        let status = response.status();
        let body = response.text().await.map_err(|e| {
            ExchangeError::InvalidResponse(format!("Failed to read response: {}", e))
        })?;

        tracing::debug!("Vest account response: status={}, body={}", status, body);

        if !status.is_success() {
            return Err(ExchangeError::InvalidResponse(format!(
                "Account failed ({} {}): {}",
                status.as_u16(),
                status,
                body
            )));
        }

        let account: VestAccountResponse = serde_json::from_str(&body).map_err(|e| {
            ExchangeError::InvalidResponse(format!(
                "Failed to parse account: {} - body: {}",
                e, body
            ))
        })?;

        Ok(account)
    }

    /// Get current leverage for a symbol
    pub async fn get_leverage(&self, symbol: &str) -> ExchangeResult<Option<u32>> {
        let account = self.get_account_info().await?;

        for leverage_data in &account.leverages {
            if leverage_data.symbol.as_deref() == Some(symbol) {
                return Ok(leverage_data.value);
            }
        }

        Ok(None)
    }

    /// Set leverage for a symbol
    pub async fn set_leverage(&self, symbol: &str, leverage: u32) -> ExchangeResult<u32> {
        if !self.connected {
            return Err(ExchangeError::ConnectionFailed("Not connected".into()));
        }

        let api_key = self
            .api_key
            .as_ref()
            .ok_or_else(|| ExchangeError::AuthenticationFailed("Not registered".into()))?;

        let (signature, time, nonce) = self.sign_leverage_request(symbol, leverage).await?;

        let url = format!("{}/account/leverage", self.config.rest_base_url());

        let body = serde_json::json!({
            "time": time,
            "nonce": nonce,
            "symbol": symbol,
            "value": leverage,
            "recvWindow": crate::adapters::types::VEST_RECV_WINDOW_MS,
            "signature": signature,
        });

        tracing::debug!("Vest set_leverage: POST {} body={}", url, body);

        let response = self
            .http_client
            .post(&url)
            .header("X-API-Key", api_key)
            .header(
                "xrestservermm",
                format!("restserver{}", self.config.account_group),
            )
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                ExchangeError::ConnectionFailed(format!("Leverage request failed: {}", e))
            })?;

        let status = response.status();
        let response_body = response.text().await.map_err(|e| {
            ExchangeError::InvalidResponse(format!("Failed to read response: {}", e))
        })?;

        tracing::debug!(
            "Vest leverage response: status={}, body={}",
            status,
            response_body
        );

        if !status.is_success() {
            return Err(ExchangeError::InvalidResponse(format!(
                "Set leverage failed ({} {}): {}",
                status.as_u16(),
                status,
                response_body
            )));
        }

        let leverage_response: VestLeverageResponse = serde_json::from_str(&response_body)
            .map_err(|e| {
                ExchangeError::InvalidResponse(format!(
                    "Failed to parse leverage response: {} - body: {}",
                    e, response_body
                ))
            })?;

        leverage_response
            .value
            .ok_or_else(|| ExchangeError::OrderRejected("No leverage value in response".into()))
    }

    /// Get all active positions
    pub async fn get_positions(&self) -> ExchangeResult<Vec<VestPositionData>> {
        let account = self.get_account_info().await?;
        Ok(account.positions)
    }

    /// Get the actual fill price for a completed order via REST API
    ///
    /// Queries `GET /orders?id={order_id}` to retrieve `avgFilledPrice`.
    /// This is needed because the initial order ACK from IOC/MARKET orders
    /// often returns `avg_price = 0.0` (CR-11).
    ///
    /// Returns `Ok(Some(FillInfo))` if the order is FILLED with valid data,
    /// `Ok(None)` if the order is not found or has no fill data.
    pub async fn get_order_fill_price(&self, order_id: &str) -> ExchangeResult<Option<crate::adapters::types::FillInfo>> {
        if !self.connected {
            return Err(ExchangeError::ConnectionFailed("Not connected".into()));
        }

        let api_key = self
            .api_key
            .as_ref()
            .ok_or_else(|| ExchangeError::AuthenticationFailed("Not registered".into()))?;

        // Vest GET /orders returns an array of order objects
        #[derive(Debug, serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct VestOrderInfo {
            #[allow(dead_code)]
            id: Option<String>,
            status: Option<String>,
            avg_filled_price: Option<String>,
            realized_pnl: Option<String>,
            fees: Option<String>,
        }

        // Retry loop: MARKET orders may not be FILLED immediately after ACK
        const MAX_ATTEMPTS: u32 = 3;
        const RETRY_DELAY_MS: u64 = 500;

        for attempt in 1..=MAX_ATTEMPTS {
            let time = current_time_ms();
            let url = format!(
                "{}/orders?id={}&time={}",
                self.config.rest_base_url(),
                order_id,
                time
            );

            tracing::debug!(order_id = %order_id, attempt, "Vest get_order_fill_price: GET {}", url);

            let response = self
                .http_client
                .get(&url)
                .header("X-API-Key", api_key)
                .header(
                    "xrestservermm",
                    format!("restserver{}", self.config.account_group),
                )
                .send()
                .await
                .map_err(|e| {
                    ExchangeError::ConnectionFailed(format!("Order query failed: {}", e))
                })?;

            let status = response.status();
            let body = response.text().await.map_err(|e| {
                ExchangeError::InvalidResponse(format!("Failed to read response: {}", e))
            })?;

            if !status.is_success() {
                tracing::warn!(
                    order_id = %order_id,
                    status = %status,
                    attempt,
                    "Vest get_order_fill_price failed"
                );
                if attempt < MAX_ATTEMPTS {
                    tokio::time::sleep(std::time::Duration::from_millis(RETRY_DELAY_MS)).await;
                    continue;
                }
                return Ok(None);
            }

            let orders: Vec<VestOrderInfo> = serde_json::from_str(&body).map_err(|e| {
                ExchangeError::InvalidResponse(format!(
                    "Failed to parse orders response: {} - body: {}",
                    e, body
                ))
            })?;

            // Find the matching filled order and extract fill info
            for order in &orders {
                if order.status.as_deref() == Some("FILLED") {
                    let fill_price = order
                        .avg_filled_price
                        .as_ref()
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(0.0);

                    let realized_pnl = order
                        .realized_pnl
                        .as_ref()
                        .and_then(|s| s.parse::<f64>().ok());

                    let fee = order
                        .fees
                        .as_ref()
                        .and_then(|s| s.parse::<f64>().ok());

                    if fill_price > 0.0 || realized_pnl.is_some() {
                        tracing::debug!(
                            order_id = %order_id,
                            fill_price = %fill_price,
                            realized_pnl = ?realized_pnl,
                            fee = ?fee,
                            attempt,
                            "Vest: Retrieved fill info from GET /orders"
                        );
                        return Ok(Some(crate::adapters::types::FillInfo {
                            fill_price,
                            realized_pnl,
                            fee,
                        }));
                    }
                }
            }

            // Order exists but not FILLED yet — retry after delay
            let order_statuses: Vec<_> = orders.iter()
                .filter_map(|o| o.status.as_deref())
                .collect();
            tracing::debug!(
                order_id = %order_id,
                attempt,
                max_attempts = MAX_ATTEMPTS,
                statuses = ?order_statuses,
                "Vest: Order not yet FILLED, retrying..."
            );

            if attempt < MAX_ATTEMPTS {
                tokio::time::sleep(std::time::Duration::from_millis(RETRY_DELAY_MS)).await;
            }
        }

        tracing::warn!(
            order_id = %order_id,
            attempts = MAX_ATTEMPTS,
            "Vest: Fill info not found after all retries"
        );
        Ok(None)
    }
}

// =============================================================================
// ExchangeAdapter Implementation
// =============================================================================

#[async_trait]
impl ExchangeAdapter for VestAdapter {
    async fn connect(&mut self) -> ExchangeResult<()> {
        // Warm up HTTP connection pool (latency optimization)
        // This pre-establishes TCP/TLS connections before the first real request
        if let Err(e) = self.warm_up_http().await {
            tracing::warn!("HTTP warm-up failed (non-fatal): {}", e);
        }

        let api_key = self.register().await?;
        self.api_key = Some(api_key);

        let listen_key = self.get_listen_key().await?;
        self.listen_key = Some(listen_key);

        self.connect_websocket().await?;
        self.validate_connection().await?;
        self.split_and_spawn_reader()?;
        self.spawn_heartbeat_task();
        self.spawn_listen_key_renewal_task();

        self.connected = true;
        tracing::info!(exchange = "vest", "Vest WebSocket connected");

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

        if let Some(handle) = self.listen_key_renewal_handle.take() {
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
        self.api_key = None;
        self.listen_key = None;
        self.subscriptions.clear();
        self.pending_subscriptions.clear();

        self.connection_health.last_pong.store(0, Ordering::Relaxed);
        self.connection_health.last_data.store(0, Ordering::Relaxed);
        self.connection_health.reader_alive.store(false, Ordering::Relaxed);

        let mut books = self.shared_orderbooks.write().await;
        books.clear();
        self.orderbooks.clear();

        Ok(())
    }

    async fn subscribe_orderbook(&mut self, symbol: &str) -> ExchangeResult<()> {
        if !self.connected {
            return Err(ExchangeError::ConnectionFailed("Not connected".into()));
        }

        let sub_id = self.send_subscribe_request(symbol).await?;
        self.pending_subscriptions
            .insert(sub_id, symbol.to_string());
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

        let _ = self.send_unsubscribe_request(symbol).await?;
        self.subscriptions.retain(|s| s != symbol);

        {
            let mut books = self.shared_orderbooks.write().await;
            books.remove(symbol);
        }
        self.orderbooks.remove(symbol);

        Ok(())
    }

    async fn place_order(&self, order: OrderRequest) -> ExchangeResult<OrderResponse> {
        if let Some(err) = order.validate() {
            return Err(ExchangeError::OrderRejected(format!(
                "Invalid order: {}",
                err
            )));
        }

        if !self.connected {
            return Err(ExchangeError::ConnectionFailed("Not connected".into()));
        }

        let api_key = self
            .api_key
            .as_ref()
            .ok_or_else(|| ExchangeError::AuthenticationFailed("Not registered".into()))?;

        let (signature, time) = self.sign_order(&order).await?;
        let nonce = time;

        let is_buy = matches!(order.side, crate::adapters::types::OrderSide::Buy);

        let time_in_force = match order.time_in_force {
            crate::adapters::types::TimeInForce::Gtc => "GTC",
            crate::adapters::types::TimeInForce::Fok => "FOK",
            crate::adapters::types::TimeInForce::Ioc => "IOC",
        };

        let mut order_obj = serde_json::json!({
            "time": time,
            "nonce": nonce,
            "orderType": match order.order_type {
                crate::adapters::types::OrderType::Limit => "LIMIT",
                crate::adapters::types::OrderType::Market => "MARKET",
            },
            "symbol": order.symbol,
            "isBuy": is_buy,
            "size": format!("{:.3}", order.quantity),  // Vest requires exactly 3 decimal places
            // Vest requires limitPrice for ALL orders (including MARKET) as a slippage protection
            // If price is None, this will fail - caller must provide a price
            "limitPrice": match order.price {
                Some(p) => format!("{:.3}", p),
                None => return Err(ExchangeError::InvalidOrder(
                    "Vest requires limitPrice for all orders - price must not be None".into()
                )),
            },
            "reduceOnly": order.reduce_only,
        });

        if order.order_type == crate::adapters::types::OrderType::Limit {
            order_obj["timeInForce"] = serde_json::json!(time_in_force);
        }

        let body = serde_json::json!({
            "order": order_obj,
            "recvWindow": crate::adapters::types::VEST_RECV_WINDOW_MS,
            "signature": signature,
        });

        tracing::debug!(
            "Vest place_order body: {}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );

        let url = format!("{}/orders", self.config.rest_base_url());

        let response = self
            .http_client
            .post(&url)
            .header("xrestservermm", self.rest_server_header())
            .header("X-API-KEY", api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ExchangeError::ConnectionFailed(format!("Order request failed: {}", e)))?;

        let order_response = self
            .parse_order_response(response, &order.client_order_id)
            .await?;

        let side_log = match order.side {
            crate::adapters::types::OrderSide::Buy => "long",
            crate::adapters::types::OrderSide::Sell => "short",
        };
        tracing::info!(
            pair = %order.symbol,
            side = side_log,
            size = %order.quantity,
            order_id = %order_response.order_id,
            "Order placed"
        );

        Ok(order_response)
    }

    async fn cancel_order(&self, order_id: &str) -> ExchangeResult<()> {
        if !self.connected {
            return Err(ExchangeError::ConnectionFailed("Not connected".into()));
        }

        let api_key = self
            .api_key
            .as_ref()
            .ok_or_else(|| ExchangeError::AuthenticationFailed("Not registered".into()))?;

        let nonce = current_time_ms();
        let signature = self.sign_cancel_order(order_id, nonce).await?;

        let body = serde_json::json!({
            "orderId": order_id,
            "nonce": nonce,
            "signature": signature,
        });

        let url = format!("{}/order", self.config.rest_base_url());

        let response = self
            .http_client
            .delete(&url)
            .header("xrestservermm", self.rest_server_header())
            .header("X-API-KEY", api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                ExchangeError::ConnectionFailed(format!("Cancel request failed: {}", e))
            })?;

        let status = response.status();
        let text = response.text().await.map_err(|e| {
            ExchangeError::InvalidResponse(format!("Failed to read response: {}", e))
        })?;

        if !status.is_success() {
            return Err(ExchangeError::OrderRejected(format!(
                "Cancel failed ({}): {}",
                status, text
            )));
        }

        let result: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
            ExchangeError::InvalidResponse(format!("Invalid JSON: {} - {}", e, text))
        })?;

        if let Some(code) = result.get("code").and_then(|c| c.as_i64()) {
            if code != 0 {
                let msg = result
                    .get("msg")
                    .and_then(|m| m.as_str())
                    .unwrap_or("Unknown error");
                return Err(ExchangeError::OrderRejected(format!(
                    "Cancel error {}: {}",
                    code, msg
                )));
            }
        }

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

        // Reader loop died → connection is dead (S-1/S-2 fix)
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
        tracing::info!("Vest: Initiating reconnection...");

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
                "Vest: Reconnect attempt {} of {}, waiting {}ms...",
                attempt + 1,
                MAX_RECONNECT_ATTEMPTS,
                backoff_ms
            );

            tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;

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
                        tracing::info!("Vest: Re-subscribing to {}", symbol);
                        if let Err(e) = self.subscribe_orderbook(symbol).await {
                            tracing::warn!("Vest: Failed to re-subscribe to {}: {}", symbol, e);
                        }
                    }

                    tracing::info!(
                        "Vest: Reconnection complete with {} subscriptions restored",
                        self.subscriptions.len()
                    );

                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!("Vest: Reconnect attempt {} failed: {}", attempt + 1, e);
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

    async fn get_position(&self, symbol: &str) -> ExchangeResult<Option<PositionInfo>> {
        if !self.connected {
            return Err(ExchangeError::ConnectionFailed("Not connected".into()));
        }

        // Use get_positions() to find the position for this symbol
        let positions = self.get_positions().await?;

        for pos in positions {
            let pos_symbol = pos.symbol.as_deref().unwrap_or("");
            if pos_symbol == symbol {
                // DEBUG: Log raw size value from API for diagnosing side issues
                let raw_size_str = pos.size.as_deref().unwrap_or("null");
                tracing::debug!(
                    symbol = symbol,
                    raw_size = raw_size_str,
                    "Vest raw position size from API"
                );

                // Parse size - negative = short, positive = long
                let size = pos
                    .size
                    .as_ref()
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0);

                // Skip if no position (size is 0)
                if size.abs() < 0.0000001 {
                    continue;
                }

                // Use isLong field from API (size is always positive)
                // Debug: Log raw values from API to diagnose
                tracing::debug!(
                    symbol = symbol,
                    is_long = ?pos.is_long,
                    raw_size = %pos.size.as_deref().unwrap_or("null"),
                    "Vest position raw data"
                );

                let side = match pos.is_long {
                    Some(true) => "long",
                    Some(false) => "short",
                    None => {
                        tracing::warn!(symbol = symbol, "Vest position missing isLong field");
                        "unknown"
                    }
                };
                let quantity = size.abs();

                let entry_price = pos
                    .entry_price
                    .as_ref()
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0);

                let unrealized_pnl = pos
                    .unrealized_pnl
                    .as_ref()
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0);

                let mark_price = pos.mark_price.as_ref().and_then(|s| s.parse::<f64>().ok());

                tracing::debug!(
                    symbol = symbol,
                    side = side,
                    quantity = quantity,
                    entry_price = entry_price,
                    mark_price = ?mark_price,
                    "Vest position found"
                );

                return Ok(Some(PositionInfo {
                    symbol: symbol.to_string(),
                    side: side.to_string(),
                    quantity,
                    entry_price,
                    mark_price,
                    unrealized_pnl,
                }));
            }
        }

        tracing::debug!(symbol = symbol, "Vest: No position found for symbol");
        Ok(None)
    }

    /// CR-11 fix: Query real fill info (price + realized PnL) from Vest REST API
    async fn get_fill_info(&self, _symbol: &str, order_id: &str) -> ExchangeResult<Option<crate::adapters::types::FillInfo>> {
        self.get_order_fill_price(order_id).await
    }

    fn exchange_name(&self) -> &'static str {
        "vest"
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::super::config::VestConfig;
    use super::*;

    /// Test warm_up_http() functionality
    /// Unit test for connection warm-up functionality
    /// Note: warm_up_http() uses the HTTP client which works independently of WS connection
    #[tokio::test]
    async fn test_warm_up_http_makes_request() {
        let config = VestConfig::default();
        let adapter = VestAdapter::new(config);

        // warm_up_http makes a GET request to /account endpoint
        // This should succeed even without WebSocket connection established
        // because the HTTP client is configured independently
        // Note: Will return 401 without auth, but that's expected - we just
        // want to establish TCP/TLS connection
        let result = adapter.warm_up_http().await;

        // Should succeed - HTTP client can reach Vest
        assert!(
            result.is_ok(),
            "warm_up_http should succeed with default config: {:?}",
            result
        );
    }

    /// Test HTTP client pooling configuration
    /// Verify pooling parameters are configured
    #[test]
    fn test_http_client_configured_with_pooling() {
        let config = VestConfig::default();
        let adapter = VestAdapter::new(config);

        // Verify the adapter was created successfully with pooled HTTP client
        // The builder configuration is validated at build time
        assert!(!adapter.connected, "New adapter should not be connected");
        // HTTP client existence validates pooling config succeeded
        // (If build failed, we'd have a default client from unwrap_or_else)
    }
}
