//! Shared test utilities for adapter testing
//!
//! Provides a configurable `TestMockAdapter` that consolidates duplicated mock
//! implementations from `traits.rs` and `execution.rs` test modules.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::adapters::errors::{ExchangeError, ExchangeResult};
use crate::adapters::traits::ExchangeAdapter;
use crate::adapters::types::{
    OrderRequest, OrderResponse, OrderStatus, Orderbook, PositionInfo,
};

/// Unified mock adapter for testing — replaces per-module duplicates
///
/// Configurable to support both connection-state tests (stale flag, reconnect
/// tracking, orderbook storage) and execution tests (should_fail, order counting).
pub struct TestMockAdapter {
    /// Whether the adapter is "connected"
    pub connected: bool,
    /// When true, `place_order` returns an error
    pub should_fail: bool,
    /// Counter for how many orders were placed (thread-safe for async tests)
    pub order_count: Arc<AtomicU64>,
    /// Exchange name returned by `exchange_name()`
    pub name: &'static str,
    /// In-memory orderbooks keyed by symbol
    pub orderbooks: HashMap<String, Orderbook>,
    /// List of subscribed symbols
    pub subscriptions: Vec<String>,
    /// Explicit stale flag for testing `is_stale()`
    pub is_stale_flag: bool,
    /// Counter for `reconnect()` calls
    pub reconnect_count: usize,
}

impl TestMockAdapter {
    /// Create a new mock adapter with the given exchange name
    pub fn new(name: &'static str) -> Self {
        Self {
            connected: true,
            should_fail: false,
            order_count: Arc::new(AtomicU64::new(0)),
            name,
            orderbooks: HashMap::new(),
            subscriptions: Vec::new(),
            is_stale_flag: false,
            reconnect_count: 0,
        }
    }

    /// Create a mock that always fails on `place_order`
    pub fn with_failure(name: &'static str) -> Self {
        let mut mock = Self::new(name);
        mock.should_fail = true;
        mock
    }

    /// Set the stale flag for testing `is_stale()`
    pub fn set_stale(&mut self, stale: bool) {
        self.is_stale_flag = stale;
    }

    /// Get the number of times `reconnect()` was called
    pub fn reconnect_call_count(&self) -> usize {
        self.reconnect_count
    }
}

impl Default for TestMockAdapter {
    fn default() -> Self {
        Self::new("mock")
    }
}

#[async_trait]
impl ExchangeAdapter for TestMockAdapter {
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
        self.orderbooks
            .insert(symbol.to_string(), Orderbook::default());
        Ok(())
    }

    async fn unsubscribe_orderbook(&mut self, symbol: &str) -> ExchangeResult<()> {
        self.subscriptions.retain(|s| s != symbol);
        self.orderbooks.remove(symbol);
        Ok(())
    }

    async fn place_order(&self, order: OrderRequest) -> ExchangeResult<OrderResponse> {
        self.order_count.fetch_add(1, Ordering::Relaxed);

        // Small simulated latency for parallel-execution tests
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        if self.should_fail {
            return Err(ExchangeError::OrderRejected("Mock failure".to_string()));
        }

        Ok(OrderResponse {
            order_id: format!("{}-{}", self.name, order.client_order_id),
            client_order_id: order.client_order_id,
            status: OrderStatus::Filled,
            filled_quantity: order.quantity,
            avg_price: Some(42000.0),
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
        self.is_stale_flag
    }

    async fn sync_orderbooks(&mut self) {
        // No-op for mock — orderbooks are directly written
    }

    async fn reconnect(&mut self) -> ExchangeResult<()> {
        self.reconnect_count += 1;
        self.connect().await
    }

    async fn get_position(&self, _symbol: &str) -> ExchangeResult<Option<PositionInfo>> {
        Ok(None)
    }

    fn exchange_name(&self) -> &'static str {
        self.name
    }

    fn get_shared_orderbooks(&self) -> crate::core::channels::SharedOrderbooks {
        // Tests don't use shared orderbooks — return fresh empty one
        std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()))
    }

    fn get_shared_best_prices(&self) -> crate::core::channels::SharedBestPrices {
        std::sync::Arc::new(crate::core::channels::AtomicBestPrices::new())
    }

    fn set_orderbook_notify(&mut self, _notify: crate::core::channels::OrderbookNotify) {
        // No-op for tests
    }
}
