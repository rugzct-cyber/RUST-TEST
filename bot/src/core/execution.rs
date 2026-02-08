//! Delta-Neutral Execution Engine (Simplified)
//!
//! This module orchestrates simultaneous order execution on two exchanges
//! for delta-neutral arbitrage trades.
//!
//! # Architecture
//! - `DeltaNeutralExecutor`: Orchestrates parallel order placement
//! - `DeltaNeutralResult`: Captures outcome of both legs
//! - `LegStatus`: Tracks individual leg success/failure

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Instant;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use crate::adapters::{
    types::{OrderRequest, OrderResponse, OrderSide, OrderStatus},
    ExchangeAdapter, ExchangeError, ExchangeResult, OrderBuilder,
};
use crate::core::channels::SpreadOpportunity;
use crate::core::events::{
    current_timestamp_ms, format_pct, log_event, log_system_event, SystemEvent, TimingBreakdown,
    TradingEvent,
};
use crate::core::spread::SpreadDirection;

// =============================================================================
// Constants
// =============================================================================

/// Slippage buffer for LIMIT IOC and MARKET orders (0.5% = 50 basis points)
/// Used as price protection on both Vest and Paradex orders
const SLIPPAGE_BUFFER_PCT: f64 = 0.02;

// =============================================================================
// Trade Timing Breakdown
// =============================================================================

/// Struct to hold timing measurements during trade execution
///
/// # Call Order (CRITICAL - Red Team F2)
/// 1. `new()` - At function entry (captures start Instant)
/// 2. `mark_signal_received()` - AFTER create_orders() returns
/// 3. `mark_order_sent()` - Before tokio::join! on place_order
/// 4. `mark_order_confirmed()` - After tokio::join! completes
/// 5. `total_latency_ms()` - For result struct
#[derive(Debug, Clone)]
pub struct TradeTimings {
    start: Instant,
    pub t_signal: u64,
    pub t_order_sent: u64,
    pub t_order_confirmed: u64,
}

impl TradeTimings {
    /// Create new timing tracker. Does NOT capture t_signal (Red Team F1)
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            t_signal: 0, // Captured explicitly via mark_signal_received()
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
///
/// Called from runtime.rs AFTER verify_positions() returns real fill prices.
/// The `verified_vest_price` and `verified_paradex_price` override the
/// (often 0.0) avg_price from the initial order response.
pub fn log_successful_trade(
    opportunity: &SpreadOpportunity,
    result: &DeltaNeutralResult,
    timings: &TradeTimings,
    verified_vest_price: f64,
    verified_paradex_price: f64,
) {
    // Use verified prices (from get_position) instead of avg_price (often None for IOC)
    let vest_price = if verified_vest_price > 0.0 {
        verified_vest_price
    } else {
        result.vest_fill_price
    };
    let paradex_price = if verified_paradex_price > 0.0 {
        verified_paradex_price
    } else {
        result.paradex_fill_price
    };

    let direction_str = format!("{:?}", opportunity.direction);
    let entry_event = TradingEvent::trade_entry(
        &opportunity.pair,
        opportunity.spread_percent,
        0.0, // spread_threshold not needed for entry log
        &direction_str,
        &result.long_exchange,
        &result.short_exchange,
        result.execution_latency_ms,
        vest_price,
        paradex_price,
    );
    log_event(&entry_event);

    // Calculate execution spread from verified prices
    let execution_spread = calculate_execution_spread(vest_price, paradex_price);

    let timing = TimingBreakdown::new(
        opportunity.detected_at_ms,
        timings.t_signal,
        timings.t_order_sent,
        timings.t_order_confirmed,
    );

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
// Position Verification (Task 1-3: Red Team V1 hardened)
// =============================================================================

/// Position verification data for entry confirmation
///
/// Created via `from_positions()` which extracts prices from
/// already-fetched position results (no lock acquisition).
///
/// Note: Intentionally private to this module - fields accessed only
/// within module scope (including tests).
struct PositionVerification {
    vest_price: f64,
    paradex_price: f64,
    captured_spread: f64,
}

impl PositionVerification {
    /// Create from already-fetched position results
    ///
    /// # Red Team V1 Fix
    /// This method is pure and takes results that were already fetched.
    /// It does NOT acquire any locks, preventing deadlock risk.
    fn from_positions(
        vest_pos: &ExchangeResult<Option<crate::adapters::types::PositionInfo>>,
        paradex_pos: &ExchangeResult<Option<crate::adapters::types::PositionInfo>>,
        direction: Option<SpreadDirection>,
    ) -> Self {
        let vest_price = vest_pos
            .as_ref()
            .ok()
            .and_then(|p| p.as_ref())
            .map(|p| p.entry_price)
            .unwrap_or(0.0);

        let paradex_price = paradex_pos
            .as_ref()
            .ok()
            .and_then(|p| p.as_ref())
            .map(|p| p.entry_price)
            .unwrap_or(0.0);

        let captured_spread = direction
            .map(|dir| match dir {
                // AOverB: long Vest (buy), short Paradex (sell)
                SpreadDirection::AOverB => dir.calculate_captured_spread(vest_price, paradex_price),
                // BOverA: long Paradex (buy), short Vest (sell)
                SpreadDirection::BOverA => dir.calculate_captured_spread(paradex_price, vest_price),
            })
            .unwrap_or(0.0);

        Self {
            vest_price,
            paradex_price,
            captured_spread,
        }
    }

    /// Log structured entry verification summary
    fn log_summary(&self, entry_spread: f64, exit_target: f64, direction: Option<SpreadDirection>) {
        log_system_event(&SystemEvent::position_verified(
            self.vest_price,
            self.paradex_price,
            self.captured_spread,
        ));
        debug!(
            direction = ?direction.unwrap_or(SpreadDirection::AOverB),
            detected_spread = %format_pct(entry_spread),
            exit_target = %format_pct(exit_target),
            "Entry spread details"
        );
    }
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
    /// Fill price from Vest order (avg_fill_price)
    pub vest_fill_price: f64,
    /// Fill price from Paradex order (avg_fill_price)
    pub paradex_fill_price: f64,
    /// Exchange-reported realized PnL from Vest (includes funding + fees)
    pub vest_realized_pnl: Option<f64>,
    /// Exchange-reported realized PnL from Paradex
    pub paradex_realized_pnl: Option<f64>,
    /// Trade timing breakdown for deferred logging
    pub timings: Option<TradeTimings>,
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
            entry_direction: AtomicU8::new(0), // 0 = no position
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
                log_system_event(&SystemEvent::adapter_reconnect(
                    "paradex",
                    "stale - reconnecting",
                ));
                paradex.reconnect().await?;
            }
        }

        // Check Vest adapter
        {
            let mut vest = self.vest_adapter.lock().await;
            if vest.is_stale() {
                log_system_event(&SystemEvent::adapter_reconnect(
                    "vest",
                    "stale - reconnecting",
                ));
                vest.reconnect().await?;
            }
        }

        Ok(())
    }

    /// Get the default quantity used for trades
    pub fn get_default_quantity(&self) -> f64 {
        self.default_quantity
    }

    /// Test helper: simulate an open position so close_position can work
    #[cfg(test)]
    pub fn simulate_open_position(&self, direction: SpreadDirection) {
        self.position_open.store(true, Ordering::SeqCst);
        self.entry_direction
            .store(direction.to_u8(), Ordering::SeqCst);
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
        if self
            .position_open
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            debug!(
                event_type = "TRADE_SKIPPED",
                reason = "position_already_open",
                "Position already open - skipping entry"
            );
            return Err(ExchangeError::OrderRejected("Position already open".into()));
        }

        log_system_event(&SystemEvent::trade_started());
        let mut timings = TradeTimings::new();

        // Determine which exchange gets long vs short based on direction
        let (long_exchange, short_exchange) = opportunity.direction.to_exchanges();

        // Create unique client order IDs
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let long_order_id = format!("dn-long-{}", timestamp);
        let short_order_id = format!("dn-short-{}", timestamp);

        // Create order requests based on direction
        let (vest_order, paradex_order) =
            self.create_orders(&opportunity, long_exchange, &long_order_id, &short_order_id)?;

        // Capture t_signal AFTER create_orders (critical fix)
        timings.mark_signal_received();

        // Execute both orders in parallel (no retry)
        // We lock both adapters and then place orders concurrently
        let vest_guard = self.vest_adapter.lock().await;
        let paradex_guard = self.paradex_adapter.lock().await;

        // Mark order sent timestamp
        timings.mark_order_sent();

        let (vest_result, paradex_result) = tokio::join!(
            vest_guard.place_order(vest_order),
            paradex_guard.place_order(paradex_order)
        );

        // Mark order confirmed timestamp
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

        // Extract fill prices by exchange (use vest/paradex instead of long/short)
        let vest_fill_price = if long_exchange == "vest" {
            match &long_status {
                LegStatus::Success(resp) => resp.avg_price.unwrap_or(0.0),
                _ => 0.0,
            }
        } else {
            match &short_status {
                LegStatus::Success(resp) => resp.avg_price.unwrap_or(0.0),
                _ => 0.0,
            }
        };
        let paradex_fill_price = if long_exchange == "paradex" {
            match &long_status {
                LegStatus::Success(resp) => resp.avg_price.unwrap_or(0.0),
                _ => 0.0,
            }
        } else {
            match &short_status {
                LegStatus::Success(resp) => resp.avg_price.unwrap_or(0.0),
                _ => 0.0,
            }
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
            vest_fill_price,
            paradex_fill_price,
            vest_realized_pnl: None,
            paradex_realized_pnl: None,
            timings: Some(timings),
        };

        if success {
            // Store the entry direction for exit spread calculation
            self.entry_direction
                .store(opportunity.direction.to_u8(), Ordering::SeqCst);

            // TRADE_ENTRY + SLIPPAGE_ANALYSIS logging is deferred to runtime.rs
            // after verify_positions() provides real fill prices
        } else {
            // Rollback the successful leg if the other failed
            let long_ok = long_status.is_success();
            let short_ok = short_status.is_success();

            warn!(
                event_type = "TRADE_FAILED",
                spread = %format_pct(opportunity.spread_percent),
                long_success = long_ok,
                short_success = short_ok,
                latency_ms = execution_latency_ms,
                "Execution failed - attempting rollback"
            );

            // Attempt rollback of the filled leg
            if long_ok && !short_ok {
                // Long leg filled, short failed → close the long (sell on long_exchange)
                let rollback_result = if long_exchange == "vest" {
                    let order = OrderBuilder::new(
                        &self.vest_symbol,
                        OrderSide::Sell,
                        self.default_quantity,
                    )
                    .client_order_id(format!("rollback-vest-{}", timestamp))
                    .market()
                    .price(opportunity.dex_a_bid * (1.0 - SLIPPAGE_BUFFER_PCT))
                    // Vest rejects reduce_only - use correct side+qty instead
                    .build()
                    .map_err(|e| ExchangeError::InvalidOrder(e.to_string()));
                    match order {
                        Ok(o) => vest_guard.place_order(o).await,
                        Err(e) => Err(e),
                    }
                } else {
                    let order = OrderBuilder::new(
                        &self.paradex_symbol,
                        OrderSide::Sell,
                        self.default_quantity,
                    )
                    .client_order_id(format!("rollback-paradex-{}", timestamp))
                    .market()
                    .reduce_only()
                    .build()
                    .map_err(|e| ExchangeError::InvalidOrder(e.to_string()));
                    match order {
                        Ok(o) => paradex_guard.place_order(o).await,
                        Err(e) => Err(e),
                    }
                };
                match rollback_result {
                    Ok(_) => {
                        info!(event_type = "ROLLBACK_SUCCESS", exchange = %long_exchange, "Rolled back long leg")
                    }
                    Err(e) => {
                        error!(event_type = "ROLLBACK_FAILED", exchange = %long_exchange, error = %e, "CRITICAL: Rollback failed - manual intervention required")
                    }
                }
            } else if short_ok && !long_ok {
                // Short leg filled, long failed → close the short (buy on short_exchange)
                let rollback_result = if short_exchange == "vest" {
                    let order =
                        OrderBuilder::new(&self.vest_symbol, OrderSide::Buy, self.default_quantity)
                            .client_order_id(format!("rollback-vest-{}", timestamp))
                            .market()
                            .price(opportunity.dex_a_ask * (1.0 + SLIPPAGE_BUFFER_PCT))
                            // Vest rejects reduce_only - use correct side+qty instead
                            .build()
                            .map_err(|e| ExchangeError::InvalidOrder(e.to_string()));
                    match order {
                        Ok(o) => vest_guard.place_order(o).await,
                        Err(e) => Err(e),
                    }
                } else {
                    let order = OrderBuilder::new(
                        &self.paradex_symbol,
                        OrderSide::Buy,
                        self.default_quantity,
                    )
                    .client_order_id(format!("rollback-paradex-{}", timestamp))
                    .market()
                    .reduce_only()
                    .build()
                    .map_err(|e| ExchangeError::InvalidOrder(e.to_string()));
                    match order {
                        Ok(o) => paradex_guard.place_order(o).await,
                        Err(e) => Err(e),
                    }
                };
                match rollback_result {
                    Ok(_) => {
                        info!(event_type = "ROLLBACK_SUCCESS", exchange = %short_exchange, "Rolled back short leg")
                    }
                    Err(e) => {
                        error!(event_type = "ROLLBACK_FAILED", exchange = %short_exchange, error = %e, "CRITICAL: Rollback failed - manual intervention required")
                    }
                }
            }
            // else: both failed, nothing to rollback

            // Reset position guard so bot can continue trading
            self.position_open.store(false, Ordering::SeqCst);
            self.entry_direction.store(0, Ordering::SeqCst);
        }

        Ok(partial_result)
    }

    /// Verify positions on both exchanges after trade execution
    ///
    /// Logs entry prices, entry spread, and exit target to confirm trade placement.
    /// Returns (vest_entry_price, paradex_entry_price) for TUI display.
    pub async fn verify_positions(
        &self,
        entry_spread: f64,
        exit_spread_target: f64,
    ) -> (Option<f64>, Option<f64>) {
        let vest = self.vest_adapter.lock().await;
        let paradex = self.paradex_adapter.lock().await;

        let (vest_pos, paradex_pos) = tokio::join!(
            vest.get_position(&self.vest_symbol),
            paradex.get_position(&self.paradex_symbol)
        );

        // Use PositionVerification struct for extraction and logging (Task 4)
        let entry_direction = self.get_entry_direction();
        let verification =
            PositionVerification::from_positions(&vest_pos, &paradex_pos, entry_direction);
        verification.log_summary(entry_spread, exit_spread_target, entry_direction);

        // Extract entry prices for TUI
        let vest_entry = match &vest_pos {
            Ok(Some(pos)) => Some(pos.entry_price),
            _ => None,
        };
        let paradex_entry = match &paradex_pos {
            Ok(Some(pos)) => Some(pos.entry_price),
            _ => None,
        };

        // Individual position logging stays inline (different event_type)
        match vest_pos {
            Ok(Some(pos)) => log_system_event(&SystemEvent::position_detail(
                "vest",
                &pos.side,
                pos.quantity,
                pos.entry_price,
            )),
            Ok(None) => warn!(
                event_type = "POSITION_DETAIL",
                exchange = "vest",
                "No position"
            ),
            Err(e) => {
                warn!(event_type = "POSITION_DETAIL", exchange = "vest", error = %e, "Position check failed")
            }
        }

        match paradex_pos {
            Ok(Some(pos)) => log_system_event(&SystemEvent::position_detail(
                "paradex",
                &pos.side,
                pos.quantity,
                pos.entry_price,
            )),
            Ok(None) => warn!(
                event_type = "POSITION_DETAIL",
                exchange = "paradex",
                "No position"
            ),
            Err(e) => {
                warn!(event_type = "POSITION_DETAIL", exchange = "paradex", error = %e, "Position check failed")
            }
        }

        (vest_entry, paradex_entry)
    }

    /// Get the entry direction (0=none, 1=AOverB, 2=BOverA)
    pub fn get_entry_direction(&self) -> Option<SpreadDirection> {
        SpreadDirection::from_u8(self.entry_direction.load(Ordering::SeqCst))
    }

    /// Close the current position by executing inverse trades
    ///
    /// Vest: closes via opposite side + same quantity (reduce_only rejected by platform).
    /// Paradex: uses reduce_only=true to ensure we only close, not open new positions.
    /// Resets position_open and entry_direction after successful close.
    ///
    /// # Arguments
    /// - `exit_spread`: The spread percentage at exit time
    /// - `vest_bid`: Current best bid from Vest orderbook (already in RAM)
    /// - `vest_ask`: Current best ask from Vest orderbook (already in RAM)
    ///
    /// # Pricing Strategy
    /// Uses live orderbook prices (0ms) instead of `get_position()` API call (200-500ms).
    /// Applies SLIPPAGE_BUFFER_PCT to the live price, same pattern as entry.
    pub async fn close_position(
        &self,
        exit_spread: f64,
        vest_bid: f64,
        vest_ask: f64,
    ) -> ExchangeResult<DeltaNeutralResult> {
        let start = Instant::now();

        // Get the entry direction
        let entry_dir = match self.get_entry_direction() {
            Some(dir) => dir,
            None => {
                return Err(ExchangeError::InvalidOrder(
                    "No position to close".to_string(),
                ));
            }
        };

        // Determine close direction (inverse of entry)
        let (vest_side, paradex_side) = entry_dir.to_close_sides();

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);

        // Vest requires a limit price even for MARKET orders (slippage protection)
        // Use live orderbook price + slippage buffer (same pattern as entry)
        let vest_limit_price = if vest_side == OrderSide::Buy {
            // Buying to close short → use ask + slippage buffer above
            vest_ask * (1.0 + SLIPPAGE_BUFFER_PCT)
        } else {
            // Selling to close long → use bid - slippage buffer below
            vest_bid * (1.0 - SLIPPAGE_BUFFER_PCT)
        };

        debug!(
            event_type = "CLOSE_POSITION",
            vest_side = ?vest_side,
            vest_bid = %format!("{:.2}", vest_bid),
            vest_ask = %format!("{:.2}", vest_ask),
            vest_limit = %format!("{:.2}", vest_limit_price),
            slippage_pct = %format!("{:.3}", SLIPPAGE_BUFFER_PCT * 100.0),
            "Vest close price from live orderbook"
        );

        let vest_order = OrderBuilder::new(&self.vest_symbol, vest_side, self.default_quantity)
            .client_order_id(format!("close-vest-{}", timestamp))
            .market()
            .price(vest_limit_price) // Vest requires limit price for all orders
            // Vest rejects reduce_only - use correct side+qty instead
            .build()
            .map_err(|e| ExchangeError::InvalidOrder(e.to_string()))?;

        let paradex_order =
            OrderBuilder::new(&self.paradex_symbol, paradex_side, self.default_quantity)
                .client_order_id(format!("close-paradex-{}", timestamp))
                .market()
                .reduce_only()
                .build()
                .map_err(|e| ExchangeError::InvalidOrder(e.to_string()))?;

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
        // For close: the "long" side is closing the long (i.e. selling), "short" is closing the short (buying)
        let (long_exchange, short_exchange) = entry_dir.to_exchanges();
        let (long_status, short_status) = if matches!(entry_dir, SpreadDirection::AOverB) {
            (vest_status, paradex_status)
        } else {
            (paradex_status.clone(), vest_status.clone())
        };

        // Extract fill prices from close order responses for PnL calculation
        // CR-11 fix: The initial OrderResponse.avg_price is often 0.0 for IOC/MARKET orders.
        // First, get fallback prices and order IDs from the immediate response,
        // then query the exchange APIs for the real fill prices.

        // Determine which leg is vest vs paradex based on entry direction
        let (vest_leg, paradex_leg) = if matches!(entry_dir, SpreadDirection::AOverB) {
            (&long_status, &short_status)
        } else {
            (&short_status, &long_status)
        };

        // Extract fallback prices and order IDs from the immediate ACK
        let vest_fallback = match vest_leg {
            LegStatus::Success(resp) => resp.avg_price.unwrap_or(0.0),
            _ => 0.0,
        };
        let paradex_fallback = match paradex_leg {
            LegStatus::Success(resp) => resp.avg_price.unwrap_or(0.0),
            _ => 0.0,
        };
        let vest_order_id = match vest_leg {
            LegStatus::Success(resp) => Some(resp.order_id.clone()),
            _ => None,
        };
        let paradex_order_id = match paradex_leg {
            LegStatus::Success(resp) => Some(resp.order_id.clone()),
            _ => None,
        };

        // Drop the adapter guards before re-acquiring for fill-price queries
        drop(vest_guard);
        drop(paradex_guard);

        // Query real fill info from exchange APIs (best-effort, fallback to ACK price)
        let mut vest_realized_pnl: Option<f64> = None;
        let mut paradex_realized_pnl: Option<f64> = None;

        let vest_fill = if let Some(oid) = &vest_order_id {
            let guard = self.vest_adapter.lock().await;
            match guard.get_fill_info(&self.vest_symbol, oid).await {
                Ok(Some(info)) => {
                    tracing::info!(
                        order_id = %oid,
                        fill_price = %info.fill_price,
                        realized_pnl = ?info.realized_pnl,
                        fee = ?info.fee,
                        fallback = %vest_fallback,
                        "CR-11 fix: Using real Vest fill info from API"
                    );
                    vest_realized_pnl = info.realized_pnl;
                    info.fill_price
                }
                Ok(None) => {
                    tracing::warn!(
                        order_id = %oid,
                        fallback = %vest_fallback,
                        "Vest fill info not found, using fallback"
                    );
                    vest_fallback
                }
                Err(e) => {
                    tracing::warn!(
                        order_id = %oid,
                        error = %e,
                        fallback = %vest_fallback,
                        "Vest fill info query failed, using fallback"
                    );
                    vest_fallback
                }
            }
        } else {
            vest_fallback
        };

        let paradex_fill = if let Some(oid) = &paradex_order_id {
            let guard = self.paradex_adapter.lock().await;
            match guard.get_fill_info(&self.paradex_symbol, oid).await {
                Ok(Some(info)) => {
                    tracing::info!(
                        order_id = %oid,
                        fill_price = %info.fill_price,
                        realized_pnl = ?info.realized_pnl,
                        fee = ?info.fee,
                        fallback = %paradex_fallback,
                        "CR-11 fix: Using real Paradex fill info from API"
                    );
                    paradex_realized_pnl = info.realized_pnl;
                    info.fill_price
                }
                Ok(None) => {
                    tracing::warn!(
                        order_id = %oid,
                        fallback = %paradex_fallback,
                        "Paradex fill info not found, using fallback"
                    );
                    paradex_fallback
                }
                Err(e) => {
                    tracing::warn!(
                        order_id = %oid,
                        error = %e,
                        fallback = %paradex_fallback,
                        "Paradex fill info query failed, using fallback"
                    );
                    paradex_fallback
                }
            }
        } else {
            paradex_fallback
        };

        Ok(DeltaNeutralResult {
            long_order: long_status,
            short_order: short_status,
            execution_latency_ms,
            success,
            spread_percent: exit_spread,
            long_exchange: long_exchange.to_string(),
            short_exchange: short_exchange.to_string(),
            vest_fill_price: vest_fill,
            paradex_fill_price: paradex_fill,
            vest_realized_pnl,
            paradex_realized_pnl,
            timings: None,
        })
    }

    /// Create order requests for both exchanges based on spread direction
    fn create_orders(
        &self,
        opportunity: &SpreadOpportunity,
        long_exchange: &str,
        long_order_id: &str,
        short_order_id: &str,
    ) -> Result<(OrderRequest, OrderRequest), ExchangeError> {
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
        let vest_order = OrderBuilder::new(&self.vest_symbol, vest_side, quantity)
            .client_order_id(vest_order_id)
            .market()
            .price(vest_price) // Required by Vest as slippage protection (keeps Market type)
            .build()
            .map_err(|e| ExchangeError::InvalidOrder(e.to_string()))?;

        // Paradex: LIMIT IOC for immediate fill with price protection
        let paradex_order = OrderBuilder::new(&self.paradex_symbol, paradex_side, quantity)
            .client_order_id(paradex_order_id)
            .limit(paradex_price)
            .build()
            .map_err(|e| ExchangeError::InvalidOrder(e.to_string()))?;

        Ok((vest_order, paradex_order))
    }
}

/// Convert ExchangeResult to LegStatus
fn result_to_leg_status(result: ExchangeResult<OrderResponse>, exchange: &str) -> LegStatus {
    match result {
        Ok(response) => {
            // Check actual order status - REJECTED/CANCELLED are failures even if API returned 200
            match response.status {
                OrderStatus::Rejected => {
                    error!(exchange = %exchange, order_id = %response.order_id, "Order REJECTED by exchange");
                    LegStatus::Failed(format!("Order rejected by {}", exchange))
                }
                OrderStatus::Cancelled => {
                    warn!(exchange = %exchange, order_id = %response.order_id, "Order CANCELLED by exchange");
                    LegStatus::Failed(format!("Order cancelled by {}", exchange))
                }
                _ => LegStatus::Success(response),
            }
        }
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
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    // Use shared TestMockAdapter — alias for zero test-code changes
    use crate::adapters::test_utils::TestMockAdapter as MockAdapter;

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
        assert!(
            result.execution_latency_ms < 100,
            "Latency was {}ms",
            result.execution_latency_ms
        );
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

    // =========================================================================
    // PositionVerification Tests (Task 5-6)
    // =========================================================================

    #[test]
    fn test_position_verification_calculates_spread_correctly() {
        // Tech-spec: tech-spec-code-quality-phase-1.md Task 5
        use crate::adapters::{types::PositionInfo, ExchangeResult};
        use crate::core::spread::SpreadDirection;

        // Mock position results
        let vest_pos: ExchangeResult<Option<PositionInfo>> = Ok(Some(PositionInfo {
            symbol: "BTC-PERP".to_string(),
            quantity: 0.01,
            side: "long".to_string(),
            entry_price: 42000.0,
            mark_price: None,
            unrealized_pnl: 0.0,
        }));

        let paradex_pos: ExchangeResult<Option<PositionInfo>> = Ok(Some(PositionInfo {
            symbol: "BTC-USD-PERP".to_string(),
            quantity: 0.01,
            side: "short".to_string(),
            entry_price: 42100.0,
            mark_price: None,
            unrealized_pnl: 0.0,
        }));

        let verification = PositionVerification::from_positions(
            &vest_pos,
            &paradex_pos,
            Some(SpreadDirection::AOverB),
        );

        assert_eq!(verification.vest_price, 42000.0);
        assert_eq!(verification.paradex_price, 42100.0);
        // AOverB: (paradex - vest) / vest * 100 = (42100 - 42000) / 42000 * 100
        // Exact: 100.0 / 42000.0 = 0.00238095... (as percentage)
        let expected = (100.0 / 42000.0) * 100.0;
        assert!((verification.captured_spread - expected).abs() < 0.0001);
    }

    #[test]
    fn test_position_verification_handles_missing_positions() {
        // Tech-spec: tech-spec-code-quality-phase-1.md Task 6
        use crate::adapters::{types::PositionInfo, ExchangeResult};
        use crate::core::spread::SpreadDirection;

        // Division by zero handled in calculate_captured_spread()
        // See spread.rs: `if vest_price > 0.0 { ... } else { 0.0 }`

        // Missing vest position
        let vest_pos: ExchangeResult<Option<PositionInfo>> = Ok(None);
        let paradex_pos: ExchangeResult<Option<PositionInfo>> = Ok(Some(PositionInfo {
            symbol: "BTC-USD-PERP".to_string(),
            quantity: 0.01,
            side: "short".to_string(),
            entry_price: 42100.0,
            mark_price: None,
            unrealized_pnl: 0.0,
        }));

        let verification = PositionVerification::from_positions(
            &vest_pos,
            &paradex_pos,
            Some(SpreadDirection::AOverB),
        );

        // Missing position defaults to 0.0
        assert_eq!(verification.vest_price, 0.0);
        assert_eq!(verification.paradex_price, 42100.0);
        // Spread calculation handles gracefully (no panic)
        assert!(verification.captured_spread.is_finite());
    }

    #[test]
    fn test_position_verification_b_over_a_direction() {
        // F7 Fix: Symmetry test for BOverA direction
        use crate::adapters::{types::PositionInfo, ExchangeResult};
        use crate::core::spread::SpreadDirection;

        // BOverA: Long Paradex, Short Vest
        let vest_pos: ExchangeResult<Option<PositionInfo>> = Ok(Some(PositionInfo {
            symbol: "BTC-PERP".to_string(),
            quantity: 0.01,
            side: "short".to_string(),
            entry_price: 42100.0,
            mark_price: None,
            unrealized_pnl: 0.0,
        }));

        let paradex_pos: ExchangeResult<Option<PositionInfo>> = Ok(Some(PositionInfo {
            symbol: "BTC-USD-PERP".to_string(),
            quantity: 0.01,
            side: "long".to_string(),
            entry_price: 42000.0,
            mark_price: None,
            unrealized_pnl: 0.0,
        }));

        let verification = PositionVerification::from_positions(
            &vest_pos,
            &paradex_pos,
            Some(SpreadDirection::BOverA),
        );

        // BOverA: (vest - paradex) / paradex * 100
        let expected = (100.0 / 42000.0) * 100.0;
        assert!((verification.captured_spread - expected).abs() < 0.0001);
    }

    // =========================================================================
    // Additional Tests (Task 7-14)
    // =========================================================================

    #[tokio::test]
    async fn test_position_guard_prevents_duplicate_trade() {
        let vest = MockAdapter::new("vest");
        let paradex = MockAdapter::new("paradex");
        let vest_count = vest.order_count.clone();

        let executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
        );

        // First trade should succeed
        let opp1 = create_test_opportunity();
        let result1 = executor.execute_delta_neutral(opp1).await;
        assert!(result1.is_ok(), "First trade should succeed");

        // Second trade on same executor should be blocked by position guard
        let opp2 = create_test_opportunity();
        let result2 = executor.execute_delta_neutral(opp2).await;
        assert!(
            result2.is_err(),
            "Second trade should be rejected by position guard"
        );

        // Only 2 orders placed (vest + paradex from first trade)
        assert_eq!(vest_count.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn test_single_leg_failure_triggers_rollback_state() {
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

        // On single-leg failure, result exists but success=false
        assert!(!result.success, "Result should indicate failure");
        // One leg succeeded, one failed
        let long_ok = result.long_order.is_success();
        let short_ok = result.short_order.is_success();
        assert!(
            long_ok != short_ok,
            "Exactly one leg should succeed in partial failure"
        );
    }

    #[test]
    fn test_result_to_leg_status_rejected_order() {
        let response = OrderResponse {
            order_id: "test-123".to_string(),
            client_order_id: "client-123".to_string(),
            status: OrderStatus::Rejected,
            filled_quantity: 0.0,
            avg_price: None,
        };

        let status = result_to_leg_status(Ok(response), "vest");
        assert!(
            !status.is_success(),
            "Rejected order should be treated as failure"
        );
    }

    #[test]
    fn test_result_to_leg_status_cancelled_order() {
        let response = OrderResponse {
            order_id: "test-456".to_string(),
            client_order_id: "client-456".to_string(),
            status: OrderStatus::Cancelled,
            filled_quantity: 0.0,
            avg_price: None,
        };

        let status = result_to_leg_status(Ok(response), "paradex");
        assert!(
            !status.is_success(),
            "Cancelled order should be treated as failure"
        );
    }

    #[test]
    fn test_result_to_leg_status_error() {
        let err = ExchangeError::OrderRejected("insufficient margin".into());
        let status = result_to_leg_status(Err(err), "vest");
        assert!(!status.is_success(), "Error should be treated as failure");
    }

    #[tokio::test]
    async fn test_force_reset_position_guard() {
        let vest = MockAdapter::new("vest");
        let paradex = MockAdapter::new("paradex");

        let executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
        );

        // First trade succeeds and locks guard
        let opp = create_test_opportunity();
        let _ = executor.execute_delta_neutral(opp).await;

        // Guard is now locked
        assert!(
            executor
                .position_open
                .load(std::sync::atomic::Ordering::SeqCst),
            "Guard should be set after trade"
        );

        // Force reset
        executor
            .position_open
            .store(false, std::sync::atomic::Ordering::SeqCst);

        // Now another trade should succeed
        let opp2 = create_test_opportunity();
        let result = executor.execute_delta_neutral(opp2).await;
        assert!(
            result.is_ok(),
            "Trade should succeed after guard reset"
        );
    }

    #[test]
    fn test_trade_timings_total_latency() {
        let timings = TradeTimings::new();
        // total_latency_ms() returns start.elapsed() — wall-clock time since new()
        // Sleep briefly to ensure it's measurable
        std::thread::sleep(std::time::Duration::from_millis(5));
        let latency = timings.total_latency_ms();
        assert!(latency >= 5, "Total latency {}ms should be >= 5ms", latency);
        assert!(latency < 500, "Total latency {}ms should be < 500ms (sanity)", latency);
    }

    #[test]
    fn test_delta_neutral_result_fields() {
        let result = DeltaNeutralResult {
            long_order: LegStatus::Success(OrderResponse {
                order_id: "long-1".to_string(),
                client_order_id: "cl-long-1".to_string(),
                status: OrderStatus::Filled,
                filled_quantity: 0.01,
                avg_price: Some(42000.0),
            }),
            short_order: LegStatus::Failed("test failure".to_string()),
            execution_latency_ms: 15,
            success: false,
            spread_percent: 0.35,
            long_exchange: "vest".to_string(),
            short_exchange: "paradex".to_string(),
            vest_fill_price: 42000.0,
            paradex_fill_price: 0.0,
            vest_realized_pnl: None,
            paradex_realized_pnl: None,
            timings: None,
        };

        assert!(result.long_order.is_success());
        assert!(!result.short_order.is_success());
        assert!(!result.success);
        assert_eq!(result.execution_latency_ms, 15);
        assert_eq!(result.long_exchange, "vest");
        assert_eq!(result.short_exchange, "paradex");
    }
}
