//! Delta-Neutral Execution Engine (Story 2.3, 2.4, 2.5)
//!
//! Story 2.5: Auto-close exposed leg when one leg fails (NFR7).
//!
//! This module orchestrates simultaneous order execution on two exchanges
//! for delta-neutral arbitrage trades. Key NFR: execution latency < 500ms.
//!
//! # Architecture
//! - `DeltaNeutralExecutor`: Orchestrates parallel order placement
//! - `DeltaNeutralResult`: Captures outcome of both legs
//! - `LegStatus`: Tracks individual leg success/failure
//! - `RetryResult`: Captures retry operation details (Story 2.4)

use std::time::{Duration, Instant};
use tracing::{info, warn, error};

use crate::adapters::{
    ExchangeAdapter, ExchangeResult,
    types::{OrderRequest, OrderResponse, OrderSide, OrderType, TimeInForce},
};
use crate::config::constants::{max_retry_attempts, retry_delay_ms};
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

// =============================================================================
// RetryResult (Story 2.4 - Task 3)
// =============================================================================

/// Result of retry operation capturing attempt details
#[derive(Debug, Clone)]
pub struct RetryResult {
    /// Was the final attempt successful?
    pub success: bool,
    /// How many attempts were made (1 = succeeded first try)
    pub attempts: u32,
    /// Error message if all attempts failed
    pub final_error: Option<String>,
    /// Response if successful
    pub response: Option<OrderResponse>,
}

impl RetryResult {
    /// Create a successful retry result
    pub fn success(response: OrderResponse, attempts: u32) -> Self {
        Self {
            success: true,
            attempts,
            final_error: None,
            response: Some(response),
        }
    }

    /// Create a failed retry result (all attempts exhausted)
    pub fn failed(attempts: u32, error: String) -> Self {
        Self {
            success: false,
            attempts,
            final_error: Some(error),
            response: None,
        }
    }
}

impl From<RetryResult> for LegStatus {
    fn from(result: RetryResult) -> Self {
        if result.success {
            LegStatus::Success(result.response.expect("successful retry must have response"))
        } else {
            LegStatus::Failed(result.final_error.unwrap_or_else(|| "Unknown error".to_string()))
        }
    }
}

// =============================================================================
// AutoCloseResult (Story 2.5 - Task 3)
// =============================================================================

/// Result of auto-close operation when one leg fails
#[derive(Debug, Clone)]
pub struct AutoCloseResult {
    /// Was auto-close needed (true if partial failure occurred)?
    pub was_needed: bool,
    /// Was the close operation successful?
    pub success: bool,
    /// How many attempts were made (0 if not needed)
    pub attempts: u32,
    /// Close order response if successful
    pub close_response: Option<OrderResponse>,
    /// Error message if close failed
    pub error: Option<String>,
    /// Which exchange was closed
    pub exchange: String,
}

impl AutoCloseResult {
    /// Create a result when no auto-close was needed (both succeeded or both failed)
    pub fn not_needed() -> Self {
        Self {
            was_needed: false,
            success: true,
            attempts: 0,
            close_response: None,
            error: None,
            exchange: String::new(),
        }
    }

    /// Create a successful auto-close result
    pub fn closed(response: OrderResponse, attempts: u32, exchange: String) -> Self {
        Self {
            was_needed: true,
            success: true,
            attempts,
            close_response: Some(response),
            error: None,
            exchange,
        }
    }

    /// Create a failed auto-close result
    pub fn failed(error: String, attempts: u32, exchange: String) -> Self {
        Self {
            was_needed: true,
            success: false,
            attempts,
            close_response: None,
            error: Some(error),
            exchange,
        }
    }
}

// =============================================================================
// Auto-Close Helpers (Story 2.5 - Tasks 1, 2)
// =============================================================================

/// Create a closing order from a successful order response (Task 1)
///
/// Inverts the side (Buy → Sell, Sell → Buy) and sets reduce_only=true
pub fn create_closing_order(
    original_response: &OrderResponse,
    original_side: OrderSide,
    symbol: &str,
    quantity: f64,
) -> OrderRequest {
    let close_side = match original_side {
        OrderSide::Buy => OrderSide::Sell,
        OrderSide::Sell => OrderSide::Buy,
    };

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);

    OrderRequest {
        client_order_id: format!("auto-close-{}-{}", original_response.order_id, timestamp),
        symbol: symbol.to_string(),
        side: close_side,
        order_type: OrderType::Market,
        price: None,
        quantity,
        time_in_force: TimeInForce::Ioc,
        reduce_only: true,
    }
}

/// Auto-close a successful leg when the other leg failed (Task 2)
///
/// Uses retry_order() for resilience. Logs safety warnings.
pub async fn auto_close_leg<A: ExchangeAdapter>(
    adapter: &A,
    original_response: &OrderResponse,
    original_side: OrderSide,
    symbol: &str,
    quantity: f64,
    exchange_name: &str,
) -> AutoCloseResult {
    warn!(
        exchange = %exchange_name,
        order_id = %original_response.order_id,
        "[SAFETY] Initiating auto-close for exposed leg"
    );

    let close_order = create_closing_order(original_response, original_side, symbol, quantity);

    match retry_order(adapter, close_order, exchange_name).await {
        Ok(retry_result) => {
            if retry_result.success {
                info!(
                    exchange = %exchange_name,
                    attempts = retry_result.attempts,
                    "[SAFETY] Successfully closed exposed leg"
                );
                AutoCloseResult::closed(
                    retry_result.response.expect("successful retry has response"),
                    retry_result.attempts,
                    exchange_name.to_string(),
                )
            } else {
                error!(
                    exchange = %exchange_name,
                    attempts = retry_result.attempts,
                    "[SAFETY] CRITICAL: Failed to close exposed leg"
                );
                AutoCloseResult::failed(
                    retry_result.final_error.unwrap_or_else(|| "Unknown error".to_string()),
                    retry_result.attempts,
                    exchange_name.to_string(),
                )
            }
        }
        Err(e) => {
            error!(
                exchange = %exchange_name,
                error = %e,
                "[SAFETY] CRITICAL: Failed to close exposed leg"
            );
            AutoCloseResult::failed(e.to_string(), 0, exchange_name.to_string())
        }
    }
}

// =============================================================================
// Retry Logic (Story 2.4 - Task 2)
// =============================================================================

/// Retry an order placement with configurable attempts and delay
///
/// # Arguments
/// * `adapter` - Exchange adapter implementing `ExchangeAdapter`
/// * `order` - Order request to place (cloned for each attempt)
/// * `exchange_name` - Name for logging (e.g., "vest", "paradex")
///
/// # Returns
/// `RetryResult` capturing success/failure and attempt count
pub async fn retry_order<A: ExchangeAdapter>(
    adapter: &A,
    order: OrderRequest,
    exchange_name: &str,
) -> ExchangeResult<RetryResult> {
    let max_attempts = max_retry_attempts();
    let delay = Duration::from_millis(retry_delay_ms());

    for attempt in 1..=max_attempts {
        match adapter.place_order(order.clone()).await {
            Ok(response) => {
                if attempt > 1 {
                    info!(
                        exchange = %exchange_name,
                        attempt = attempt,
                        "[RETRY] Order succeeded after retry"
                    );
                }
                return Ok(RetryResult::success(response, attempt));
            }
            Err(e) => {
                warn!(
                    exchange = %exchange_name,
                    attempt = attempt,
                    max = max_attempts,
                    error = %e,
                    "[RETRY] Order failed, attempt {}/{}", attempt, max_attempts
                );

                if attempt < max_attempts {
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    error!(
        exchange = %exchange_name,
        total_attempts = max_attempts,
        "[RETRY] All attempts failed"
    );
    Ok(RetryResult::failed(max_attempts, format!("All {} retry attempts exhausted", max_attempts)))
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
    /// Auto-close result if one leg failed (Story 2.5 - NFR7)
    pub auto_close_result: Option<AutoCloseResult>,
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
    /// in parallel using `tokio::join!`. Orders are retried up to
    /// `max_retry_attempts()` times with `retry_delay_ms()` between attempts.
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

        // Execute both orders in parallel with retry logic (Story 2.4)
        let (vest_retry_result, paradex_retry_result) = tokio::join!(
            retry_order(&self.vest_adapter, vest_order, "vest"),
            retry_order(&self.paradex_adapter, paradex_order, "paradex")
        );

        let execution_latency_ms = start.elapsed().as_millis() as u64;

        // Extract RetryResults (infallible since retry_order returns Ok(RetryResult))
        let vest_retry = vest_retry_result?;
        let paradex_retry = paradex_retry_result?;

        // Log total attempts for observability
        let total_vest_attempts = vest_retry.attempts;
        let total_paradex_attempts = paradex_retry.attempts;

        // Convert RetryResult to LegStatus based on which exchange got which side
        let (long_status, short_status, long_attempts, short_attempts) = if long_exchange == "vest" {
            (
                LegStatus::from(vest_retry),
                LegStatus::from(paradex_retry),
                total_vest_attempts,
                total_paradex_attempts,
            )
        } else {
            (
                LegStatus::from(paradex_retry),
                LegStatus::from(vest_retry),
                total_paradex_attempts,
                total_vest_attempts,
            )
        };

        let success = long_status.is_success() && short_status.is_success();

        // Story 2.5: Auto-close exposed leg on partial failure (NFR7)
        let auto_close_result = if success {
            // Both legs succeeded - no auto-close needed
            info!(
                spread = %format!("{:.4}%", opportunity.spread_percent),
                long = %long_exchange,
                short = %short_exchange,
                latency_ms = execution_latency_ms,
                pair = %opportunity.pair,
                long_attempts = long_attempts,
                short_attempts = short_attempts,
                "[TRADE] Entry executed"
            );
            None
        } else {
            // Partial failure - determine which leg needs closing
            match (long_status.is_success(), short_status.is_success()) {
                (true, false) => {
                    // Long succeeded, short failed - close the long leg
                    warn!(
                        spread = %format!("{:.4}%", opportunity.spread_percent),
                        long_success = true,
                        short_success = false,
                        latency_ms = execution_latency_ms,
                        long_attempts = long_attempts,
                        short_attempts = short_attempts,
                        "[TRADE] Delta-neutral partial failure - closing long leg"
                    );
                    
                    if let LegStatus::Success(ref response) = long_status {
                        // Call auto_close_leg with the correct concrete adapter type
                        if long_exchange == "vest" {
                            Some(auto_close_leg(
                                &self.vest_adapter,
                                response,
                                OrderSide::Buy,
                                &self.vest_symbol,
                                self.default_quantity,
                                long_exchange,
                            ).await)
                        } else {
                            Some(auto_close_leg(
                                &self.paradex_adapter,
                                response,
                                OrderSide::Buy,
                                &self.paradex_symbol,
                                self.default_quantity,
                                long_exchange,
                            ).await)
                        }
                    } else {
                        None
                    }
                }
                (false, true) => {
                    // Short succeeded, long failed - close the short leg
                    warn!(
                        spread = %format!("{:.4}%", opportunity.spread_percent),
                        long_success = false,
                        short_success = true,
                        latency_ms = execution_latency_ms,
                        long_attempts = long_attempts,
                        short_attempts = short_attempts,
                        "[TRADE] Delta-neutral partial failure - closing short leg"
                    );
                    
                    if let LegStatus::Success(ref response) = short_status {
                        // Call auto_close_leg with the correct concrete adapter type
                        if short_exchange == "vest" {
                            Some(auto_close_leg(
                                &self.vest_adapter,
                                response,
                                OrderSide::Sell,
                                &self.vest_symbol,
                                self.default_quantity,
                                short_exchange,
                            ).await)
                        } else {
                            Some(auto_close_leg(
                                &self.paradex_adapter,
                                response,
                                OrderSide::Sell,
                                &self.paradex_symbol,
                                self.default_quantity,
                                short_exchange,
                            ).await)
                        }
                    } else {
                        None
                    }
                }
                (false, false) => {
                    // Both legs failed - no position to close
                    info!(
                        spread = %format!("{:.4}%", opportunity.spread_percent),
                        latency_ms = execution_latency_ms,
                        long_attempts = long_attempts,
                        short_attempts = short_attempts,
                        "[TRADE] Both legs failed - no position to close"
                    );
                    Some(AutoCloseResult::not_needed())
                }
                (true, true) => {
                    // Should not reach here as success would be true
                    None
                }
            }
        };

        Ok(DeltaNeutralResult {
            long_order: long_status,
            short_order: short_status,
            execution_latency_ms,
            success,
            spread_percent: opportunity.spread_percent,
            long_exchange: long_exchange.to_string(),
            short_exchange: short_exchange.to_string(),
            auto_close_result,
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

    // =========================================================================
    // Story 2.4 - Retry Logic Tests (Task 6)
    // =========================================================================

    /// Mock adapter that fails N times, then succeeds
    struct FailNTimesAdapter {
        fail_count: Arc<AtomicU64>,
        fail_until: u64,
        name: &'static str,
    }

    impl FailNTimesAdapter {
        fn new(name: &'static str, fail_until: u64) -> Self {
            Self {
                fail_count: Arc::new(AtomicU64::new(0)),
                fail_until,
                name,
            }
        }

        /// Create an adapter that will always fail (fails 100 times, more than max retries)
        fn new_always_fail(name: &'static str) -> Self {
            Self::new(name, 100)
        }
    }

    #[async_trait]
    impl ExchangeAdapter for FailNTimesAdapter {
        async fn connect(&mut self) -> ExchangeResult<()> { Ok(()) }
        async fn disconnect(&mut self) -> ExchangeResult<()> { Ok(()) }
        async fn subscribe_orderbook(&mut self, _symbol: &str) -> ExchangeResult<()> { Ok(()) }
        async fn unsubscribe_orderbook(&mut self, _symbol: &str) -> ExchangeResult<()> { Ok(()) }
        async fn cancel_order(&self, _order_id: &str) -> ExchangeResult<()> { Ok(()) }
        fn get_orderbook(&self, _symbol: &str) -> Option<&Orderbook> { None }
        fn is_connected(&self) -> bool { true }
        fn is_stale(&self) -> bool { false }
        async fn sync_orderbooks(&mut self) {}
        async fn reconnect(&mut self) -> ExchangeResult<()> { Ok(()) }
        async fn get_position(&self, _symbol: &str) -> ExchangeResult<Option<crate::adapters::types::PositionInfo>> { Ok(None) }
        fn exchange_name(&self) -> &'static str { self.name }

        async fn place_order(&self, order: OrderRequest) -> ExchangeResult<OrderResponse> {
            let count = self.fail_count.fetch_add(1, Ordering::SeqCst);
            if count < self.fail_until {
                return Err(ExchangeError::OrderRejected(format!("Failure #{}", count + 1)));
            }
            Ok(OrderResponse {
                order_id: format!("{}-{}", self.name, order.client_order_id),
                client_order_id: order.client_order_id,
                status: OrderStatus::Filled,
                filled_quantity: order.quantity,
                avg_price: Some(42000.0),
            })
        }
    }

    fn create_test_order() -> OrderRequest {
        OrderRequest {
            client_order_id: "test-order".to_string(),
            symbol: "BTC-PERP".to_string(),
            side: OrderSide::Buy,
            order_type: crate::adapters::types::OrderType::Market,
            price: None,
            quantity: 0.01,
            time_in_force: TimeInForce::Ioc,
            reduce_only: false,
        }
    }

    /// Story 2.4 - Task 6.1: Retry succeeds on first attempt
    #[tokio::test]
    async fn test_retry_succeeds_first_attempt() {
        let adapter = MockAdapter::new("test");
        let order = create_test_order();

        let result = retry_order(&adapter, order, "test").await.unwrap();

        assert!(result.success);
        assert_eq!(result.attempts, 1);
        assert!(result.response.is_some());
        assert!(result.final_error.is_none());
    }

    /// Story 2.4 - Task 6.2: Retry succeeds after second attempt
    #[tokio::test]
    async fn test_retry_succeeds_second_attempt() {
        // Fail once, then succeed
        let adapter = FailNTimesAdapter::new("test", 1);
        let order = create_test_order();

        let result = retry_order(&adapter, order, "test").await.unwrap();

        assert!(result.success);
        assert_eq!(result.attempts, 2);
        assert!(result.response.is_some());
    }

    /// Story 2.4 - Task 6.3: All retry attempts fail
    #[tokio::test]
    async fn test_retry_all_attempts_fail() {
        // Fail all 3 default attempts
        let adapter = FailNTimesAdapter::new("test", 10);
        let order = create_test_order();

        let result = retry_order(&adapter, order, "test").await.unwrap();

        assert!(!result.success);
        assert_eq!(result.attempts, 3); // max_retry_attempts() default
        assert!(result.response.is_none());
        assert!(result.final_error.is_some());
        assert!(result.final_error.unwrap().contains("retry attempts"));
    }

    /// Story 2.4 - Task 6.5: RetryResult converts to LegStatus correctly
    #[test]
    fn test_retry_result_to_leg_status_success() {
        let response = OrderResponse {
            order_id: "123".to_string(),
            client_order_id: "client-123".to_string(),
            status: OrderStatus::Filled,
            filled_quantity: 0.01,
            avg_price: Some(42000.0),
        };
        let retry_result = RetryResult::success(response.clone(), 2);

        let leg_status: LegStatus = retry_result.into();

        assert!(leg_status.is_success());
        if let LegStatus::Success(resp) = leg_status {
            assert_eq!(resp.order_id, "123");
        }
    }

    #[test]
    fn test_retry_result_to_leg_status_failed() {
        let retry_result = RetryResult::failed(3, "All retries exhausted".to_string());

        let leg_status: LegStatus = retry_result.into();

        assert!(!leg_status.is_success());
        if let LegStatus::Failed(msg) = leg_status {
            assert!(msg.contains("All retries"));
        }
    }

    /// Story 2.4 - Task 7.1: Delta-neutral with one leg retry
    #[tokio::test]
    async fn test_delta_neutral_with_retry_one_leg() {
        // Vest succeeds first try, Paradex fails once then succeeds
        let vest = MockAdapter::new("vest");
        let paradex = FailNTimesAdapter::new("paradex", 1);
        let vest_count = vest.order_count.clone();
        let paradex_count = paradex.fail_count.clone();

        let executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
        );

        let opportunity = create_test_opportunity();
        let result = executor.execute_delta_neutral(opportunity).await.unwrap();

        // Both legs should ultimately succeed
        assert!(result.success);
        assert!(result.long_order.is_success());
        assert!(result.short_order.is_success());

        // Vest should have 1 attempt (retry_order calls place_order once)
        assert_eq!(vest_count.load(Ordering::Relaxed), 1);
        // Paradex should have 2 attempts (1 fail + 1 success)
        assert_eq!(paradex_count.load(Ordering::Relaxed), 2);
    }

    /// Story 2.4 - Task 7.2: Delta-neutral with both legs retrying
    #[tokio::test]
    async fn test_delta_neutral_with_retry_both_legs() {
        // Both fail once, then succeed
        let vest = FailNTimesAdapter::new("vest", 1);
        let paradex = FailNTimesAdapter::new("paradex", 2);
        let vest_count = vest.fail_count.clone();
        let paradex_count = paradex.fail_count.clone();

        let executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
        );

        let opportunity = create_test_opportunity();
        let result = executor.execute_delta_neutral(opportunity).await.unwrap();

        // Both legs should succeed (after retries)
        assert!(result.success);

        // Vest: 2 attempts (1 fail + 1 success)
        assert_eq!(vest_count.load(Ordering::Relaxed), 2);
        // Paradex: 3 attempts (2 fail + 1 success)
        assert_eq!(paradex_count.load(Ordering::Relaxed), 3);
    }

    // =========================================================================
    // Story 2.5: Auto-Close Tests (Task 6)
    // =========================================================================

    /// Story 2.5 - Task 6.4: create_closing_order inverts Buy to Sell
    #[test]
    fn test_create_closing_order_inverts_buy_to_sell() {
        let response = OrderResponse {
            order_id: "test-order-123".to_string(),
            client_order_id: "dn-long-12345".to_string(),
            status: OrderStatus::Filled,
            filled_quantity: 0.01,
            avg_price: Some(98000.0),
        };

        let close_order = create_closing_order(&response, OrderSide::Buy, "BTC-PERP", 0.01);

        assert_eq!(close_order.side, OrderSide::Sell, "Buy should be inverted to Sell");
        assert_eq!(close_order.symbol, "BTC-PERP");
        assert_eq!(close_order.quantity, 0.01);
    }

    /// Story 2.5 - Task 6.4: create_closing_order inverts Sell to Buy
    #[test]
    fn test_create_closing_order_inverts_sell_to_buy() {
        let response = OrderResponse {
            order_id: "test-order-456".to_string(),
            client_order_id: "dn-short-12345".to_string(),
            status: OrderStatus::Filled,
            filled_quantity: 0.005,
            avg_price: Some(98500.0),
        };

        let close_order = create_closing_order(&response, OrderSide::Sell, "BTC-USD-PERP", 0.005);

        assert_eq!(close_order.side, OrderSide::Buy, "Sell should be inverted to Buy");
        assert_eq!(close_order.symbol, "BTC-USD-PERP");
        assert_eq!(close_order.quantity, 0.005);
    }

    /// Story 2.5 - Task 6.5: create_closing_order sets reduce_only=true
    #[test]
    fn test_create_closing_order_sets_reduce_only() {
        let response = OrderResponse {
            order_id: "test-order-789".to_string(),
            client_order_id: "dn-long-54321".to_string(),
            status: OrderStatus::Filled,
            filled_quantity: 0.02,
            avg_price: Some(97500.0),
        };

        let close_order = create_closing_order(&response, OrderSide::Buy, "ETH-PERP", 0.02);

        assert!(close_order.reduce_only, "Closing order must have reduce_only=true");
        assert_eq!(close_order.order_type, OrderType::Market, "Should use Market order for immediate close");
        assert_eq!(close_order.time_in_force, TimeInForce::Ioc);
    }

    /// Story 2.5 - Task 6.6: AutoCloseResult::not_needed
    #[test]
    fn test_auto_close_result_not_needed() {
        let result = AutoCloseResult::not_needed();

        assert!(!result.was_needed, "not_needed should set was_needed=false");
        assert!(result.success, "not_needed should have success=true");
        assert_eq!(result.attempts, 0);
        assert!(result.close_response.is_none());
        assert!(result.error.is_none());
    }

    /// Story 2.5 - Task 6.6: AutoCloseResult::closed
    #[test]
    fn test_auto_close_result_closed() {
        let response = OrderResponse {
            order_id: "close-123".to_string(),
            client_order_id: "auto-close-test".to_string(),
            status: OrderStatus::Filled,
            filled_quantity: 0.01,
            avg_price: Some(98000.0),
        };

        let result = AutoCloseResult::closed(response, 2, "vest".to_string());

        assert!(result.was_needed, "closed should set was_needed=true");
        assert!(result.success, "closed should have success=true");
        assert_eq!(result.attempts, 2);
        assert!(result.close_response.is_some());
        assert_eq!(result.exchange, "vest");
        assert!(result.error.is_none());
    }

    /// Story 2.5 - Task 6.6: AutoCloseResult::failed
    #[test]
    fn test_auto_close_result_failed() {
        let result = AutoCloseResult::failed(
            "Connection timeout".to_string(),
            3,
            "paradex".to_string(),
        );

        assert!(result.was_needed, "failed should set was_needed=true");
        assert!(!result.success, "failed should have success=false");
        assert_eq!(result.attempts, 3);
        assert!(result.close_response.is_none());
        assert_eq!(result.exchange, "paradex");
        assert_eq!(result.error, Some("Connection timeout".to_string()));
    }

    /// Story 2.5 - Task 6.2: Auto-close NOT triggered when both legs succeed
    #[tokio::test]
    async fn test_auto_close_not_triggered_when_both_succeed() {
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

        assert!(result.success);
        assert!(result.auto_close_result.is_none(), "No auto-close when both succeed");
    }

    /// Story 2.5 - Task 6.3: Auto-close NOT triggered when both legs fail
    #[tokio::test]
    async fn test_auto_close_not_triggered_when_both_fail() {
        // Both adapters will fail all retries (fail 10 times, max is 3)
        let vest = FailNTimesAdapter::new_always_fail("vest");
        let paradex = FailNTimesAdapter::new_always_fail("paradex");

        let executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
        );

        let opportunity = create_test_opportunity();
        let result = executor.execute_delta_neutral(opportunity).await.unwrap();

        assert!(!result.success);
        assert!(!result.long_order.is_success());
        assert!(!result.short_order.is_success());
        
        // Auto-close should exist but with was_needed=false (no position to close)
        if let Some(auto_close) = &result.auto_close_result {
            assert!(!auto_close.was_needed, "No position to close when both fail");
        }
    }
}
