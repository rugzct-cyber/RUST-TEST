//! Delta-Neutral Execution Engine (Story 2.3)
//!
//! This module orchestrates simultaneous order execution on two exchanges
//! for delta-neutral arbitrage trades. Key NFR: execution latency < 500ms.
//!
//! # Architecture
//! - `DeltaNeutralExecutor`: Orchestrates parallel order placement
//! - `DeltaNeutralResult`: Captures outcome of both legs
//! - `LegStatus`: Tracks individual leg success/failure

use std::time::Instant;
use tracing::{info, warn};

use crate::adapters::{
    ExchangeAdapter, ExchangeResult,
    types::{OrderRequest, OrderResponse, OrderSide, TimeInForce},
};
use crate::core::channels::SpreadOpportunity;
use crate::core::spread::SpreadDirection;

// =============================================================================
// Types (Task 3)
// =============================================================================

/// Status of a single leg in the delta-neutral trade
#[derive(Debug, Clone)]
pub enum LegStatus {
    /// Order successfully placed
    Success(OrderResponse),
    /// Order failed with error message
    Failed(String),
}

impl LegStatus {
    /// Check if this leg was successful
    pub fn is_success(&self) -> bool {
        matches!(self, LegStatus::Success(_))
    }
}

/// Result of delta-neutral execution (both legs)
#[derive(Debug, Clone)]
pub struct DeltaNeutralResult {
    /// Long leg status (Buy order)
    pub long_order: LegStatus,
    /// Short leg status (Sell order)
    pub short_order: LegStatus,
    /// Total execution latency in milliseconds
    pub execution_latency_ms: u64,
    /// True if both legs succeeded
    pub success: bool,
    /// Spread percentage at execution time
    pub spread_percent: f64,
    /// Exchange that received the long order
    pub long_exchange: String,
    /// Exchange that received the short order
    pub short_exchange: String,
}

// =============================================================================
// DeltaNeutralExecutor (Tasks 1-4)
// =============================================================================

/// Executor for delta-neutral trades across two exchanges
///
/// Uses `tokio::join!` for parallel order placement to minimize latency.
/// NFR2: Total execution latency < 500ms.
pub struct DeltaNeutralExecutor<V, P>
where
    V: ExchangeAdapter + Send + Sync,
    P: ExchangeAdapter + Send + Sync,
{
    vest_adapter: V,
    paradex_adapter: P,
    /// Fixed quantity for MVP (TODO: VWAP sizing in future stories)
    default_quantity: f64,
    /// Symbol for Vest (e.g., "BTC-PERP")
    vest_symbol: String,
    /// Symbol for Paradex (e.g., "BTC-USD-PERP")
    paradex_symbol: String,
}

impl<V, P> DeltaNeutralExecutor<V, P>
where
    V: ExchangeAdapter + Send + Sync,
    P: ExchangeAdapter + Send + Sync,
{
    /// Create a new DeltaNeutralExecutor with both adapters
    ///
    /// # Arguments
    /// * `vest_adapter` - Adapter for Vest exchange
    /// * `paradex_adapter` - Adapter for Paradex exchange
    /// * `default_quantity` - Fixed trade quantity for MVP
    /// * `vest_symbol` - Symbol on Vest (e.g., "BTC-PERP")
    /// * `paradex_symbol` - Symbol on Paradex (e.g., "BTC-USD-PERP")
    pub fn new(
        vest_adapter: V,
        paradex_adapter: P,
        default_quantity: f64,
        vest_symbol: String,
        paradex_symbol: String,
    ) -> Self {
        Self {
            vest_adapter,
            paradex_adapter,
            default_quantity,
            vest_symbol,
            paradex_symbol,
        }
    }

    /// Execute delta-neutral trade based on spread opportunity
    ///
    /// Places a long order on one exchange and short order on the other,
    /// in parallel using `tokio::join!`.
    ///
    /// # Returns
    /// `DeltaNeutralResult` with status of both legs and timing info
    pub async fn execute_delta_neutral(
        &self,
        opportunity: SpreadOpportunity,
    ) -> ExchangeResult<DeltaNeutralResult> {
        let start = Instant::now();

        // Determine which exchange gets long vs short based on direction
        let (long_exchange, short_exchange) = match opportunity.direction {
            SpreadDirection::AOverB => {
                // A's ask > B's bid: Buy on A (vest), Sell on B (paradex)
                ("vest", "paradex")
            }
            SpreadDirection::BOverA => {
                // B's ask > A's bid: Buy on B (paradex), Sell on A (vest)
                ("paradex", "vest")
            }
        };

        // Create unique client order IDs
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let long_order_id = format!("dn-long-{}", timestamp);
        let short_order_id = format!("dn-short-{}", timestamp);

        // Create order requests based on direction
        let (vest_order, paradex_order) = self.create_orders(
            &opportunity,
            long_exchange,
            &long_order_id,
            &short_order_id,
        );

        // Execute both orders in parallel
        let (vest_result, paradex_result) = tokio::join!(
            self.vest_adapter.place_order(vest_order),
            self.paradex_adapter.place_order(paradex_order)
        );

        let execution_latency_ms = start.elapsed().as_millis() as u64;

        // Convert results to LegStatus based on which exchange got which side
        let (long_status, short_status) = if long_exchange == "vest" {
            (
                result_to_leg_status(vest_result),
                result_to_leg_status(paradex_result),
            )
        } else {
            (
                result_to_leg_status(paradex_result),
                result_to_leg_status(vest_result),
            )
        };

        let success = long_status.is_success() && short_status.is_success();

        // Structured logging (Task 4)
        if success {
            info!(
                spread = %format!("{:.4}%", opportunity.spread_percent),
                long = %long_exchange,
                short = %short_exchange,
                latency_ms = execution_latency_ms,
                pair = %opportunity.pair,
                "[TRADE] Entry executed"
            );
        } else {
            warn!(
                spread = %format!("{:.4}%", opportunity.spread_percent),
                long_success = %long_status.is_success(),
                short_success = %short_status.is_success(),
                latency_ms = execution_latency_ms,
                "[TRADE] Delta-neutral partial failure"
            );
        }

        Ok(DeltaNeutralResult {
            long_order: long_status,
            short_order: short_status,
            execution_latency_ms,
            success,
            spread_percent: opportunity.spread_percent,
            long_exchange: long_exchange.to_string(),
            short_exchange: short_exchange.to_string(),
        })
    }

    /// Create order requests for both exchanges based on spread direction
    fn create_orders(
        &self,
        _opportunity: &SpreadOpportunity,
        long_exchange: &str,
        long_order_id: &str,
        short_order_id: &str,
    ) -> (OrderRequest, OrderRequest) {
        let quantity = self.default_quantity;

        // Vest order
        let vest_side = if long_exchange == "vest" {
            OrderSide::Buy
        } else {
            OrderSide::Sell
        };
        let vest_order_id = if long_exchange == "vest" {
            long_order_id.to_string()
        } else {
            short_order_id.to_string()
        };

        // Paradex order
        let paradex_side = if long_exchange == "paradex" {
            OrderSide::Buy
        } else {
            OrderSide::Sell
        };
        let paradex_order_id = if long_exchange == "paradex" {
            long_order_id.to_string()
        } else {
            short_order_id.to_string()
        };

        // Create IOC limit orders at market price
        // TODO: Get actual market price from orderbook for proper limit price
        // For MVP, use market orders (via high limit price for buy, low for sell)
        let vest_order = OrderRequest {
            client_order_id: vest_order_id,
            symbol: self.vest_symbol.clone(),
            side: vest_side,
            order_type: crate::adapters::types::OrderType::Market,
            price: None,
            quantity,
            time_in_force: TimeInForce::Ioc,
            reduce_only: false,
        };

        let paradex_order = OrderRequest {
            client_order_id: paradex_order_id,
            symbol: self.paradex_symbol.clone(),
            side: paradex_side,
            order_type: crate::adapters::types::OrderType::Market,
            price: None,
            quantity,
            time_in_force: TimeInForce::Ioc,
            reduce_only: false,
        };

        (vest_order, paradex_order)
    }
}

/// Convert ExchangeResult to LegStatus
fn result_to_leg_status(result: ExchangeResult<OrderResponse>) -> LegStatus {
    match result {
        Ok(response) => LegStatus::Success(response),
        Err(e) => LegStatus::Failed(e.to_string()),
    }
}

// =============================================================================
// Tests (Task 6)
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::errors::ExchangeError;
    use crate::adapters::types::{OrderStatus, Orderbook};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    /// Mock adapter for testing
    struct MockAdapter {
        connected: bool,
        should_fail: bool,
        order_count: Arc<AtomicU64>,
        name: &'static str,
    }

    impl MockAdapter {
        fn new(name: &'static str) -> Self {
            Self {
                connected: true,
                should_fail: false,
                order_count: Arc::new(AtomicU64::new(0)),
                name,
            }
        }

        fn with_failure(name: &'static str) -> Self {
            Self {
                connected: true,
                should_fail: true,
                order_count: Arc::new(AtomicU64::new(0)),
                name,
            }
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
            Ok(())
        }

        async fn subscribe_orderbook(&mut self, _symbol: &str) -> ExchangeResult<()> {
            Ok(())
        }

        async fn unsubscribe_orderbook(&mut self, _symbol: &str) -> ExchangeResult<()> {
            Ok(())
        }

        async fn place_order(&self, order: OrderRequest) -> ExchangeResult<OrderResponse> {
            self.order_count.fetch_add(1, Ordering::Relaxed);
            
            // Simulate network delay for latency tests
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

        fn get_orderbook(&self, _symbol: &str) -> Option<&Orderbook> {
            None
        }

        fn is_connected(&self) -> bool {
            self.connected
        }

        fn is_stale(&self) -> bool {
            false
        }

        async fn sync_orderbooks(&mut self) {}

        async fn reconnect(&mut self) -> ExchangeResult<()> {
            Ok(())
        }

        async fn get_position(&self, _symbol: &str) -> ExchangeResult<Option<crate::adapters::types::PositionInfo>> {
            Ok(None)
        }

        fn exchange_name(&self) -> &'static str {
            self.name
        }
    }

    fn create_test_opportunity() -> SpreadOpportunity {
        SpreadOpportunity {
            pair: "BTC-PERP".to_string(),
            dex_a: "vest".to_string(),
            dex_b: "paradex".to_string(),
            spread_percent: 0.35,
            direction: SpreadDirection::AOverB,
            detected_at_ms: 1706000000000,
        }
    }

    // =========================================================================
    // Task 6.1: test_delta_neutral_executor_creation
    // =========================================================================
    
    #[test]
    fn test_delta_neutral_executor_creation() {
        let vest = MockAdapter::new("vest");
        let paradex = MockAdapter::new("paradex");
        
        let executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
        );
        
        assert_eq!(executor.default_quantity, 0.01);
        assert_eq!(executor.vest_symbol, "BTC-PERP");
        assert_eq!(executor.paradex_symbol, "BTC-USD-PERP");
    }

    // =========================================================================
    // Task 6.2: test_execute_both_legs_parallel
    // =========================================================================
    
    #[tokio::test]
    async fn test_execute_both_legs_parallel() {
        let vest = MockAdapter::new("vest");
        let paradex = MockAdapter::new("paradex");
        let vest_count = vest.order_count.clone();
        let paradex_count = paradex.order_count.clone();
        
        let executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
        );
        
        let opportunity = create_test_opportunity();
        let result = executor.execute_delta_neutral(opportunity).await.unwrap();
        
        // Both adapters should have received exactly one order each
        assert_eq!(vest_count.load(Ordering::Relaxed), 1);
        assert_eq!(paradex_count.load(Ordering::Relaxed), 1);
        
        // Both legs should succeed
        assert!(result.success);
        assert!(result.long_order.is_success());
        assert!(result.short_order.is_success());
    }

    // =========================================================================
    // Task 6.3: test_execute_latency_measurement
    // =========================================================================
    
    #[tokio::test]
    async fn test_execute_latency_measurement() {
        let vest = MockAdapter::new("vest");
        let paradex = MockAdapter::new("paradex");
        
        let executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
        );
        
        let opportunity = create_test_opportunity();
        let result = executor.execute_delta_neutral(opportunity).await.unwrap();
        
        // Latency should be measured and > 0
        assert!(result.execution_latency_ms > 0);
        // With parallel execution, latency should be around 10ms (mock delay), not 20ms
        // Allow some margin for test execution variance
        assert!(result.execution_latency_ms < 100, "Latency was {}ms", result.execution_latency_ms);
    }

    // =========================================================================
    // Task 6.4: test_execute_one_leg_fails
    // =========================================================================
    
    #[tokio::test]
    async fn test_execute_one_leg_fails() {
        let vest = MockAdapter::new("vest");
        let paradex = MockAdapter::with_failure("paradex"); // Paradex will fail
        
        let executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
        );
        
        let opportunity = create_test_opportunity();
        let result = executor.execute_delta_neutral(opportunity).await.unwrap();
        
        // Result should indicate partial failure
        assert!(!result.success);
        
        // For AOverB direction: vest=long (should succeed), paradex=short (should fail)
        assert!(result.long_order.is_success(), "Long order should succeed");
        assert!(!result.short_order.is_success(), "Short order should fail");
    }

    // =========================================================================
    // Task 6.5: test_spread_direction_to_orders
    // =========================================================================
    
    #[tokio::test]
    async fn test_spread_direction_to_orders_a_over_b() {
        let vest = MockAdapter::new("vest");
        let paradex = MockAdapter::new("paradex");
        
        let executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
        );
        
        // AOverB: Buy on A (vest), Sell on B (paradex)
        let opportunity = SpreadOpportunity {
            pair: "BTC-PERP".to_string(),
            dex_a: "vest".to_string(),
            dex_b: "paradex".to_string(),
            spread_percent: 0.35,
            direction: SpreadDirection::AOverB,
            detected_at_ms: 1706000000000,
        };
        
        let result = executor.execute_delta_neutral(opportunity).await.unwrap();
        
        // Vest should be long, Paradex should be short
        assert_eq!(result.long_exchange, "vest");
        assert_eq!(result.short_exchange, "paradex");
    }

    #[tokio::test]
    async fn test_spread_direction_to_orders_b_over_a() {
        let vest = MockAdapter::new("vest");
        let paradex = MockAdapter::new("paradex");
        
        let executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
        );
        
        // BOverA: Buy on B (paradex), Sell on A (vest)
        let opportunity = SpreadOpportunity {
            pair: "BTC-PERP".to_string(),
            dex_a: "vest".to_string(),
            dex_b: "paradex".to_string(),
            spread_percent: 0.35,
            direction: SpreadDirection::BOverA,
            detected_at_ms: 1706000000000,
        };
        
        let result = executor.execute_delta_neutral(opportunity).await.unwrap();
        
        // Paradex should be long, Vest should be short
        assert_eq!(result.long_exchange, "paradex");
        assert_eq!(result.short_exchange, "vest");
    }

    // =========================================================================
    // Task 6: Additional edge case tests
    // =========================================================================

    #[test]
    fn test_leg_status_is_success() {
        let success = LegStatus::Success(OrderResponse {
            order_id: "123".to_string(),
            client_order_id: "client-123".to_string(),
            status: OrderStatus::Filled,
            filled_quantity: 0.01,
            avg_price: Some(42000.0),
        });
        assert!(success.is_success());
        
        let failed = LegStatus::Failed("Error".to_string());
        assert!(!failed.is_success());
    }

    #[tokio::test]
    async fn test_execute_both_legs_fail() {
        let vest = MockAdapter::with_failure("vest");
        let paradex = MockAdapter::with_failure("paradex");
        
        let executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
        );
        
        let opportunity = create_test_opportunity();
        let result = executor.execute_delta_neutral(opportunity).await.unwrap();
        
        // Both legs should fail
        assert!(!result.success);
        assert!(!result.long_order.is_success());
        assert!(!result.short_order.is_success());
    }
}
