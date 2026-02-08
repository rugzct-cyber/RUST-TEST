//! Core data types for exchange adapters
//!
//! These types are used across all exchange adapters for consistent
//! orderbook representation and order management.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

// =============================================================================
// Shared Subscription ID Counter (Refactoring)
// =============================================================================

/// Global atomic counter for unique subscription IDs across all adapters
static GLOBAL_SUBSCRIPTION_ID: AtomicU64 = AtomicU64::new(1);

/// Get next unique subscription ID (shared across all adapters)
pub fn next_subscription_id() -> u64 {
    GLOBAL_SUBSCRIPTION_ID.fetch_add(1, Ordering::SeqCst)
}

// =============================================================================
// Shared HTTP Client Builder (Refactoring)
// =============================================================================

use std::time::Duration;

// =============================================================================
// HTTP Client Constants
// =============================================================================

/// HTTP request timeout (seconds)
const HTTP_TIMEOUT_SECS: u64 = 10;
/// Max idle connections per host in connection pool
const HTTP_POOL_MAX_IDLE: usize = 2;
/// How long idle connections stay in the pool (seconds)
const HTTP_POOL_IDLE_TIMEOUT_SECS: u64 = 60;
/// TCP keepalive interval (seconds)
const HTTP_TCP_KEEPALIVE_SECS: u64 = 30;

/// Create an optimized HTTP client for HFT operations
///
/// Connection pooling configured for latency optimization
pub fn create_http_client(exchange_name: &str) -> reqwest::Client {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
        .pool_max_idle_per_host(HTTP_POOL_MAX_IDLE)
        .pool_idle_timeout(Duration::from_secs(HTTP_POOL_IDLE_TIMEOUT_SECS))
        .tcp_keepalive(Duration::from_secs(HTTP_TCP_KEEPALIVE_SECS))
        .connect_timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    tracing::info!(
        phase = "init",
        exchange = %exchange_name,
        pool_max_idle = HTTP_POOL_MAX_IDLE,
        pool_idle_timeout_s = HTTP_POOL_IDLE_TIMEOUT_SECS,
        tcp_keepalive_s = HTTP_TCP_KEEPALIVE_SECS,
        "HTTP client configured"
    );
    client
}

/// Maximum number of orderbook levels (bids/asks) to retain after parsing
pub const MAX_ORDERBOOK_DEPTH: usize = 10;

/// Vest API recvWindow in milliseconds (time validity of signed requests)
pub const VEST_RECV_WINDOW_MS: u64 = 60_000;

/// WebSocket ping / health-check interval (seconds)
pub const WS_PING_INTERVAL_SECS: u64 = 30;

/// Threshold in milliseconds after which adapter data is considered stale (30 seconds)
pub const STALE_THRESHOLD_MS: u64 = 30_000;

/// How often to extend the Vest listen key (seconds) — well within 60-min TTL
pub const VEST_LISTEN_KEY_RENEWAL_SECS: u64 = 45 * 60;

// =============================================================================
// Connection Health Types
// =============================================================================

/// Connection state for WebSocket health monitoring
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ConnectionState {
    /// WebSocket is connected and healthy
    Connected,
    /// WebSocket is disconnected
    #[default]
    Disconnected,
    /// WebSocket is attempting to reconnect
    Reconnecting,
}

/// Shared connection health state for tracking WebSocket health
///
/// This struct contains atomic/lockable fields that can be shared
/// across tasks (heartbeat task, reader loop, adapter methods).
#[derive(Debug)]
pub struct ConnectionHealth {
    /// Current connection state (Connected, Disconnected, Reconnecting)
    pub state: Arc<RwLock<ConnectionState>>,
    /// Timestamp of last PONG received (Unix ms)
    pub last_pong: Arc<AtomicU64>,
    /// Timestamp of last data received (Unix ms) - any message counts
    pub last_data: Arc<AtomicU64>,
    /// Set to false when the WS reader loop exits (Close frame or error).
    /// Checked by is_stale() for immediate dead-connection detection.
    pub reader_alive: Arc<AtomicBool>,
}

impl ConnectionHealth {
    /// Create a new ConnectionHealth with default values
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(ConnectionState::Disconnected)),
            last_pong: Arc::new(AtomicU64::new(0)),
            last_data: Arc::new(AtomicU64::new(0)),
            reader_alive: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Clone the Arc references for sharing with other tasks
    pub fn clone_refs(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
            last_pong: Arc::clone(&self.last_pong),
            last_data: Arc::clone(&self.last_data),
            reader_alive: Arc::clone(&self.reader_alive),
        }
    }
}

impl Default for ConnectionHealth {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for ConnectionHealth {
    fn clone(&self) -> Self {
        self.clone_refs()
    }
}

// =============================================================================
// Orderbook Types
// =============================================================================

/// A single level in the orderbook (price + quantity)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrderbookLevel {
    /// Price at this level
    pub price: f64,
    /// Quantity available at this price
    pub quantity: f64,
}

impl OrderbookLevel {
    /// Create a new orderbook level
    pub fn new(price: f64, quantity: f64) -> Self {
        Self { price, quantity }
    }
}

/// Orderbook snapshot with bid and ask levels
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Orderbook {
    /// Bid levels sorted descending by price (best bid first)
    pub bids: Vec<OrderbookLevel>,
    /// Ask levels sorted ascending by price (best ask first)
    pub asks: Vec<OrderbookLevel>,
    /// Timestamp in Unix milliseconds
    pub timestamp: u64,
}

impl Orderbook {
    /// Create a new empty orderbook
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the best bid price (highest bid)
    pub fn best_bid(&self) -> Option<f64> {
        self.bids.first().map(|l| l.price)
    }

    /// Get the best ask price (lowest ask)
    pub fn best_ask(&self) -> Option<f64> {
        self.asks.first().map(|l| l.price)
    }

    /// Calculate mid price
    pub fn mid_price(&self) -> Option<f64> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => Some((bid + ask) / 2.0),
            _ => None,
        }
    }
}

/// Order side (buy or sell)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderSide {
    Buy,
    Sell,
}

/// Order type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderType {
    Limit,
    Market,
}

/// Time in force for orders
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeInForce {
    /// Immediate or Cancel - fill what you can, cancel rest
    Ioc,
    /// Good till Cancelled
    Gtc,
    /// Fill or Kill - fill completely or cancel entirely
    Fok,
}

/// Order request to be sent to exchange
#[derive(Debug, Clone)]
pub struct OrderRequest {
    /// Unique client-generated order ID for idempotence
    pub client_order_id: String,
    /// Trading symbol (e.g., "BTC-PERP")
    pub symbol: String,
    /// Buy or Sell
    pub side: OrderSide,
    /// Limit or Market
    pub order_type: OrderType,
    /// Price (required for Limit orders)
    pub price: Option<f64>,
    /// Quantity to trade
    pub quantity: f64,
    /// Time in force
    pub time_in_force: TimeInForce,
    /// Reduce only - if true, only reduces existing position (for closing)
    pub reduce_only: bool,
}

impl OrderRequest {
    /// Validate order request consistency
    /// Returns error message if invalid, None if valid
    pub fn validate(&self) -> Option<&'static str> {
        // Limit orders require a price
        if self.order_type == OrderType::Limit && self.price.is_none() {
            return Some("Limit orders require a price");
        }
        // Quantity must be positive
        if self.quantity <= 0.0 {
            return Some("Quantity must be positive");
        }
        // Price must be positive if specified
        if let Some(price) = self.price {
            if price <= 0.0 {
                return Some("Price must be positive");
            }
        }
        None
    }

    /// Create a new limit order request
    pub fn limit(
        client_order_id: String,
        symbol: String,
        side: OrderSide,
        price: f64,
        quantity: f64,
        time_in_force: TimeInForce,
    ) -> Self {
        Self {
            client_order_id,
            symbol,
            side,
            order_type: OrderType::Limit,
            price: Some(price),
            quantity,
            time_in_force,
            reduce_only: false, // New orders open positions by default
        }
    }

    /// Create a new IOC limit order (most common for HFT)
    pub fn ioc_limit(
        client_order_id: String,
        symbol: String,
        side: OrderSide,
        price: f64,
        quantity: f64,
    ) -> Self {
        Self::limit(
            client_order_id,
            symbol,
            side,
            price,
            quantity,
            TimeInForce::Ioc,
        )
    }
}

// =============================================================================
// Order Builder (Stage 6 Refactoring)
// =============================================================================

/// Builder for OrderRequest with sensible defaults for HFT
///
/// Defaults:
/// - `time_in_force`: `TimeInForce::Ioc` (Immediate-or-Cancel)
/// - `order_type`: `OrderType::Limit`
/// - `reduce_only`: `false`
/// - `client_order_id`: Empty (must be set)
///
/// # Examples
/// ```ignore
/// let order = OrderBuilder::new("BTC-PERP", OrderSide::Buy, 0.1)
///     .client_order_id("trade-123")
///     .limit(42000.0)
///     .build()
///     .expect("valid order");
/// ```
#[derive(Debug)]
pub struct OrderBuilder {
    symbol: String,
    side: OrderSide,
    quantity: f64,
    client_order_id: String,
    order_type: OrderType,
    price: Option<f64>,
    time_in_force: TimeInForce,
    reduce_only: bool,
}

impl OrderBuilder {
    /// Create a new OrderBuilder with required fields and HFT defaults
    ///
    /// Defaults:
    /// - `time_in_force`: `Ioc`
    /// - `order_type`: `Limit`
    /// - `reduce_only`: `false`
    pub fn new(symbol: &str, side: OrderSide, quantity: f64) -> Self {
        Self {
            symbol: symbol.to_string(),
            side,
            quantity,
            client_order_id: String::new(),
            order_type: OrderType::Limit,
            price: None,
            time_in_force: TimeInForce::Ioc,
            reduce_only: false,
        }
    }

    /// Set the client order ID (required for valid orders)
    pub fn client_order_id(mut self, id: impl Into<String>) -> Self {
        self.client_order_id = id.into();
        self
    }

    /// Set order type to Market (no price needed)
    pub fn market(mut self) -> Self {
        self.order_type = OrderType::Market;
        self
    }

    /// Set order type to Limit with specified price
    pub fn limit(mut self, price: f64) -> Self {
        self.order_type = OrderType::Limit;
        self.price = Some(price);
        self
    }

    /// Set price without changing order type
    ///
    /// # Warning: Exchange-Specific Behavior
    /// This is primarily for **Vest MARKET orders** which require a `limitPrice`
    /// as slippage protection. Most exchanges don't support price on MARKET orders.
    ///
    /// For standard limit orders, prefer `.limit(price)` instead.
    ///
    /// # Example (Vest slippage protection)
    /// ```ignore
    /// OrderBuilder::new("BTC-PERP", OrderSide::Buy, 0.1)
    ///     .client_order_id("vest-123")
    ///     .market()
    ///     .price(42100.0)  // Slippage ceiling for Vest
    ///     .build()
    /// ```
    pub fn price(mut self, price: f64) -> Self {
        self.price = Some(price);
        self
    }

    /// Set reduce_only to true (for closing positions)
    pub fn reduce_only(mut self) -> Self {
        self.reduce_only = true;
        self
    }

    /// Build the OrderRequest with validation
    ///
    /// # Errors
    /// - `"client_order_id is required"` if client_order_id is empty
    /// - `"Limit orders require a price"` if order_type is Limit but no price set
    /// - `"Quantity must be positive"` if quantity <= 0
    /// - `"Price must be positive"` if price is set but <= 0
    pub fn build(self) -> Result<OrderRequest, &'static str> {
        // Red Team hardening: validate client_order_id
        if self.client_order_id.is_empty() {
            return Err("client_order_id is required");
        }

        let order = OrderRequest {
            client_order_id: self.client_order_id,
            symbol: self.symbol,
            side: self.side,
            order_type: self.order_type,
            price: self.price,
            quantity: self.quantity,
            time_in_force: self.time_in_force,
            reduce_only: self.reduce_only,
        };

        // Red Team hardening: run OrderRequest validation
        if let Some(err) = order.validate() {
            return Err(err);
        }

        Ok(order)
    }
}

/// Order status from exchange
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderStatus {
    Pending,
    Filled,
    PartiallyFilled,
    Cancelled,
    Rejected,
}

/// Order response from exchange
#[derive(Debug, Clone)]
pub struct OrderResponse {
    /// Exchange-assigned order ID
    pub order_id: String,
    /// Client-generated order ID (for matching)
    pub client_order_id: String,
    /// Current status
    pub status: OrderStatus,
    /// Quantity that was filled
    pub filled_quantity: f64,
    /// Average fill price (if any fills occurred)
    pub avg_price: Option<f64>,
}

/// Orderbook update event for streaming
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderbookUpdate {
    /// Trading symbol
    pub symbol: String,
    /// Exchange identifier (e.g., "vest", "paradex")
    pub exchange: String,
    /// Updated orderbook snapshot
    pub orderbook: Orderbook,
}

// =============================================================================
// Position Types
// =============================================================================

/// Position data fetched from an exchange
///
/// Used by the reconciliation loop to compare local state with exchange state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionInfo {
    /// Trading pair symbol
    pub symbol: String,
    /// Position quantity in base units (unsigned)
    pub quantity: f64,
    /// Position side: "long" or "short"
    pub side: String,
    /// Entry price
    pub entry_price: f64,
    /// Current mark/market price (if available from exchange)
    pub mark_price: Option<f64>,
    /// Unrealized PnL
    pub unrealized_pnl: f64,
}

/// Fill information returned by exchange APIs after order execution
///
/// Contains the actual fill price, realized PnL, and fee directly from the
/// exchange. This avoids manual PnL calculation from unreliable `avg_price` values.
#[derive(Debug, Clone)]
pub struct FillInfo {
    /// Actual execution price
    pub fill_price: f64,
    /// Exchange-reported realized PnL (includes funding and fees on Vest)
    /// `None` if the exchange doesn't provide this field (e.g. Paradex fills)
    pub realized_pnl: Option<f64>,
    /// Trading fee
    pub fee: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_orderbook_level_creation() {
        let level = OrderbookLevel::new(42150.5, 1.5);
        assert_eq!(level.price, 42150.5);
        assert_eq!(level.quantity, 1.5);
    }

    #[test]
    fn test_orderbook_level_serialization() {
        let level = OrderbookLevel::new(42150.5, 1.5);
        let json = serde_json::to_string(&level).unwrap();
        assert!(json.contains("42150.5"));

        let deserialized: OrderbookLevel = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.price, level.price);
        assert_eq!(deserialized.quantity, level.quantity);
    }

    #[test]
    fn test_orderbook_best_prices() {
        let mut ob = Orderbook::new();
        ob.bids = vec![
            OrderbookLevel::new(42100.0, 1.0),
            OrderbookLevel::new(42050.0, 2.0),
        ];
        ob.asks = vec![
            OrderbookLevel::new(42150.0, 1.5),
            OrderbookLevel::new(42200.0, 2.0),
        ];

        assert_eq!(ob.best_bid(), Some(42100.0));
        assert_eq!(ob.best_ask(), Some(42150.0));
        assert_eq!(ob.mid_price(), Some(42125.0));
    }

    #[test]
    fn test_orderbook_empty() {
        let ob = Orderbook::new();
        assert_eq!(ob.best_bid(), None);
        assert_eq!(ob.best_ask(), None);
        assert_eq!(ob.mid_price(), None);
    }

    #[test]
    fn test_order_request_construction() {
        let order = OrderRequest {
            client_order_id: "test-123".to_string(),
            symbol: "BTC-PERP".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            price: Some(42000.0),
            quantity: 0.1,
            time_in_force: TimeInForce::Ioc,
            reduce_only: false,
        };
        assert_eq!(order.symbol, "BTC-PERP");
        assert_eq!(order.side, OrderSide::Buy);
        assert_eq!(order.order_type, OrderType::Limit);
    }

    #[test]
    fn test_ioc_limit_helper() {
        let order = OrderRequest::ioc_limit(
            "order-456".to_string(),
            "ETH-PERP".to_string(),
            OrderSide::Sell,
            2800.0,
            1.0,
        );
        assert_eq!(order.client_order_id, "order-456");
        assert_eq!(order.symbol, "ETH-PERP");
        assert_eq!(order.side, OrderSide::Sell);
        assert_eq!(order.order_type, OrderType::Limit);
        assert_eq!(order.price, Some(2800.0));
        assert_eq!(order.time_in_force, TimeInForce::Ioc);
    }

    #[test]
    fn test_order_status_values() {
        assert_eq!(OrderStatus::Pending, OrderStatus::Pending);
        assert_ne!(OrderStatus::Filled, OrderStatus::Cancelled);
    }

    #[test]
    fn test_order_request_validate_limit_without_price() {
        let order = OrderRequest {
            client_order_id: "test-123".to_string(),
            symbol: "BTC-PERP".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            price: None, // Invalid for Limit!
            quantity: 0.1,
            time_in_force: TimeInForce::Ioc,
            reduce_only: false,
        };
        assert_eq!(order.validate(), Some("Limit orders require a price"));
    }

    #[test]
    fn test_order_request_validate_negative_quantity() {
        let order = OrderRequest {
            client_order_id: "test-123".to_string(),
            symbol: "BTC-PERP".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Market,
            price: None,
            quantity: -0.1,
            time_in_force: TimeInForce::Ioc,
            reduce_only: false,
        };
        assert_eq!(order.validate(), Some("Quantity must be positive"));
    }

    #[test]
    fn test_order_request_validate_valid_order() {
        let order = OrderRequest::ioc_limit(
            "test-123".to_string(),
            "BTC-PERP".to_string(),
            OrderSide::Buy,
            42000.0,
            0.1,
        );
        assert_eq!(order.validate(), None);
    }

    #[test]
    fn test_orderbook_serialization() {
        let mut ob = Orderbook::new();
        ob.bids = vec![OrderbookLevel::new(42100.0, 1.0)];
        ob.asks = vec![OrderbookLevel::new(42150.0, 1.5)];
        ob.timestamp = 1706000000000;

        let json = serde_json::to_string(&ob).unwrap();
        assert!(json.contains("42100"));
        assert!(json.contains("42150"));

        let deserialized: Orderbook = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.bids.len(), 1);
        assert_eq!(deserialized.asks.len(), 1);
    }

    #[test]
    fn test_orderbook_update_serialization() {
        let update = OrderbookUpdate {
            symbol: "BTC-PERP".to_string(),
            exchange: "vest".to_string(),
            orderbook: Orderbook::new(),
        };
        let json = serde_json::to_string(&update).unwrap();
        assert!(json.contains("BTC-PERP"));
    }

    #[test]
    fn test_orderbook_only_bids() {
        let mut ob = Orderbook::new();
        ob.bids = vec![OrderbookLevel::new(42100.0, 1.0)];
        // No asks
        assert_eq!(ob.best_bid(), Some(42100.0));
        assert_eq!(ob.best_ask(), None);
        assert_eq!(ob.mid_price(), None); // Can't calculate mid without both
    }

    #[test]
    fn test_orderbook_only_asks() {
        let mut ob = Orderbook::new();
        // No bids
        ob.asks = vec![OrderbookLevel::new(42150.0, 1.5)];
        assert_eq!(ob.best_bid(), None);
        assert_eq!(ob.best_ask(), Some(42150.0));
        assert_eq!(ob.mid_price(), None); // Can't calculate mid without both
    }

    // =========================================================================
    // Connection Health Tests
    // =========================================================================

    #[test]
    fn test_connection_state_default() {
        let state = ConnectionState::default();
        assert_eq!(state, ConnectionState::Disconnected);
    }

    #[test]
    fn test_connection_state_equality() {
        assert_eq!(ConnectionState::Connected, ConnectionState::Connected);
        assert_ne!(ConnectionState::Connected, ConnectionState::Disconnected);
        assert_ne!(ConnectionState::Reconnecting, ConnectionState::Connected);
    }

    #[test]
    fn test_connection_state_serialization() {
        let state = ConnectionState::Connected;
        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains("Connected"));

        let deserialized: ConnectionState = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, ConnectionState::Connected);
    }

    #[test]
    fn test_connection_health_new() {
        let health = ConnectionHealth::new();
        // Initial state should be Disconnected
        let state = health.state.try_read().unwrap();
        assert_eq!(*state, ConnectionState::Disconnected);
        drop(state);

        // Initial timestamps should be 0
        assert_eq!(
            health.last_pong.load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        assert_eq!(
            health.last_data.load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    #[test]
    fn test_connection_health_clone_refs() {
        use std::sync::atomic::Ordering;

        let health = ConnectionHealth::new();
        let cloned = health.clone_refs();

        // Modify through original
        health.last_pong.store(12345, Ordering::Relaxed);

        // Should be visible through clone (same Arc)
        assert_eq!(cloned.last_pong.load(Ordering::Relaxed), 12345);
    }

    #[test]
    fn test_connection_health_default() {
        let health = ConnectionHealth::default();
        let state = health.state.try_read().unwrap();
        assert_eq!(*state, ConnectionState::Disconnected);
    }

    // =========================================================================
    // Heartbeat Monitoring Tests
    // =========================================================================

    #[test]
    fn test_stale_detection_threshold_30_seconds() {
        use std::sync::atomic::Ordering;
        use std::time::{SystemTime, UNIX_EPOCH};

        let health = ConnectionHealth::new();

        // Get current time in ms
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        // Set last_data to 31 seconds ago (should be stale)
        health
            .last_data
            .store(now_ms.saturating_sub(31_000), Ordering::Relaxed);

        let last_data = health.last_data.load(Ordering::Relaxed);
        let elapsed = now_ms.saturating_sub(last_data);
        const STALE_THRESHOLD_MS: u64 = 30_000;

        assert!(elapsed > STALE_THRESHOLD_MS, "31s old data should be stale");

        // Set last_data to 29 seconds ago (should NOT be stale)
        health
            .last_data
            .store(now_ms.saturating_sub(29_000), Ordering::Relaxed);
        let last_data = health.last_data.load(Ordering::Relaxed);
        let elapsed = now_ms.saturating_sub(last_data);

        assert!(
            elapsed < STALE_THRESHOLD_MS,
            "29s old data should NOT be stale"
        );
    }

    #[test]
    fn test_connection_state_transitions() {
        use std::sync::atomic::Ordering;

        let health = ConnectionHealth::new();

        // Initial state should be Disconnected
        {
            let state = health.state.try_read().unwrap();
            assert_eq!(*state, ConnectionState::Disconnected);
        }

        // Simulate state transitions like reconnect() does
        {
            let mut state = health.state.try_write().unwrap();
            *state = ConnectionState::Reconnecting;
        }
        {
            let state = health.state.try_read().unwrap();
            assert_eq!(*state, ConnectionState::Reconnecting);
        }

        {
            let mut state = health.state.try_write().unwrap();
            *state = ConnectionState::Connected;
        }
        {
            let state = health.state.try_read().unwrap();
            assert_eq!(*state, ConnectionState::Connected);
        }

        // Test that timestamps can be updated during transitions
        health.last_pong.store(12345, Ordering::Relaxed);
        health.last_data.store(67890, Ordering::Relaxed);
        assert_eq!(health.last_pong.load(Ordering::Relaxed), 12345);
        assert_eq!(health.last_data.load(Ordering::Relaxed), 67890);
    }

    #[test]
    fn test_exponential_backoff_timing() {
        // Test the exponential backoff formula used in reconnect()
        // Formula: min(500 * 2^attempt, 5000)
        // Expected: 500ms, 1000ms, 2000ms, 4000ms, 5000ms (capped)

        let backoffs: Vec<u64> = (0..5)
            .map(|attempt| std::cmp::min(500 * (1u64 << attempt), 5000))
            .collect();

        assert_eq!(backoffs[0], 500, "Attempt 0: 500ms");
        assert_eq!(backoffs[1], 1000, "Attempt 1: 1000ms");
        assert_eq!(backoffs[2], 2000, "Attempt 2: 2000ms");
        assert_eq!(backoffs[3], 4000, "Attempt 3: 4000ms");
        assert_eq!(backoffs[4], 5000, "Attempt 4: capped at 5000ms");

        // Verify cap works for higher attempts
        let attempt_10 = std::cmp::min(500 * (1u64 << 10), 5000);
        assert_eq!(attempt_10, 5000, "High attempt should cap at 5000ms");
    }

    // =========================================================================
    // Reader Alive Flag Tests (S-1/S-2 Stability)
    // =========================================================================

    #[test]
    fn test_reader_alive_flag_default() {
        use std::sync::atomic::Ordering;
        let health = ConnectionHealth::new();
        // reader_alive should be false by default (reader not started yet)
        assert!(!health.reader_alive.load(Ordering::Relaxed));
    }

    #[test]
    fn test_reader_alive_shared_across_clones() {
        use std::sync::atomic::Ordering;
        let health = ConnectionHealth::new();
        let cloned = health.clone_refs();

        // Set via original, read via clone
        health.reader_alive.store(true, Ordering::Relaxed);
        assert!(cloned.reader_alive.load(Ordering::Relaxed));

        // Set via clone, read via original
        cloned.reader_alive.store(false, Ordering::Relaxed);
        assert!(!health.reader_alive.load(Ordering::Relaxed));
    }

    #[test]
    fn test_reader_alive_transitions() {
        use std::sync::atomic::Ordering;
        let health = ConnectionHealth::new();

        // Simulate reader startup
        health.reader_alive.store(true, Ordering::Relaxed);
        assert!(health.reader_alive.load(Ordering::Relaxed));

        // Simulate reader death (Close frame or error)
        health.reader_alive.store(false, Ordering::Relaxed);
        assert!(!health.reader_alive.load(Ordering::Relaxed));
    }

    // =========================================================================
    // OrderBuilder Tests (Stage 6 Refactoring)
    // =========================================================================

    #[test]
    fn test_order_builder_happy_path() {
        let order = OrderBuilder::new("BTC-PERP", OrderSide::Buy, 0.1)
            .client_order_id("test-123")
            .limit(42000.0)
            .build();

        assert!(order.is_ok());
        let order = order.unwrap();
        assert_eq!(order.symbol, "BTC-PERP");
        assert_eq!(order.side, OrderSide::Buy);
        assert_eq!(order.quantity, 0.1);
        assert_eq!(order.client_order_id, "test-123");
        assert_eq!(order.order_type, OrderType::Limit);
        assert_eq!(order.price, Some(42000.0));
        assert_eq!(order.time_in_force, TimeInForce::Ioc);
        assert!(!order.reduce_only);
    }

    #[test]
    fn test_order_builder_missing_client_order_id() {
        let result = OrderBuilder::new("BTC-PERP", OrderSide::Buy, 0.1)
            .limit(42000.0)
            .build();

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "client_order_id is required");
    }

    #[test]
    fn test_order_builder_limit_without_price() {
        // OrderBuilder defaults to Limit, so if we don't call .limit() or .price(),
        // the order_type is Limit but price is None - this should fail validation
        let result = OrderBuilder::new("BTC-PERP", OrderSide::Buy, 0.1)
            .client_order_id("test-123")
            .build();

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Limit orders require a price");
    }

    #[test]
    fn test_order_builder_market_order() {
        let order = OrderBuilder::new("BTC-PERP", OrderSide::Sell, 0.5)
            .client_order_id("market-456")
            .market()
            .build();

        assert!(order.is_ok());
        let order = order.unwrap();
        assert_eq!(order.order_type, OrderType::Market);
        assert_eq!(order.side, OrderSide::Sell);
        assert_eq!(order.quantity, 0.5);
    }

    #[test]
    fn test_order_builder_reduce_only() {
        let order = OrderBuilder::new("ETH-PERP", OrderSide::Buy, 1.0)
            .client_order_id("close-789")
            .market()
            .reduce_only()
            .build();

        assert!(order.is_ok());
        let order = order.unwrap();
        assert!(order.reduce_only);
        assert_eq!(order.order_type, OrderType::Market);
    }

    #[test]
    fn test_order_builder_market_with_price_slippage() {
        // Test Vest-style: MARKET order with price as slippage protection
        let order = OrderBuilder::new("BTC-PERP", OrderSide::Buy, 0.1)
            .client_order_id("vest-123")
            .market()
            .price(42100.0) // Slippage protection price
            .build();

        assert!(order.is_ok());
        let order = order.unwrap();
        assert_eq!(order.order_type, OrderType::Market);
        assert_eq!(order.price, Some(42100.0));
    }
}
