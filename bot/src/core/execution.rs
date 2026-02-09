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
// Mutex removed (Axe 7): executor is single-owner, no shared access
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
/// The `verified_dex_a_price` and `verified_dex_b_price` override the
/// (often 0.0) avg_price from the initial order response.
pub fn log_successful_trade(
    opportunity: &SpreadOpportunity,
    result: &DeltaNeutralResult,
    timings: &TradeTimings,
    verified_dex_a_price: f64,
    verified_dex_b_price: f64,
) {
    // Use verified prices (from get_position) instead of avg_price (often None for IOC)
    let dex_a_price = if verified_dex_a_price > 0.0 {
        verified_dex_a_price
    } else {
        result.dex_a_fill_price
    };
    let dex_b_price = if verified_dex_b_price > 0.0 {
        verified_dex_b_price
    } else {
        result.dex_b_fill_price
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
        dex_a_price,
        dex_b_price,
    );
    log_event(&entry_event);

    // Calculate execution spread from verified prices (direction-aware)
    let (long_price, short_price) = match opportunity.direction {
        crate::core::spread::SpreadDirection::AOverB => (dex_a_price, dex_b_price),
        crate::core::spread::SpreadDirection::BOverA => (dex_b_price, dex_a_price),
    };
    let execution_spread = calculate_execution_spread(long_price, short_price);

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
    price_a: f64,
    price_b: f64,
    captured_spread: f64,
}

impl PositionVerification {
    fn from_positions(
        pos_a: &ExchangeResult<Option<crate::adapters::types::PositionInfo>>,
        pos_b: &ExchangeResult<Option<crate::adapters::types::PositionInfo>>,
        direction: Option<SpreadDirection>,
    ) -> Self {
        let price_a = pos_a
            .as_ref()
            .ok()
            .and_then(|p| p.as_ref())
            .map(|p| p.entry_price)
            .unwrap_or(0.0);

        let price_b = pos_b
            .as_ref()
            .ok()
            .and_then(|p| p.as_ref())
            .map(|p| p.entry_price)
            .unwrap_or(0.0);

        let captured_spread = direction
            .map(|dir| match dir {
                SpreadDirection::AOverB => dir.calculate_captured_spread(price_a, price_b),
                SpreadDirection::BOverA => dir.calculate_captured_spread(price_b, price_a),
            })
            .unwrap_or(0.0);

        Self {
            price_a,
            price_b,
            captured_spread,
        }
    }

    fn log_summary(&self, entry_spread: f64, exit_target: f64, direction: Option<SpreadDirection>) {
        log_system_event(&SystemEvent::position_verified(
            self.price_a,
            self.price_b,
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
    /// Fill price from DEX A order (avg_fill_price)
    pub dex_a_fill_price: f64,
    /// Fill price from DEX B order (avg_fill_price)
    pub dex_b_fill_price: f64,
    /// Exchange-reported realized PnL from DEX A (includes funding + fees)
    pub dex_a_realized_pnl: Option<f64>,
    /// Exchange-reported realized PnL from DEX B
    pub dex_b_realized_pnl: Option<f64>,
    /// Trade timing breakdown for deferred logging
    pub timings: Option<TradeTimings>,
}

// =============================================================================
// DeltaNeutralExecutor
// =============================================================================

/// Executor for delta-neutral trades across two exchanges
///
/// Uses `tokio::join!` for parallel order placement to minimize latency.
pub struct DeltaNeutralExecutor<A, B>
where
    A: ExchangeAdapter + Send + Sync,
    B: ExchangeAdapter + Send + Sync,
{
    adapter_a: A,
    adapter_b: B,
    /// Fixed quantity for MVP
    default_quantity: f64,
    /// Symbol for DEX A (e.g., "BTC-PERP")
    symbol_a: String,
    /// Symbol for DEX B (e.g., "BTC-USD-PERP")
    symbol_b: String,
    /// Human-readable name for DEX A (e.g., "vest")
    dex_a_name: String,
    /// Human-readable name for DEX B (e.g., "paradex")
    dex_b_name: String,
    /// Atomic guard: true = position open, false = can trade
    position_open: AtomicBool,
    /// Entry direction: 0=none, 1=AOverB (long A), 2=BOverA (long B)
    entry_direction: AtomicU8,
}

impl<A, B> DeltaNeutralExecutor<A, B>
where
    A: ExchangeAdapter + Send + Sync,
    B: ExchangeAdapter + Send + Sync,
{
    /// Create a new DeltaNeutralExecutor with both adapters
    pub fn new(
        adapter_a: A,
        adapter_b: B,
        default_quantity: f64,
        symbol_a: String,
        symbol_b: String,
        dex_a_name: String,
        dex_b_name: String,
    ) -> Self {
        Self {
            adapter_a,
            adapter_b,
            default_quantity,
            symbol_a,
            symbol_b,
            dex_a_name,
            dex_b_name,
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
    pub async fn ensure_ready(&mut self) -> ExchangeResult<()> {
        // Check and refresh adapter B if needed
        if self.adapter_b.is_stale() {
            log_system_event(&SystemEvent::adapter_reconnect(
                &self.dex_b_name,
                "stale - reconnecting",
            ));
            self.adapter_b.reconnect().await?;
        }

        // Check adapter A
        if self.adapter_a.is_stale() {
            log_system_event(&SystemEvent::adapter_reconnect(
                &self.dex_a_name,
                "stale - reconnecting",
            ));
            self.adapter_a.reconnect().await?;
        }

        Ok(())
    }

    /// Get the default quantity used for trades
    pub fn get_default_quantity(&self) -> f64 {
        self.default_quantity
    }

    /// Update the stored quantity (used after scaling layers fill)
    /// This ensures close_position uses the correct total filled size.
    pub fn set_quantity(&mut self, qty: f64) {
        self.default_quantity = qty;
    }

    /// Mark an open position in executor state (used by recovery and tests)
    pub fn mark_position_open(&self, direction: SpreadDirection) {
        self.position_open.store(true, Ordering::SeqCst);
        self.entry_direction
            .store(direction.to_u8(), Ordering::SeqCst);
    }

    /// Test helper alias
    #[cfg(test)]
    pub fn simulate_open_position(&self, direction: SpreadDirection) {
        self.mark_position_open(direction);
    }

    /// Recover an existing position from exchange state at startup.
    ///
    /// Queries both adapters for open positions on the configured symbols.
    /// If positions are found on both exchanges, determines the SpreadDirection
    /// and sets internal state so the bot can resume exit monitoring.
    ///
    /// Returns `Some((direction, quantity, dex_a_entry_price, dex_b_entry_price))` if recovered.
    pub async fn recover_position(&mut self) -> Option<(SpreadDirection, f64, f64, f64)> {
        tracing::info!(
            event_type = "POSITION_RECOVERY",
            symbol_a = %self.symbol_a,
            symbol_b = %self.symbol_b,
            "Checking for existing positions on startup..."
        );

        let (pos_a, pos_b) = tokio::join!(
            self.adapter_a.get_position(&self.symbol_a),
            self.adapter_b.get_position(&self.symbol_b)
        );

        let position_a = match pos_a {
            Ok(Some(pos)) => {
                tracing::info!(
                    event_type = "POSITION_RECOVERY",
                    exchange = %self.dex_a_name,
                    side = %pos.side,
                    quantity = %format!("{:.6}", pos.quantity),
                    entry_price = %format!("{:.2}", pos.entry_price),
                    "Found existing position on DEX A"
                );
                pos
            }
            Ok(None) => {
                tracing::info!(
                    event_type = "POSITION_RECOVERY",
                    "No existing positions found — starting fresh"
                );
                return None;
            }
            Err(e) => {
                tracing::warn!(
                    event_type = "POSITION_RECOVERY",
                    error = %e,
                    exchange = %self.dex_a_name,
                    "Failed to query position — starting fresh"
                );
                return None;
            }
        };

        let position_b = match pos_b {
            Ok(Some(pos)) => {
                tracing::info!(
                    event_type = "POSITION_RECOVERY",
                    exchange = %self.dex_b_name,
                    side = %pos.side,
                    quantity = %format!("{:.6}", pos.quantity),
                    entry_price = %format!("{:.2}", pos.entry_price),
                    "Found existing position on DEX B"
                );
                pos
            }
            Ok(None) => {
                tracing::warn!(
                    event_type = "POSITION_RECOVERY",
                    dex_a = %self.dex_a_name,
                    dex_b = %self.dex_b_name,
                    "DEX A has position but DEX B does not — ORPHANED position!"
                );
                return None;
            }
            Err(e) => {
                tracing::warn!(
                    event_type = "POSITION_RECOVERY",
                    error = %e,
                    exchange = %self.dex_b_name,
                    "Failed to query position — starting fresh"
                );
                return None;
            }
        };

        // Determine direction from position sides
        // AOverB: A=long, B=short
        // BOverA: A=short, B=long
        let direction = if position_a.side == "long" && position_b.side == "short" {
            SpreadDirection::AOverB
        } else if position_a.side == "short" && position_b.side == "long" {
            SpreadDirection::BOverA
        } else {
            tracing::error!(
                event_type = "POSITION_RECOVERY",
                dex_a_side = %position_a.side,
                dex_b_side = %position_b.side,
                "Unexpected position sides — both same direction? Cannot recover"
            );
            return None;
        };

        // Use the larger quantity (should be equal, but be safe)
        let quantity = position_a.quantity.max(position_b.quantity);

        // Set executor state
        self.mark_position_open(direction);
        self.set_quantity(quantity);

        tracing::info!(
            event_type = "POSITION_RECOVERY",
            direction = ?direction,
            quantity = %format!("{:.6}", quantity),
            dex_a_entry = %format!("{:.2}", position_a.entry_price),
            dex_b_entry = %format!("{:.2}", position_b.entry_price),
            "✅ Position recovered — will resume exit monitoring"
        );

        Some((direction, quantity, position_a.entry_price, position_b.entry_price))
    }

    /// Execute delta-neutral trade based on spread opportunity
    ///
    /// Places a long order on one exchange and short order on the other,
    /// in parallel using `tokio::join!`.
    pub async fn execute_delta_neutral(
        &mut self,
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
        let (long_exchange, short_exchange) = opportunity.direction.to_exchanges(&self.dex_a_name, &self.dex_b_name);

        // Create unique client order IDs
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let long_order_id = format!("dn-long-{}", timestamp);
        let short_order_id = format!("dn-short-{}", timestamp);

        // Create order requests based on direction
        let (order_a, order_b) =
            self.create_orders(&opportunity, long_exchange, &long_order_id, &short_order_id)?;

        // Capture t_signal AFTER create_orders (critical fix)
        timings.mark_signal_received();

        // Execute both orders in parallel (no retry)
        // No lock needed — single-owner executor (Axe 7)

        // Mark order sent timestamp
        timings.mark_order_sent();

        let (result_a, result_b) = tokio::join!(
            self.adapter_a.place_order(order_a),
            self.adapter_b.place_order(order_b)
        );

        // Mark order confirmed timestamp
        timings.mark_order_confirmed();
        let execution_latency_ms = timings.total_latency_ms();

        // Convert results to LegStatus — A is always first, B is always second
        let status_a = result_to_leg_status(result_a, &self.dex_a_name);
        let status_b = result_to_leg_status(result_b, &self.dex_b_name);
        let (long_status, short_status) = if long_exchange == self.dex_a_name {
            (status_a, status_b)
        } else {
            (status_b, status_a)
        };

        let success = long_status.is_success() && short_status.is_success();

        // Extract fill prices by exchange (A/B, not long/short)
        let dex_a_fill_price = if long_exchange == self.dex_a_name {
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
        let dex_b_fill_price = if long_exchange == self.dex_b_name {
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
            dex_a_fill_price,
            dex_b_fill_price,
            dex_a_realized_pnl: None,
            dex_b_realized_pnl: None,
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
                let rollback_result = if long_exchange == self.dex_a_name {
                    let order = OrderBuilder::new(
                        &self.symbol_a,
                        OrderSide::Sell,
                        self.default_quantity,
                    )
                    .client_order_id(format!("rollback-a-{}", timestamp))
                    .market()
                    .price(opportunity.dex_a_bid * (1.0 - SLIPPAGE_BUFFER_PCT))
                    .build()
                    .map_err(|e| ExchangeError::InvalidOrder(e.to_string()));
                    match order {
                        Ok(o) => self.adapter_a.place_order(o).await,
                        Err(e) => Err(e),
                    }
                } else {
                    let order = OrderBuilder::new(
                        &self.symbol_b,
                        OrderSide::Sell,
                        self.default_quantity,
                    )
                    .client_order_id(format!("rollback-b-{}", timestamp))
                    .market()
                    .reduce_only()
                    .build()
                    .map_err(|e| ExchangeError::InvalidOrder(e.to_string()));
                    match order {
                        Ok(o) => self.adapter_b.place_order(o).await,
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
                let rollback_result = if short_exchange == self.dex_a_name {
                    let order =
                        OrderBuilder::new(&self.symbol_a, OrderSide::Buy, self.default_quantity)
                            .client_order_id(format!("rollback-a-{}", timestamp))
                            .market()
                            .price(opportunity.dex_a_ask * (1.0 + SLIPPAGE_BUFFER_PCT))
                            .build()
                            .map_err(|e| ExchangeError::InvalidOrder(e.to_string()));
                    match order {
                        Ok(o) => self.adapter_a.place_order(o).await,
                        Err(e) => Err(e),
                    }
                } else {
                    let order = OrderBuilder::new(
                        &self.symbol_b,
                        OrderSide::Buy,
                        self.default_quantity,
                    )
                    .client_order_id(format!("rollback-b-{}", timestamp))
                    .market()
                    .reduce_only()
                    .build()
                    .map_err(|e| ExchangeError::InvalidOrder(e.to_string()));
                    match order {
                        Ok(o) => self.adapter_b.place_order(o).await,
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

    /// Execute delta-neutral trade with explicit quantity (for scaling layers)
    ///
    /// Same as `execute_delta_neutral` but uses the provided quantity instead
    /// of `self.default_quantity`. The position_open guard is NOT checked here
    /// because the runtime manages layer-level state. The caller must ensure
    /// this is only called as part of an active scaling-in sequence.
    pub async fn execute_delta_neutral_with_quantity(
        &mut self,
        opportunity: SpreadOpportunity,
        quantity: f64,
        layer_index: usize,
        is_first_layer: bool,
    ) -> ExchangeResult<DeltaNeutralResult> {
        // For the first layer, use the normal atomic guard
        if is_first_layer {
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
        } else {
            // Subsequent layers: verify position IS open
            if !self.position_open.load(Ordering::SeqCst) {
                return Err(ExchangeError::OrderRejected(
                    "No position open for layer add-on".into(),
                ));
            }
        }

        log_system_event(&SystemEvent::trade_started());
        info!(
            event_type = "LAYER_ENTRY",
            layer = layer_index,
            quantity = %format!("{:.6}", quantity),
            spread = %format_pct(opportunity.spread_percent),
            "Entering scaling layer"
        );
        let mut timings = TradeTimings::new();

        let (long_exchange, short_exchange) = opportunity.direction.to_exchanges(&self.dex_a_name, &self.dex_b_name);

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let long_order_id = format!("dn-L{}-long-{}", layer_index, timestamp);
        let short_order_id = format!("dn-L{}-short-{}", layer_index, timestamp);

        let (order_a, order_b) = self.create_orders_with_quantity(
            &opportunity,
            long_exchange,
            &long_order_id,
            &short_order_id,
            quantity,
        )?;

        timings.mark_signal_received();
        timings.mark_order_sent();

        let (result_a, result_b) = tokio::join!(
            self.adapter_a.place_order(order_a),
            self.adapter_b.place_order(order_b)
        );

        timings.mark_order_confirmed();
        let execution_latency_ms = timings.total_latency_ms();

        let status_a = result_to_leg_status(result_a, &self.dex_a_name);
        let status_b = result_to_leg_status(result_b, &self.dex_b_name);
        let (long_status, short_status) = if long_exchange == self.dex_a_name {
            (status_a, status_b)
        } else {
            (status_b, status_a)
        };

        let success = long_status.is_success() && short_status.is_success();

        let dex_a_fill_price = if long_exchange == self.dex_a_name {
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
        let dex_b_fill_price = if long_exchange == self.dex_b_name {
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

        let partial_result = DeltaNeutralResult {
            long_order: long_status.clone(),
            short_order: short_status.clone(),
            execution_latency_ms,
            success,
            spread_percent: opportunity.spread_percent,
            long_exchange: long_exchange.to_string(),
            short_exchange: short_exchange.to_string(),
            dex_a_fill_price,
            dex_b_fill_price,
            dex_a_realized_pnl: None,
            dex_b_realized_pnl: None,
            timings: Some(timings),
        };

        if success {
            self.entry_direction
                .store(opportunity.direction.to_u8(), Ordering::SeqCst);
            info!(
                event_type = "LAYER_FILLED",
                layer = layer_index,
                quantity = %format!("{:.6}", quantity),
                latency_ms = execution_latency_ms,
                "Layer filled successfully"
            );
        } else {
            warn!(
                event_type = "LAYER_FAILED",
                layer = layer_index,
                spread = %format_pct(opportunity.spread_percent),
                latency_ms = execution_latency_ms,
                "Layer entry failed"
            );
            // If first layer failed, reset position guard
            if is_first_layer {
                self.position_open.store(false, Ordering::SeqCst);
                self.entry_direction.store(0, Ordering::SeqCst);
            }
            // Note: for subsequent layer failures, we keep the position open
            // since earlier layers are already filled
        }

        Ok(partial_result)
    }

    /// Verify positions on both exchanges after trade execution
    ///
    /// Logs entry prices, entry spread, and exit target to confirm trade placement.
    /// Returns (dex_a_entry_price, dex_b_entry_price) for TUI display.
    pub async fn verify_positions(
        &mut self,
        entry_spread: f64,
        exit_spread_target: f64,
    ) -> (Option<f64>, Option<f64>) {
        let (pos_a, pos_b) = tokio::join!(
            self.adapter_a.get_position(&self.symbol_a),
            self.adapter_b.get_position(&self.symbol_b)
        );

        // Use PositionVerification struct for extraction and logging (Task 4)
        let entry_direction = self.get_entry_direction();
        let verification =
            PositionVerification::from_positions(&pos_a, &pos_b, entry_direction);
        verification.log_summary(entry_spread, exit_spread_target, entry_direction);

        // Extract entry prices for TUI
        let entry_a = match &pos_a {
            Ok(Some(pos)) => Some(pos.entry_price),
            _ => None,
        };
        let entry_b = match &pos_b {
            Ok(Some(pos)) => Some(pos.entry_price),
            _ => None,
        };

        // Individual position logging stays inline (different event_type)
        match pos_a {
            Ok(Some(pos)) => log_system_event(&SystemEvent::position_detail(
                &self.dex_a_name,
                &pos.side,
                pos.quantity,
                pos.entry_price,
            )),
            Ok(None) => warn!(
                event_type = "POSITION_DETAIL",
                exchange = %self.dex_a_name,
                "No position"
            ),
            Err(e) => {
                warn!(event_type = "POSITION_DETAIL", exchange = %self.dex_a_name, error = %e, "Position check failed")
            }
        }

        match pos_b {
            Ok(Some(pos)) => log_system_event(&SystemEvent::position_detail(
                &self.dex_b_name,
                &pos.side,
                pos.quantity,
                pos.entry_price,
            )),
            Ok(None) => warn!(
                event_type = "POSITION_DETAIL",
                exchange = %self.dex_b_name,
                "No position"
            ),
            Err(e) => {
                warn!(event_type = "POSITION_DETAIL", exchange = %self.dex_b_name, error = %e, "Position check failed")
            }
        }

        (entry_a, entry_b)
    }

    /// Get the entry direction (0=none, 1=AOverB, 2=BOverA)
    pub fn get_entry_direction(&self) -> Option<SpreadDirection> {
        SpreadDirection::from_u8(self.entry_direction.load(Ordering::SeqCst))
    }

    /// Close the current position by executing inverse trades
    ///
    /// DEX A: closes via opposite side + same quantity.
    /// DEX B: uses LIMIT IOC + reduce_only for price protection while closing.
    /// Resets position_open and entry_direction after successful close.
    ///
    /// # Arguments
    /// - `exit_spread`: The spread percentage at exit time
    /// - `dex_a_bid`: Current best bid from DEX A orderbook
    /// - `dex_a_ask`: Current best ask from DEX A orderbook
    /// - `dex_b_bid`: Current best bid from DEX B orderbook
    /// - `dex_b_ask`: Current best ask from DEX B orderbook
    ///
    /// # Pricing Strategy
    /// Uses live orderbook prices (0ms) instead of `get_position()` API call (200-500ms).
    /// Both legs use SLIPPAGE_BUFFER_PCT for price protection, same pattern as entry.
    pub async fn close_position(
        &mut self,
        exit_spread: f64,
        dex_a_bid: f64,
        dex_a_ask: f64,
        dex_b_bid: f64,
        dex_b_ask: f64,
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
        let (side_a, side_b) = entry_dir.to_close_sides();

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);

        // DEX A requires a limit price even for MARKET orders (slippage protection)
        let limit_price_a = if side_a == OrderSide::Buy {
            dex_a_ask * (1.0 + SLIPPAGE_BUFFER_PCT)
        } else {
            dex_a_bid * (1.0 - SLIPPAGE_BUFFER_PCT)
        };

        // DEX B: LIMIT IOC with slippage buffer
        let limit_price_b = if side_b == OrderSide::Buy {
            dex_b_ask * (1.0 + SLIPPAGE_BUFFER_PCT)
        } else {
            dex_b_bid * (1.0 - SLIPPAGE_BUFFER_PCT)
        };

        debug!(
            event_type = "CLOSE_POSITION",
            side_a = ?side_a,
            dex_a_bid = %format!("{:.2}", dex_a_bid),
            dex_a_ask = %format!("{:.2}", dex_a_ask),
            limit_a = %format!("{:.2}", limit_price_a),
            side_b = ?side_b,
            dex_b_bid = %format!("{:.2}", dex_b_bid),
            dex_b_ask = %format!("{:.2}", dex_b_ask),
            limit_b = %format!("{:.2}", limit_price_b),
            slippage_pct = %format!("{:.3}", SLIPPAGE_BUFFER_PCT * 100.0),
            "Close prices from live orderbook (LIMIT IOC both legs)"
        );

        let order_a = OrderBuilder::new(&self.symbol_a, side_a, self.default_quantity)
            .client_order_id(format!("close-a-{}", timestamp))
            .market()
            .price(limit_price_a)
            .build()
            .map_err(|e| ExchangeError::InvalidOrder(e.to_string()))?;

        // DEX B: LIMIT IOC with price protection + reduce_only
        let order_b =
            OrderBuilder::new(&self.symbol_b, side_b, self.default_quantity)
                .client_order_id(format!("close-b-{}", timestamp))
                .limit(limit_price_b)
                .reduce_only()
                .build()
                .map_err(|e| ExchangeError::InvalidOrder(e.to_string()))?;

        // Execute close orders in parallel (no lock — Axe 7)

        let (result_a, result_b) = tokio::join!(
            self.adapter_a.place_order(order_a),
            self.adapter_b.place_order(order_b)
        );

        let execution_latency_ms = start.elapsed().as_millis() as u64;

        let status_a = result_to_leg_status(result_a, &self.dex_a_name);
        let status_b = result_to_leg_status(result_b, &self.dex_b_name);
        let success = status_a.is_success() && status_b.is_success();

        if success {
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
                dex_a_success = status_a.is_success(),
                dex_b_success = status_b.is_success(),
                latency_ms = execution_latency_ms,
                "Close failed - manual intervention required"
            );
        }

        // Determine which leg was long/short based on entry direction
        // For close: the "long" side is closing the long (i.e. selling), "short" is closing the short (buying)
        let (long_exchange, short_exchange) = entry_dir.to_exchanges(&self.dex_a_name, &self.dex_b_name);
        let (long_status, short_status) = if matches!(entry_dir, SpreadDirection::AOverB) {
            (status_a, status_b)
        } else {
            (status_b.clone(), status_a.clone())
        };

        // Extract fill prices from close order responses for PnL calculation
        // CR-11 fix: The initial OrderResponse.avg_price is often 0.0 for IOC/MARKET orders.
        // First, get fallback prices and order IDs from the immediate response,
        // then query the exchange APIs for the real fill prices.

        // Determine which leg is A vs B based on entry direction
        let (leg_a, leg_b) = if matches!(entry_dir, SpreadDirection::AOverB) {
            (&long_status, &short_status)
        } else {
            (&short_status, &long_status)
        };

        let fallback_a = match leg_a {
            LegStatus::Success(resp) => resp.avg_price.unwrap_or(0.0),
            _ => 0.0,
        };
        let fallback_b = match leg_b {
            LegStatus::Success(resp) => resp.avg_price.unwrap_or(0.0),
            _ => 0.0,
        };
        let order_id_a = match leg_a {
            LegStatus::Success(resp) => Some(resp.order_id.clone()),
            _ => None,
        };
        let order_id_b = match leg_b {
            LegStatus::Success(resp) => Some(resp.order_id.clone()),
            _ => None,
        };

        // No guard drops needed (Axe 7 — no Mutex)

        // Query real fill info from exchange APIs (best-effort, fallback to ACK price)
        let mut pnl_a: Option<f64> = None;
        let mut pnl_b: Option<f64> = None;

        let fill_a = if let Some(oid) = &order_id_a {
            match self.adapter_a.get_fill_info(&self.symbol_a, oid).await {
                Ok(Some(info)) => {
                    tracing::info!(
                        order_id = %oid,
                        fill_price = %info.fill_price,
                        realized_pnl = ?info.realized_pnl,
                        fee = ?info.fee,
                        fallback = %fallback_a,
                        "CR-11 fix: Using real DEX A fill info from API"
                    );
                    pnl_a = info.realized_pnl;
                    info.fill_price
                }
                Ok(None) => {
                    tracing::warn!(
                        order_id = %oid,
                        fallback = %fallback_a,
                        "DEX A fill info not found, using fallback"
                    );
                    fallback_a
                }
                Err(e) => {
                    tracing::warn!(
                        order_id = %oid,
                        error = %e,
                        fallback = %fallback_a,
                        "DEX A fill info query failed, using fallback"
                    );
                    fallback_a
                }
            }
        } else {
            fallback_a
        };

        let fill_b = if let Some(oid) = &order_id_b {
            match self.adapter_b.get_fill_info(&self.symbol_b, oid).await {
                Ok(Some(info)) => {
                    tracing::info!(
                        order_id = %oid,
                        fill_price = %info.fill_price,
                        realized_pnl = ?info.realized_pnl,
                        fee = ?info.fee,
                        fallback = %fallback_b,
                        "CR-11 fix: Using real DEX B fill info from API"
                    );
                    pnl_b = info.realized_pnl;
                    info.fill_price
                }
                Ok(None) => {
                    tracing::warn!(
                        order_id = %oid,
                        fallback = %fallback_b,
                        "DEX B fill info not found, using fallback"
                    );
                    fallback_b
                }
                Err(e) => {
                    tracing::warn!(
                        order_id = %oid,
                        error = %e,
                        fallback = %fallback_b,
                        "DEX B fill info query failed, using fallback"
                    );
                    fallback_b
                }
            }
        } else {
            fallback_b
        };

        Ok(DeltaNeutralResult {
            long_order: long_status,
            short_order: short_status,
            execution_latency_ms,
            success,
            spread_percent: exit_spread,
            long_exchange: long_exchange.to_string(),
            short_exchange: short_exchange.to_string(),
            dex_a_fill_price: fill_a,
            dex_b_fill_price: fill_b,
            dex_a_realized_pnl: pnl_a,
            dex_b_realized_pnl: pnl_b,
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
        self.create_orders_with_quantity(
            opportunity,
            long_exchange,
            long_order_id,
            short_order_id,
            self.default_quantity,
        )
    }

    /// Create order requests with explicit quantity (for scaling layers)
    fn create_orders_with_quantity(
        &self,
        opportunity: &SpreadOpportunity,
        long_exchange: &str,
        long_order_id: &str,
        short_order_id: &str,
        quantity: f64,
    ) -> Result<(OrderRequest, OrderRequest), ExchangeError> {

        // DEX A order
        let side_a = if long_exchange == self.dex_a_name {
            OrderSide::Buy
        } else {
            OrderSide::Sell
        };
        let order_id_a = if long_exchange == self.dex_a_name {
            long_order_id.to_string()
        } else {
            short_order_id.to_string()
        };

        // DEX B order
        let side_b = if long_exchange == self.dex_b_name {
            OrderSide::Buy
        } else {
            OrderSide::Sell
        };
        let order_id_b = if long_exchange == self.dex_b_name {
            long_order_id.to_string()
        } else {
            short_order_id.to_string()
        };

        // DEX A: Use MARKET orders with limitPrice as slippage protection
        let price_a = match side_a {
            OrderSide::Buy => opportunity.dex_a_ask * (1.0 + SLIPPAGE_BUFFER_PCT),
            OrderSide::Sell => opportunity.dex_a_bid * (1.0 - SLIPPAGE_BUFFER_PCT),
        };

        // DEX B: Use LIMIT IOC with slippage buffer to ensure fill
        let price_b = match side_b {
            OrderSide::Buy => opportunity.dex_b_ask * (1.0 + SLIPPAGE_BUFFER_PCT),
            OrderSide::Sell => opportunity.dex_b_bid * (1.0 - SLIPPAGE_BUFFER_PCT),
        };

        // DEX A: MARKET order (limitPrice acts as slippage protection)
        let order_a = OrderBuilder::new(&self.symbol_a, side_a, quantity)
            .client_order_id(order_id_a)
            .market()
            .price(price_a)
            .build()
            .map_err(|e| ExchangeError::InvalidOrder(e.to_string()))?;

        // DEX B: LIMIT IOC for immediate fill with price protection
        let order_b = OrderBuilder::new(&self.symbol_b, side_b, quantity)
            .client_order_id(order_id_b)
            .limit(price_b)
            .build()
            .map_err(|e| ExchangeError::InvalidOrder(e.to_string()))?;

        Ok((order_a, order_b))
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
            pair: Arc::from("BTC-PERP"),
            dex_a: "vest",
            dex_b: "paradex",
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
            "vest".to_string(),
            "paradex".to_string(),
        );

        assert_eq!(executor.default_quantity, 0.01);
        assert_eq!(executor.symbol_a, "BTC-PERP");
        assert_eq!(executor.symbol_b, "BTC-USD-PERP");
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

        let mut executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
            "vest".to_string(),
            "paradex".to_string(),
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

        let mut executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
            "vest".to_string(),
            "paradex".to_string(),
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

        let mut executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
            "vest".to_string(),
            "paradex".to_string(),
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

        let mut executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
            "vest".to_string(),
            "paradex".to_string(),
        );

        let opportunity = SpreadOpportunity {
            pair: Arc::from("BTC-PERP"),
            dex_a: "vest",
            dex_b: "paradex",
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

        let mut executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
            "vest".to_string(),
            "paradex".to_string(),
        );

        let opportunity = SpreadOpportunity {
            pair: Arc::from("BTC-PERP"),
            dex_a: "vest",
            dex_b: "paradex",
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

        let mut executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
            "vest".to_string(),
            "paradex".to_string(),
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

        assert_eq!(verification.price_a, 42000.0);
        assert_eq!(verification.price_b, 42100.0);
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
        assert_eq!(verification.price_a, 0.0);
        assert_eq!(verification.price_b, 42100.0);
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

        let mut executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
            "vest".to_string(),
            "paradex".to_string(),
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

        let mut executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
            "vest".to_string(),
            "paradex".to_string(),
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

        let mut executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
            "vest".to_string(),
            "paradex".to_string(),
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
            dex_a_fill_price: 42000.0,
            dex_b_fill_price: 0.0,
            dex_a_realized_pnl: None,
            dex_b_realized_pnl: None,
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
