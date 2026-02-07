//! Exchange adapter trait definition
//!
//! The ExchangeAdapter trait defines the common interface that all
//! exchange adapters must implement for consistent behavior.

use async_trait::async_trait;


use crate::adapters::errors::ExchangeResult;
use crate::adapters::types::{Orderbook, OrderRequest, OrderResponse, PositionInfo};



/// Common trait for all exchange adapters
///
/// This trait defines the interface for connecting to exchanges,
/// subscribing to market data, and placing orders.
///
/// # Example Implementation
///
/// ```ignore
/// use async_trait::async_trait;
///
/// struct VestAdapter {
///     connected: bool,
///     orderbooks: HashMap<String, Orderbook>,
/// }
///
/// #[async_trait]
/// impl ExchangeAdapter for VestAdapter {
///     async fn connect(&mut self) -> ExchangeResult<()> {
///         // EIP-712 authentication with ethers-rs
///         self.connected = true;
///         Ok(())
///     }
///     // ... other methods
/// }
/// ```
#[async_trait]
pub trait ExchangeAdapter: Send + Sync {
    /// Establish WebSocket connection and authenticate with the exchange
    ///
    /// This method should:
    /// 1. Open WebSocket connection to exchange endpoint
    /// 2. Perform authentication (EIP-712 for Vest, Starknet for Paradex, etc.)
    /// 3. Set internal connected state
    async fn connect(&mut self) -> ExchangeResult<()>;

    /// Gracefully disconnect from the exchange
    ///
    /// Should properly close WebSocket connection and cleanup resources
    async fn disconnect(&mut self) -> ExchangeResult<()>;

    /// Subscribe to orderbook updates for a trading symbol
    ///
    /// # Arguments
    /// * `symbol` - Trading pair symbol (e.g., "BTC-PERP")
    ///
    /// After subscription, orderbook updates will be available via
    /// `get_orderbook()` and/or `orderbook_stream()`.
    async fn subscribe_orderbook(&mut self, symbol: &str) -> ExchangeResult<()>;

    /// Unsubscribe from orderbook updates
    ///
    /// # Arguments
    /// * `symbol` - Trading pair symbol to unsubscribe from
    async fn unsubscribe_orderbook(&mut self, symbol: &str) -> ExchangeResult<()>;

    /// Place an order on the exchange
    ///
    /// # Arguments
    /// * `order` - Order request containing all order parameters
    ///
    /// # Returns
    /// Order response with exchange-assigned order ID and fill status
    async fn place_order(&self, order: OrderRequest) -> ExchangeResult<OrderResponse>;

    /// Cancel an existing order
    ///
    /// # Arguments
    /// * `order_id` - Exchange-assigned order ID to cancel
    async fn cancel_order(&self, order_id: &str) -> ExchangeResult<()>;

    /// Get cached orderbook for a symbol (synchronous read)
    ///
    /// This returns the latest cached orderbook snapshot.
    /// Returns None if symbol is not subscribed or no data available.
    ///
    /// # Arguments
    /// * `symbol` - Trading pair symbol
    fn get_orderbook(&self, symbol: &str) -> Option<&Orderbook>;

    /// Check if adapter is currently connected to the exchange
    fn is_connected(&self) -> bool;
    
    /// Check if connection is stale (no data received in last 30 seconds)
    /// 
    /// A stale connection indicates that while the WebSocket may still be
    /// technically connected, no data has been received recently, which
    /// suggests the connection may be unhealthy.
    /// 
    /// # Story 2.6: Heartbeat Monitoring
    fn is_stale(&self) -> bool;
    
    /// Sync local orderbook cache from shared storage
    /// 
    /// This method copies orderbooks from the shared arc-rwlock storage
    /// (updated by background reader) to the local cache (used by get_orderbook).
    /// Call this before get_orderbook to ensure up-to-date data.
    /// 
    /// # Story 10.3: Required for spread calculation with dual adapters
    async fn sync_orderbooks(&mut self);
    
    /// Attempt to reconnect to the exchange
    /// 
    /// This method should:
    /// 1. Clean up existing connection resources
    /// 2. Re-establish WebSocket connection
    /// 3. Re-authenticate if necessary
    /// 4. Re-subscribe to all previously subscribed symbols
    /// 
    /// # Story 2.6: Auto-Reconnect
    async fn reconnect(&mut self) -> ExchangeResult<()>;

    /// Get current position for a symbol (Story 5.3 - Reconciliation)
    ///
    /// Fetches position data from the exchange via REST API.
    /// Returns None if no position exists for the symbol.
    ///
    /// # Arguments
    /// * `symbol` - Trading pair symbol (e.g., "BTC-PERP")
    ///
    /// # Returns
    /// * `Ok(Some(PositionInfo))` - Position exists
    /// * `Ok(None)` - No position for this symbol
    /// * `Err(...)` - Exchange error occurred
    async fn get_position(&self, symbol: &str) -> ExchangeResult<Option<PositionInfo>>;

    /// Get the exchange name identifier
    ///
    /// Returns a static string like "vest", "paradex", "hyperliquid", etc.
    fn exchange_name(&self) -> &'static str;
}

#[cfg(any(test, doc))]
pub mod tests {
    use super::*;
    use crate::adapters::types::{OrderSide, OrderStatus};

    // Use shared TestMockAdapter (CR-4) — alias for minimal test changes
    use crate::adapters::test_utils::TestMockAdapter;

    /// Create a disconnected mock adapter (matching old MockAdapter::new() behaviour)
    fn new_mock() -> TestMockAdapter {
        let mut m = TestMockAdapter::default();
        m.connected = false; // traits.rs tests expect disconnected on new()
        m
    }

    #[tokio::test]
    async fn test_mock_adapter_connect() {
        let mut adapter = new_mock();
        assert!(!adapter.is_connected());

        adapter.connect().await.unwrap();
        assert!(adapter.is_connected());
    }

    #[tokio::test]
    async fn test_mock_adapter_disconnect() {
        let mut adapter = new_mock();
        adapter.connect().await.unwrap();
        adapter.disconnect().await.unwrap();
        assert!(!adapter.is_connected());
    }

    #[tokio::test]
    async fn test_mock_adapter_subscribe_orderbook() {
        let mut adapter = new_mock();
        adapter.connect().await.unwrap();
        adapter.subscribe_orderbook("BTC-PERP").await.unwrap();

        assert!(adapter.get_orderbook("BTC-PERP").is_some());
        assert!(adapter.get_orderbook("ETH-PERP").is_none());
    }

    #[tokio::test]
    async fn test_mock_adapter_unsubscribe_orderbook() {
        let mut adapter = new_mock();
        adapter.connect().await.unwrap();
        adapter.subscribe_orderbook("BTC-PERP").await.unwrap();
        adapter.unsubscribe_orderbook("BTC-PERP").await.unwrap();

        assert!(adapter.get_orderbook("BTC-PERP").is_none());
    }

    #[tokio::test]
    async fn test_mock_adapter_place_order() {
        let adapter = new_mock();
        let order = OrderRequest::ioc_limit(
            "test-order-1".to_string(),
            "BTC-PERP".to_string(),
            OrderSide::Buy,
            42000.0,
            0.1,
        );

        let response = adapter.place_order(order).await.unwrap();
        assert_eq!(response.client_order_id, "test-order-1");
        assert_eq!(response.status, OrderStatus::Filled);
        assert_eq!(response.filled_quantity, 0.1);
    }

    #[tokio::test]
    async fn test_mock_adapter_exchange_name() {
        let adapter = new_mock();
        assert_eq!(adapter.exchange_name(), "mock");
    }

    #[tokio::test]
    async fn test_mock_adapter_cancel_order() {
        let adapter = new_mock();
        // cancel_order should succeed even for non-existent orders in mock
        let result = adapter.cancel_order("non-existent-order").await;
        assert!(result.is_ok());
    }
}
