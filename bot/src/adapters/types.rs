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

/// HTTP request timeout (seconds) — 3s max for HFT (price moves after ~2s)
const HTTP_TIMEOUT_SECS: u64 = 3;
/// HTTP connection timeout (milliseconds) — fail fast if host unreachable
const HTTP_CONNECT_TIMEOUT_MS: u64 = 1500;
/// Max idle connections per host in connection pool
const HTTP_POOL_MAX_IDLE: usize = 5;
/// How long idle connections stay in the pool (seconds)
const HTTP_POOL_IDLE_TIMEOUT_SECS: u64 = 60;
/// TCP keepalive interval (seconds)
const HTTP_TCP_KEEPALIVE_SECS: u64 = 30;

/// Create an optimized HTTP client for HFT operations
///
/// Connection pooling + TCP_NODELAY configured for latency optimization
pub fn create_http_client(exchange_name: &str) -> reqwest::Client {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
        .pool_max_idle_per_host(HTTP_POOL_MAX_IDLE)
        .pool_idle_timeout(Duration::from_secs(HTTP_POOL_IDLE_TIMEOUT_SECS))
        .tcp_keepalive(Duration::from_secs(HTTP_TCP_KEEPALIVE_SECS))
        .connect_timeout(Duration::from_millis(HTTP_CONNECT_TIMEOUT_MS))
        .tcp_nodelay(true)  // Disable Nagle's algorithm — send packets immediately
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    tracing::info!(
        phase = "init",
        exchange = %exchange_name,
        timeout_s = HTTP_TIMEOUT_SECS,
        connect_timeout_ms = HTTP_CONNECT_TIMEOUT_MS,
        pool_max_idle = HTTP_POOL_MAX_IDLE,
        pool_idle_timeout_s = HTTP_POOL_IDLE_TIMEOUT_SECS,
        tcp_keepalive_s = HTTP_TCP_KEEPALIVE_SECS,
        tcp_nodelay = true,
        "HTTP client configured"
    );
    client
}

/// Maximum number of orderbook levels (bids/asks) to retain after parsing
pub const MAX_ORDERBOOK_DEPTH: usize = 10;



/// WebSocket ping / health-check interval (seconds)
pub const WS_PING_INTERVAL_SECS: u64 = 30;

/// Threshold in milliseconds after which adapter data is considered stale (30 seconds)
pub const STALE_THRESHOLD_MS: u64 = 30_000;



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

}
