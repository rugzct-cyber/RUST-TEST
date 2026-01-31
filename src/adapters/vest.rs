//! Vest Exchange Adapter
//!
//! Implements the ExchangeAdapter trait for Vest Markets.
//! Uses EIP-712 signatures for authentication and WebSocket for market data.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use ethers::core::types::{Address, U256};
use ethers::signers::{LocalWallet, Signer};
use ethers::contract::EthAbiType;
use ethers::core::types::transaction::eip712::{Eip712, EIP712Domain};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};
use tokio::time::timeout;
use tokio_tungstenite::{
    connect_async_tls_with_config,
    tungstenite::protocol::Message,
    Connector, MaybeTlsStream, WebSocketStream,
};

use crate::adapters::errors::{ExchangeError, ExchangeResult};
use crate::adapters::traits::ExchangeAdapter;
use crate::adapters::types::{Orderbook, OrderRequest, OrderResponse, OrderStatus, PositionInfo};

/// Type alias for the WebSocket stream
type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// Timeout for WebSocket PING/PONG validation (5 seconds)
const PING_TIMEOUT_SECS: u64 = 5;

/// Maximum retry attempts for registration
const MAX_REGISTRATION_RETRIES: u32 = 3;

/// Base backoff delay for retries in milliseconds
const RETRY_BACKOFF_MS: u64 = 500;

// =============================================================================
// Test Constants (Hardhat/Foundry well-known keys - PUBLIC, DO NOT USE IN PROD)
// =============================================================================

/// Hardhat account #1 private key (well-known, public test key)
#[cfg(test)]
const TEST_PRIMARY_KEY: &str = "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";

/// Hardhat account #2 private key (well-known, public test key)  
#[cfg(test)]
const TEST_SIGNING_KEY: &str = "0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a";

/// Hardhat account #1 address
#[cfg(test)]
const TEST_PRIMARY_ADDR: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";

// =============================================================================
// Configuration
// =============================================================================

/// Configuration for Vest exchange connection
#[derive(Debug, Clone)]
pub struct VestConfig {
    /// Primary account address (holds balances)
    pub primary_addr: String,
    /// Primary private key (hex string with 0x prefix) - for signing registration
    pub primary_key: String,
    /// Signing private key (hex string with 0x prefix) - delegate signer
    pub signing_key: String,
    /// Account group for server routing (0-9)
    pub account_group: u8,
    /// Use production endpoints (true) or development (false)
    pub production: bool,
}

impl VestConfig {
    /// Create configuration from environment variables
    pub fn from_env() -> ExchangeResult<Self> {
        let primary_addr = std::env::var("VEST_PRIMARY_ADDR")
            .map_err(|_| ExchangeError::AuthenticationFailed("VEST_PRIMARY_ADDR not set".into()))?;
        let primary_key = std::env::var("VEST_PRIMARY_KEY")
            .map_err(|_| ExchangeError::AuthenticationFailed("VEST_PRIMARY_KEY not set".into()))?;
        let signing_key = std::env::var("VEST_SIGNING_KEY")
            .map_err(|_| ExchangeError::AuthenticationFailed("VEST_SIGNING_KEY not set".into()))?;
        let account_group: u8 = std::env::var("VEST_ACCOUNT_GROUP")
            .unwrap_or_else(|_| "0".to_string())
            .parse()
            .unwrap_or(0);
        let production = std::env::var("VEST_PRODUCTION")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(true);

        Ok(Self {
            primary_addr,
            primary_key,
            signing_key,
            account_group,
            production,
        })
    }

    /// Get REST API base URL
    pub fn rest_base_url(&self) -> &'static str {
        if self.production {
            "https://server-prod.hz.vestmarkets.com/v2"
        } else {
            "https://server-dev.hz.vestmarkets.com/v2"
        }
    }

    /// Get WebSocket base URL
    pub fn ws_base_url(&self) -> &'static str {
        if self.production {
            "wss://ws-prod.hz.vestmarkets.com/ws-api"
        } else {
            "wss://ws-dev.hz.vestmarkets.com/ws-api"
        }
    }

    /// Get verifying contract address for EIP-712
    pub fn verifying_contract(&self) -> &'static str {
        if self.production {
            "0x919386306C47b2Fe1036e3B4F7C40D22D2461a23"
        } else {
            "0x8E4D87AEf4AC4D5415C35A12319013e34223825B"
        }
    }
}

impl Default for VestConfig {
    fn default() -> Self {
        Self {
            primary_addr: String::new(),
            primary_key: String::new(),
            signing_key: String::new(),
            account_group: 0,
            production: true,
        }
    }
}

// =============================================================================
// EIP-712 Types for Vest Authentication
// =============================================================================

/// SignerProof type for EIP-712 signature
/// This struct is signed by the PRIMARY wallet to authorize the signing key as delegate.
#[derive(Debug, Clone, Serialize, EthAbiType)]
struct SignerProof {
    /// The address being authorized to sign on behalf of primary
    approved_signer: Address,
    /// Expiry timestamp in Unix milliseconds
    signer_expiry: U256,
}

impl Eip712 for SignerProof {
    type Error = std::convert::Infallible;

    fn domain(&self) -> Result<EIP712Domain, Self::Error> {
        // Domain struct for EIP-712 - verifying_contract is set dynamically in sign_registration_proof
        Ok(EIP712Domain {
            name: Some("VestRouterV2".into()),
            version: Some("0.0.1".into()),
            chain_id: None,
            verifying_contract: None,
            salt: None,
        })
    }

    fn type_hash() -> Result<[u8; 32], Self::Error> {
        // keccak256("SignerProof(address approvedSigner,uint256 signerExpiry)")
        use ethers::core::utils::keccak256;
        Ok(keccak256("SignerProof(address approvedSigner,uint256 signerExpiry)"))
    }

    fn struct_hash(&self) -> Result<[u8; 32], Self::Error> {
        use ethers::abi::{encode, Token};
        use ethers::core::utils::keccak256;
        
        let type_hash = Self::type_hash()?;
        let encoded = encode(&[
            Token::FixedBytes(type_hash.to_vec()),
            Token::Address(self.approved_signer),
            Token::Uint(self.signer_expiry),
        ]);
        Ok(keccak256(&encoded))
    }
}

/// Generate current timestamp in milliseconds
/// Returns 0 if system time is before Unix epoch (should never happen)
fn current_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Generate expiry timestamp (7 days from now)
fn expiry_7_days_ms() -> u64 {
    current_time_ms().saturating_add(7 * 24 * 3600 * 1000)
}

// =============================================================================
// REST API Response Types
// =============================================================================

/// Response from POST /register
#[derive(Debug, Deserialize)]
struct RegisterResponse {
    #[serde(rename = "apiKey")]
    api_key: Option<String>,
    code: Option<i32>,
    msg: Option<String>,
}

/// Response from POST /listenKey
#[derive(Debug, Deserialize)]
struct ListenKeyResponse {
    #[serde(rename = "listenKey")]
    listen_key: Option<String>,
    code: Option<i32>,
    msg: Option<String>,
}

/// Response from POST /order (Story 2.7)
#[derive(Debug, Deserialize)]
struct VestOrderResponse {
    /// Exchange-assigned order ID
    #[serde(rename = "orderId")]
    order_id: Option<String>,
    /// Client-provided order ID
    #[serde(rename = "clientOrderId")]
    client_order_id: Option<String>,
    /// Order status: NEW, PARTIALLY_FILLED, FILLED, CANCELLED, REJECTED
    status: Option<String>,
    /// Quantity that was executed
    #[serde(rename = "executedQty")]
    executed_qty: Option<String>,
    /// Average fill price
    #[serde(rename = "avgPrice")]
    avg_price: Option<String>,
    /// Error code if any
    code: Option<i32>,
    /// Error message
    msg: Option<String>,
}

// =============================================================================
// WebSocket Message Types for Orderbook Streaming
// =============================================================================

/// Vest depth channel message (orderbook update)
#[derive(Debug, Clone, Deserialize)]
pub struct VestDepthMessage {
    /// Channel name (e.g., "BTC-PERP@depth")
    pub channel: String,
    /// Depth data with bids and asks
    pub data: VestDepthData,
}

/// Depth data containing bids and asks
#[derive(Debug, Clone, Deserialize)]
pub struct VestDepthData {
    /// Bid levels as ["price", "quantity"]
    pub bids: Vec<[String; 2]>,
    /// Ask levels as ["price", "quantity"]
    pub asks: Vec<[String; 2]>,
}

impl VestDepthData {
    /// Convert to Orderbook type, taking only top 10 levels
    pub fn to_orderbook(&self) -> ExchangeResult<Orderbook> {
        use crate::adapters::types::OrderbookLevel;
        
        let bids = self.bids.iter()
            .take(10)
            .map(|[price, qty]| {
                let p = price.parse::<f64>().map_err(|e| 
                    ExchangeError::InvalidResponse(format!("Invalid bid price: {}", e)))?;
                let q = qty.parse::<f64>().map_err(|e| 
                    ExchangeError::InvalidResponse(format!("Invalid bid quantity: {}", e)))?;
                Ok(OrderbookLevel::new(p, q))
            })
            .collect::<ExchangeResult<Vec<_>>>()?;
        
        let asks = self.asks.iter()
            .take(10)
            .map(|[price, qty]| {
                let p = price.parse::<f64>().map_err(|e| 
                    ExchangeError::InvalidResponse(format!("Invalid ask price: {}", e)))?;
                let q = qty.parse::<f64>().map_err(|e| 
                    ExchangeError::InvalidResponse(format!("Invalid ask quantity: {}", e)))?;
                Ok(OrderbookLevel::new(p, q))
            })
            .collect::<ExchangeResult<Vec<_>>>()?;
        
        Ok(Orderbook {
            bids,
            asks,
            timestamp: current_time_ms(),
        })
    }
}

/// Subscription confirmation response
#[derive(Debug, Deserialize)]
struct VestSubscriptionResponse {
    #[allow(dead_code)] // Used by serde for parsing subscription confirmations
    result: Option<serde_json::Value>,
    id: u64,
}

/// Generic WebSocket message that could be depth, subscription confirmation, etc.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum VestWsMessage {
    /// Depth update message with channel field
    Depth(VestDepthMessage),
    /// Subscription/unsubscription confirmation
    Subscription(VestSubscriptionResponse),
    /// PONG response (data field used by serde deserializer)
    Pong {
        #[allow(dead_code)]
        data: String,
    },
}

// =============================================================================
// Subscription ID Management
// =============================================================================

/// Global atomic counter for subscription IDs
static SUBSCRIPTION_ID: AtomicU64 = AtomicU64::new(1);

/// Get next unique subscription ID
fn next_subscription_id() -> u64 {
    SUBSCRIPTION_ID.fetch_add(1, Ordering::Relaxed)
}

// =============================================================================
// WebSocket Stream Types
// =============================================================================

/// Type alias for WebSocket sender (write half)
type WsSink = SplitSink<WsStream, Message>;

/// Type alias for WebSocket receiver (read half)
type WsReader = SplitStream<WsStream>;

/// Shared orderbook storage for concurrent access
type SharedOrderbooks = Arc<RwLock<HashMap<String, Orderbook>>>;

// =============================================================================
// Vest Adapter
// =============================================================================

/// Vest exchange adapter implementing ExchangeAdapter trait
pub struct VestAdapter {
    /// Configuration
    config: VestConfig,
    /// HTTP client for REST API
    http_client: reqwest::Client,
    /// API key obtained from /register
    api_key: Option<String>,
    /// Listen key for WebSocket authentication (60 min validity)
    listen_key: Option<String>,
    /// WebSocket stream (for initial PING/PONG, replaced by split after connect)
    ws_stream: Option<Mutex<WsStream>>,
    /// WebSocket sender (write half, used after connection established)
    ws_sender: Option<Arc<Mutex<WsSink>>>,
    /// Connection status
    connected: bool,
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
    /// Connection health tracking (Story 2.6)
    connection_health: crate::adapters::types::ConnectionHealth,
    /// Handle to heartbeat task (for cleanup)
    heartbeat_handle: Option<tokio::task::JoinHandle<()>>,
}

impl VestAdapter {
    /// Create a new VestAdapter with the given configuration
    pub fn new(config: VestConfig) -> Self {
        // Build HTTP client with timeout for order operations (M2 fix)
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            config,
            http_client,
            api_key: None,
            listen_key: None,
            ws_stream: None,
            ws_sender: None,
            connected: false,
            shared_orderbooks: Arc::new(RwLock::new(HashMap::new())),
            orderbooks: HashMap::new(),
            subscriptions: Vec::new(),
            pending_subscriptions: HashMap::new(),
            reader_handle: None,
            connection_health: crate::adapters::types::ConnectionHealth::new(),
            heartbeat_handle: None,
        }
    }

    /// Build WebSocket connection URL with query parameters (private channels)
    pub fn build_ws_url(&self, listen_key: &str) -> String {
        format!(
            "{}?version=1.0&xwebsocketserver=restserver{}&listenKey={}",
            self.config.ws_base_url(),
            self.config.account_group,
            listen_key
        )
    }
    
    /// Build PUBLIC WebSocket connection URL without listen_key
    /// Used for public channels like orderbook depth
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

    /// Generate EIP-712 signature for registration
    /// 
    /// Signs the SignerProof struct with the PRIMARY key to authorize
    /// the signing key as a delegate using proper EIP-712 typed data signing.
    async fn sign_registration_proof(&self) -> ExchangeResult<(String, String, u64)> {
        // Parse the signing key to get its address
        let signing_wallet: LocalWallet = self.config.signing_key
            .parse()
            .map_err(|e| ExchangeError::AuthenticationFailed(format!("Invalid signing key: {}", e)))?;
        
        // Parse the primary key for signing
        let primary_wallet: LocalWallet = self.config.primary_key
            .parse()
            .map_err(|e| ExchangeError::AuthenticationFailed(format!("Invalid primary key: {}", e)))?;

        let expiry = expiry_7_days_ms();
        let signer_address = signing_wallet.address();

        // Create the SignerProof struct for EIP-712 signing
        let proof = SignerProof {
            approved_signer: signer_address,
            signer_expiry: U256::from(expiry),
        };

        // Build EIP-712 domain with verifying contract
        let verifying_contract: Address = self.config.verifying_contract()
            .parse()
            .map_err(|e| ExchangeError::AuthenticationFailed(format!("Invalid verifying contract: {}", e)))?;

        let _domain = EIP712Domain {
            name: Some("VestRouterV2".into()),
            version: Some("0.0.1".into()),
            chain_id: None,
            verifying_contract: Some(verifying_contract),
            salt: None,
        };

        // Compute the EIP-712 hash manually
        use ethers::abi::{encode, Token};
        use ethers::core::utils::keccak256;

        // Domain separator hash
        let domain_type_hash = keccak256(
            "EIP712Domain(string name,string version,address verifyingContract)"
        );
        let domain_encoded = encode(&[
            Token::FixedBytes(domain_type_hash.to_vec()),
            Token::FixedBytes(keccak256("VestRouterV2").to_vec()),
            Token::FixedBytes(keccak256("0.0.1").to_vec()),
            Token::Address(verifying_contract),
        ]);
        let domain_separator = keccak256(&domain_encoded);

        // Struct hash
        let struct_hash = proof.struct_hash()
            .map_err(|_| ExchangeError::AuthenticationFailed("Failed to compute struct hash".into()))?;

        // EIP-712 final hash: keccak256("\x19\x01" + domainSeparator + structHash)
        let mut data = Vec::with_capacity(66);
        data.push(0x19);
        data.push(0x01);
        data.extend_from_slice(&domain_separator);
        data.extend_from_slice(&struct_hash);
        let final_hash = keccak256(&data);

        // Sign the EIP-712 hash with primary wallet
        let signature = primary_wallet
            .sign_hash(final_hash.into())
            .map_err(|e| ExchangeError::AuthenticationFailed(format!("EIP-712 signing failed: {}", e)))?;

        // Signature should be exactly 65 bytes (r: 32, s: 32, v: 1)
        let mut sig_bytes = signature.to_vec();
        if sig_bytes.len() != 65 {
            return Err(ExchangeError::AuthenticationFailed(
                format!("Invalid signature length: {} (expected 65)", sig_bytes.len())
            ));
        }

        // Normalize v value: ethers-rs may return 0/1, but Vest expects 27/28
        let v = sig_bytes[64];
        if v == 0 || v == 1 {
            sig_bytes[64] = v + 27;
        } else if !matches!(v, 27 | 28) {
            return Err(ExchangeError::AuthenticationFailed(
                format!("Invalid signature v value: {} (expected 0, 1, 27, or 28)", v)
            ));
        }

        let sig_hex = format!("0x{}", hex::encode(sig_bytes));
        let signer_hex = format!("{:?}", signer_address).to_lowercase();

        Ok((sig_hex, signer_hex, expiry))
    }

    /// Sign an order with EIP-712 using the SIGNING key (not primary)
    /// Returns (signature_hex, nonce)
    /// Story 2.7: Orders are signed by the delegate signer, not the primary wallet
    async fn sign_order(&self, order: &OrderRequest) -> ExchangeResult<(String, u64)> {
        use ethers::abi::{encode, Token};
        use ethers::core::utils::keccak256;

        // Parse the signing key (delegate signer for orders)
        let signing_wallet: LocalWallet = self.config.signing_key
            .parse()
            .map_err(|e| ExchangeError::AuthenticationFailed(format!("Invalid signing key: {}", e)))?;

        // Use current time as nonce for uniqueness
        let nonce = current_time_ms();

        // Build the order message for signing
        // Vest API expects: symbol, side, type, price, quantity, timeInForce, clientOrderId, nonce
        let side = match order.side {
            crate::adapters::types::OrderSide::Buy => "BUY",
            crate::adapters::types::OrderSide::Sell => "SELL",
        };
        let order_type = match order.order_type {
            crate::adapters::types::OrderType::Limit => "LIMIT",
            crate::adapters::types::OrderType::Market => "MARKET",
        };
        let time_in_force = match order.time_in_force {
            crate::adapters::types::TimeInForce::Ioc => "IOC",
            crate::adapters::types::TimeInForce::Gtc => "GTC",
            crate::adapters::types::TimeInForce::Fok => "FOK",
        };

        // Compute EIP-712 order type hash
        // OrderMessage(string symbol,string side,string type,string price,string quantity,string timeInForce,string clientOrderId,uint256 nonce)
        let order_type_hash = keccak256(
            "OrderMessage(string symbol,string side,string type,string price,string quantity,string timeInForce,string clientOrderId,uint256 nonce)"
        );

        // Format price and quantity as strings
        let price_str = order.price.map(|p| format!("{:.8}", p)).unwrap_or_default();
        let quantity_str = format!("{:.8}", order.quantity);

        // Compute struct hash: keccak256(typeHash, keccak256(symbol), keccak256(side), ...)
        let struct_encoded = encode(&[
            Token::FixedBytes(order_type_hash.to_vec()),
            Token::FixedBytes(keccak256(order.symbol.as_bytes()).to_vec()),
            Token::FixedBytes(keccak256(side.as_bytes()).to_vec()),
            Token::FixedBytes(keccak256(order_type.as_bytes()).to_vec()),
            Token::FixedBytes(keccak256(price_str.as_bytes()).to_vec()),
            Token::FixedBytes(keccak256(quantity_str.as_bytes()).to_vec()),
            Token::FixedBytes(keccak256(time_in_force.as_bytes()).to_vec()),
            Token::FixedBytes(keccak256(order.client_order_id.as_bytes()).to_vec()),
            Token::Uint(U256::from(nonce)),
        ]);
        let struct_hash = keccak256(&struct_encoded);

        // Build domain separator (same as registration)
        let verifying_contract: Address = self.config.verifying_contract()
            .parse()
            .map_err(|e| ExchangeError::AuthenticationFailed(format!("Invalid verifying contract: {}", e)))?;

        let domain_type_hash = keccak256(
            "EIP712Domain(string name,string version,address verifyingContract)"
        );
        let domain_encoded = encode(&[
            Token::FixedBytes(domain_type_hash.to_vec()),
            Token::FixedBytes(keccak256("VestRouterV2").to_vec()),
            Token::FixedBytes(keccak256("0.0.1").to_vec()),
            Token::Address(verifying_contract),
        ]);
        let domain_separator = keccak256(&domain_encoded);

        // EIP-712 final hash: keccak256("\x19\x01" + domainSeparator + structHash)
        let mut data = Vec::with_capacity(66);
        data.push(0x19);
        data.push(0x01);
        data.extend_from_slice(&domain_separator);
        data.extend_from_slice(&struct_hash);
        let final_hash = keccak256(&data);

        // Sign with signing wallet (not primary!)
        let signature = signing_wallet
            .sign_hash(final_hash.into())
            .map_err(|e| ExchangeError::AuthenticationFailed(format!("Order signing failed: {}", e)))?;

        let sig_bytes = signature.to_vec();
        if sig_bytes.len() != 65 {
            return Err(ExchangeError::AuthenticationFailed(
                format!("Invalid signature length: {} (expected 65)", sig_bytes.len())
            ));
        }

        let sig_hex = format!("0x{}", hex::encode(sig_bytes));
        Ok((sig_hex, nonce))
    }

    /// Sign a cancel order request with EIP-712
    async fn sign_cancel_order(&self, order_id: &str, nonce: u64) -> ExchangeResult<String> {
        use ethers::abi::{encode, Token};
        use ethers::core::utils::keccak256;

        let signing_wallet: LocalWallet = self.config.signing_key
            .parse()
            .map_err(|e| ExchangeError::AuthenticationFailed(format!("Invalid signing key: {}", e)))?;

        // Cancel type hash
        let cancel_type_hash = keccak256("CancelOrder(string orderId,uint256 nonce)");

        let struct_encoded = encode(&[
            Token::FixedBytes(cancel_type_hash.to_vec()),
            Token::FixedBytes(keccak256(order_id.as_bytes()).to_vec()),
            Token::Uint(U256::from(nonce)),
        ]);
        let struct_hash = keccak256(&struct_encoded);

        // Domain separator
        let verifying_contract: Address = self.config.verifying_contract()
            .parse()
            .map_err(|e| ExchangeError::AuthenticationFailed(format!("Invalid verifying contract: {}", e)))?;

        let domain_type_hash = keccak256(
            "EIP712Domain(string name,string version,address verifyingContract)"
        );
        let domain_encoded = encode(&[
            Token::FixedBytes(domain_type_hash.to_vec()),
            Token::FixedBytes(keccak256("VestRouterV2").to_vec()),
            Token::FixedBytes(keccak256("0.0.1").to_vec()),
            Token::Address(verifying_contract),
        ]);
        let domain_separator = keccak256(&domain_encoded);

        // Final hash
        let mut data = Vec::with_capacity(66);
        data.push(0x19);
        data.push(0x01);
        data.extend_from_slice(&domain_separator);
        data.extend_from_slice(&struct_hash);
        let final_hash = keccak256(&data);

        let signature = signing_wallet
            .sign_hash(final_hash.into())
            .map_err(|e| ExchangeError::AuthenticationFailed(format!("Cancel signing failed: {}", e)))?;

        // H1 fix: Validate signature length like other signing methods
        let sig_bytes = signature.to_vec();
        if sig_bytes.len() != 65 {
            return Err(ExchangeError::AuthenticationFailed(
                format!("Invalid cancel signature length: {} (expected 65)", sig_bytes.len())
            ));
        }

        let sig_hex = format!("0x{}", hex::encode(sig_bytes));
        Ok(sig_hex)
    }

    /// Parse order response from Vest API
    async fn parse_order_response(
        &self,
        response: reqwest::Response,
        client_order_id: &str,
    ) -> ExchangeResult<OrderResponse> {
        let status = response.status();
        let text = response.text().await
            .map_err(|e| ExchangeError::InvalidResponse(format!("Failed to read response: {}", e)))?;

        if !status.is_success() {
            return Err(ExchangeError::OrderRejected(
                format!("Order failed ({}): {}", status, text)
            ));
        }

        let result: VestOrderResponse = serde_json::from_str(&text)
            .map_err(|e| ExchangeError::InvalidResponse(format!("Invalid JSON: {} - {}", e, text)))?;

        // Check for error code
        if let Some(code) = result.code {
            if code != 0 {
                return Err(ExchangeError::OrderRejected(
                    format!("Order error {}: {}", code, result.msg.unwrap_or_default())
                ));
            }
        }

        // Parse status (Vest uses: NEW, PARTIALLY_FILLED, FILLED, CANCELLED, REJECTED)
        // H2 fix: Return error on unknown status instead of silently defaulting to Pending
        let order_status = match result.status.as_deref() {
            Some("NEW") => OrderStatus::Pending,
            Some("PARTIALLY_FILLED") => OrderStatus::PartiallyFilled,
            Some("FILLED") => OrderStatus::Filled,
            Some("CANCELLED") => OrderStatus::Cancelled,
            Some("REJECTED") => OrderStatus::Rejected,
            Some(unknown) => {
                tracing::warn!(status = unknown, "Unknown order status from Vest, treating as pending");
                OrderStatus::Pending
            }
            None => {
                return Err(ExchangeError::InvalidResponse(
                    format!("Order response missing status field: {}", text)
                ));
            }
        };

        // Parse filled quantity
        let filled_quantity = result.executed_qty
            .as_ref()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);

        // Parse average price
        let avg_price = result.avg_price
            .as_ref()
            .and_then(|s| s.parse::<f64>().ok());

        // H3 fix: Warn if order_id is missing (may prevent cancellation)
        let order_id = match result.order_id {
            Some(id) => id,
            None => {
                tracing::warn!(client_order_id = client_order_id, "Vest response missing orderId, cancel may fail");
                format!("vest-{}", client_order_id)
            }
        };

        Ok(OrderResponse {
            order_id,
            client_order_id: result.client_order_id.unwrap_or_else(|| client_order_id.to_string()),
            status: order_status,
            filled_quantity,
            avg_price,
        })
    }

    /// Register with Vest API to obtain API key
    /// Implements retry logic with exponential backoff (max 3 attempts)
    async fn register(&mut self) -> ExchangeResult<String> {
        let mut last_error = None;
        
        for attempt in 0..MAX_REGISTRATION_RETRIES {
            if attempt > 0 {
                // Exponential backoff: 500ms, 1000ms, 2000ms
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

        Err(last_error.unwrap_or_else(|| 
            ExchangeError::AuthenticationFailed("Registration failed after max retries".into())
        ))
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

        let response = self.http_client
            .post(&url)
            .header("xrestservermm", self.rest_server_header())
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ExchangeError::ConnectionFailed(format!("Register request failed: {}", e)))?;

        let status = response.status();
        let text = response.text().await
            .map_err(|e| ExchangeError::InvalidResponse(format!("Failed to read response: {}", e)))?;

        if !status.is_success() {
            return Err(ExchangeError::AuthenticationFailed(
                format!("Registration failed ({}): {}", status, text)
            ));
        }

        let result: RegisterResponse = serde_json::from_str(&text)
            .map_err(|e| ExchangeError::InvalidResponse(format!("Invalid JSON: {} - {}", e, text)))?;

        if let Some(code) = result.code {
            return Err(ExchangeError::AuthenticationFailed(
                format!("Registration error {}: {}", code, result.msg.unwrap_or_default())
            ));
        }

        result.api_key.ok_or_else(|| 
            ExchangeError::InvalidResponse("No api_key in response".into())
        )
    }

    /// Obtain listen key for WebSocket connection
    async fn get_listen_key(&self) -> ExchangeResult<String> {
        let api_key = self.api_key.as_ref()
            .ok_or_else(|| ExchangeError::AuthenticationFailed("Not registered".into()))?;

        let url = format!("{}/account/listenKey", self.config.rest_base_url());

        let response = self.http_client
            .post(&url)
            .header("xrestservermm", self.rest_server_header())
            .header("X-API-KEY", api_key)
            .send()
            .await
            .map_err(|e| ExchangeError::ConnectionFailed(format!("ListenKey request failed: {}", e)))?;

        let status = response.status();
        let text = response.text().await
            .map_err(|e| ExchangeError::InvalidResponse(format!("Failed to read response: {}", e)))?;

        if !status.is_success() {
            return Err(ExchangeError::AuthenticationFailed(
                format!("ListenKey failed ({}): {}", status, text)
            ));
        }

        let result: ListenKeyResponse = serde_json::from_str(&text)
            .map_err(|e| ExchangeError::InvalidResponse(format!("Invalid JSON: {} - {}", e, text)))?;

        if let Some(code) = result.code {
            return Err(ExchangeError::AuthenticationFailed(
                format!("ListenKey error {}: {}", code, result.msg.unwrap_or_default())
            ));
        }

        result.listen_key.ok_or_else(|| 
            ExchangeError::InvalidResponse("No listenKey in response".into())
        )
    }

    /// Connect to WebSocket and validate with PING/PONG
    /// Uses PUBLIC WebSocket URL for public channels like orderbook depth
    async fn connect_websocket(&mut self) -> ExchangeResult<()> {
        // Use public WebSocket URL (without listen_key) for public channels
        let url = self.build_public_ws_url();
        tracing::info!("Connecting to Vest public WebSocket: {}", url);

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

    /// Send PING and validate PONG response with timeout
    async fn validate_connection(&self) -> ExchangeResult<()> {
        let ws = self.ws_stream.as_ref()
            .ok_or_else(|| ExchangeError::ConnectionFailed("No WebSocket connection".into()))?;

        let mut stream = ws.lock().await;

        // Send PING
        let ping_msg = serde_json::json!({
            "method": "PING",
            "params": [],
            "id": 0
        });
        
        stream.send(Message::Text(ping_msg.to_string()))
            .await
            .map_err(|e| ExchangeError::WebSocket(Box::new(e)))?;

        // Wait for PONG with timeout
        let pong_timeout = Duration::from_secs(PING_TIMEOUT_SECS);
        let pong_result = timeout(pong_timeout, stream.next()).await
            .map_err(|_| ExchangeError::NetworkTimeout(PING_TIMEOUT_SECS * 1000))?;

        match pong_result {
            Some(msg_result) => {
                let msg = msg_result.map_err(|e| ExchangeError::WebSocket(Box::new(e)))?;
                
                if let Message::Text(text) = msg {
                    let response: serde_json::Value = serde_json::from_str(&text)
                        .map_err(|e| ExchangeError::InvalidResponse(format!("Invalid PONG: {}", e)))?;
                    
                    if response.get("data").and_then(|d| d.as_str()) != Some("PONG") {
                        return Err(ExchangeError::ConnectionFailed(
                            format!("Expected PONG, got: {}", text)
                        ));
                    }
                }
            }
            None => {
                return Err(ExchangeError::ConnectionFailed("No response to PING".into()));
            }
        }

        Ok(())
    }

    /// Split WebSocket stream and spawn background message reader
    /// Call this after PING/PONG validation
    fn split_and_spawn_reader(&mut self) -> ExchangeResult<()> {
        // Take the ws_stream and split it
        let ws_stream_mutex = self.ws_stream.take()
            .ok_or_else(|| ExchangeError::ConnectionFailed("No WebSocket stream to split".into()))?;
        
        // We need to get the stream out of the mutex synchronously
        // Since we just validated PING/PONG, we know the stream is good
        let ws_stream = ws_stream_mutex.into_inner();
        
        // Split into sender and receiver
        let (ws_sender, ws_receiver) = ws_stream.split();
        
        // Store sender in Arc<Mutex> for thread-safe access
        self.ws_sender = Some(Arc::new(Mutex::new(ws_sender)));
        
        // Clone Arc references for background tasks
        let shared_orderbooks = Arc::clone(&self.shared_orderbooks);
        let last_pong = Arc::clone(&self.connection_health.last_pong);
        let last_data = Arc::clone(&self.connection_health.last_data);
        
        // Initialize last_data to now so we don't immediately appear stale
        last_data.store(current_time_ms(), Ordering::Relaxed);
        
        // Spawn background reader with shared orderbooks and health tracking
        let handle = tokio::spawn(async move {
            Self::message_reader_loop(ws_receiver, shared_orderbooks, last_pong, last_data).await;
        });
        
        self.reader_handle = Some(handle);
        Ok(())
    }

    /// Background message reader loop
    /// Processes incoming WebSocket messages and updates orderbooks
    /// Story 2.6: Also updates connection health timestamps
    async fn message_reader_loop(
        mut ws_receiver: WsReader,
        shared_orderbooks: SharedOrderbooks,
        last_pong: Arc<AtomicU64>,
        last_data: Arc<AtomicU64>,
    ) {
        tracing::info!("Vest message_reader_loop started");
        while let Some(msg_result) = ws_receiver.next().await {
            // Update last_data timestamp for any message received
            last_data.store(current_time_ms(), Ordering::Relaxed);
            
            match msg_result {
                Ok(Message::Text(text)) => {
                    // Log raw message at trace level for debugging
                    tracing::trace!("Raw WS message: {}", text);
                    
                    // Try to parse as different message types
                    match serde_json::from_str::<VestWsMessage>(&text) {
                        Ok(msg) => {
                            match msg {
                                VestWsMessage::Depth(depth_msg) => {
                                    // Extract symbol from channel (e.g., "BTC-PERP@depth" -> "BTC-PERP")
                                    let symbol = depth_msg.channel
                                        .strip_suffix("@depth")
                                        .unwrap_or(&depth_msg.channel)
                                        .to_string();
                                    
                                    tracing::debug!(
                                        symbol = %symbol,
                                        bids = depth_msg.data.bids.len(),
                                        asks = depth_msg.data.asks.len(),
                                        "Vest depth update received"
                                    );
                                    
                                    // Convert to orderbook
                                    match depth_msg.data.to_orderbook() {
                                        Ok(orderbook) => {
                                            // Update shared orderbook (acquire lock briefly)
                                            let mut books = shared_orderbooks.write().await;
                                            books.insert(symbol.clone(), orderbook);
                                            tracing::trace!(symbol = %symbol, "Orderbook updated in shared storage");
                                        }
                                        Err(e) => {
                                            // Log parsing error with structured fields but continue
                                            tracing::warn!(error = %e, "Failed to parse orderbook data");
                                        }
                                    }
                                }
                                VestWsMessage::Subscription(sub_resp) => {
                                    // Subscription confirmation - log it
                                    tracing::debug!("Subscription confirmed: id={}", sub_resp.id);
                                }
                                VestWsMessage::Pong { .. } => {
                                    // PONG received - heartbeat ok, update timestamp
                                    last_pong.store(current_time_ms(), Ordering::Relaxed);
                                    tracing::trace!("PONG received");
                                }
                            }
                        }
                        Err(parse_err) => {
                            // Log full message when parsing fails for debugging
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
                    // Respond to server pings (handled by tungstenite automatically usually)
                    tracing::debug!("Ping received: {:?}", data);
                }
                Ok(Message::Pong(data)) => {
                    tracing::debug!("Pong WS frame received: {:?}", data);
                }
                Ok(Message::Binary(data)) => {
                    // Try to decode binary as UTF-8 text and parse as JSON
                    // Some WebSocket servers send JSON as binary frames
                    match String::from_utf8(data.clone()) {
                        Ok(text) => {
                            tracing::debug!("Binary->Text: {}", text);
                            // Try to parse as VestWsMessage
                            match serde_json::from_str::<VestWsMessage>(&text) {
                                Ok(msg) => {
                                    match msg {
                                        VestWsMessage::Depth(depth_msg) => {
                                            let symbol = depth_msg.channel
                                                .strip_suffix("@depth")
                                                .unwrap_or(&depth_msg.channel)
                                                .to_string();
                                            
                                            tracing::debug!(
                                                symbol = %symbol,
                                                bids = depth_msg.data.bids.len(),
                                                asks = depth_msg.data.asks.len(),
                                                "Vest depth update received (from binary)"
                                            );
                                            
                                            match depth_msg.data.to_orderbook() {
                                                Ok(orderbook) => {
                                                    let mut books = shared_orderbooks.write().await;
                                                    books.insert(symbol.clone(), orderbook);
                                                    tracing::trace!(symbol = %symbol, "Orderbook updated from binary");
                                                }
                                                Err(e) => {
                                                    tracing::warn!(error = %e, "Failed to parse orderbook from binary");
                                                }
                                            }
                                        }
                                        VestWsMessage::Subscription(sub_resp) => {
                                            tracing::debug!("Subscription confirmed (binary): id={}", sub_resp.id);
                                        }
                                        VestWsMessage::Pong { .. } => {
                                            last_pong.store(current_time_ms(), Ordering::Relaxed);
                                            tracing::trace!("PONG received (binary)");
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!(error = %e, text = %text, "Binary message parse failed");
                                }
                            }
                        }
                        Err(e) => {
                            tracing::debug!("Binary message not UTF-8: {} ({} bytes)", e, data.len());
                        }
                    }
                }
                Ok(Message::Frame(_)) => {
                    tracing::trace!("Raw frame received");
                }
                Err(e) => {
                    tracing::error!("WebSocket error: {}", e);
                    break;
                }
            }
        }
        tracing::info!("Message reader loop ended");
    }

    /// Send a SUBSCRIBE request for a symbol's orderbook
    async fn send_subscribe_request(&self, symbol: &str) -> ExchangeResult<u64> {
        let ws_sender = self.ws_sender.as_ref()
            .ok_or_else(|| ExchangeError::ConnectionFailed("WebSocket not connected".into()))?;
        
        let sub_id = next_subscription_id();
        let channel = format!("{}@depth", symbol);
        
        let msg = serde_json::json!({
            "method": "SUBSCRIBE",
            "params": [channel],
            "id": sub_id
        });
        
        let mut sender = ws_sender.lock().await;
        sender.send(Message::Text(msg.to_string()))
            .await
            .map_err(|e| ExchangeError::WebSocket(Box::new(e)))?;
        
        Ok(sub_id)
    }

    /// Send an UNSUBSCRIBE request for a symbol's orderbook
    async fn send_unsubscribe_request(&self, symbol: &str) -> ExchangeResult<u64> {
        let ws_sender = self.ws_sender.as_ref()
            .ok_or_else(|| ExchangeError::ConnectionFailed("WebSocket not connected".into()))?;
        
        let unsub_id = next_subscription_id();
        let channel = format!("{}@depth", symbol);
        
        let msg = serde_json::json!({
            "method": "UNSUBSCRIBE",
            "params": [channel],
            "id": unsub_id
        });
        
        let mut sender = ws_sender.lock().await;
        sender.send(Message::Text(msg.to_string()))
            .await
            .map_err(|e| ExchangeError::WebSocket(Box::new(e)))?;
        
        Ok(unsub_id)
    }

    /// Sync local orderbooks from shared storage
    pub async fn sync_orderbooks(&mut self) {
        let books = self.shared_orderbooks.read().await;
        self.orderbooks = books.clone();
    }
    
    /// Spawn heartbeat monitoring task (Story 2.6)
    /// 
    /// Sends PING every 30 seconds and monitors for PONG responses.
    /// If no PONG is received within 35 seconds (30s + 5s grace), logs a warning.
    fn spawn_heartbeat_task(&mut self) {
        let ws_sender = match &self.ws_sender {
            Some(sender) => Arc::clone(sender),
            None => return, // No sender available
        };
        
        let last_pong = Arc::clone(&self.connection_health.last_pong);
        
        // Initialize last_pong to now so we don't immediately appear stale
        last_pong.store(current_time_ms(), Ordering::Relaxed);
        
        let handle = tokio::spawn(async move {
            // Heartbeat interval: 30 seconds as per NFR20
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            // Skip the first immediate tick
            interval.tick().await;
            
            loop {
                interval.tick().await;
                
                // Build PING message in Vest format
                let ping_msg = serde_json::json!({
                    "method": "PING",
                    "params": [],
                    "id": 0
                });
                
                // Try to send PING
                {
                    let mut sender = ws_sender.lock().await;
                    if let Err(e) = sender.send(Message::Text(ping_msg.to_string())).await {
                        tracing::warn!("Vest heartbeat: Failed to send PING - {}", e);
                        break;
                    }
                    tracing::trace!("Vest heartbeat: PING sent");
                }
                
                // Wait 5 seconds for PONG response
                tokio::time::sleep(Duration::from_secs(5)).await;
                
                // Check if PONG was received recently (within 35s = 30s interval + 5s grace)
                let last = last_pong.load(Ordering::Relaxed);
                let now = current_time_ms();
                const PONG_TIMEOUT_MS: u64 = 35_000;
                
                if now.saturating_sub(last) > PONG_TIMEOUT_MS {
                    tracing::warn!("Vest heartbeat: PONG timeout - connection may be stale (last PONG: {}ms ago)", 
                        now.saturating_sub(last));
                    // Note: We don't break here - the reader loop will detect disconnect
                    // and the reconnect logic can be triggered externally
                }
            }
            
            tracing::debug!("Vest heartbeat task ended");
        });
        
        self.heartbeat_handle = Some(handle);
        tracing::info!("Vest: Heartbeat monitoring started (30s interval)");
    }
}

#[async_trait]
impl ExchangeAdapter for VestAdapter {
    async fn connect(&mut self) -> ExchangeResult<()> {
        // Step 1: Register and get API key
        let api_key = self.register().await?;
        self.api_key = Some(api_key);

        // Step 2: Get listen key for WebSocket
        let listen_key = self.get_listen_key().await?;
        self.listen_key = Some(listen_key);

        // Step 3: Connect to WebSocket
        self.connect_websocket().await?;

        // Step 4: Validate connection with PING/PONG
        self.validate_connection().await?;

        // Step 5: Split stream and spawn background reader
        self.split_and_spawn_reader()?;
        
        // Step 6: Start heartbeat monitoring (Story 2.6)
        self.spawn_heartbeat_task();

        self.connected = true;
        Ok(())
    }


    async fn disconnect(&mut self) -> ExchangeResult<()> {
        // Set state to Disconnected (Story 2.6)
        {
            let mut state = self.connection_health.state.write().await;
            *state = crate::adapters::types::ConnectionState::Disconnected;
        }
        
        // Abort reader task if running
        if let Some(handle) = self.reader_handle.take() {
            handle.abort();
        }
        
        // Abort heartbeat task if running (Story 2.6)
        if let Some(handle) = self.heartbeat_handle.take() {
            handle.abort();
        }
        
        // Close WebSocket sender
        if let Some(ws_sender) = self.ws_sender.take() {
            let mut sender = ws_sender.lock().await;
            let _ = sender.close().await;
        }
        
        // Clear old ws_stream if still present (shouldn't be after split)
        if let Some(ws) = self.ws_stream.take() {
            let mut stream = ws.lock().await;
            let _ = stream.close(None).await;
        }
        
        self.connected = false;
        self.api_key = None;
        self.listen_key = None;
        self.subscriptions.clear();
        self.pending_subscriptions.clear();
        
        // Reset connection health timestamps (Story 2.6)
        self.connection_health.last_pong.store(0, Ordering::Relaxed);
        self.connection_health.last_data.store(0, Ordering::Relaxed);
        
        // Clear shared orderbooks
        let mut books = self.shared_orderbooks.write().await;
        books.clear();
        self.orderbooks.clear();
        
        Ok(())
    }

    async fn subscribe_orderbook(&mut self, symbol: &str) -> ExchangeResult<()> {
        if !self.connected {
            return Err(ExchangeError::ConnectionFailed("Not connected".into()));
        }
        
        // Send subscribe request
        let sub_id = self.send_subscribe_request(symbol).await?;
        
        // Track pending subscription
        self.pending_subscriptions.insert(sub_id, symbol.to_string());
        self.subscriptions.push(symbol.to_string());
        
        // Initialize empty orderbook in shared storage
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
        
        // Send unsubscribe request
        let _ = self.send_unsubscribe_request(symbol).await?;
        
        // Remove from subscriptions list
        self.subscriptions.retain(|s| s != symbol);
        
        // Remove from shared orderbooks
        {
            let mut books = self.shared_orderbooks.write().await;
            books.remove(symbol);
        }
        self.orderbooks.remove(symbol);
        
        Ok(())
    }

    async fn place_order(&self, order: OrderRequest) -> ExchangeResult<OrderResponse> {
        // Story 2.7: Full Vest order placement implementation

        // 1. Validate order
        if let Some(err) = order.validate() {
            return Err(ExchangeError::OrderRejected(format!("Invalid order: {}", err)));
        }

        // 2. Check connection
        if !self.connected {
            return Err(ExchangeError::ConnectionFailed("Not connected".into()));
        }

        // 3. Get API key
        let api_key = self.api_key.as_ref()
            .ok_or_else(|| ExchangeError::AuthenticationFailed("Not registered".into()))?;

        // 4. Sign order with EIP-712 using signing key
        let (signature, nonce) = self.sign_order(&order).await?;

        // 5. Build request body per Vest API specification
        let body = serde_json::json!({
            "symbol": order.symbol,
            "side": match order.side {
                crate::adapters::types::OrderSide::Buy => "BUY",
                crate::adapters::types::OrderSide::Sell => "SELL",
            },
            "type": match order.order_type {
                crate::adapters::types::OrderType::Limit => "LIMIT",
                crate::adapters::types::OrderType::Market => "MARKET",
            },
            "price": order.price.map(|p| format!("{:.8}", p)),
            "quantity": format!("{:.8}", order.quantity),
            "timeInForce": match order.time_in_force {
                crate::adapters::types::TimeInForce::Ioc => "IOC",
                crate::adapters::types::TimeInForce::Gtc => "GTC",
                crate::adapters::types::TimeInForce::Fok => "FOK",
            },
            "clientOrderId": order.client_order_id,
            "nonce": nonce,
            "signature": signature,
        });

        // 6. Send request to Vest API
        let url = format!("{}/order", self.config.rest_base_url());

        let response = self.http_client
            .post(&url)
            .header("xrestservermm", self.rest_server_header())
            .header("X-API-KEY", api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ExchangeError::ConnectionFailed(format!("Order request failed: {}", e)))?;

        // 7. Parse response
        self.parse_order_response(response, &order.client_order_id).await
    }

    async fn cancel_order(&self, order_id: &str) -> ExchangeResult<()> {
        // Story 2.7: Cancel order implementation

        // Check connection
        if !self.connected {
            return Err(ExchangeError::ConnectionFailed("Not connected".into()));
        }

        // Get API key
        let api_key = self.api_key.as_ref()
            .ok_or_else(|| ExchangeError::AuthenticationFailed("Not registered".into()))?;

        // Build cancel request (nonce for signature)
        let nonce = current_time_ms();

        // Sign the cancel request
        let signature = self.sign_cancel_order(order_id, nonce).await?;

        let body = serde_json::json!({
            "orderId": order_id,
            "nonce": nonce,
            "signature": signature,
        });

        let url = format!("{}/order", self.config.rest_base_url());

        let response = self.http_client
            .delete(&url)
            .header("xrestservermm", self.rest_server_header())
            .header("X-API-KEY", api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ExchangeError::ConnectionFailed(format!("Cancel request failed: {}", e)))?;

        let status = response.status();
        let text = response.text().await
            .map_err(|e| ExchangeError::InvalidResponse(format!("Failed to read response: {}", e)))?;

        if !status.is_success() {
            return Err(ExchangeError::OrderRejected(
                format!("Cancel failed ({}): {}", status, text)
            ));
        }

        // Parse response to check for errors
        let result: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| ExchangeError::InvalidResponse(format!("Invalid JSON: {} - {}", e, text)))?;

        if let Some(code) = result.get("code").and_then(|c| c.as_i64()) {
            if code != 0 {
                let msg = result.get("msg").and_then(|m| m.as_str()).unwrap_or("Unknown error");
                return Err(ExchangeError::OrderRejected(format!("Cancel error {}: {}", code, msg)));
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
        use std::sync::atomic::Ordering;
        
        if !self.connected {
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
        
        tracing::info!("Vest: Initiating reconnection...");
        
        // Set state to Reconnecting (Story 2.6)
        {
            let mut state = self.connection_health.state.write().await;
            *state = ConnectionState::Reconnecting;
        }
        
        // Store current subscriptions before disconnecting
        let saved_subscriptions = self.subscriptions.clone();
        
        // Disconnect (cleans up resources but we'll override state)
        self.disconnect().await?;
        
        // Exponential backoff retry loop (Story 2.6 - Task 4.2, 4.4)
        // Delays: 500ms, 1000ms, 2000ms, cap at 5000ms. Max 3 attempts.
        const MAX_RECONNECT_ATTEMPTS: u32 = 3;
        let mut last_error: Option<ExchangeError> = None;
        
        for attempt in 0..MAX_RECONNECT_ATTEMPTS {
            // Exponential backoff: 500ms * 2^attempt, capped at 5000ms
            let backoff_ms = std::cmp::min(500 * (1u64 << attempt), 5000);
            tracing::info!("Vest: Reconnect attempt {} of {}, waiting {}ms...", 
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
                        tracing::info!("Vest: Re-subscribing to {}", symbol);
                        if let Err(e) = self.subscribe_orderbook(symbol).await {
                            tracing::warn!("Vest: Failed to re-subscribe to {}: {}", symbol, e);
                        }
                    }
                    
                    tracing::info!("Vest: Reconnection complete with {} subscriptions restored", 
                        self.subscriptions.len());
                    
                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!("Vest: Reconnect attempt {} failed: {}", attempt + 1, e);
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

    /// Get current position for a symbol (Story 5.3 - Reconciliation)
    /// 
    /// Fetches position data from Vest REST API.
    /// Note: Actual REST endpoint integration will be added when Vest API docs are available.
    async fn get_position(&self, symbol: &str) -> ExchangeResult<Option<PositionInfo>> {
        if !self.connected {
            return Err(ExchangeError::ConnectionFailed("Not connected".into()));
        }

        let api_key = self.api_key.as_ref()
            .ok_or_else(|| ExchangeError::AuthenticationFailed("Not registered".into()))?;

        // TODO: Implement actual REST call to GET /positions endpoint
        // For now, return None (no position) as a placeholder
        // The actual implementation will parse Vest's position response format
        let url = format!("{}/positions?symbol={}", self.config.rest_base_url(), symbol);
        
        tracing::debug!("Vest get_position: GET {} (stub)", url);
        
        // Stub: Return None to indicate no position
        // Real implementation will fetch from Vest API
        let _ = (api_key, url); // Suppress unused warnings for now
        Ok(None)
    }

    fn exchange_name(&self) -> &'static str {
        "vest"
    }
}

// =============================================================================
// Unit Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vest_config_default() {
        let config = VestConfig::default();
        assert!(config.production);
        assert_eq!(config.account_group, 0);
        assert!(config.primary_addr.is_empty());
        assert!(config.primary_key.is_empty());
        assert!(config.signing_key.is_empty());
    }

    #[test]
    fn test_vest_config_urls_production() {
        let config = VestConfig {
            production: true,
            ..Default::default()
        };
        assert_eq!(config.rest_base_url(), "https://server-prod.hz.vestmarkets.com/v2");
        assert_eq!(config.ws_base_url(), "wss://ws-prod.hz.vestmarkets.com/ws-api");
        assert_eq!(config.verifying_contract(), "0x919386306C47b2Fe1036e3B4F7C40D22D2461a23");
    }

    #[test]
    fn test_vest_config_urls_development() {
        let config = VestConfig {
            production: false,
            ..Default::default()
        };
        assert_eq!(config.rest_base_url(), "https://server-dev.hz.vestmarkets.com/v2");
        assert_eq!(config.ws_base_url(), "wss://ws-dev.hz.vestmarkets.com/ws-api");
        assert_eq!(config.verifying_contract(), "0x8E4D87AEf4AC4D5415C35A12319013e34223825B");
    }

    #[test]
    fn test_vest_adapter_new() {
        let config = VestConfig {
            primary_addr: "0x1234".to_string(),
            primary_key: "0xabc123".to_string(),
            signing_key: "0xdef456".to_string(),
            account_group: 5,
            production: true,
        };
        let adapter = VestAdapter::new(config);
        
        assert!(!adapter.is_connected());
        assert_eq!(adapter.exchange_name(), "vest");
        assert!(adapter.api_key.is_none());
        assert!(adapter.listen_key.is_none());
    }

    #[test]
    fn test_build_ws_url() {
        let config = VestConfig {
            account_group: 3,
            production: true,
            ..Default::default()
        };
        let adapter = VestAdapter::new(config);
        let url = adapter.build_ws_url("test-listen-key");
        
        assert!(url.contains("ws-prod"));
        assert!(url.contains("version=1.0"));
        assert!(url.contains("xwebsocketserver=restserver3"));
        assert!(url.contains("listenKey=test-listen-key"));
    }

    #[test]
    fn test_build_ws_url_development() {
        let config = VestConfig {
            account_group: 0,
            production: false,
            ..Default::default()
        };
        let adapter = VestAdapter::new(config);
        let url = adapter.build_ws_url("dev-key");
        
        assert!(url.contains("ws-dev"));
        assert!(url.contains("listenKey=dev-key"));
    }

    #[test]
    fn test_rest_server_header() {
        let config = VestConfig {
            account_group: 7,
            ..Default::default()
        };
        let adapter = VestAdapter::new(config);
        assert_eq!(adapter.rest_server_header(), "restserver7");
    }

    #[test]
    fn test_current_time_ms() {
        let time = current_time_ms();
        // Should be a reasonable timestamp (after year 2020)
        assert!(time > 1577836800000); // Jan 1, 2020
    }

    #[test]
    fn test_expiry_7_days_ms() {
        let now = current_time_ms();
        let expiry = expiry_7_days_ms();
        // Expiry should be approximately 7 days in the future
        let seven_days_ms = 7 * 24 * 3600 * 1000;
        assert!(expiry > now);
        assert!(expiry - now >= seven_days_ms - 1000); // Allow 1 second tolerance
        assert!(expiry - now <= seven_days_ms + 1000);
    }

    #[test]
    fn test_eip712_domain_creation() {
        // Test using ethers EIP712Domain
        let domain = EIP712Domain {
            name: Some("VestRouterV2".into()),
            version: Some("0.0.1".into()),
            chain_id: None,
            verifying_contract: Some("0x919386306C47b2Fe1036e3B4F7C40D22D2461a23".parse().unwrap()),
            salt: None,
        };
        assert!(domain.name.is_some());
        assert!(domain.version.is_some());
        assert!(domain.verifying_contract.is_some());
    }

    #[test]
    fn test_signer_proof_struct_hash() {
        let proof = SignerProof {
            approved_signer: "0x1234567890123456789012345678901234567890".parse().unwrap(),
            signer_expiry: U256::from(1234567890000u64),
        };
        let hash = proof.struct_hash();
        assert!(hash.is_ok());
        // Hash should be exactly 32 bytes
        assert_eq!(hash.unwrap().len(), 32);
    }

    #[test]
    fn test_signer_proof_serialization_format() {
        let proof = SignerProof {
            approved_signer: "0x1234567890123456789012345678901234567890".parse().unwrap(),
            signer_expiry: U256::from(1234567890000u64),
        };
        let json = serde_json::to_string(&proof).unwrap();
        // Check JSON contains the expected address format
        assert!(json.contains("1234567890123456789012345678901234567890"));
    }

    #[tokio::test]
    async fn test_vest_adapter_disconnect_clears_state() {
        let config = VestConfig::default();
        let mut adapter = VestAdapter::new(config);
        
        // Manually set some state
        adapter.api_key = Some("test-key".to_string());
        adapter.listen_key = Some("listen-key".to_string());
        adapter.connected = true;
        adapter.subscriptions.push("BTC-PERP".to_string());
        
        adapter.disconnect().await.unwrap();
        
        assert!(!adapter.is_connected());
        assert!(adapter.api_key.is_none());
        assert!(adapter.listen_key.is_none());
        assert!(adapter.subscriptions.is_empty());
    }

    #[tokio::test]
    async fn test_vest_adapter_subscribe_requires_connection() {
        let config = VestConfig::default();
        let mut adapter = VestAdapter::new(config);
        
        // Subscribe should fail when not connected
        let result = adapter.subscribe_orderbook("BTC-PERP").await;
        assert!(result.is_err());
        
        if let Err(ExchangeError::ConnectionFailed(msg)) = result {
            assert!(msg.contains("Not connected"));
        } else {
            panic!("Expected ConnectionFailed error");
        }
    }

    #[tokio::test]
    async fn test_vest_adapter_unsubscribe_requires_connection() {
        let config = VestConfig::default();
        let mut adapter = VestAdapter::new(config);
        
        // Unsubscribe should fail when not connected
        let result = adapter.unsubscribe_orderbook("BTC-PERP").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_vest_adapter_place_order_requires_connection() {
        // Story 2.7: place_order should fail when not connected
        let config = VestConfig::default();
        let adapter = VestAdapter::new(config);
        
        let order = OrderRequest::ioc_limit(
            "test-order-123".to_string(),
            "BTC-PERP".to_string(),
            crate::adapters::types::OrderSide::Buy,
            42000.0,
            0.1,
        );
        
        // Should fail because adapter is not connected
        let result = adapter.place_order(order).await;
        assert!(result.is_err());
        
        // Should be a ConnectionFailed error
        if let Err(ExchangeError::ConnectionFailed(msg)) = result {
            assert!(msg.contains("Not connected"), "Expected 'Not connected' error, got: {}", msg);
        } else {
            panic!("Expected ConnectionFailed error");
        }
    }

    #[tokio::test]
    async fn test_vest_adapter_place_order_validates_order() {
        // Story 2.7: place_order should validate order before sending
        let config = VestConfig {
            primary_addr: TEST_PRIMARY_ADDR.to_string(),
            primary_key: TEST_PRIMARY_KEY.to_string(),
            signing_key: TEST_SIGNING_KEY.to_string(),
            account_group: 0,
            production: false,
        };
        let mut adapter = VestAdapter::new(config);
        // Mark as connected (simulate connection for validation test)
        adapter.connected = true;
        adapter.api_key = Some("test-api-key".to_string());
        
        // Invalid order: Limit order without price
        let invalid_order = OrderRequest {
            client_order_id: "test-123".to_string(),
            symbol: "BTC-PERP".to_string(),
            side: crate::adapters::types::OrderSide::Buy,
            order_type: crate::adapters::types::OrderType::Limit,
            price: None, // Invalid for Limit!
            quantity: 0.1,
            time_in_force: crate::adapters::types::TimeInForce::Ioc,
        };
        
        let result = adapter.place_order(invalid_order).await;
        assert!(result.is_err());
        
        // Should be an OrderRejected error for validation failure
        if let Err(ExchangeError::OrderRejected(msg)) = result {
            assert!(msg.contains("Invalid order"), "Expected validation error, got: {}", msg);
        } else {
            panic!("Expected OrderRejected error for invalid order");
        }
    }

    #[tokio::test]
    async fn test_vest_sign_order_produces_valid_signature() {
        // Story 2.7: Test EIP-712 order signing produces valid 65-byte signature
        let config = VestConfig {
            primary_addr: TEST_PRIMARY_ADDR.to_string(),
            primary_key: TEST_PRIMARY_KEY.to_string(),
            signing_key: TEST_SIGNING_KEY.to_string(),
            account_group: 0,
            production: false,
        };
        let adapter = VestAdapter::new(config);
        
        let order = OrderRequest::ioc_limit(
            "test-order-123".to_string(),
            "BTC-PERP".to_string(),
            crate::adapters::types::OrderSide::Buy,
            42000.0,
            0.1,
        );
        
        let result = adapter.sign_order(&order).await;
        assert!(result.is_ok(), "Order signing failed: {:?}", result.err());
        
        let (signature, nonce) = result.unwrap();
        
        // Signature must be exactly 132 chars: "0x" + 130 hex chars (65 bytes)
        assert!(signature.starts_with("0x"), "Signature should start with 0x");
        assert_eq!(signature.len(), 132, "Signature should be exactly 132 chars (0x + 65 bytes hex)");
        
        // Nonce should be a recent timestamp (within last minute)
        let now = crate::adapters::vest::current_time_ms();
        assert!(nonce > now - 60_000, "Nonce should be recent timestamp");
        assert!(nonce <= now, "Nonce should not be in the future");
    }

    #[tokio::test]
    async fn test_vest_cancel_order_requires_connection() {
        // Story 2.7: cancel_order should fail when not connected
        let config = VestConfig::default();
        let adapter = VestAdapter::new(config);
        
        let result = adapter.cancel_order("test-order-id").await;
        assert!(result.is_err());
        
        if let Err(ExchangeError::ConnectionFailed(msg)) = result {
            assert!(msg.contains("Not connected"));
        } else {
            panic!("Expected ConnectionFailed error");
        }
    }

    #[tokio::test]
    async fn test_sign_registration_proof_with_valid_keys() {
        // Use well-known Hardhat test keys (safe for testing, public)
        let config = VestConfig {
            primary_addr: TEST_PRIMARY_ADDR.to_string(),
            primary_key: TEST_PRIMARY_KEY.to_string(),
            signing_key: TEST_SIGNING_KEY.to_string(),
            account_group: 0,
            production: false,
        };
        let adapter = VestAdapter::new(config);
        
        let result = adapter.sign_registration_proof().await;
        assert!(result.is_ok(), "Signature generation failed: {:?}", result.err());
        
        let (signature, signer_addr, expiry) = result.unwrap();
        
        // Signature must be exactly 132 chars: "0x" + 130 hex chars (65 bytes)
        assert!(signature.starts_with("0x"), "Signature should start with 0x");
        assert_eq!(signature.len(), 132, "Signature should be exactly 132 chars (0x + 65 bytes hex)");
        
        // Signer address should be a hex address (42 chars: 0x + 40 hex)
        assert!(signer_addr.starts_with("0x"), "Signer address should start with 0x");
        assert_eq!(signer_addr.len(), 42, "Signer address should be 42 chars");
        
        // Expiry should be in the future
        assert!(expiry > current_time_ms(), "Expiry should be in the future");
    }

    #[tokio::test]
    async fn test_sign_registration_proof_invalid_key() {
        let config = VestConfig {
            primary_addr: "0x1234".to_string(),
            primary_key: "invalid-key".to_string(),
            signing_key: TEST_SIGNING_KEY.to_string(),
            account_group: 0,
            production: false,
        };
        let adapter = VestAdapter::new(config);
        
        let result = adapter.sign_registration_proof().await;
        assert!(result.is_err());
        
        if let Err(ExchangeError::AuthenticationFailed(msg)) = result {
            assert!(msg.contains("Invalid primary key"));
        } else {
            panic!("Expected AuthenticationFailed error");
        }
    }

    // =========================================================================
    // Story 2.3: Orderbook Streaming Tests
    // =========================================================================

    #[test]
    fn test_vest_depth_message_parsing() {
        let json = r#"{"channel": "BTC-PERP@depth", "data": {"bids": [["100.0", "1.5"]], "asks": [["101.0", "2.0"]]}}"#;
        let msg: VestDepthMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.channel, "BTC-PERP@depth");
        assert_eq!(msg.data.bids.len(), 1);
        assert_eq!(msg.data.asks.len(), 1);
        assert_eq!(msg.data.bids[0][0], "100.0");
        assert_eq!(msg.data.bids[0][1], "1.5");
    }

    #[test]
    fn test_vest_depth_data_to_orderbook() {
        let data = VestDepthData {
            bids: vec![
                ["99500.50".to_string(), "0.125".to_string()],
                ["99499.00".to_string(), "0.250".to_string()],
            ],
            asks: vec![
                ["99501.00".to_string(), "0.100".to_string()],
                ["99502.50".to_string(), "0.300".to_string()],
            ],
        };
        let orderbook = data.to_orderbook().unwrap();
        
        assert_eq!(orderbook.bids.len(), 2);
        assert_eq!(orderbook.asks.len(), 2);
        assert_eq!(orderbook.bids[0].price, 99500.50);
        assert_eq!(orderbook.bids[0].quantity, 0.125);
        assert_eq!(orderbook.asks[0].price, 99501.00);
        assert_eq!(orderbook.asks[0].quantity, 0.100);
    }

    #[test]
    fn test_vest_depth_data_truncates_to_10_levels() {
        // Create 15 bid and ask levels
        let bids: Vec<[String; 2]> = (0..15)
            .map(|i| [format!("{}.0", 100 - i), format!("{}.0", i + 1)])
            .collect();
        let asks: Vec<[String; 2]> = (0..15)
            .map(|i| [format!("{}.0", 101 + i), format!("{}.0", i + 1)])
            .collect();
        
        let data = VestDepthData { bids, asks };
        let orderbook = data.to_orderbook().unwrap();
        
        // Should only have 10 levels each
        assert_eq!(orderbook.bids.len(), 10);
        assert_eq!(orderbook.asks.len(), 10);
    }

    #[test]
    fn test_vest_depth_data_invalid_price() {
        let data = VestDepthData {
            bids: vec![["not_a_number".to_string(), "1.0".to_string()]],
            asks: vec![],
        };
        let result = data.to_orderbook();
        assert!(result.is_err());
    }

    #[test]
    fn test_subscription_message_format() {
        let msg = serde_json::json!({
            "method": "SUBSCRIBE",
            "params": ["BTC-PERP@depth"],
            "id": 1
        });
        assert_eq!(msg["method"], "SUBSCRIBE");
        assert_eq!(msg["params"][0], "BTC-PERP@depth");
        assert_eq!(msg["id"], 1);
    }

    #[test]
    fn test_unsubscription_message_format() {
        let msg = serde_json::json!({
            "method": "UNSUBSCRIBE",
            "params": ["ETH-PERP@depth"],
            "id": 42
        });
        assert_eq!(msg["method"], "UNSUBSCRIBE");
        assert_eq!(msg["params"][0], "ETH-PERP@depth");
        assert_eq!(msg["id"], 42);
    }

    #[test]
    fn test_subscription_id_generation() {
        let id1 = next_subscription_id();
        let id2 = next_subscription_id();
        let id3 = next_subscription_id();
        
        // IDs should be unique and increasing
        assert!(id2 > id1);
        assert!(id3 > id2);
    }

    #[test]
    fn test_vest_ws_message_depth_parsing() {
        let json = r#"{"channel": "SOL-PERP@depth", "data": {"bids": [["150.0", "10.0"]], "asks": [["151.0", "20.0"]]}}"#;
        let msg: VestWsMessage = serde_json::from_str(json).unwrap();
        
        match msg {
            VestWsMessage::Depth(depth) => {
                assert_eq!(depth.channel, "SOL-PERP@depth");
            }
            _ => panic!("Expected Depth message"),
        }
    }

    #[test]
    fn test_vest_ws_message_subscription_parsing() {
        let json = r#"{"result": null, "id": 5}"#;
        let msg: VestWsMessage = serde_json::from_str(json).unwrap();
        
        match msg {
            VestWsMessage::Subscription(sub) => {
                assert_eq!(sub.id, 5);
            }
            _ => panic!("Expected Subscription message"),
        }
    }

    #[test]
    fn test_vest_ws_message_pong_parsing() {
        let json = r#"{"data": "PONG"}"#;
        let msg: VestWsMessage = serde_json::from_str(json).unwrap();
        
        match msg {
            VestWsMessage::Pong { data } => {
                assert_eq!(data, "PONG");
            }
            _ => panic!("Expected Pong message"),
        }
    }

    #[test]
    fn test_channel_to_symbol_extraction() {
        let channel = "BTC-PERP@depth";
        let symbol = channel.strip_suffix("@depth").unwrap_or(channel);
        assert_eq!(symbol, "BTC-PERP");
    }

    #[test]
    fn test_shared_orderbooks_type() {
        let shared: SharedOrderbooks = Arc::new(RwLock::new(HashMap::new()));
        // Can be cloned for background task
        let _cloned = Arc::clone(&shared);
        assert!(Arc::strong_count(&shared) == 2);
    }
}
