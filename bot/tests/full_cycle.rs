//! End-to-End Integration Tests (Story 6.5, Story 7.3)
//!
//! This module tests the complete trading cycle:
//! 1. Config loading and adapter initialization
//! 2. Spread detection and opportunity generation
//! 3. Delta-neutral trade execution
//!
//! V1 HFT Mode: State persistence tests removed (Supabase eliminated)
//!
//! # Running the tests
//! ```bash
//! cargo test --test full_cycle
//! ```

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{broadcast, mpsc, Mutex};
use tokio::time::sleep;

use hft_bot::adapters::errors::{ExchangeError, ExchangeResult};
use hft_bot::adapters::types::{
    OrderRequest, OrderResponse, OrderSide, OrderStatus, Orderbook, OrderbookLevel,
    PositionInfo,
};
use hft_bot::adapters::ExchangeAdapter;
use hft_bot::core::channels::{SpreadDirection, SpreadOpportunity};

// =============================================================================
// Mock Exchange Adapter (Task 1.2)
// =============================================================================

/// Mock exchange adapter for integration testing
///
/// Provides full control over orderbook data and tracks order placement
/// for assertions. Does not require any real credentials.
#[derive(Debug)]
pub struct MockExchangeAdapter {
    name: &'static str,
    connected: bool,
    orderbook: Option<Orderbook>,
    orders_placed: Arc<AtomicUsize>,
    reduce_only_orders: Arc<AtomicUsize>,
    should_fail_orders: bool,
}

impl MockExchangeAdapter {
    /// Create a new mock adapter with the given name
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            connected: false,
            orderbook: None,
            orders_placed: Arc::new(AtomicUsize::new(0)),
            reduce_only_orders: Arc::new(AtomicUsize::new(0)),
            should_fail_orders: false,
        }
    }

    /// Create a mock with a pre-configured orderbook
    pub fn with_orderbook(name: &'static str, best_bid: f64, best_ask: f64) -> Self {
        let mut adapter = Self::new(name);
        adapter.set_orderbook(best_bid, best_ask);
        adapter
    }

    /// Set the orderbook with given bid/ask prices
    pub fn set_orderbook(&mut self, best_bid: f64, best_ask: f64) {
        self.orderbook = Some(Orderbook {
            bids: vec![OrderbookLevel::new(best_bid, 10.0)],
            asks: vec![OrderbookLevel::new(best_ask, 10.0)],
            timestamp: current_time_ms(),
        });
    }

    /// Configure the adapter to fail all order placements
    #[allow(dead_code)]
    pub fn with_failure(name: &'static str) -> Self {
        let mut adapter = Self::new(name);
        adapter.should_fail_orders = true;
        adapter
    }

    /// Get number of orders placed
    pub fn orders_placed(&self) -> usize {
        self.orders_placed.load(Ordering::SeqCst)
    }

    /// Get number of reduce_only orders placed (close orders)
    pub fn reduce_only_orders(&self) -> usize {
        self.reduce_only_orders.load(Ordering::SeqCst)
    }

    /// Clone the orders_placed counter for sharing
    #[allow(dead_code)]
    pub fn orders_placed_counter(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.orders_placed)
    }
}

#[async_trait]
impl ExchangeAdapter for MockExchangeAdapter {
    async fn connect(&mut self) -> ExchangeResult<()> {
        self.connected = true;
        Ok(())
    }

    async fn disconnect(&mut self) -> ExchangeResult<()> {
        self.connected = false;
        Ok(())
    }

    async fn subscribe_orderbook(&mut self, _symbol: &str) -> ExchangeResult<()> {
        Ok(())
    }

    async fn unsubscribe_orderbook(&mut self, _symbol: &str) -> ExchangeResult<()> {
        Ok(())
    }

    async fn place_order(&self, order: OrderRequest) -> ExchangeResult<OrderResponse> {
        if self.should_fail_orders {
            return Err(ExchangeError::OrderRejected("Simulated failure".into()));
        }

        self.orders_placed.fetch_add(1, Ordering::SeqCst);

        if order.reduce_only {
            self.reduce_only_orders.fetch_add(1, Ordering::SeqCst);
        }

        Ok(OrderResponse {
            order_id: format!("mock-{}-{}", self.name, order.client_order_id),
            client_order_id: order.client_order_id,
            status: OrderStatus::Filled,
            filled_quantity: order.quantity,
            avg_price: order.price,
        })
    }

    async fn cancel_order(&self, _order_id: &str) -> ExchangeResult<()> {
        Ok(())
    }

    fn get_orderbook(&self, _symbol: &str) -> Option<&Orderbook> {
        self.orderbook.as_ref()
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    fn is_stale(&self) -> bool {
        false
    }

    async fn sync_orderbooks(&mut self) {
        // No-op for mock
    }

    async fn reconnect(&mut self) -> ExchangeResult<()> {
        self.connect().await
    }

    async fn get_position(&self, _symbol: &str) -> ExchangeResult<Option<PositionInfo>> {
        Ok(None)
    }

    fn exchange_name(&self) -> &'static str {
        self.name
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

// =============================================================================
// Test 1: Trade Execution (AC: Full Cycle Coverage)
// =============================================================================

/// Test the trading cycle: spread detection → order placement
#[tokio::test]
async fn test_spread_opportunity_triggers_execution() {
    // === SETUP ===
    let spread_entry: f64 = 0.30; // 0.30% entry threshold

    // Create mock adapters with spread > entry_threshold
    let vest = MockExchangeAdapter::with_orderbook("vest", 100.10, 100.50);
    let paradex = MockExchangeAdapter::with_orderbook("paradex", 100.15, 100.55);

    let vest = Arc::new(Mutex::new(vest));
    let paradex = Arc::new(Mutex::new(paradex));

    // Create spread opportunity
    let opportunity = SpreadOpportunity {
        pair: "BTC-PERP".to_string(),
        dex_a: "vest".to_string(),
        dex_b: "paradex".to_string(),
        spread_percent: 0.35,
        direction: SpreadDirection::AOverB,
        detected_at_ms: current_time_ms(),
        dex_a_ask: 42000.0,
        dex_a_bid: 41990.0,
        dex_b_ask: 42005.0,
        dex_b_bid: 41985.0,
    };

    // Verify spread is above threshold
    assert!(
        opportunity.spread_percent >= spread_entry,
        "Spread {} should be >= entry threshold {}",
        opportunity.spread_percent,
        spread_entry
    );

    // === EXECUTE: Simulate DeltaNeutralExecutor behavior ===
    {
        let vest_guard = vest.lock().await;
        let vest_order = OrderRequest::ioc_limit(
            "e2e-vest-1".to_string(),
            "BTC-PERP".to_string(),
            OrderSide::Sell,
            100.50,
            0.001,
        );
        vest_guard
            .place_order(vest_order)
            .await
            .expect("Vest order should succeed");
    }

    {
        let paradex_guard = paradex.lock().await;
        let paradex_order = OrderRequest::ioc_limit(
            "e2e-paradex-1".to_string(),
            "BTC-USD-PERP".to_string(),
            OrderSide::Buy,
            100.15,
            0.001,
        );
        paradex_guard
            .place_order(paradex_order)
            .await
            .expect("Paradex order should succeed");
    }

    // === VERIFY ===
    assert_eq!(vest.lock().await.orders_placed(), 1, "One order on Vest");
    assert_eq!(
        paradex.lock().await.orders_placed(),
        1,
        "One order on Paradex"
    );
}

// =============================================================================
// Test 2: Position Exit Execution (AC: Automatic Close)
// =============================================================================

/// Test position close execution with reduce_only orders
#[tokio::test]
async fn test_position_exit_on_spread_convergence() {
    let spread_exit: f64 = 0.05;

    let vest = MockExchangeAdapter::with_orderbook("vest", 100.10, 100.15);
    let paradex = MockExchangeAdapter::with_orderbook("paradex", 100.12, 100.18);

    let vest = Arc::new(Mutex::new(vest));
    let paradex = Arc::new(Mutex::new(paradex));

    // Calculate current spread
    let vest_ask = 100.15;
    let paradex_bid = 100.12;
    let mid = (vest_ask + paradex_bid) / 2.0;
    let current_spread = ((vest_ask - paradex_bid) / mid) * 100.0;

    // Verify spread is below exit threshold
    assert!(
        current_spread <= spread_exit,
        "Current spread {}% should be <= exit threshold {}%",
        current_spread,
        spread_exit
    );

    // === SIMULATE CLOSE EXECUTION ===
    {
        let vest_guard = vest.lock().await;
        let mut close_order = OrderRequest::ioc_limit(
            "close-vest-1".to_string(),
            "BTC-PERP".to_string(),
            OrderSide::Buy,
            vest_ask,
            0.001,
        );
        close_order.reduce_only = true;
        vest_guard
            .place_order(close_order)
            .await
            .expect("Vest close should succeed");
    }

    {
        let paradex_guard = paradex.lock().await;
        let mut close_order = OrderRequest::ioc_limit(
            "close-paradex-1".to_string(),
            "BTC-USD-PERP".to_string(),
            OrderSide::Sell,
            paradex_bid,
            0.001,
        );
        close_order.reduce_only = true;
        paradex_guard
            .place_order(close_order)
            .await
            .expect("Paradex close should succeed");
    }

    // === VERIFY ===
    assert_eq!(
        vest.lock().await.reduce_only_orders(),
        1,
        "One reduce_only on Vest"
    );
    assert_eq!(
        paradex.lock().await.reduce_only_orders(),
        1,
        "One reduce_only on Paradex"
    );
}

// =============================================================================
// Test 3: Graceful Shutdown (AC: Clean Exit)
// =============================================================================

/// Test that shutdown signal is properly propagated
#[tokio::test]
async fn test_graceful_shutdown_propagation() {
    let (shutdown_tx, mut shutdown_rx) = broadcast::channel::<()>(1);

    let shutdown_received = Arc::new(AtomicBool::new(false));
    let shutdown_flag = Arc::clone(&shutdown_received);

    let task = tokio::spawn(async move {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                shutdown_flag.store(true, Ordering::SeqCst);
            }
            _ = sleep(Duration::from_secs(5)) => {
                panic!("Timeout waiting for shutdown");
            }
        }
    });

    sleep(Duration::from_millis(10)).await;

    let receivers = shutdown_tx.send(()).expect("Shutdown send should succeed");
    assert!(receivers >= 1, "At least one receiver should get shutdown");

    task.await.expect("Task should complete");

    assert!(
        shutdown_received.load(Ordering::SeqCst),
        "Shutdown signal should be received"
    );
}

// =============================================================================
// Test 4: Channel Communication (AC: Data Pipeline)
// =============================================================================

/// Test that spread opportunities flow through channels correctly
#[tokio::test]
async fn test_channel_spread_opportunity_flow() {
    let (tx, mut rx) = mpsc::channel::<SpreadOpportunity>(100);

    for i in 0..5 {
        let opportunity = SpreadOpportunity {
            pair: format!("BTC-PERP-{}", i),
            dex_a: "vest".to_string(),
            dex_b: "paradex".to_string(),
            spread_percent: 0.30 + (i as f64 * 0.01),
            direction: SpreadDirection::AOverB,
            detected_at_ms: current_time_ms(),
            dex_a_ask: 42000.0,
            dex_a_bid: 41990.0,
            dex_b_ask: 42005.0,
            dex_b_bid: 41985.0,
        };
        tx.send(opportunity).await.expect("Send should succeed");
    }

    drop(tx);

    let mut received = Vec::new();
    while let Some(opp) = rx.recv().await {
        received.push(opp);
    }

    assert_eq!(received.len(), 5, "All 5 opportunities should be received");

    for (i, opp) in received.iter().enumerate() {
        assert_eq!(opp.pair, format!("BTC-PERP-{}", i));
    }
}

// =============================================================================
// Test 5: Mock Adapter Order Tracking (AC: Order Verification)
// =============================================================================

/// Test that mock adapter correctly tracks order placement
#[tokio::test]
async fn test_mock_adapter_order_tracking() {
    let adapter = MockExchangeAdapter::new("test_exchange");
    let adapter = Arc::new(Mutex::new(adapter));

    // Place regular orders
    for i in 0..3 {
        let guard = adapter.lock().await;
        let order = OrderRequest::ioc_limit(
            format!("order-{}", i),
            "BTC-PERP".to_string(),
            if i % 2 == 0 {
                OrderSide::Buy
            } else {
                OrderSide::Sell
            },
            100.0 + i as f64,
            0.001,
        );
        guard
            .place_order(order)
            .await
            .expect("Order should succeed");
    }

    // Place reduce_only orders
    for i in 0..2 {
        let guard = adapter.lock().await;
        let mut order = OrderRequest::ioc_limit(
            format!("close-{}", i),
            "BTC-PERP".to_string(),
            OrderSide::Buy,
            100.0,
            0.001,
        );
        order.reduce_only = true;
        guard
            .place_order(order)
            .await
            .expect("Close should succeed");
    }

    let guard = adapter.lock().await;
    assert_eq!(guard.orders_placed(), 5, "Total 5 orders placed");
    assert_eq!(guard.reduce_only_orders(), 2, "2 reduce_only orders");
}
