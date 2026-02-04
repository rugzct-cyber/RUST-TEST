//! Paradex Adapter Implementation
//!
//! Main ParadexAdapter struct implementing ExchangeAdapter trait.
//! Uses modules: config, types, signing for sub-components.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use starknet_crypto::FieldElement;
use tokio::sync::{Mutex, RwLock};
use tokio::time::timeout;
use tokio_tungstenite::{
    connect_async_tls_with_config,
    tungstenite::protocol::Message,
    Connector, MaybeTlsStream, WebSocketStream,
};

use crate::adapters::errors::{ExchangeError, ExchangeResult};
use crate::adapters::traits::ExchangeAdapter;
use crate::adapters::types::{
    Orderbook, OrderRequest, OrderResponse, OrderType, OrderSide, OrderStatus, PositionInfo,
    TimeInForce,
};

// Import from our sub-modules
use super::config::{ParadexConfig, ParadexSystemConfig};
#[cfg(test)]
use super::config::{TEST_ACCOUNT_ADDRESS, TEST_PRIVATE_KEY};
use super::types::{
    AuthResponse, JsonRpcResponse, ParadexOrderResponse, ParadexWsMessage,
};
use super::signing::{
    build_ws_auth_message, current_time_ms, derive_account_address,
    sign_auth_message, sign_order_message, OrderSignParams,
};

/// Type alias for the WebSocket stream
type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// Timeout for WebSocket authentication (5 seconds)
const AUTH_TIMEOUT_SECS: u64 = 5;

/// Timeout for REST API calls (10 seconds)
const REST_TIMEOUT_SECS: u64 = 10;

/// JWT token lifetime in milliseconds (5 minutes = 300,000 ms)
const JWT_LIFETIME_MS: u64 = 300_000;

/// JWT refresh buffer in milliseconds (refresh 2 minutes before expiry)
const JWT_REFRESH_BUFFER_MS: u64 = 120_000;

// =============================================================================
// Subscription ID Management
// =============================================================================

/// Global atomic counter for subscription IDs
static PARADEX_SUBSCRIPTION_ID: AtomicU64 = AtomicU64::new(1);

/// Get next unique subscription ID
fn next_subscription_id() -> u64 {
    PARADEX_SUBSCRIPTION_ID.fetch_add(1, Ordering::Relaxed)
}

// =============================================================================
// WebSocket Stream Types
// =============================================================================

/// Type alias for WebSocket sender (write half)
type WsSink = SplitSink<WsStream, Message>;

/// Type alias for WebSocket receiver (read half)
type WsReader = SplitStream<WsStream>;

/// Shared orderbook storage for concurrent access (Story 7.3: lock-free monitoring)
pub type SharedOrderbooks = Arc<RwLock<HashMap<String, Orderbook>>>;


// =============================================================================
// Paradex Adapter
// =============================================================================

/// Paradex exchange adapter implementing ExchangeAdapter trait
pub struct ParadexAdapter {
    /// Configuration
    config: ParadexConfig,
    /// HTTP client for REST API
    http_client: reqwest::Client,
    /// JWT token obtained from /auth
    jwt_token: Option<String>,
    /// JWT token expiry timestamp (ms)
    jwt_expiry: Option<u64>,
    /// WebSocket stream (for initial auth, replaced by split after connect)
    ws_stream: Option<Mutex<WsStream>>,
    /// WebSocket sender (write half, used after connection established)
    ws_sender: Option<Arc<Mutex<WsSink>>>,
    /// Connection status
    connected: bool,
    /// Authenticated on WebSocket
    ws_authenticated: bool,
    /// Shared orderbooks (thread-safe for background reader)
    shared_orderbooks: SharedOrderbooks,
    /// Local reference for get_orderbook (synced from shared)
    orderbooks: HashMap<String, Orderbook>,
    /// Active subscriptions by symbol
    subscriptions: Vec<String>,
    /// Pending subscription IDs for confirmation tracking
    pending_subscriptions: HashMap<u64, String>,
    /// Handle to message reader task (for cleanup)
    reader_handle: Option<tokio::task::JoinHandle<()>>,
    /// Connection health tracking 
    connection_health: crate::adapters::types::ConnectionHealth,
    /// Handle to heartbeat task (for cleanup)
    heartbeat_handle: Option<tokio::task::JoinHandle<()>>,
    /// Starknet chain ID from system config (cached for order signing)
    starknet_chain_id: Option<String>,
}

impl ParadexAdapter {
    /// Create a new ParadexAdapter with the given configuration
    pub fn new(config: ParadexConfig) -> Self {
        Self {
            config,
            http_client: {
                let client = reqwest::Client::builder()
                    .timeout(Duration::from_secs(REST_TIMEOUT_SECS))
                    .pool_max_idle_per_host(2)           // Keep 2 idle connections per host
                    .pool_idle_timeout(Duration::from_secs(60))  // Keep connections for 60s
                    .tcp_keepalive(Duration::from_secs(30))      // TCP keepalive every 30s
                    .connect_timeout(Duration::from_secs(10))    // Connection timeout
                    .build()
                    .unwrap_or_else(|_| reqwest::Client::new());
                tracing::info!("[INIT] Paradex HTTP client configured: pool_max_idle=2, pool_idle_timeout=60s, tcp_keepalive=30s");
                client
            },
            jwt_token: None,
            jwt_expiry: None,
            ws_stream: None,
            ws_sender: None,
            connected: false,
            ws_authenticated: false,
            shared_orderbooks: Arc::new(RwLock::new(HashMap::new())),
            orderbooks: HashMap::new(),
            subscriptions: Vec::new(),
            pending_subscriptions: HashMap::new(),
            reader_handle: None,
            connection_health: crate::adapters::types::ConnectionHealth::new(),
            heartbeat_handle: None,
            starknet_chain_id: None,
        }
    }

    /// Get shared orderbooks for lock-free monitoring (Story 7.3)
    /// 
    /// Returns Arc<RwLock<...>> that can be read directly without acquiring
    /// the adapter's Mutex. This enables high-frequency orderbook polling
    /// without blocking execution.
    pub fn get_shared_orderbooks(&self) -> SharedOrderbooks {
        Arc::clone(&self.shared_orderbooks)
    }

    /// Authenticate with Paradex REST API to obtain JWT token
    /// 
    /// Implements the full Starknet typed data signing flow:
    /// 1. Generate Starknet signature over timestamp using SNIP-12 typed data
    /// 2. POST /auth with signature in headers
    /// 3. Receive JWT token (expires in 5 minutes)
    async fn authenticate(&mut self) -> ExchangeResult<String> {
        // Step 1: Fetch system config to get correct class hashes and chain_id
        let system_config = self.fetch_system_config().await?;
        tracing::info!("Fetched system config: chain_id={}", system_config.starknet_chain_id);
        
        // Cache chain_id for order signing
        self.starknet_chain_id = Some(system_config.starknet_chain_id.clone());
        
        // CRITICAL: For UI-exported credentials, the derived address differs from actual!
        // Technical report: "credentials L2 ont été exportées directement depuis l'UI Paradex,
        // PAS dérivées via grind_key() - l'UI génère une adresse DIFFÉRENTE"
        // Solution: Use PARADEX_ACCOUNT_ADDRESS from .env if provided, only derive if empty
        let account_address_to_use = if !self.config.account_address.is_empty() 
            && self.config.account_address != "0x0" 
            && self.config.account_address.len() > 4 
        {
            tracing::info!("Using account address from .env (UI-exported credentials)");
            tracing::info!("  .env address: {}", self.config.account_address);
            self.config.account_address.clone()
        } else {
            // Only derive if no address provided
            tracing::info!("No account address in .env, deriving from private key...");
            let derived_address = derive_account_address(
                &self.config.private_key,
                &system_config.paraclear_account_hash,
                &system_config.paraclear_account_proxy_hash,
            )?;
            let derived = format!("0x{:x}", derived_address);
            tracing::info!("  Derived address: {}", derived);
            derived
        };
        
        // Paradex expects timestamp in SECONDS (not milliseconds)
        let timestamp_ms = current_time_ms();
        let timestamp = timestamp_ms / 1000;
        // Signature expiration: 24 hours from now (like official Python SDK)
        let expiration = timestamp + 24 * 60 * 60;
        
        // Use chain_id from system config
        let chain_id = &system_config.starknet_chain_id;
        
        // Derive public key from private key (for logging/debugging)
        let pk = FieldElement::from_hex_be(&self.config.private_key)
            .map_err(|e| ExchangeError::AuthenticationFailed(format!("Invalid private key: {}", e)))?;
        let public_key = starknet_crypto::get_public_key(&pk);
        let public_key_hex = format!("0x{:x}", public_key);
        
        // CRITICAL: Working Python test_auth.py uses /auth NOT /auth/{public_key}
        // See: url = f"{PARADEX_HTTP_URL}/auth" (line 129 of test_auth.py)
        let url = format!("{}/auth", self.config.rest_base_url());
        
        // Generate Starknet signature with TypedData format
        // Use the DERIVED address in the signature
        let (sig_r, sig_s) = sign_auth_message(
            &self.config.private_key,
            &account_address_to_use,
            timestamp,
            expiration,
            chain_id,
        )?;
        
        // Build signature string in Paradex format: ["decimal_r", "decimal_s"]
        let signature_header = format!("[\"{}\",\"{}\"]", sig_r, sig_s);
        
        // Debug log auth request details
        tracing::info!("Paradex Auth Request:");
        tracing::info!("  URL: {}", url);
        tracing::info!("  Public key: {}", public_key_hex);
        tracing::info!("  Account address (derived): {}", account_address_to_use);
        tracing::info!("  Chain ID: {}", chain_id);
        tracing::info!("  Timestamp: {}", timestamp);
        tracing::info!("  Expiration: {}", expiration);
        tracing::info!("  Signature: {}", signature_header);
        
        let response = self.http_client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("PARADEX-STARKNET-ACCOUNT", &account_address_to_use)
            .header("PARADEX-STARKNET-SIGNATURE", &signature_header)
            .header("PARADEX-TIMESTAMP", timestamp.to_string())
            .header("PARADEX-SIGNATURE-EXPIRATION", expiration.to_string())
            .send()
            .await
            .map_err(|e| ExchangeError::ConnectionFailed(format!("Auth request failed: {}", e)))?;

        let status = response.status();
        let text = response.text().await
            .map_err(|e| ExchangeError::InvalidResponse(format!("Failed to read response: {}", e)))?;

        if !status.is_success() {
            return Err(ExchangeError::AuthenticationFailed(
                format!("Authentication failed ({}): {}", status, text)
            ));
        }

        let result: AuthResponse = serde_json::from_str(&text)
            .map_err(|e| ExchangeError::InvalidResponse(format!("Invalid JSON: {} - {}", e, text)))?;

        if let Some(err) = result.error {
            return Err(ExchangeError::AuthenticationFailed(
                format!("Auth error {}: {}", err.code, err.message)
            ));
        }
        
        // Store JWT with expiry tracking (5 min = 300,000 ms)
        let jwt = result.jwt_token.ok_or_else(|| 
            ExchangeError::InvalidResponse("No jwt_token in response".into())
        )?;
        
        self.jwt_expiry = Some(timestamp_ms + JWT_LIFETIME_MS);
        tracing::info!("✅ JWT obtained successfully, expires at: {}", self.jwt_expiry.unwrap());
        
        Ok(jwt)
    }
    
    /// Fetch system configuration from Paradex /system/config endpoint
    async fn fetch_system_config(&self) -> ExchangeResult<ParadexSystemConfig> {
        let url = format!("{}/system/config", self.config.rest_base_url());
        
        let response = self.http_client
            .get(&url)
            .send()
            .await
            .map_err(|e| ExchangeError::ConnectionFailed(format!("Failed to fetch system config: {}", e)))?;
        
        let status = response.status();
        let text = response.text().await
            .map_err(|e| ExchangeError::InvalidResponse(format!("Failed to read config response: {}", e)))?;
        
        if !status.is_success() {
            return Err(ExchangeError::ConnectionFailed(
                format!("System config request failed ({}): {}", status, text)
            ));
        }
        
        let config: ParadexSystemConfig = serde_json::from_str(&text)
            .map_err(|e| ExchangeError::InvalidResponse(format!("Invalid system config JSON: {}", e)))?;
        
        Ok(config)
    }

    
    /// Check if JWT needs refresh (within 2 minutes of expiry)
    fn jwt_needs_refresh(&self) -> bool {
        match self.jwt_expiry {
            Some(expiry) => current_time_ms() > expiry.saturating_sub(JWT_REFRESH_BUFFER_MS),
            None => true, // No JWT, needs refresh
        }
    }

    /// Connect to WebSocket endpoint
    async fn connect_websocket(&mut self) -> ExchangeResult<()> {
        let url = self.config.ws_base_url();

        // Build TLS connector
        let tls = native_tls::TlsConnector::builder()
            .min_protocol_version(Some(native_tls::Protocol::Tlsv12))
            .build()
            .map_err(|e| ExchangeError::ConnectionFailed(format!("TLS error: {}", e)))?;

        let (ws_stream, _response) = connect_async_tls_with_config(
            url,
            None,
            false,
            Some(Connector::NativeTls(tls))
        )
        .await
        .map_err(|e| ExchangeError::WebSocket(Box::new(e)))?;

        self.ws_stream = Some(Mutex::new(ws_stream));
        Ok(())
    }

    /// Authenticate the WebSocket connection using JWT token
    async fn authenticate_websocket(&mut self) -> ExchangeResult<()> {
        let jwt_token = self.jwt_token.as_ref()
            .ok_or_else(|| ExchangeError::AuthenticationFailed("No JWT token".into()))?;

        let ws = self.ws_stream.as_ref()
            .ok_or_else(|| ExchangeError::ConnectionFailed("No WebSocket connection".into()))?;

        let mut stream = ws.lock().await;

        // Send auth message
        let auth_msg = build_ws_auth_message(jwt_token);
        stream.send(Message::Text(auth_msg))
            .await
            .map_err(|e| ExchangeError::WebSocket(Box::new(e)))?;

        // Wait for auth response with timeout
        let auth_timeout = Duration::from_secs(AUTH_TIMEOUT_SECS);
        let auth_result = timeout(auth_timeout, stream.next()).await
            .map_err(|_| ExchangeError::NetworkTimeout(AUTH_TIMEOUT_SECS * 1000))?;

        match auth_result {
            Some(msg_result) => {
                let msg = msg_result.map_err(|e| ExchangeError::WebSocket(Box::new(e)))?;
                
                if let Message::Text(text) = msg {
                    let response: JsonRpcResponse = serde_json::from_str(&text)
                        .map_err(|e| ExchangeError::InvalidResponse(format!("Invalid auth response: {}", e)))?;
                    
                    if let Some(err) = response.error {
                        // Map Paradex error codes
                        let error_msg = match err.code {
                            40110 => format!("Malformed Bearer Token: {}", err.message),
                            40111 => format!("Invalid Bearer Token: {}", err.message),
                            40112 => format!("Geo IP blocked: {}", err.message),
                            _ => format!("Auth error {}: {}", err.code, err.message),
                        };
                        return Err(ExchangeError::AuthenticationFailed(error_msg));
                    }
                    
                    if response.result.is_some() {
                        self.ws_authenticated = true;
                        tracing::info!("Paradex WebSocket authenticated successfully");
                    }
                }
            }
            None => {
                return Err(ExchangeError::ConnectionFailed("No response to auth message".into()));
            }
        }

        Ok(())
    }

    /// Split WebSocket stream and spawn background message reader
    fn split_and_spawn_reader(&mut self) -> ExchangeResult<()> {
        // Take the ws_stream and split it
        let ws_stream_mutex = self.ws_stream.take()
            .ok_or_else(|| ExchangeError::ConnectionFailed("No WebSocket stream to split".into()))?;
        
        // Get the stream out of the mutex
        let ws_stream = ws_stream_mutex.into_inner();
        
        // Split into sender and receiver
        let (ws_sender, ws_receiver) = ws_stream.split();
        
        // Store sender in Arc<Mutex> for thread-safe access
        self.ws_sender = Some(Arc::new(Mutex::new(ws_sender)));
        
        // Clone Arc references for background tasks
        let shared_orderbooks = Arc::clone(&self.shared_orderbooks);
        let last_data = Arc::clone(&self.connection_health.last_data);
        
        // Initialize last_data to now so we don't immediately appear stale
        last_data.store(current_time_ms(), Ordering::Relaxed);
        
        // Spawn background reader with shared orderbooks and health tracking
        let handle = tokio::spawn(async move {
            Self::message_reader_loop(ws_receiver, shared_orderbooks, last_data).await;
        });
        
        self.reader_handle = Some(handle);
        Ok(())
    }

    /// Background message reader loop
    /// Processes incoming WebSocket messages and updates orderbooks
    /// Also updates connection health timestamps
    async fn message_reader_loop(
        mut ws_receiver: WsReader,
        shared_orderbooks: SharedOrderbooks,
        last_data: Arc<AtomicU64>,
    ) {
        tracing::info!("Paradex message_reader_loop started");
        while let Some(msg_result) = ws_receiver.next().await {
            // Update last_data timestamp for any message received
            last_data.store(current_time_ms(), Ordering::Relaxed);
            
            match msg_result {
                Ok(Message::Text(text)) => {
                    // Log raw message at trace level
                    tracing::trace!("Paradex raw WS message: {}", text);
                    
                    // First, check for order channel messages (different structure than orderbook)
                    // Parse as raw JSON to check channel type
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                        // Check if this is a subscription notification for orders channel
                        if let Some(params) = json.get("params") {
                            if let Some(channel) = params.get("channel").and_then(|c| c.as_str()) {
                                if channel.starts_with("orders.") {
                                    // This is an order channel notification
                                    if let Some(data) = params.get("data") {
                                        let order_id = data.get("id")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("?");
                                        let status = data.get("status")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("?");
                                        let side = data.get("side")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("?");
                                        let market = data.get("market")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("?");
                                        
                                        tracing::info!(
                                            "[ORDER] Paradex order confirmed via WS: id={}, status={}, side={}, market={}",
                                            order_id, status, side, market
                                        );
                                    }
                                    continue; // Skip regular parsing for order messages
                                }
                            }
                        }
                    }
                    
                    // Try to parse as typed message (orderbook updates, etc.)
                    match serde_json::from_str::<ParadexWsMessage>(&text) {
                        Ok(msg) => {
                            match msg {
                                ParadexWsMessage::SubscriptionNotification(notif) => {
                                    // JSON-RPC subscription notification with orderbook data
                                    let symbol = notif.params.data.market.clone();
                                    
                                    tracing::debug!(
                                        symbol = %symbol,
                                        channel = %notif.params.channel,
                                        levels = notif.params.data.inserts.len(),
                                        "Paradex subscription orderbook update received"
                                    );
                                    
                                    // Convert to orderbook
                                    match notif.params.data.to_orderbook() {
                                        Ok(orderbook) => {
                                            // Update shared orderbook (acquire lock briefly)
                                            let mut books = shared_orderbooks.write().await;
                                            books.insert(symbol.clone(), orderbook);
                                            tracing::trace!(symbol = %symbol, "Paradex orderbook updated from subscription");
                                        }
                                        Err(e) => {
                                            tracing::warn!(error = %e, "Failed to parse Paradex orderbook from subscription");
                                        }
                                    }
                                }
                                ParadexWsMessage::Orderbook(orderbook_msg) => {
                                    // Direct orderbook message format (legacy/fallback)
                                    let symbol = orderbook_msg.data.market.clone();
                                    
                                    tracing::debug!(
                                        symbol = %symbol,
                                        levels = orderbook_msg.data.inserts.len(),
                                        "Paradex orderbook update received (direct format)"
                                    );
                                    
                                    // Convert to orderbook
                                    match orderbook_msg.data.to_orderbook() {
                                        Ok(orderbook) => {
                                            // Update shared orderbook (acquire lock briefly)
                                            let mut books = shared_orderbooks.write().await;
                                            books.insert(symbol.clone(), orderbook);
                                            tracing::trace!(symbol = %symbol, "Paradex orderbook updated in shared storage");
                                        }
                                        Err(e) => {
                                            tracing::warn!(error = %e, "Failed to parse Paradex orderbook data");
                                        }
                                    }
                                }
                                ParadexWsMessage::JsonRpc(rpc_resp) => {
                                    // JSON-RPC response - subscription confirmation, etc.
                                    if let Some(err) = rpc_resp.error {
                                        tracing::warn!("JSON-RPC error {}: {}", err.code, err.message);
                                    } else {
                                        tracing::debug!("JSON-RPC response: id={}", rpc_resp.id);
                                    }
                                }
                            }
                        }
                        Err(parse_err) => {
                            // Log full message when parsing fails
                            tracing::warn!(
                                error = %parse_err,
                                message = %text,
                                "Paradex message parse failed"
                            );
                        }
                    }
                }
                Ok(Message::Close(_)) => {
                    tracing::info!("Paradex WebSocket closed by server");
                    break;
                }
                Ok(Message::Ping(data)) => {
                    // Respond to pings (handled by tungstenite automatically)
                    tracing::trace!("Ping received: {:?}", data);
                }
                Ok(_) => {
                    // Binary or other messages - ignore
                }
                Err(e) => {
                    tracing::error!("Paradex WebSocket error: {}", e);
                    break;
                }
            }
        }
        tracing::info!("Paradex message reader loop ended");
    }

    /// Send a subscribe request for a symbol's orderbook
    async fn send_subscribe_request(&self, symbol: &str) -> ExchangeResult<u64> {
        let ws_sender = self.ws_sender.as_ref()
            .ok_or_else(|| ExchangeError::ConnectionFailed("WebSocket not connected".into()))?;
        
        let sub_id = next_subscription_id();
        // Paradex orderbook channel format: order_book.{symbol}.snapshot@15@100ms
        let channel = format!("order_book.{}.snapshot@15@100ms", symbol);
        
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "subscribe",
            "params": {
                "channel": channel
            },
            "id": sub_id
        });
        
        let mut sender = ws_sender.lock().await;
        sender.send(Message::Text(msg.to_string()))
            .await
            .map_err(|e| ExchangeError::WebSocket(Box::new(e)))?;
        
        Ok(sub_id)
    }

    /// Send an unsubscribe request for a symbol's orderbook
    async fn send_unsubscribe_request(&self, symbol: &str) -> ExchangeResult<u64> {
        let ws_sender = self.ws_sender.as_ref()
            .ok_or_else(|| ExchangeError::ConnectionFailed("WebSocket not connected".into()))?;
        
        let unsub_id = next_subscription_id();
        let channel = format!("order_book.{}.snapshot@15@100ms", symbol);
        
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "unsubscribe",
            "params": {
                "channel": channel
            },
            "id": unsub_id
        });
        
        let mut sender = ws_sender.lock().await;
        sender.send(Message::Text(msg.to_string()))
            .await
            .map_err(|e| ExchangeError::WebSocket(Box::new(e)))?;
        
        Ok(unsub_id)
    }
    
    /// Subscribe to order status updates for a symbol via WebSocket
    /// 
    /// This is a private channel that requires authentication.
    /// Order updates will be received in the message reader loop and logged with [ORDER] prefix.
    pub async fn subscribe_orders(&self, symbol: &str) -> ExchangeResult<u64> {
        if !self.ws_authenticated {
            return Err(ExchangeError::AuthenticationFailed(
                "WebSocket not authenticated - cannot subscribe to private orders channel".into()
            ));
        }
        
        let ws_sender = self.ws_sender.as_ref()
            .ok_or_else(|| ExchangeError::ConnectionFailed("WebSocket not connected".into()))?;
        
        let sub_id = next_subscription_id();
        // Paradex orders channel format: orders.{symbol}
        let channel = format!("orders.{}", symbol);
        
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "subscribe",
            "params": {
                "channel": channel
            },
            "id": sub_id
        });
        
        let mut sender = ws_sender.lock().await;
        sender.send(Message::Text(msg.to_string()))
            .await
            .map_err(|e| ExchangeError::WebSocket(Box::new(e)))?;
        
        tracing::info!(
            "[ORDER] Subscribed to Paradex orders channel: {} (sub_id={})",
            channel, sub_id
        );
        
        Ok(sub_id)
    }
    
    /// Spawn heartbeat monitoring task 
    /// 
    /// Paradex uses native WebSocket PING/PONG which tokio-tungstenite handles automatically.
    /// This task monitors the last_data timestamp to detect stale connections.
    fn spawn_heartbeat_task(&mut self) {
        let last_data = Arc::clone(&self.connection_health.last_data);
        
        // Initialize last_data to now so we don't immediately appear stale
        last_data.store(current_time_ms(), Ordering::Relaxed);
        
        let handle = tokio::spawn(async move {
            // Check every 30 seconds as per NFR20
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            // Skip the first immediate tick
            interval.tick().await;
            
            loop {
                interval.tick().await;
                
                // Just trace-level health check, no warning needed
                let last = last_data.load(Ordering::Relaxed);
                let now = current_time_ms();
                tracing::trace!(
                    "Paradex heartbeat: last data was {}ms ago", 
                    now.saturating_sub(last)
                );
            }
        });
        
        self.heartbeat_handle = Some(handle);
        tracing::info!("Paradex: Heartbeat monitoring started (30s interval)");
    }

    /// Warm up HTTP connection pool by making a lightweight request
    /// 
    /// This establishes TCP/TLS connections upfront to avoid handshake latency
    /// on the first real request. Uses GET /system/time as it's lightweight.
    async fn warm_up_http(&self) -> ExchangeResult<()> {
        let url = format!("{}/system/time", self.config.rest_base_url());
        let start = std::time::Instant::now();
        
        let response = self.http_client.get(&url).send().await
            .map_err(|e| ExchangeError::ConnectionFailed(format!("HTTP warm-up failed: {}", e)))?;
        
        let elapsed = start.elapsed();
        
        if response.status().is_success() {
            tracing::info!(
                "[INIT] Paradex HTTP connection pool warmed up (latency={}ms)",
                elapsed.as_millis()
            );
        } else {
            tracing::warn!(
                "[INIT] Paradex HTTP warm-up returned status {} (latency={}ms)",
                response.status(),
                elapsed.as_millis()
            );
        }
        
        Ok(())
    }
}

// =============================================================================
// Public API: Leverage/Margin Methods
// =============================================================================

impl ParadexAdapter {
    /// Get current leverage for a market
    /// 
    /// Uses GET /v1/account/margin?market={symbol} to fetch margin configuration.
    /// Returns None if no margin config exists for this market.
    pub async fn get_leverage(&self, symbol: &str) -> ExchangeResult<Option<u32>> {
        if !self.connected {
            return Err(ExchangeError::ConnectionFailed("Not connected".into()));
        }

        let jwt = self.jwt_token.as_ref()
            .ok_or_else(|| ExchangeError::AuthenticationFailed("No JWT token".into()))?;

        let url = format!("{}/account/margin?market={}", self.config.rest_base_url(), symbol);
        tracing::debug!("Paradex get_leverage: GET {}", url);

        let response = self.http_client
            .get(&url)
            .header("Authorization", format!("Bearer {}", jwt))
            .send()
            .await
            .map_err(|e| ExchangeError::ConnectionFailed(format!("Margin request failed: {}", e)))?;

        let status = response.status();
        let body = response.text().await
            .map_err(|e| ExchangeError::InvalidResponse(format!("Failed to read response: {}", e)))?;

        tracing::debug!("Paradex margin response: status={}, body={}", status, body);

        if !status.is_success() {
            // 404 means no margin config for this market - return None
            if status.as_u16() == 404 {
                return Ok(None);
            }
            return Err(ExchangeError::OrderRejected(format!(
                "Get margin failed ({} {}): {}",
                status.as_u16(), status, body
            )));
        }

        let margin_response: super::types::ParadexMarginResponse = serde_json::from_str(&body)
            .map_err(|e| ExchangeError::InvalidResponse(format!("Failed to parse margin: {} - body: {}", e, body)))?;

        // Find leverage in configs for this market
        if let Some(configs) = margin_response.configs {
            for config in configs {
                if config.market.as_deref() == Some(symbol) {
                    return Ok(config.leverage);
                }
            }
        }

        Ok(None)
    }

    /// Set leverage for a market
    ///
    /// Uses POST /v1/account/margin/{market} with body {"leverage": X, "margin_type": "CROSS"}.
    /// Returns the confirmed leverage value on success.
    pub async fn set_leverage(&self, symbol: &str, leverage: u32) -> ExchangeResult<u32> {
        if !self.connected {
            return Err(ExchangeError::ConnectionFailed("Not connected".into()));
        }

        let jwt = self.jwt_token.as_ref()
            .ok_or_else(|| ExchangeError::AuthenticationFailed("No JWT token".into()))?;

        let url = format!("{}/account/margin/{}", self.config.rest_base_url(), symbol);

        let body = serde_json::json!({
            "leverage": leverage,
            "margin_type": "CROSS"
        });

        tracing::debug!("Paradex set_leverage: POST {} body={}", url, body);

        let response = self.http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", jwt))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ExchangeError::OrderRejected(format!("Leverage request failed: {}", e)))?;

        let status = response.status();
        let response_body = response.text().await
            .map_err(|e| ExchangeError::OrderRejected(format!("Failed to read response: {}", e)))?;

        tracing::debug!("Paradex set_leverage response: status={}, body={}", status, response_body);

        if !status.is_success() {
            return Err(ExchangeError::OrderRejected(format!(
                "Set leverage failed ({} {}): {}",
                status.as_u16(), status, response_body
            )));
        }

        let leverage_response: super::types::ParadexSetMarginResponse = serde_json::from_str(&response_body)
            .map_err(|e| ExchangeError::OrderRejected(format!(
                "Failed to parse leverage response: {} - body: {}",
                e, response_body
            )))?;

        leverage_response.leverage
            .ok_or_else(|| ExchangeError::OrderRejected("No leverage value in response".into()))
    }
}

// =============================================================================
// ExchangeAdapter Trait Implementation
// =============================================================================

#[async_trait]
impl ExchangeAdapter for ParadexAdapter {
    /// Connect to Paradex: REST auth + WebSocket connection + WS auth
    async fn connect(&mut self) -> ExchangeResult<()> {
        tracing::info!("Connecting to Paradex...");
        
        // Step 1: Authenticate via REST to get JWT token (required for private channels)
        // Check if we have valid credentials and need to authenticate
        // Note: account_address can be empty - it will be derived from private_key during authenticate()
        if !self.config.private_key.is_empty() {
            if self.jwt_token.is_none() || self.jwt_needs_refresh() {
                tracing::info!("Authenticating with Paradex REST API...");
                let jwt = self.authenticate().await?;
                self.jwt_token = Some(jwt);
                tracing::info!("JWT token obtained successfully");
            }
        } else {
            tracing::info!("No credentials provided - using public channels only");
        }
        
        // Step 2: Connect WebSocket
        self.connect_websocket().await?;
        tracing::info!(exchange = "paradex", "Paradex WebSocket connected");
        
        // Step 3: Authenticate WebSocket (only if we have JWT for private channels)
        if self.jwt_token.is_some() {
            self.authenticate_websocket().await?;
        }
        
        // Step 4: Split stream and spawn reader
        self.split_and_spawn_reader()?;
        
        // Step 5: Start heartbeat monitoring 
        self.spawn_heartbeat_task();
        
        // Step 6: Warm up HTTP connection pool (establish TCP/TLS upfront)
        if let Err(e) = self.warm_up_http().await {
            tracing::warn!("HTTP warm-up failed (non-fatal): {}", e);
        }
        
        self.connected = true;
        tracing::info!("Paradex adapter fully connected");
        
        Ok(())
    }

    /// Disconnect from Paradex
    async fn disconnect(&mut self) -> ExchangeResult<()> {
        tracing::info!("Disconnecting from Paradex...");
        
        // Set state to Disconnected 
        {
            let mut state = self.connection_health.state.write().await;
            *state = crate::adapters::types::ConnectionState::Disconnected;
        }
        
        // Cancel reader task
        if let Some(handle) = self.reader_handle.take() {
            handle.abort();
        }
        
        // Abort heartbeat task if running 
        if let Some(handle) = self.heartbeat_handle.take() {
            handle.abort();
        }
        
        // Close WebSocket sender
        if let Some(ws_sender) = self.ws_sender.take() {
            let mut sender = ws_sender.lock().await;
            let _ = sender.close().await;
        }
        
        // Clear state
        self.connected = false;
        self.ws_authenticated = false;
        self.jwt_token = None;
        self.jwt_expiry = None;
        self.subscriptions.clear();
        self.orderbooks.clear();
        
        // Reset connection health timestamps 
        self.connection_health.last_data.store(0, Ordering::Relaxed);
        
        // Clear shared orderbooks
        let mut books = self.shared_orderbooks.write().await;
        books.clear();
        
        tracing::info!("Paradex adapter disconnected");
        Ok(())
    }

    /// Subscribe to orderbook updates for a trading symbol
    async fn subscribe_orderbook(&mut self, symbol: &str) -> ExchangeResult<()> {
        if !self.connected {
            return Err(ExchangeError::ConnectionFailed("Not connected".into()));
        }
        
        let sub_id = self.send_subscribe_request(symbol).await?;
        self.pending_subscriptions.insert(sub_id, symbol.to_string());
        self.subscriptions.push(symbol.to_string());
        
        tracing::info!("Subscribed to Paradex orderbook: {} (id={})", symbol, sub_id);
        Ok(())
    }

    /// Unsubscribe from orderbook updates
    async fn unsubscribe_orderbook(&mut self, symbol: &str) -> ExchangeResult<()> {
        if !self.connected {
            return Err(ExchangeError::ConnectionFailed("Not connected".into()));
        }
        
        let unsub_id = self.send_unsubscribe_request(symbol).await?;
        self.subscriptions.retain(|s| s != symbol);
        
        // Remove from local orderbooks
        self.orderbooks.remove(symbol);
        
        // Remove from shared orderbooks
        let mut books = self.shared_orderbooks.write().await;
        books.remove(symbol);
        
        tracing::info!("Unsubscribed from Paradex orderbook: {} (id={})", symbol, unsub_id);
        Ok(())
    }

    /// Place an order on Paradex
    /// 
    /// Implements full order placement flow:
    /// 1. Validate order request
    /// 2. Ensure JWT is valid (refresh if needed)
    /// 3. Sign order with Starknet ECDSA
    /// 4. POST to /orders with Authorization Bearer and signature
    /// 5. Parse response to OrderResponse
    async fn place_order(&self, order: OrderRequest) -> ExchangeResult<OrderResponse> {
        // 1. Validate order
        if let Some(err) = order.validate() {
            return Err(ExchangeError::InvalidOrder(err.to_string()));
        }
        
        // 2. Check connection
        if !self.connected {
            return Err(ExchangeError::ConnectionFailed("Not connected".into()));
        }
        
        // 3. Get JWT token (must be valid)
        let jwt = self.jwt_token.as_ref()
            .ok_or_else(|| ExchangeError::AuthenticationFailed("No JWT token - not authenticated".into()))?;
        
        // Note: JWT refresh requires &mut self, but place_order takes &self per trait.
        // IMPORTANT: Caller MUST call reconnect() or re-authenticate() before placing orders
        // if jwt_needs_refresh() returns true. The trait signature prevents inline refresh.
        if self.jwt_needs_refresh() {
            return Err(ExchangeError::AuthenticationFailed(
                "JWT token expired or near expiry - call reconnect() first".into()
            ));
        }
        
        // 4. Prepare order fields
        // Note: timestamp for order signature in MILLISECONDS
        let timestamp = current_time_ms();
        let side_str = match order.side {
            OrderSide::Buy => "BUY",
            OrderSide::Sell => "SELL",
        };
        let type_str = match order.order_type {
            OrderType::Limit => "LIMIT",
            OrderType::Market => "MARKET",
        };
        let instruction = match order.time_in_force {
            TimeInForce::Ioc => "IOC",
            TimeInForce::Gtc => "GTC",
            TimeInForce::Fok => "FOK",
        };
        // For MARKET orders, use "0" as price placeholder for signature (Paradex convention)
        // For LIMIT orders, format to 1 decimal place to match body format
        let price_str = match order.order_type {
            OrderType::Market => "0".to_string(),
            OrderType::Limit => order.price.map(|p| format!("{:.1}", p)).unwrap_or_else(|| "0".to_string()),
        };
        let size_str = order.quantity.to_string();
        
        // 5. Get chain ID for signing (from system config, cached during auth)
        let chain_id = self.starknet_chain_id.as_ref()
            .ok_or_else(|| ExchangeError::AuthenticationFailed(
                "Chain ID not available - authenticate first".into()
            ))?;
        
        // === PROFILING: Start signature timing ===
        let sig_start = std::time::Instant::now();
        
        // 6. Sign the order
        let params = OrderSignParams {
            private_key: &self.config.private_key,
            account_address: &self.config.account_address,
            market: &order.symbol,
            side: side_str,
            order_type: type_str,
            size: &size_str,
            price: &price_str,
            client_id: &order.client_order_id,
            timestamp_ms: timestamp,
            chain_id,
        };
        
        let (sig_r, sig_s) = sign_order_message(params)?;
        
        let sig_elapsed = sig_start.elapsed();
        
        // 7. Build signature string in Paradex format: ["0x...", "0x..."]
        let signature_str = format!("[\"{}\",\"{}\"]", sig_r, sig_s);
        
        // === PROFILING: Start JSON build timing ===
        let json_start = std::time::Instant::now();
        
        // 8. Build request body
        let mut body = serde_json::json!({
            "market": order.symbol,
            "side": side_str,
            "type": type_str,
            "size": size_str,
            "instruction": instruction,
            "signature": signature_str,
            "signature_timestamp": timestamp,
        });
        
        // Add price for limit orders - format to 1 decimal place for Paradex
        if order.order_type == OrderType::Limit {
            if let Some(price) = order.price {
                // Paradex requires price as string with consistent decimal precision
                body["price"] = serde_json::json!(format!("{:.1}", price));
            }
        }
        
        // Add client_id if provided
        if !order.client_order_id.is_empty() {
            body["client_id"] = serde_json::json!(order.client_order_id);
        }
        
        // Add REDUCE_ONLY flag if needed - critical for closing positions
        if order.reduce_only {
            body["flags"] = serde_json::json!(["REDUCE_ONLY"]);
        }
        
        let json_elapsed = json_start.elapsed();
        
        // 9. Send request
        let url = format!("{}/orders", self.config.rest_base_url());
        tracing::debug!("Paradex place_order: POST {} body={}", url, body);
        
        // === PROFILING: Start HTTP timing ===
        let http_start = std::time::Instant::now();
        
        let response = self.http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", jwt))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ExchangeError::ConnectionFailed(format!("Order request failed: {}", e)))?;
        
        let http_elapsed = http_start.elapsed();
        
        // === PROFILING: Start parse timing ===
        let parse_start = std::time::Instant::now();
        
        // 10. Parse response
        let status_code = response.status();
        let text = response.text().await
            .map_err(|e| ExchangeError::InvalidResponse(format!("Failed to read response: {}", e)))?;
        
        let parse_elapsed = parse_start.elapsed();
        
        // === PROFILING: Log all timings ===
        tracing::info!(
            "📊 Order latency breakdown: signature={}ms, json={}μs, http={}ms, parse={}μs, total={}ms",
            sig_elapsed.as_millis(),
            json_elapsed.as_micros(),
            http_elapsed.as_millis(),
            parse_elapsed.as_micros(),
            (sig_elapsed + json_elapsed + http_elapsed + parse_elapsed).as_millis()
        );
        
        tracing::debug!("Paradex order response ({}): {}", status_code, text);
        
        tracing::debug!("Paradex order response ({}): {}", status_code, text);
        
        if !status_code.is_success() {
            // H3 Fix: Check for auth errors (401/403) separately from order rejections
            if status_code.as_u16() == 401 || status_code.as_u16() == 403 {
                return Err(ExchangeError::AuthenticationFailed(format!(
                    "JWT authentication failed ({}): {}",
                    status_code, text
                )));
            }
            
            // Try to parse error response for order-specific errors
            if let Ok(err_resp) = serde_json::from_str::<ParadexOrderResponse>(&text) {
                if let Some(err) = err_resp.error {
                    return Err(ExchangeError::OrderRejected(format!(
                        "Order rejected: {} - {}",
                        err.code.unwrap_or_default(),
                        err.message.unwrap_or_default()
                    )));
                }
            }
            return Err(ExchangeError::OrderRejected(format!(
                "Order failed ({}): {}",
                status_code, text
            )));
        }
        
        // Parse successful response
        tracing::info!("Paradex order response: {}", text);
        let resp: ParadexOrderResponse = serde_json::from_str(&text)
            .map_err(|e| ExchangeError::InvalidResponse(format!("Invalid order response: {} - {}", e, text)))?;
        
        // Map Paradex status to OrderStatus
        let order_status = match resp.status.as_deref() {
            Some("NEW") => OrderStatus::Pending,
            Some("OPEN") => OrderStatus::Pending,
            Some("CLOSED") => {
                // Check cancel_reason to determine if filled or cancelled
                match resp.cancel_reason.as_deref() {
                    Some("FILLED") | None => {
                        // Check filled_qty
                        let filled = resp.filled_qty.as_ref()
                            .and_then(|s| s.parse::<f64>().ok())
                            .unwrap_or(0.0);
                        let size = resp.size.as_ref()
                            .and_then(|s| s.parse::<f64>().ok())
                            .unwrap_or(0.0);
                        if filled >= size && size > 0.0 {
                            OrderStatus::Filled
                        } else if filled > 0.0 {
                            OrderStatus::PartiallyFilled
                        } else {
                            OrderStatus::Cancelled
                        }
                    }
                    Some(_) => OrderStatus::Cancelled,
                }
            }
            _ => {
                tracing::warn!("Paradex: Unknown order status: {:?}", resp.status);
                OrderStatus::Pending
            }
        };
        
        // Extract filled quantity and average price
        let filled_quantity = resp.filled_qty
            .as_ref()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        
        let avg_price = resp.avg_fill_price
            .as_ref()
            .and_then(|s| s.parse::<f64>().ok());
        
        // Get order ID (warn if missing for cancellation purposes)
        let order_id = resp.id.unwrap_or_else(|| {
            tracing::warn!("Paradex: order_id is null in response - cancellation may fail");
            String::new()
        });
        
        let order_response = OrderResponse {
            order_id,
            client_order_id: resp.client_id.unwrap_or_else(|| order.client_order_id.clone()),
            status: order_status,
            filled_quantity,
            avg_price,
        };
        
        // Structured log for order placement
        let side_log = match order.side {
            OrderSide::Buy => "long",
            OrderSide::Sell => "short",
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

    /// Cancel an order on Paradex
    /// 
    /// Sends DELETE request to /orders/{order_id} with JWT auth
    async fn cancel_order(&self, order_id: &str) -> ExchangeResult<()> {
        // Check connection
        if !self.connected {
            return Err(ExchangeError::ConnectionFailed("Not connected".into()));
        }
        
        // Get JWT token
        let jwt = self.jwt_token.as_ref()
            .ok_or_else(|| ExchangeError::AuthenticationFailed("No JWT token - not authenticated".into()))?;
        
        // Send DELETE request
        let url = format!("{}/orders/{}", self.config.rest_base_url(), order_id);
        tracing::debug!("Paradex cancel_order: DELETE {}", url);
        
        let response = self.http_client
            .delete(&url)
            .header("Authorization", format!("Bearer {}", jwt))
            .send()
            .await
            .map_err(|e| ExchangeError::ConnectionFailed(format!("Cancel request failed: {}", e)))?;
        
        let status_code = response.status();
        let text = response.text().await
            .map_err(|e| ExchangeError::InvalidResponse(format!("Failed to read response: {}", e)))?;
        
        tracing::debug!("Paradex cancel response ({}): {}", status_code, text);
        
        if !status_code.is_success() {
            // Try to parse error
            if let Ok(err_resp) = serde_json::from_str::<ParadexOrderResponse>(&text) {
                if let Some(err) = err_resp.error {
                    return Err(ExchangeError::OrderRejected(format!(
                        "Cancel rejected: {} - {}",
                        err.code.unwrap_or_default(),
                        err.message.unwrap_or_default()
                    )));
                }
            }
            return Err(ExchangeError::OrderRejected(format!(
                "Cancel failed ({}): {}",
                status_code, text
            )));
        }
        
        tracing::info!("Paradex order cancelled: {}", order_id);
        Ok(())
    }

    /// Get cached orderbook for a symbol
    /// 
    /// NOTE: This method synchronously reads from the local cache.
    /// Use `get_orderbook_async()` to read from the shared orderbooks updated by the background reader.
    fn get_orderbook(&self, symbol: &str) -> Option<&Orderbook> {
        self.orderbooks.get(symbol)
    }

    /// Check if adapter is connected
    fn is_connected(&self) -> bool {
        self.connected
    }
    
    fn is_stale(&self) -> bool {
        if !self.connected {
            return true;
        }
        
        // Check if JWT needs refresh (critical for order placement)
        if self.jwt_needs_refresh() {
            return true;
        }
        
        let last_data = self.connection_health.last_data.load(Ordering::Relaxed);
        if last_data == 0 {
            // No data ever received - check if we just connected
            return false;
        }
        
        let now = current_time_ms();
        const STALE_THRESHOLD_MS: u64 = 30_000; // 30 seconds
        
        now.saturating_sub(last_data) > STALE_THRESHOLD_MS
    }
    
    async fn sync_orderbooks(&mut self) {
        let books = self.shared_orderbooks.read().await;
        self.orderbooks = books.clone();
    }
    
    async fn reconnect(&mut self) -> ExchangeResult<()> {
        use crate::adapters::types::ConnectionState;
        
        tracing::info!("Paradex: Initiating reconnection...");
        
        // Set state to Reconnecting 
        {
            let mut state = self.connection_health.state.write().await;
            *state = ConnectionState::Reconnecting;
        }
        
        // Store current subscriptions before disconnecting
        let saved_subscriptions = self.subscriptions.clone();
        
        // Disconnect (cleans up resources but we'll override state)
        self.disconnect().await?;
        
        // Exponential backoff retry loop 
        // Delays: 500ms, 1000ms, 2000ms, cap at 5000ms. Max 3 attempts.
        const MAX_RECONNECT_ATTEMPTS: u32 = 3;
        let mut last_error: Option<ExchangeError> = None;
        
        for attempt in 0..MAX_RECONNECT_ATTEMPTS {
            // Exponential backoff: 500ms * 2^attempt, capped at 5000ms
            let backoff_ms = std::cmp::min(500 * (1u64 << attempt), 5000);
            tracing::info!("Paradex: Reconnect attempt {} of {}, waiting {}ms...", 
                attempt + 1, MAX_RECONNECT_ATTEMPTS, backoff_ms);
            
            tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
            
            // Update state to Reconnecting for each attempt
            {
                let mut state = self.connection_health.state.write().await;
                *state = ConnectionState::Reconnecting;
            }
            
            // Try to reconnect
            match self.connect().await {
                Ok(()) => {
                    // Success! Set state to Connected
                    {
                        let mut state = self.connection_health.state.write().await;
                        *state = ConnectionState::Connected;
                    }
                    
                    // Re-subscribe to all previously subscribed symbols
                    for symbol in &saved_subscriptions {
                        tracing::info!("Paradex: Re-subscribing to {}", symbol);
                        if let Err(e) = self.subscribe_orderbook(symbol).await {
                            tracing::warn!("Paradex: Failed to re-subscribe to {}: {}", symbol, e);
                        }
                    }
                    
                    tracing::info!("Paradex: Reconnection complete with {} subscriptions restored", 
                        self.subscriptions.len());
                    
                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!("Paradex: Reconnect attempt {} failed: {}", attempt + 1, e);
                    last_error = Some(e);
                }
            }
        }
        
        // All attempts failed - set state to Disconnected
        {
            let mut state = self.connection_health.state.write().await;
            *state = ConnectionState::Disconnected;
        }
        
        Err(last_error.unwrap_or_else(|| 
            ExchangeError::ConnectionFailed("Reconnection failed after max attempts".into())
        ))
    }

    /// Get current position for a symbol
    /// 
    /// Fetches position data from Paradex REST API.
    async fn get_position(&self, symbol: &str) -> ExchangeResult<Option<PositionInfo>> {
        if !self.connected {
            return Err(ExchangeError::ConnectionFailed("Not connected".into()));
        }

        let jwt = self.jwt_token.as_ref()
            .ok_or_else(|| ExchangeError::AuthenticationFailed("No JWT token".into()))?;

        // Check JWT validity
        if self.jwt_needs_refresh() {
            return Err(ExchangeError::AuthenticationFailed(
                "JWT token expired - call reconnect() first".into()
            ));
        }

        // GET /positions endpoint
        let url = format!("{}/positions", self.config.rest_base_url());
        
        let response = self.http_client
            .get(&url)
            .header("Authorization", format!("Bearer {}", jwt))
            .send()
            .await
            .map_err(|e| ExchangeError::ConnectionFailed(format!("Request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(ExchangeError::InvalidResponse(
                format!("GET /positions failed ({}): {}", status, text)
            ));
        }

        let text = response.text().await
            .map_err(|e| ExchangeError::InvalidResponse(format!("Failed to read response: {}", e)))?;
        
        // Parse response - Paradex returns { "results": [...] }
        #[derive(Debug, Deserialize)]
        struct PositionsResponse {
            results: Option<Vec<ParadexPosition>>,
        }
        
        #[derive(Debug, Deserialize)]
        struct ParadexPosition {
            market: String,
            side: String,  // "LONG" or "SHORT"
            size: String,  // Size as string
            #[serde(default)]
            average_entry_price: Option<String>,
            #[serde(default)]
            unrealized_pnl: Option<String>,
        }
        
        let positions_resp: PositionsResponse = serde_json::from_str(&text)
            .map_err(|e| ExchangeError::InvalidResponse(format!("Invalid JSON: {} - {}", e, text)))?;
        
        // Find position for the requested symbol
        if let Some(positions) = positions_resp.results {
            for pos in positions {
                if pos.market == symbol {
                    let size: f64 = pos.size.parse().unwrap_or(0.0);
                    if size.abs() < 1e-10 {
                        // Zero position = no position
                        continue;
                    }
                    
                    let side = if pos.side == "LONG" { "long" } else { "short" };
                    let entry_price: f64 = pos.average_entry_price
                        .as_deref()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0.0);
                    let unrealized_pnl: f64 = pos.unrealized_pnl
                        .as_deref()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0.0);
                    
                    return Ok(Some(PositionInfo {
                        symbol: symbol.to_string(),
                        quantity: size.abs(),
                        side: side.to_string(),
                        entry_price,
                        unrealized_pnl,
                    }));
                }
            }
        }
        
        // No position found for this symbol
        Ok(None)
    }

    /// Get exchange name
    fn exchange_name(&self) -> &'static str {
        "paradex"
    }
}

// =============================================================================
// Additional ParadexAdapter Methods (not part of ExchangeAdapter trait)
// =============================================================================

impl ParadexAdapter {
    /// Get orderbook from shared storage (async, reads from background reader updates)
    pub async fn get_orderbook_async(&self, symbol: &str) -> Option<Orderbook> {
        let books = self.shared_orderbooks.read().await;
        books.get(symbol).cloned()
    }
    
    /// Sync local orderbooks cache from shared storage
    /// Call this periodically to update the local cache for synchronous access
    pub async fn sync_orderbooks(&mut self) {
        let books = self.shared_orderbooks.read().await;
        self.orderbooks = books.clone();
    }
}

// =============================================================================
// Unit Tests (Minimal - essential adapter tests only)
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Test ParadexAdapter construction
    #[test]
    fn test_paradex_adapter_new() {
        let config = ParadexConfig {
            private_key: TEST_PRIVATE_KEY.to_string(),
            account_address: TEST_ACCOUNT_ADDRESS.to_string(),
            production: true,
        };
        
        let adapter = ParadexAdapter::new(config);
        
        assert!(!adapter.is_connected());
        assert_eq!(adapter.exchange_name(), "paradex");
        assert!(adapter.jwt_token.is_none());
    }

    /// Test exchange name returns "paradex"
    #[test]
    fn test_exchange_name() {
        let config = ParadexConfig::default();
        let adapter = ParadexAdapter::new(config);
        assert_eq!(adapter.exchange_name(), "paradex");
    }

    /// Test JWT expiry check
    #[test]
    fn test_jwt_needs_refresh_when_no_token() {
        let config = ParadexConfig::default();
        let adapter = ParadexAdapter::new(config);
        // No JWT token = needs refresh
        assert!(adapter.jwt_needs_refresh());
    }

    /// Test adapter is not connected initially
    #[test]
    fn test_adapter_not_connected_initially() {
        let config = ParadexConfig::default();
        let adapter = ParadexAdapter::new(config);
        assert!(!adapter.is_connected());
    }

    /// Test warm_up_http() functionality
    /// Story 7.1 CR-4: Unit test for connection warm-up functionality
    /// Note: warm_up_http() uses the HTTP client which works independently of WS connection
    #[tokio::test]
    async fn test_warm_up_http_makes_request() {
        let config = ParadexConfig::default();
        let adapter = ParadexAdapter::new(config);
        
        // warm_up_http makes a GET request to /system/time
        // This should succeed even without WebSocket connection established
        // because the HTTP client is configured independently
        let result = adapter.warm_up_http().await;
        
        // Should succeed - HTTP client can reach Paradex
        assert!(result.is_ok(), "warm_up_http should succeed with default config: {:?}", result);
    }
}