//! Delta-Neutral Execution Engine (Simplified)
//!
//! This module orchestrates simultaneous order execution on two exchanges
//! for delta-neutral arbitrage trades.
//!
//! # Architecture
//! - `DeltaNeutralExecutor`: Orchestrates parallel order placement
//! - `DeltaNeutralResult`: Captures outcome of both legs
//! - `LegStatus`: Tracks individual leg success/failure

use std::time::Instant;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use tokio::sync::Mutex;
use tracing::{info, warn, error};

use crate::adapters::{
    ExchangeAdapter, ExchangeResult, ExchangeError,
    types::{OrderRequest, OrderResponse, OrderSide, OrderType, TimeInForce},
};
use crate::core::channels::SpreadOpportunity;
use crate::core::spread::SpreadDirection;
use crate::core::events::{TradingEvent, TimingBreakdown, current_timestamp_ms, log_event, format_pct, fmt_price};

// =============================================================================
// Constants
// =============================================================================

/// Slippage buffer for LIMIT IOC and MARKET orders (0.5% = 50 basis points)
/// Used as price protection on both Vest and Paradex orders
const SLIPPAGE_BUFFER_PCT: f64 = 0.005;

// =============================================================================
// Trade Timing Breakdown (Story 8.1 refactoring)
// =============================================================================

/// Struct to hold timing measurements during trade execution
/// 
/// # Call Order (CRITICAL - Red Team F2)
/// 1. `new()` - At function entry (captures start Instant)
/// 2. `mark_signal_received()` - AFTER create_orders() returns
/// 3. `mark_order_sent()` - Before tokio::join! on place_order
/// 4. `mark_order_confirmed()` - After tokio::join! completes
/// 5. `total_latency_ms()` - For result struct
#[derive(Debug)]
struct TradeTimings {
    start: Instant,
    t_signal: u64,
    t_order_sent: u64,
    t_order_confirmed: u64,
}

impl TradeTimings {
    /// Create new timing tracker. Does NOT capture t_signal (Red Team F1)
    fn new() -> Self {
        Self {
            start: Instant::now(),
            t_signal: 0,  // Captured explicitly via mark_signal_received()
            t_order_sent: 0,
            t_order_confirmed: 0,
        }
    }
    
    /// Mark when signal is received (after create_orders)
    fn mark_signal_received(&mut self) {
        self.t_signal = current_timestamp_ms();
    }
    
    fn mark_order_sent(&mut self) {
        self.t_order_sent = current_timestamp_ms();
    }
    
    fn mark_order_confirmed(&mut self) {
        self.t_order_confirmed = current_timestamp_ms();
    }
    
    fn total_latency_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }
}

/// Calculate execution spread from fill prices
/// Returns: (short_fill - long_fill) / long_fill * 100.0
fn calculate_execution_spread(long_fill_price: f64, short_fill_price: f64) -> f64 {
    if long_fill_price > 0.0 {
        ((short_fill_price - long_fill_price) / long_fill_price) * 100.0
    } else {
        0.0
    }
}

/// Log successful trade with timing and slippage analysis
fn log_successful_trade(
    opportunity: &SpreadOpportunity,
    result: &DeltaNeutralResult,
    timings: &TradeTimings,
) {
    info!(
        event_type = "TRADE_ENTRY",
        spread = %format_pct(opportunity.spread_percent),
        long = %result.long_exchange,
        short = %result.short_exchange,
        latency_ms = result.execution_latency_ms,
        pair = %opportunity.pair,
        "Entry executed"
    );
    
    // Story 8.1: Calculate execution spread and emit SlippageAnalysis event
    let execution_spread = calculate_execution_spread(
        result.long_fill_price,
        result.short_fill_price,
    );
    
    let timing = TimingBreakdown::new(
        opportunity.detected_at_ms,
        timings.t_signal,
        timings.t_order_sent,
        timings.t_order_confirmed,
    );
    
    let direction_str = format!("{:?}", opportunity.direction);
    let slippage_event = TradingEvent::slippage_analysis(
        &opportunity.pair,
        opportunity.spread_percent,
        execution_spread,
        timing,
        &result.long_exchange,
        &result.short_exchange,
        &direction_str,
    );
    log_event(&slippage_event);
}

// =============================================================================
// Types
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
    /// Fill price from long order (avg_fill_price)
    pub long_fill_price: f64,
    /// Fill price from short order (avg_fill_price)
    pub short_fill_price: f64,
}

// =============================================================================
// DeltaNeutralExecutor
// =============================================================================

/// Executor for delta-neutral trades across two exchanges
///
/// Uses `tokio::join!` for parallel order placement to minimize latency.
pub struct DeltaNeutralExecutor<V, P>
where
    V: ExchangeAdapter + Send + Sync,
    P: ExchangeAdapter + Send + Sync,
{
    vest_adapter: Mutex<V>,
    paradex_adapter: Mutex<P>,
    /// Fixed quantity for MVP
    default_quantity: f64,
    /// Symbol for Vest (e.g., "BTC-PERP")
    vest_symbol: String,
    /// Symbol for Paradex (e.g., "BTC-USD-PERP")
    paradex_symbol: String,
    /// Atomic guard: true = position open, false = can trade
    position_open: AtomicBool,
    /// Entry direction: 0=none, 1=AOverB (long Vest), 2=BOverA (long Paradex)
    entry_direction: AtomicU8,
}

impl<V, P> DeltaNeutralExecutor<V, P>
where
    V: ExchangeAdapter + Send + Sync,
    P: ExchangeAdapter + Send + Sync,
{
    /// Create a new DeltaNeutralExecutor with both adapters
    pub fn new(
        vest_adapter: V,
        paradex_adapter: P,
        default_quantity: f64,
        vest_symbol: String,
        paradex_symbol: String,
    ) -> Self {
        Self {
            vest_adapter: Mutex::new(vest_adapter),
            paradex_adapter: Mutex::new(paradex_adapter),
            default_quantity,
            vest_symbol,
            paradex_symbol,
            position_open: AtomicBool::new(false),
            entry_direction: AtomicU8::new(0),  // 0 = no position
        }
    }
    
    /// Check if a position is currently open
    pub fn has_position(&self) -> bool {
        self.position_open.load(Ordering::SeqCst)
    }
    
    /// Ensure adapters are ready for order placement
    /// 
    /// Calls reconnect() on adapters if they are stale or need JWT refresh.
    /// This is critical for Paradex which has JWT expiry.
    pub async fn ensure_ready(&self) -> ExchangeResult<()> {
        // Check and refresh Paradex adapter if needed
        {
            let mut paradex = self.paradex_adapter.lock().await;
            if paradex.is_stale() {
                info!(event_type = "ADAPTER_RECONNECT", exchange = "paradex", "Adapter stale - reconnecting");
                paradex.reconnect().await?;
                info!(event_type = "ADAPTER_RECONNECT", exchange = "paradex", "Adapter reconnected");
            }
        }
        
        // Check Vest adapter
        {
            let mut vest = self.vest_adapter.lock().await;
            if vest.is_stale() {
                info!(event_type = "ADAPTER_RECONNECT", exchange = "vest", "Adapter stale - reconnecting");
                vest.reconnect().await?;
                info!(event_type = "ADAPTER_RECONNECT", exchange = "vest", "Adapter reconnected");
            }
        }
        
        Ok(())
    }
    
    /// Get the default quantity used for trades
    pub fn get_default_quantity(&self) -> f64 {
        self.default_quantity
    }

    /// Execute delta-neutral trade based on spread opportunity
    ///
    /// Places a long order on one exchange and short order on the other,
    /// in parallel using `tokio::join!`.
    pub async fn execute_delta_neutral(
        &self,
        opportunity: SpreadOpportunity,
    ) -> ExchangeResult<DeltaNeutralResult> {
        // ATOMIC GUARD: Try to acquire - only one trade can pass
        if self.position_open
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            warn!(event_type = "TRADE_REJECTED", reason = "position_already_open", "Position already open - rejecting trade");
            return Err(ExchangeError::OrderRejected("Position already open".into()));
        }
        
        info!(event_type = "TRADE_STARTED", "Position lock acquired - executing delta-neutral trade");
        let mut timings = TradeTimings::new();

        // Determine which exchange gets long vs short based on direction
        // - AOverB: spread = (bid_B - ask_A)/ask_A → BUY on A, SELL on B
        // - BOverA: spread = (bid_A - ask_B)/ask_B → BUY on B, SELL on A
        let (long_exchange, short_exchange) = match opportunity.direction {
            SpreadDirection::AOverB => ("vest", "paradex"),
            SpreadDirection::BOverA => ("paradex", "vest"),
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
        
        // Story 8.1: Capture t_signal AFTER create_orders (Red Team V1 critical fix)
        timings.mark_signal_received();

        // Execute both orders in parallel (no retry)
        // We lock both adapters and then place orders concurrently
        let vest_guard = self.vest_adapter.lock().await;
        let paradex_guard = self.paradex_adapter.lock().await;
        
        // Story 8.1: Mark order sent timestamp
        timings.mark_order_sent();
        
        let (vest_result, paradex_result) = tokio::join!(
            vest_guard.place_order(vest_order),
            paradex_guard.place_order(paradex_order)
        );
        
        // Story 8.1: Mark order confirmed timestamp
        timings.mark_order_confirmed();
        let execution_latency_ms = timings.total_latency_ms();

        // Convert results to LegStatus based on which exchange got which side
        let (long_status, short_status) = if long_exchange == "vest" {
            (
                result_to_leg_status(vest_result, "vest"),
                result_to_leg_status(paradex_result, "paradex"),
            )
        } else {
            (
                result_to_leg_status(paradex_result, "paradex"),
                result_to_leg_status(vest_result, "vest"),
            )
        };

        let success = long_status.is_success() && short_status.is_success();
        
        // Extract fill prices from order responses (Story 8.1: needed for slippage calculation)
        let long_fill_price = match &long_status {
            LegStatus::Success(resp) => resp.avg_price.unwrap_or(0.0),
            _ => 0.0,
        };
        let short_fill_price = match &short_status {
            LegStatus::Success(resp) => resp.avg_price.unwrap_or(0.0),
            _ => 0.0,
        };

        // Log outcome
        // Build partial result for logging (before final return)
        let partial_result = DeltaNeutralResult {
            long_order: long_status.clone(),
            short_order: short_status.clone(),
            execution_latency_ms,
            success,
            spread_percent: opportunity.spread_percent,
            long_exchange: long_exchange.to_string(),
            short_exchange: short_exchange.to_string(),
            long_fill_price,
            short_fill_price,
        };
        
        if success {
            // Store the entry direction for exit spread calculation
            let dir_value = match opportunity.direction {
                SpreadDirection::AOverB => 1u8,  // Long Vest, Short Paradex
                SpreadDirection::BOverA => 2u8,  // Long Paradex, Short Vest
            };
            self.entry_direction.store(dir_value, Ordering::SeqCst);
            
            // Delegate logging to helper function (Task 3)
            log_successful_trade(&opportunity, &partial_result, &timings);
        } else {
            // Log partial/total failure - user handles exposed positions manually
            warn!(
                event_type = "TRADE_FAILED",
                spread = %format_pct(opportunity.spread_percent),
                long_success = long_status.is_success(),
                short_success = short_status.is_success(),
                latency_ms = execution_latency_ms,
                "Execution failed - manual intervention may be required"
            );
        }

        Ok(partial_result)
    }
    
    /// Verify positions on both exchanges after trade execution
    /// 
    /// Logs entry prices, entry spread, and exit target to confirm trade placement.
    pub async fn verify_positions(&self, entry_spread: f64, exit_spread_target: f64) {
        let vest = self.vest_adapter.lock().await;
        let paradex = self.paradex_adapter.lock().await;
        
        let (vest_pos, paradex_pos) = tokio::join!(
            vest.get_position(&self.vest_symbol),
            paradex.get_position(&self.paradex_symbol)
        );
        
        let vest_price = match &vest_pos {
            Ok(Some(pos)) => pos.entry_price,
            _ => 0.0,
        };
        
        let paradex_price = match &paradex_pos {
            Ok(Some(pos)) => pos.entry_price,
            _ => 0.0,
        };
        
        // Calculate actual captured spread based on entry direction
        let entry_direction = self.get_entry_direction();
        let captured_spread = match entry_direction {
            Some(SpreadDirection::AOverB) => {
                // Long Vest (bought at ask), Short Paradex (sold at bid)
                if vest_price > 0.0 {
                    ((paradex_price - vest_price) / vest_price) * 100.0
                } else { 0.0 }
            }
            Some(SpreadDirection::BOverA) => {
                // Long Paradex (bought at ask), Short Vest (sold at bid)
                if paradex_price > 0.0 {
                    ((vest_price - paradex_price) / paradex_price) * 100.0
                } else { 0.0 }
            }
            None => 0.0,
        };
        
        // Structured entry verification log (Story 5.3)
        info!(
            event_type = "POSITION_VERIFIED",
            vest_price = %fmt_price(vest_price),
            paradex_price = %fmt_price(paradex_price),
            direction = ?entry_direction.unwrap_or(SpreadDirection::AOverB),
            detected_spread = %format_pct(entry_spread),
            captured_spread = %format_pct(captured_spread),
            exit_target = %format_pct(exit_spread_target),
            "Entry positions verified"
        );
        
        // Log individual positions for detail
        match vest_pos {
            Ok(Some(pos)) => info!(
                event_type = "POSITION_DETAIL",
                exchange = "vest",
                side = %pos.side,
                quantity = %pos.quantity,
                entry_price = %pos.entry_price,
                "Position details"
            ),
            Ok(None) => warn!(event_type = "POSITION_DETAIL", exchange = "vest", "No position"),
            Err(e) => warn!(event_type = "POSITION_DETAIL", exchange = "vest", error = %e, "Position check failed"),
        }
        
        match paradex_pos {
            Ok(Some(pos)) => info!(
                event_type = "POSITION_DETAIL",
                exchange = "paradex",
                side = %pos.side,
                quantity = %pos.quantity,
                entry_price = %pos.entry_price,
                "Position details"
            ),
            Ok(None) => warn!(event_type = "POSITION_DETAIL", exchange = "paradex", "No position"),
            Err(e) => warn!(event_type = "POSITION_DETAIL", exchange = "paradex", error = %e, "Position check failed"),
        }
    }
    
    /// Get the entry direction (0=none, 1=AOverB, 2=BOverA)
    pub fn get_entry_direction(&self) -> Option<SpreadDirection> {
        match self.entry_direction.load(Ordering::SeqCst) {
            1 => Some(SpreadDirection::AOverB),
            2 => Some(SpreadDirection::BOverA),
            _ => None,
        }
    }
    
    /// Close the current position by executing inverse trades
    /// 
    /// Uses reduce_only=true to ensure we only close, not open new positions.
    /// Resets position_open and entry_direction after successful close.
    pub async fn close_position(&self, exit_spread: f64) -> ExchangeResult<DeltaNeutralResult> {
        let start = Instant::now();
        
        // Get the entry direction
        let entry_dir = match self.get_entry_direction() {
            Some(dir) => dir,
            None => {
                return Err(ExchangeError::InvalidOrder("No position to close".to_string()));
            }
        };
        
        // Determine close direction (inverse of entry)
        // Entry AOverB = Long Vest, Short Paradex
        // Close AOverB = Sell Vest (reduce_only), Buy Paradex (reduce_only)
        let (vest_side, paradex_side) = match entry_dir {
            SpreadDirection::AOverB => (OrderSide::Sell, OrderSide::Buy),
            SpreadDirection::BOverA => (OrderSide::Buy, OrderSide::Sell),
        };
        
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        
        let vest_order = OrderRequest {
            client_order_id: format!("close-vest-{}", timestamp),
            symbol: self.vest_symbol.clone(),
            side: vest_side,
            order_type: OrderType::Market,
            price: None, // Market order
            quantity: self.default_quantity,
            time_in_force: TimeInForce::Ioc,
            reduce_only: true,  // Critical: only reduce position
        };
        
        let paradex_order = OrderRequest {
            client_order_id: format!("close-paradex-{}", timestamp),
            symbol: self.paradex_symbol.clone(),
            side: paradex_side,
            order_type: OrderType::Market,
            price: None,
            quantity: self.default_quantity,
            time_in_force: TimeInForce::Ioc,
            reduce_only: true,
        };
        
        // Execute close orders in parallel
        let vest_guard = self.vest_adapter.lock().await;
        let paradex_guard = self.paradex_adapter.lock().await;
        
        let (vest_result, paradex_result) = tokio::join!(
            vest_guard.place_order(vest_order),
            paradex_guard.place_order(paradex_order)
        );
        
        let execution_latency_ms = start.elapsed().as_millis() as u64;
        
        let vest_status = result_to_leg_status(vest_result, "vest");
        let paradex_status = result_to_leg_status(paradex_result, "paradex");
        let success = vest_status.is_success() && paradex_status.is_success();
        
        if success {
            // Reset position state
            self.position_open.store(false, Ordering::SeqCst);
            self.entry_direction.store(0, Ordering::SeqCst);
            
            info!(
                event_type = "TRADE_EXIT",
                exit_spread = %format_pct(exit_spread),
                latency_ms = execution_latency_ms,
                "Position closed"
            );
        } else {
            warn!(
                event_type = "TRADE_EXIT_FAILED",
                vest_success = vest_status.is_success(),
                paradex_success = paradex_status.is_success(),
                latency_ms = execution_latency_ms,
                "Close failed - manual intervention required"
            );
        }
        
        // Determine which leg was long/short based on entry direction  
        let (long_status, short_status, long_exchange, short_exchange) = match entry_dir {
            SpreadDirection::AOverB => (vest_status, paradex_status, "vest", "paradex"),
            SpreadDirection::BOverA => (paradex_status.clone(), vest_status.clone(), "paradex", "vest"),
        };
        
        Ok(DeltaNeutralResult {
            long_order: long_status,
            short_order: short_status,
            execution_latency_ms,
            success,
            spread_percent: exit_spread,
            long_exchange: long_exchange.to_string(),
            short_exchange: short_exchange.to_string(),
            long_fill_price: 0.0,  // Not needed for close operations
            short_fill_price: 0.0, // Not needed for close operations
        })
    }

    /// Create order requests for both exchanges based on spread direction
    fn create_orders(
        &self,
        opportunity: &SpreadOpportunity,
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

        // Vest: Use MARKET orders with limitPrice as slippage protection
        // Add 0.5% buffer to ensure fill while protecting against extreme slippage
        let vest_price = match vest_side {
            OrderSide::Buy => opportunity.dex_a_ask * (1.0 + SLIPPAGE_BUFFER_PCT),
            OrderSide::Sell => opportunity.dex_a_bid * (1.0 - SLIPPAGE_BUFFER_PCT),
        };
        
        // Paradex: Use LIMIT IOC with slippage buffer to ensure fill
        // Without buffer, IOC orders get cancelled if price moves before arrival
        let paradex_price = match paradex_side {
            OrderSide::Buy => opportunity.dex_b_ask * (1.0 + SLIPPAGE_BUFFER_PCT),
            OrderSide::Sell => opportunity.dex_b_bid * (1.0 - SLIPPAGE_BUFFER_PCT),
        };

        // Vest: MARKET order (limitPrice acts as slippage protection)
        let vest_order = OrderRequest {
            client_order_id: vest_order_id,
            symbol: self.vest_symbol.clone(),
            side: vest_side,
            order_type: crate::adapters::types::OrderType::Market,
            price: Some(vest_price), // Required by Vest as slippage protection
            quantity,
            time_in_force: TimeInForce::Ioc, // Ignored for MARKET but required by struct
            reduce_only: false,
        };

        // Paradex: LIMIT IOC for immediate fill with price protection
        let paradex_order = OrderRequest {
            client_order_id: paradex_order_id,
            symbol: self.paradex_symbol.clone(),
            side: paradex_side,
            order_type: crate::adapters::types::OrderType::Limit,
            price: Some(paradex_price),
            quantity,
            time_in_force: TimeInForce::Ioc,
            reduce_only: false,
        };

        (vest_order, paradex_order)
    }
}

/// Convert ExchangeResult to LegStatus
fn result_to_leg_status(result: ExchangeResult<OrderResponse>, exchange: &str) -> LegStatus {
    match result {
        Ok(response) => LegStatus::Success(response),
        Err(e) => {
            error!(exchange = %exchange, error = %e, "[ORDER] Failed");
            LegStatus::Failed(e.to_string())
        }
    }
}

// =============================================================================
// Tests
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
            dex_a_ask: 42000.0,
            dex_a_bid: 41990.0,
            dex_b_ask: 42005.0,
            dex_b_bid: 41985.0,
        }
    }

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
    
    /// Test TradeTimings sequence (Red Team F3)
    /// Validates that t_signal is NOT auto-captured in new() and timestamps are monotonic
    #[test]
    fn test_trade_timings_sequence() {
        let mut timings = TradeTimings::new();
        
        // t_signal should be 0 initially (not auto-captured per Red Team F1)
        assert_eq!(timings.t_signal, 0);
        assert_eq!(timings.t_order_sent, 0);
        assert_eq!(timings.t_order_confirmed, 0);
        
        // Mark signal received
        timings.mark_signal_received();
        assert!(timings.t_signal > 0);
        
        // Small delay to ensure monotonic timestamps
        std::thread::sleep(std::time::Duration::from_millis(1));
        
        // Mark order sent
        timings.mark_order_sent();
        assert!(timings.t_order_sent >= timings.t_signal);
        
        // Small delay
        std::thread::sleep(std::time::Duration::from_millis(1));
        
        // Mark order confirmed
        timings.mark_order_confirmed();
        assert!(timings.t_order_confirmed >= timings.t_order_sent);
        
        // Latency should be measurable (u64 can hold the value)
        let _ = timings.total_latency_ms();
    }

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
        assert!(result.execution_latency_ms < 100, "Latency was {}ms", result.execution_latency_ms);
    }

    #[tokio::test]
    async fn test_execute_one_leg_fails() {
        let vest = MockAdapter::new("vest");
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
        
        // Result should indicate partial failure
        assert!(!result.success);
        
        // For AOverB direction: vest=long (buy), paradex=short (sell - should fail)
        assert!(result.long_order.is_success(), "Long order should succeed");
        assert!(!result.short_order.is_success(), "Short order should fail");
    }

    #[tokio::test]
    async fn test_spread_direction_a_over_b() {
        let vest = MockAdapter::new("vest");
        let paradex = MockAdapter::new("paradex");
        
        let executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
        );
        
        let opportunity = SpreadOpportunity {
            pair: "BTC-PERP".to_string(),
            dex_a: "vest".to_string(),
            dex_b: "paradex".to_string(),
            spread_percent: 0.35,
            direction: SpreadDirection::AOverB,
            detected_at_ms: 1706000000000,
            dex_a_ask: 42000.0,
            dex_a_bid: 41990.0,
            dex_b_ask: 42005.0,
            dex_b_bid: 41985.0,
        };
        
        let result = executor.execute_delta_neutral(opportunity).await.unwrap();
        
        // Vest = long, Paradex = short
        assert_eq!(result.long_exchange, "vest");
        assert_eq!(result.short_exchange, "paradex");
    }

    #[tokio::test]
    async fn test_spread_direction_b_over_a() {
        let vest = MockAdapter::new("vest");
        let paradex = MockAdapter::new("paradex");
        
        let executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
        );
        
        let opportunity = SpreadOpportunity {
            pair: "BTC-PERP".to_string(),
            dex_a: "vest".to_string(),
            dex_b: "paradex".to_string(),
            spread_percent: 0.35,
            direction: SpreadDirection::BOverA,
            detected_at_ms: 1706000000000,
            dex_a_ask: 42000.0,
            dex_a_bid: 41990.0,
            dex_b_ask: 42005.0,
            dex_b_bid: 41985.0,
        };
        
        let result = executor.execute_delta_neutral(opportunity).await.unwrap();
        
        // Paradex = long, Vest = short
        assert_eq!(result.long_exchange, "paradex");
        assert_eq!(result.short_exchange, "vest");
    }

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
