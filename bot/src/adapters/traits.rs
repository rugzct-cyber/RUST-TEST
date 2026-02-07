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
    use std::collections::HashMap;
    use crate::adapters::types::{OrderSide, OrderStatus};

    /// Mock adapter for testing trait implementation
    pub struct MockAdapter {
        connected: bool,
        orderbooks: HashMap<String, Orderbook>,
        subscriptions: Vec<String>,
        is_stale_flag: bool,
        reconnect_count: usize,
    }

    impl MockAdapter {
        pub fn new() -> Self {
            Self {
                connected: false,
                orderbooks: HashMap::new(),
                subscriptions: Vec::new(),
                is_stale_flag: false,
                reconnect_count: 0,
            }
        }
        
        /// Set the stale flag for testing purposes
        pub fn set_stale(&mut self, stale: bool) {
            self.is_stale_flag = stale;
        }
        
        /// Get the number of times reconnect() was called
        pub fn reconnect_call_count(&self) -> usize {
            self.reconnect_count
        }
    }

    impl Default for MockAdapter {
        fn default() -> Self {
            Self::new()
        }
    }

    #[async_trait]
    impl ExchangeAdapter for MockAdapter {
        async fn connect(&mut self) -> ExchangeResult<()> {
            self.connected = true;
            Ok(())
        }

        async fn disconnect(&mut self) -> ExchangeResult<()> {
            self.connected = false;
            self.subscriptions.clear();
            Ok(())
        }

        async fn subscribe_orderbook(&mut self, symbol: &str) -> ExchangeResult<()> {
            self.subscriptions.push(symbol.to_string());
            self.orderbooks.insert(symbol.to_string(), Orderbook::default());
            Ok(())
        }

        async fn unsubscribe_orderbook(&mut self, symbol: &str) -> ExchangeResult<()> {
            self.subscriptions.retain(|s| s != symbol);
            self.orderbooks.remove(symbol);
            Ok(())
        }

        async fn place_order(&self, order: OrderRequest) -> ExchangeResult<OrderResponse> {
            Ok(OrderResponse {
                order_id: format!("mock-{}", order.client_order_id),
                client_order_id: order.client_order_id,
                status: OrderStatus::Filled,
                filled_quantity: order.quantity,
                avg_price: order.price,
            })
        }

        async fn cancel_order(&self, _order_id: &str) -> ExchangeResult<()> {
            Ok(())
        }

        fn get_orderbook(&self, symbol: &str) -> Option<&Orderbook> {
            self.orderbooks.get(symbol)
        }

        fn is_connected(&self) -> bool {
            self.connected
        }
        
        fn is_stale(&self) -> bool {
            // Use explicit stale flag for testing
            self.is_stale_flag
        }
        
        async fn sync_orderbooks(&mut self) {
            // No-op for mock - orderbooks are directly written
        }
        
        async fn reconnect(&mut self) -> ExchangeResult<()> {
            // Increment counter for testing
            self.reconnect_count += 1;
            // Mock reconnect calls connect
            self.connect().await
        }

        async fn get_position(&self, _symbol: &str) -> ExchangeResult<Option<PositionInfo>> {
            // Mock adapter returns no position by default
            Ok(None)
        }

        fn exchange_name(&self) -> &'static str {
            "mock"
        }
    }

    #[tokio::test]
    async fn test_mock_adapter_connect() {
        let mut adapter = MockAdapter::new();
        assert!(!adapter.is_connected());

        adapter.connect().await.unwrap();
        assert!(adapter.is_connected());
    }

    #[tokio::test]
    async fn test_mock_adapter_disconnect() {
        let mut adapter = MockAdapter::new();
        adapter.connect().await.unwrap();
        adapter.disconnect().await.unwrap();
        assert!(!adapter.is_connected());
    }

    #[tokio::test]
    async fn test_mock_adapter_subscribe_orderbook() {
        let mut adapter = MockAdapter::new();
        adapter.connect().await.unwrap();
        adapter.subscribe_orderbook("BTC-PERP").await.unwrap();

        assert!(adapter.get_orderbook("BTC-PERP").is_some());
        assert!(adapter.get_orderbook("ETH-PERP").is_none());
    }

    #[tokio::test]
    async fn test_mock_adapter_unsubscribe_orderbook() {
        let mut adapter = MockAdapter::new();
        adapter.connect().await.unwrap();
        adapter.subscribe_orderbook("BTC-PERP").await.unwrap();
        adapter.unsubscribe_orderbook("BTC-PERP").await.unwrap();

        assert!(adapter.get_orderbook("BTC-PERP").is_none());
    }

    #[tokio::test]
    async fn test_mock_adapter_place_order() {
        let adapter = MockAdapter::new();
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
        let adapter = MockAdapter::new();
        assert_eq!(adapter.exchange_name(), "mock");
    }

    #[tokio::test]
    async fn test_mock_adapter_cancel_order() {
        let adapter = MockAdapter::new();
        // cancel_order should succeed even for non-existent orders in mock
        let result = adapter.cancel_order("non-existent-order").await;
        assert!(result.is_ok());
    }
}
