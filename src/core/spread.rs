//! Spread calculation engine for arbitrage detection
//!
//! This module provides real-time spread calculation between two DEX orderbooks.
//! Performance target: <2ms per calculation (NFR2).
//!
//! # Architecture
//! - `SpreadCalculator`: Main calculator struct for a DEX pair
//! - `SpreadResult`: Result of spread calculation with direction and prices
//! - `SpreadTick`: Event struct for mpsc broadcast to execution tasks

use crate::adapters::types::Orderbook;
use serde::{Deserialize, Serialize};

// =============================================================================
// Core Types (Task 1 & 2)
// =============================================================================

/// Direction of the spread opportunity
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpreadDirection {
    /// ASK on DEX A > BID on DEX B (buy on A, sell on B)
    AOverB,
    /// ASK on DEX B > BID on DEX A (buy on B, sell on A)
    BOverA,
}

/// Result of spread calculation between two orderbooks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpreadResult {
    /// Spread percentage (positive = opportunity exists)
    pub spread_pct: f64,
    /// Direction of the arbitrage opportunity
    pub direction: SpreadDirection,
    /// Ask price on the buying exchange
    pub ask_price: f64,
    /// Bid price on the selling exchange
    pub bid_price: f64,
    /// Midpoint used for calculation
    pub midpoint: f64,
    /// Timestamp of calculation (Unix ms)
    pub timestamp_ms: u64,
}

/// Event struct for mpsc broadcast (Prep for Story 3.6)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpreadTick {
    /// Trading pair (e.g., "BTC-PERP")
    pub pair: String,
    /// DEX A identifier
    pub dex_a: String,
    /// DEX B identifier
    pub dex_b: String,
    /// Spread calculation result
    pub result: SpreadResult,
}

// =============================================================================
// SpreadCalculator (Task 1, 3, 4)
// =============================================================================

/// Spread calculator for a DEX pair
/// 
/// # Performance
/// All methods are optimized for <2ms latency:
/// - No allocations in hot path
/// - Uses primitive f64 operations
/// - References instead of cloning orderbooks
pub struct SpreadCalculator {
    /// DEX A identifier (e.g., "vest")
    pub dex_a: String,
    /// DEX B identifier (e.g., "paradex")
    pub dex_b: String,
}

impl SpreadCalculator {
    /// Create a new spread calculator for a DEX pair
    pub fn new(dex_a: impl Into<String>, dex_b: impl Into<String>) -> Self {
        Self {
            dex_a: dex_a.into(),
            dex_b: dex_b.into(),
        }
    }

    /// Calculate spread between two orderbooks
    /// 
    /// Returns `None` if either orderbook lacks best bid/ask (empty orderbook edge case).
    /// 
    /// # Performance
    /// - Inlined for maximum performance
    /// - No allocations (uses references)
    /// - Target: <200ns per call
    #[inline]
    #[must_use]
    pub fn calculate(
        &self,
        orderbook_a: &Orderbook,
        orderbook_b: &Orderbook,
    ) -> Option<SpreadResult> {
        // Get best prices from both orderbooks
        let ask_a = orderbook_a.best_ask()?;
        let bid_a = orderbook_a.best_bid()?;
        let ask_b = orderbook_b.best_ask()?;
        let bid_b = orderbook_b.best_bid()?;

        // Calculate spreads in both directions using CORRECT formula from arbitrage-v5:
        // spread = (bid_sell - ask_buy) / ask_buy * 100
        // This represents the actual profit percentage on entry
        // Positive spread = we can sell higher than we buy = PROFIT
        
        // Direction A→B: Buy on A (at ask_a), Sell on B (at bid_b)
        let spread_a_to_b = ((bid_b - ask_a) / ask_a) * 100.0;
        
        // Direction B→A: Buy on B (at ask_b), Sell on A (at bid_a)  
        let spread_b_to_a = ((bid_a - ask_b) / ask_b) * 100.0;

        let timestamp_ms = current_time_ms();
        let midpoint = (ask_a + bid_b + ask_b + bid_a) / 4.0;

        // Return the better (higher) spread opportunity
        if spread_a_to_b >= spread_b_to_a {
            // A→B: Buy on A (Vest), Sell on B (Paradex)
            Some(SpreadResult {
                spread_pct: spread_a_to_b,
                direction: SpreadDirection::AOverB,
                ask_price: ask_a,   // We BUY at this price (on A)
                bid_price: bid_b,   // We SELL at this price (on B)
                midpoint,
                timestamp_ms,
            })
        } else {
            // B→A: Buy on B (Paradex), Sell on A (Vest)
            Some(SpreadResult {
                spread_pct: spread_b_to_a,
                direction: SpreadDirection::BOverA,
                ask_price: ask_b,   // We BUY at this price (on B)
                bid_price: bid_a,   // We SELL at this price (on A)
                midpoint,
                timestamp_ms,
            })
        }
    }

    /// Raw spread calculation: (ask - bid) / midpoint * 100
    /// 
    /// Formula: spread_pct = ((ASK_A - BID_B) / midpoint) * 100
    /// 
    /// # Edge Cases (Task 1.4)
    /// - Returns 0.0 if midpoint is zero (prevents division by zero)
    #[inline]
    pub(crate) fn raw_spread(ask: f64, bid: f64) -> f64 {
        let midpoint = (ask + bid) / 2.0;
        if midpoint == 0.0 {
            return 0.0;
        }
        ((ask - bid) / midpoint) * 100.0
    }
    
    // =========================================================================
    // Story 11.1: Dual Spread Calculation Functions
    // =========================================================================
    
    /// Calculate Entry spread: (ASK_A - BID_B) / midpoint × 100
    /// 
    /// Use this for Entry Monitor: "Buy on DEX A, Sell on DEX B" arbitrage opportunity.
    /// 
    /// # Arguments
    /// * `ask_a` - Best ask price on DEX A (we would buy at this price)
    /// * `bid_b` - Best bid price on DEX B (we would sell at this price)
    /// 
    /// # Returns
    /// * `f64` - Entry spread percentage (positive = Entry opportunity)
    /// 
    /// # Edge Cases
    /// * Returns 0.0 if midpoint is zero (prevents division by zero)
    #[inline]
    pub fn calculate_entry_spread(ask_a: f64, bid_b: f64) -> f64 {
        let midpoint = (ask_a + bid_b) / 2.0;
        if midpoint == 0.0 {
            return 0.0;
        }
        ((ask_a - bid_b) / midpoint) * 100.0
    }
    
    /// Calculate Exit spread: (BID_A - ASK_B) / midpoint × 100
    /// 
    /// Use this for Exit Monitor: "Sell on DEX A, Buy back on DEX B" to close position.
    /// 
    /// # Arguments
    /// * `bid_a` - Best bid price on DEX A (we would sell at this price)
    /// * `ask_b` - Best ask price on DEX B (we would buy back at this price)
    /// 
    /// # Returns
    /// * `f64` - Exit spread percentage (typically negative or lower than entry)
    /// 
    /// # Edge Cases
    /// * Returns 0.0 if midpoint is zero (prevents division by zero)
    #[inline]
    pub fn calculate_exit_spread(bid_a: f64, ask_b: f64) -> f64 {
        let midpoint = (bid_a + ask_b) / 2.0;
        if midpoint == 0.0 {
            return 0.0;
        }
        ((bid_a - ask_b) / midpoint) * 100.0
    }
    
    /// Calculate both Entry and Exit spreads from orderbooks
    /// 
    /// Convenience method that returns both spread values for dashboard display.
    /// 
    /// # Returns
    /// * `Option<(f64, f64)>` - (entry_spread_pct, exit_spread_pct) or None if orderbooks empty
    #[inline]
    #[must_use]
    pub fn calculate_dual_spreads(
        &self,
        orderbook_a: &Orderbook,
        orderbook_b: &Orderbook,
    ) -> Option<(f64, f64)> {
        // Get all 4 prices from both orderbooks
        let ask_a = orderbook_a.best_ask()?;
        let bid_a = orderbook_a.best_bid()?;
        let ask_b = orderbook_b.best_ask()?;
        let bid_b = orderbook_b.best_bid()?;
        
        // Calculate both spreads using the dedicated functions
        let entry_spread = Self::calculate_entry_spread(ask_a, bid_b);
        let exit_spread = Self::calculate_exit_spread(bid_a, ask_b);
        
        Some((entry_spread, exit_spread))
    }
}

// =============================================================================
// SpreadTick Conversion (Task 5)
// =============================================================================

impl SpreadResult {
    /// Convert to SpreadTick for mpsc broadcast
    /// 
    /// # Arguments
    /// * `pair` - Trading pair (e.g., "BTC-PERP")
    /// * `dex_a` - DEX A identifier
    /// * `dex_b` - DEX B identifier
    pub fn into_tick(self, pair: impl Into<String>, dex_a: impl Into<String>, dex_b: impl Into<String>) -> SpreadTick {
        SpreadTick {
            pair: pair.into(),
            dex_a: dex_a.into(),
            dex_b: dex_b.into(),
            result: self,
        }
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Get current timestamp in milliseconds (Unix epoch)
#[inline(always)]
fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// =============================================================================
// SpreadMonitor (Story 1.4 - Task 1)
// =============================================================================

use crate::adapters::types::OrderbookUpdate;
use crate::core::channels::SpreadOpportunity;
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, info};

// =============================================================================
// SpreadThresholds (Story 1.5)
// =============================================================================

/// Configuration for spread thresholds
///
/// # Story 1.5
/// Used to filter spread opportunities - only emit when spread >= threshold.
#[derive(Debug, Clone, Copy)]
pub struct SpreadThresholds {
    /// Entry threshold in percentage (e.g., 0.30 = 0.30%)
    pub entry: f64,
    /// Exit threshold in percentage (e.g., 0.05 = 0.05%)
    pub exit: f64,
}

impl SpreadThresholds {
    /// Create new thresholds with entry and exit values
    pub fn new(entry: f64, exit: f64) -> Self {
        Self { entry, exit }
    }
}

impl Default for SpreadThresholds {
    /// Default thresholds (emit on any positive spread)
    fn default() -> Self {
        Self { entry: 0.0, exit: 0.0 }
    }
}

/// Monitor task that consumes orderbook updates and calculates spreads
///
/// # Story 1.4 Implementation
/// - Consumes `OrderbookUpdate` from `orderbook_rx` channel
/// - Stores orderbooks by exchange (vest/paradex)
/// - Calculates entry/exit spreads via `SpreadCalculator.calculate_dual_spreads()`
/// - Emits `SpreadOpportunity` via `opportunity_tx` when significant spreads detected
///
/// # Performance
/// Target: < 2ms per spread calculation (NFR1)
pub struct SpreadMonitor {
    calculator: SpreadCalculator,
    vest_orderbook: Option<Orderbook>,
    paradex_orderbook: Option<Orderbook>,
    pair: String,
    /// Threshold for entry spread detection (Story 1.5)
    entry_threshold: f64,
    /// Threshold for exit spread detection (Story 1.5)
    exit_threshold: f64,
}

impl SpreadMonitor {
    /// Create a new SpreadMonitor for a trading pair with thresholds
    /// 
    /// # Arguments
    /// * `pair` - Trading pair (e.g., "BTC-PERP")
    /// * `entry_threshold` - Minimum entry spread % to emit opportunity (e.g., 0.30)
    /// * `exit_threshold` - Minimum exit spread % to emit opportunity (e.g., 0.05)
    pub fn new(pair: impl Into<String>, entry_threshold: f64, exit_threshold: f64) -> Self {
        Self {
            calculator: SpreadCalculator::new("vest", "paradex"),
            vest_orderbook: None,
            paradex_orderbook: None,
            pair: pair.into(),
            entry_threshold,
            exit_threshold,
        }
    }
    
    /// Create a SpreadMonitor with SpreadThresholds config
    pub fn with_thresholds(pair: impl Into<String>, thresholds: SpreadThresholds) -> Self {
        Self::new(pair, thresholds.entry, thresholds.exit)
    }
    
    /// Run the monitor, processing orderbook updates until shutdown
    /// 
    /// # Returns
    /// `Ok(())` on clean shutdown, `Err` on channel send failure
    pub async fn run(
        &mut self,
        mut orderbook_rx: mpsc::Receiver<OrderbookUpdate>,
        opportunity_tx: mpsc::Sender<SpreadOpportunity>,
        mut shutdown_rx: broadcast::Receiver<()>,
    ) -> Result<(), SpreadMonitorError> {
        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    debug!(pair = %self.pair, "SpreadMonitor shutting down");
                    break;
                }
                Some(update) = orderbook_rx.recv() => {
                    self.handle_orderbook_update(update, &opportunity_tx).await?;
                }
            }
        }
        Ok(())
    }
    
    /// Handle an incoming orderbook update
    async fn handle_orderbook_update(
        &mut self,
        update: OrderbookUpdate,
        tx: &mpsc::Sender<SpreadOpportunity>,
    ) -> Result<(), SpreadMonitorError> {
        // Store orderbook by exchange
        match update.exchange.as_str() {
            "vest" => self.vest_orderbook = Some(update.orderbook),
            "paradex" => self.paradex_orderbook = Some(update.orderbook),
            other => {
                debug!(exchange = %other, "Unknown exchange, ignoring update");
                return Ok(());
            }
        }
        
        // Calculate spreads if both orderbooks are present
        if let (Some(vest), Some(paradex)) = (&self.vest_orderbook, &self.paradex_orderbook) {
            if let Some((entry_spread, exit_spread)) = self.calculator.calculate_dual_spreads(vest, paradex) {
                debug!(
                    pair = %self.pair, 
                    entry_spread = entry_spread,
                    exit_spread = exit_spread,
                    "Spread calculated"
                );
                
                // Emit opportunity if entry spread >= threshold (Story 1.5)
                if entry_spread >= self.entry_threshold {
                    info!(
                        pair = %self.pair,
                        spread = %format!("{:.4}%", entry_spread),
                        threshold = %format!("{:.4}%", self.entry_threshold),
                        direction = "entry",
                        "Spread opportunity detected"
                    );
                    
                    let opportunity = SpreadOpportunity {
                        pair: self.pair.clone(),
                        dex_a: "vest".to_string(),
                        dex_b: "paradex".to_string(),
                        spread_percent: entry_spread,
                        direction: SpreadDirection::AOverB,
                        detected_at_ms: current_time_ms(),
                        dex_a_ask: vest.best_ask().unwrap_or(0.0),
                        dex_a_bid: vest.best_bid().unwrap_or(0.0),
                        dex_b_ask: paradex.best_ask().unwrap_or(0.0),
                        dex_b_bid: paradex.best_bid().unwrap_or(0.0),
                    };
                    
                    tx.send(opportunity).await.map_err(|_| SpreadMonitorError::ChannelClosed)?;
                }
                
                // Emit opportunity if exit spread >= threshold (Story 1.5)
                if exit_spread >= self.exit_threshold {
                    info!(
                        pair = %self.pair,
                        spread = %format!("{:.4}%", exit_spread),
                        threshold = %format!("{:.4}%", self.exit_threshold),
                        direction = "exit",
                        "Spread opportunity detected"
                    );
                    
                    let opportunity = SpreadOpportunity {
                        pair: self.pair.clone(),
                        dex_a: "vest".to_string(),
                        dex_b: "paradex".to_string(),
                        spread_percent: exit_spread,
                        direction: SpreadDirection::BOverA, // Exit = opposite direction
                        detected_at_ms: current_time_ms(),
                        dex_a_ask: vest.best_ask().unwrap_or(0.0),
                        dex_a_bid: vest.best_bid().unwrap_or(0.0),
                        dex_b_ask: paradex.best_ask().unwrap_or(0.0),
                        dex_b_bid: paradex.best_bid().unwrap_or(0.0),
                    };
                    
                    tx.send(opportunity).await.map_err(|_| SpreadMonitorError::ChannelClosed)?;
                }
            }
        }
        
        Ok(())
    }
    
    /// Check if monitor has received orderbooks from both exchanges
    pub fn has_both_orderbooks(&self) -> bool {
        self.vest_orderbook.is_some() && self.paradex_orderbook.is_some()
    }
}

/// Error type for SpreadMonitor
#[derive(Debug, thiserror::Error)]
pub enum SpreadMonitorError {
    #[error("Opportunity channel closed")]
    ChannelClosed,
}

// =============================================================================
// Unit Tests (Task 6)
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::types::OrderbookLevel;

    /// Helper to create an orderbook with specific best ask/bid
    fn make_orderbook(best_ask: f64, best_bid: f64) -> Orderbook {
        let mut ob = Orderbook::new();
        ob.asks.push(OrderbookLevel::new(best_ask, 1.0));
        ob.bids.push(OrderbookLevel::new(best_bid, 1.0));
        ob
    }

    // =========================================================================
    // Task 6.1: Test spread calculation with known values
    // =========================================================================

    #[test]
    fn test_spread_calculation_basic() {
        let calc = SpreadCalculator::new("vest", "paradex");
        let ob_a = make_orderbook(100.0, 99.0); // Ask=100, Bid=99
        let ob_b = make_orderbook(99.5, 98.5); // Ask=99.5, Bid=98.5

        let result = calc.calculate(&ob_a, &ob_b).unwrap();

        // NEW FORMULA: spread_A_to_B = (bid_B - ask_A) / ask_A * 100 = (98.5 - 100) / 100 = -1.5%
        //              spread_B_to_A = (bid_A - ask_B) / ask_B * 100 = (99 - 99.5) / 99.5 = -0.503%
        // B→A is less negative (better), so direction = BOverA
        assert_eq!(result.direction, SpreadDirection::BOverA);
        // Spread is the better one (B→A) which is -0.503%
        assert!(result.spread_pct < 0.0, "Spread should be negative, got {}", result.spread_pct);
        assert_eq!(result.ask_price, 99.5);  // B's ask (we BUY here)
        assert_eq!(result.bid_price, 99.0);  // A's bid (we SELL here)
    }

    #[test]
    fn test_spread_calculation_exact_values() {
        let calc = SpreadCalculator::new("vest", "paradex");
        // Test with orderbooks where one direction has positive spread
        // A: ask=98, bid=97  |  B: ask=99, bid=101 (B's bid > A's ask)
        let ob_a = make_orderbook(98.0, 97.0);
        let ob_b = make_orderbook(99.0, 101.0);

        let result = calc.calculate(&ob_a, &ob_b).unwrap();

        // NEW FORMULA: spread_A_to_B = (bid_B - ask_A) / ask_A * 100 = (101 - 98) / 98 = 3.0612%
        //              spread_B_to_A = (bid_A - ask_B) / ask_B * 100 = (97 - 99) / 99 = -2.0202%
        // A→B is positive and larger, so direction = AOverB
        let expected = (101.0 - 98.0) / 98.0 * 100.0;  // 3.0612...%
        assert!((result.spread_pct - expected).abs() < 0.0001);
        assert_eq!(result.direction, SpreadDirection::AOverB);
    }

    // =========================================================================
    // Task 6.2: Test both spread directions (A>B, B>A)
    // =========================================================================

    #[test]
    fn test_spread_direction_a_over_b() {
        let calc = SpreadCalculator::new("vest", "paradex");
        // A→B: Buy on A, Sell on B. Need bid_B > ask_A for profit
        // A: ask=98, bid=97  |  B: ask=99, bid=102 (bid_B > ask_A)
        let ob_a = make_orderbook(98.0, 97.0);
        let ob_b = make_orderbook(99.0, 102.0);

        let result = calc.calculate(&ob_a, &ob_b).unwrap();
        
        // spread_A_to_B = (102 - 98) / 98 = 4.08% (positive)
        // spread_B_to_A = (97 - 99) / 99 = -2.02% (negative)
        // A→B is better, so direction = AOverB
        assert_eq!(result.direction, SpreadDirection::AOverB);
        assert!(result.spread_pct > 0.0, "Spread should be positive, got {}", result.spread_pct);
    }

    #[test]
    fn test_spread_direction_b_over_a() {
        let calc = SpreadCalculator::new("vest", "paradex");
        // B→A: Buy on B, Sell on A. Need bid_A > ask_B for profit
        // A: ask=99, bid=102  |  B: ask=98, bid=97 (bid_A > ask_B)
        let ob_a = make_orderbook(99.0, 102.0);
        let ob_b = make_orderbook(98.0, 97.0);

        let result = calc.calculate(&ob_a, &ob_b).unwrap();

        // spread_A_to_B = (97 - 99) / 99 = -2.02% (negative)
        // spread_B_to_A = (102 - 98) / 98 = 4.08% (positive)
        // B→A is better, so direction = BOverA
        assert_eq!(result.direction, SpreadDirection::BOverA);
        assert!(result.spread_pct > 0.0, "Spread should be positive, got {}", result.spread_pct);
        assert_eq!(result.ask_price, 98.0);  // B's ask (we BUY here)
        assert_eq!(result.bid_price, 102.0); // A's bid (we SELL here)
    }

    // =========================================================================
    // Task 6.3: Test empty orderbook handling (returns None)
    // =========================================================================

    #[test]
    fn test_spread_empty_orderbook_a_returns_none() {
        let calc = SpreadCalculator::new("vest", "paradex");
        let empty = Orderbook::new();
        let full = make_orderbook(100.0, 99.0);

        assert!(calc.calculate(&empty, &full).is_none());
    }

    #[test]
    fn test_spread_empty_orderbook_b_returns_none() {
        let calc = SpreadCalculator::new("vest", "paradex");
        let empty = Orderbook::new();
        let full = make_orderbook(100.0, 99.0);

        assert!(calc.calculate(&full, &empty).is_none());
    }

    #[test]
    fn test_spread_both_empty_returns_none() {
        let calc = SpreadCalculator::new("vest", "paradex");
        let empty_a = Orderbook::new();
        let empty_b = Orderbook::new();

        assert!(calc.calculate(&empty_a, &empty_b).is_none());
    }

    #[test]
    fn test_spread_orderbook_missing_asks() {
        let calc = SpreadCalculator::new("vest", "paradex");
        let mut ob_a = Orderbook::new();
        ob_a.bids.push(OrderbookLevel::new(99.0, 1.0)); // Only bids, no asks
        let full = make_orderbook(100.0, 99.0);

        assert!(calc.calculate(&ob_a, &full).is_none());
    }

    #[test]
    fn test_spread_orderbook_missing_bids() {
        let calc = SpreadCalculator::new("vest", "paradex");
        let mut ob_a = Orderbook::new();
        ob_a.asks.push(OrderbookLevel::new(100.0, 1.0)); // Only asks, no bids
        let full = make_orderbook(100.0, 99.0);

        assert!(calc.calculate(&ob_a, &full).is_none());
    }

    // =========================================================================
    // Task 6.4: Test negative spread detection (no opportunity)
    // =========================================================================

    #[test]
    fn test_spread_negative_in_one_direction() {
        let calc = SpreadCalculator::new("vest", "paradex");
        // One direction has less negative spread
        // A: ask=100, bid=99  |  B: ask=101, bid=98
        let ob_a = make_orderbook(100.0, 99.0);
        let ob_b = make_orderbook(101.0, 98.0);

        let result = calc.calculate(&ob_a, &ob_b).unwrap();

        // spread_A_to_B = (98 - 100) / 100 = -2% 
        // spread_B_to_A = (99 - 101) / 101 = -1.98%
        // B→A is less negative (better), so direction = BOverA
        assert_eq!(result.direction, SpreadDirection::BOverA);
        // Both are negative, spread should be negative
        assert!(result.spread_pct < 0.0, "Expected negative spread, got {}", result.spread_pct);
    }

    #[test]
    fn test_spread_negative_both_directions() {
        let calc = SpreadCalculator::new("vest", "paradex");
        // Both directions have negative spread (normal market, no arbitrage)
        // A: ask=100, bid=99  |  B: ask=101, bid=98
        let ob_a = make_orderbook(100.0, 99.0);
        let ob_b = make_orderbook(101.0, 98.0);

        let result = calc.calculate(&ob_a, &ob_b).unwrap();

        // spread_A_to_B = (98 - 100) / 100 = -2%
        // spread_B_to_A = (99 - 101) / 101 = -1.98%
        // Both spreads are negative (no arbitrage opportunity)
        // Calculator returns the better (less negative) one = B→A at -1.98%
        assert!(result.spread_pct < 0.0, "Both directions negative, spread should be < 0, got {}", result.spread_pct);
    }

    // =========================================================================
    // Task 6.5: Benchmark test - verify calculation under 2ms for 10k iterations
    // =========================================================================

    #[test]
    fn test_spread_calculation_performance() {
        let calc = SpreadCalculator::new("vest", "paradex");
        let ob_a = make_orderbook(42150.50, 42149.00);
        let ob_b = make_orderbook(42151.00, 42148.50);

        let start = std::time::Instant::now();
        for _ in 0..10_000 {
            let _ = calc.calculate(&ob_a, &ob_b);
        }
        let elapsed = start.elapsed();

        // 10k iterations should complete in <2ms total (<200ns per call)
        assert!(
            elapsed.as_millis() < 2,
            "Performance: 10k calcs took {:?} (expected <2ms)",
            elapsed
        );
    }

    #[test]
    fn test_spread_calculation_performance_100k() {
        let calc = SpreadCalculator::new("vest", "paradex");
        let ob_a = make_orderbook(42150.50, 42149.00);
        let ob_b = make_orderbook(42151.00, 42148.50);

        let start = std::time::Instant::now();
        for _ in 0..100_000 {
            let _ = calc.calculate(&ob_a, &ob_b);
        }
        let elapsed = start.elapsed();

        // 100k iterations should complete in <20ms (extrapolated from NFR2)
        assert!(
            elapsed.as_millis() < 20,
            "Performance: 100k calcs took {:?} (expected <20ms)",
            elapsed
        );
    }

    // =========================================================================
    // Additional Tests: SpreadResult, SpreadTick, Edge Cases
    // =========================================================================

    #[test]
    fn test_spread_result_timestamp_is_set() {
        let calc = SpreadCalculator::new("vest", "paradex");
        let ob_a = make_orderbook(100.0, 99.0);
        let ob_b = make_orderbook(99.5, 98.5);

        let result = calc.calculate(&ob_a, &ob_b).unwrap();

        // Timestamp should be recent (within last second)
        let now_ms = current_time_ms();
        assert!(result.timestamp_ms > 0);
        assert!(result.timestamp_ms <= now_ms);
        assert!(now_ms - result.timestamp_ms < 1000, "Timestamp should be within last second");
    }

    #[test]
    fn test_spread_result_into_tick() {
        let result = SpreadResult {
            spread_pct: 1.5,
            direction: SpreadDirection::AOverB,
            ask_price: 100.0,
            bid_price: 98.5,
            midpoint: 99.25,
            timestamp_ms: 1706000000000,
        };

        let tick = result.into_tick("BTC-PERP", "vest", "paradex");

        assert_eq!(tick.pair, "BTC-PERP");
        assert_eq!(tick.dex_a, "vest");
        assert_eq!(tick.dex_b, "paradex");
        assert_eq!(tick.result.spread_pct, 1.5);
        assert_eq!(tick.result.direction, SpreadDirection::AOverB);
    }

    #[test]
    fn test_spread_direction_serialization() {
        let direction = SpreadDirection::AOverB;
        let json = serde_json::to_string(&direction).unwrap();
        assert!(json.contains("AOverB"));

        let deserialized: SpreadDirection = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, SpreadDirection::AOverB);
    }

    #[test]
    fn test_spread_result_serialization() {
        let result = SpreadResult {
            spread_pct: 1.5123,
            direction: SpreadDirection::BOverA,
            ask_price: 42150.5,
            bid_price: 42100.0,
            midpoint: 42125.25,
            timestamp_ms: 1706000000000,
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("1.5123"));
        assert!(json.contains("BOverA"));

        let deserialized: SpreadResult = serde_json::from_str(&json).unwrap();
        assert!((deserialized.spread_pct - 1.5123).abs() < 0.0001);
        assert_eq!(deserialized.direction, SpreadDirection::BOverA);
    }

    #[test]
    fn test_spread_tick_serialization() {
        let tick = SpreadTick {
            pair: "ETH-PERP".to_string(),
            dex_a: "vest".to_string(),
            dex_b: "hyperliquid".to_string(),
            result: SpreadResult {
                spread_pct: 0.25,
                direction: SpreadDirection::AOverB,
                ask_price: 2800.0,
                bid_price: 2795.0,
                midpoint: 2797.5,
                timestamp_ms: 1706000000000,
            },
        };

        let json = serde_json::to_string(&tick).unwrap();
        assert!(json.contains("ETH-PERP"));
        assert!(json.contains("vest"));
        assert!(json.contains("hyperliquid"));

        let deserialized: SpreadTick = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.pair, "ETH-PERP");
    }

    #[test]
    fn test_spread_calculator_new() {
        let calc = SpreadCalculator::new("vest", "paradex");
        assert_eq!(calc.dex_a, "vest");
        assert_eq!(calc.dex_b, "paradex");
    }

    #[test]
    fn test_raw_spread_zero_midpoint() {
        // Edge case: both ask and bid are 0 -> midpoint is 0
        // Should return 0 to prevent division by zero
        let spread = SpreadCalculator::raw_spread(0.0, 0.0);
        assert_eq!(spread, 0.0);
    }

    #[test]
    fn test_raw_spread_symmetric() {
        // Spread formula is symmetric around midpoint
        let spread = SpreadCalculator::raw_spread(102.0, 100.0);
        // (102-100) / 101 * 100 = 1.98019...
        assert!((spread - 1.9801980198).abs() < 0.0001);
    }

    // =========================================================================
    // Story 11.1: Dual Spread Calculation Tests
    // =========================================================================

    #[test]
    fn test_calculate_entry_spread_basic() {
        // Entry: ASK_A - BID_B formula
        // Ask A = 100, Bid B = 99
        // midpoint = (100 + 99) / 2 = 99.5
        // spread = (100 - 99) / 99.5 * 100 ≈ 1.005%
        let spread = SpreadCalculator::calculate_entry_spread(100.0, 99.0);
        let expected = (100.0 - 99.0) / 99.5 * 100.0;
        assert!((spread - expected).abs() < 0.0001);
        assert!(spread > 0.0, "Entry spread should be positive when ask_a > bid_b");
    }

    #[test]
    fn test_calculate_entry_spread_negative() {
        // When ASK_A < BID_B, entry spread is negative (no arb opportunity)
        // Ask A = 98, Bid B = 100
        let spread = SpreadCalculator::calculate_entry_spread(98.0, 100.0);
        assert!(spread < 0.0, "Entry spread should be negative when ask_a < bid_b");
    }

    #[test]
    fn test_calculate_entry_spread_zero_midpoint() {
        let spread = SpreadCalculator::calculate_entry_spread(0.0, 0.0);
        assert_eq!(spread, 0.0);
    }

    #[test]
    fn test_calculate_exit_spread_basic() {
        // Exit: BID_A - ASK_B formula
        // Bid A = 101, Ask B = 99
        // midpoint = (101 + 99) / 2 = 100
        // spread = (101 - 99) / 100 * 100 = 2.0%
        let spread = SpreadCalculator::calculate_exit_spread(101.0, 99.0);
        let expected = (101.0 - 99.0) / 100.0 * 100.0;
        assert!((spread - expected).abs() < 0.0001);
        assert_eq!(spread, 2.0);
    }

    #[test]
    fn test_calculate_exit_spread_negative() {
        // When BID_A < ASK_B, exit spread is negative (typical market condition)
        // Bid A = 99, Ask B = 101
        // midpoint = 100, spread = (99-101)/100*100 = -2%
        let spread = SpreadCalculator::calculate_exit_spread(99.0, 101.0);
        assert!((spread - (-2.0)).abs() < 0.0001);
        assert!(spread < 0.0, "Exit spread typically negative when bid_a < ask_b");
    }

    #[test]
    fn test_calculate_exit_spread_zero_midpoint() {
        let spread = SpreadCalculator::calculate_exit_spread(0.0, 0.0);
        assert_eq!(spread, 0.0);
    }

    #[test]
    fn test_entry_exit_spreads_are_different() {
        // Critical test: Entry and Exit MUST produce different values
        // This is the core bug that Story 11.1 fixes
        
        // Typical market: Ask > Bid on same exchange
        // DEX A: Ask=100.5, Bid=100.0
        // DEX B: Ask=100.3, Bid=99.8
        
        let ask_a = 100.5;
        let bid_a = 100.0;
        let ask_b = 100.3;
        let bid_b = 99.8;
        
        let entry = SpreadCalculator::calculate_entry_spread(ask_a, bid_b);
        let exit = SpreadCalculator::calculate_exit_spread(bid_a, ask_b);
        
        // Entry: (100.5 - 99.8) / 100.15 * 100 ≈ 0.699%
        // Exit: (100.0 - 100.3) / 100.15 * 100 ≈ -0.299%
        
        assert!(entry > 0.0, "Entry should be positive: {}", entry);
        assert!(exit < 0.0, "Exit should be negative: {}", exit);
        assert!((entry - exit).abs() > 0.5, "Entry and Exit MUST be different");
    }

    #[test]
    fn test_calculate_dual_spreads_from_orderbooks() {
        let calc = SpreadCalculator::new("vest", "paradex");
        
        // Create orderbooks with known prices
        // DEX A: Ask=101, Bid=100
        // DEX B: Ask=100.5, Bid=99.5
        let ob_a = make_orderbook(101.0, 100.0);  // ask, bid
        let ob_b = make_orderbook(100.5, 99.5);   // ask, bid
        
        let (entry, exit) = calc.calculate_dual_spreads(&ob_a, &ob_b).unwrap();
        
        // Entry: (ASK_A=101 - BID_B=99.5) / 100.25 * 100 = 1.496%
        // Exit: (BID_A=100 - ASK_B=100.5) / 100.25 * 100 = -0.498%
        let expected_entry = (101.0 - 99.5) / 100.25 * 100.0;
        let expected_exit = (100.0 - 100.5) / 100.25 * 100.0;
        
        assert!((entry - expected_entry).abs() < 0.001, "Entry mismatch: {} vs {}", entry, expected_entry);
        assert!((exit - expected_exit).abs() < 0.001, "Exit mismatch: {} vs {}", exit, expected_exit);
        
        assert!(entry > 0.0);
        assert!(exit < 0.0);
    }

    #[test]
    fn test_calculate_dual_spreads_empty_orderbook() {
        let calc = SpreadCalculator::new("vest", "paradex");
        let empty = Orderbook::new();
        let full = make_orderbook(100.0, 99.0);
        
        assert!(calc.calculate_dual_spreads(&empty, &full).is_none());
        assert!(calc.calculate_dual_spreads(&full, &empty).is_none());
        assert!(calc.calculate_dual_spreads(&empty, &empty).is_none());
    }

    #[test]
    fn test_dual_spread_realistic_btc_values() {
        // Real-world BTC prices scenario
        let calc = SpreadCalculator::new("vest", "paradex");
        
        // Vest: Ask=42150.50, Bid=42148.00
        // Paradex: Ask=42155.00, Bid=42145.00
        let ob_vest = make_orderbook(42150.50, 42148.00);
        let ob_paradex = make_orderbook(42155.00, 42145.00);
        
        let (entry, exit) = calc.calculate_dual_spreads(&ob_vest, &ob_paradex).unwrap();
        
        // Entry: (42150.50 - 42145.00) / midpoint = small positive
        // Exit: (42148.00 - 42155.00) / midpoint = small negative
        
        assert!(entry > 0.0 && entry < 1.0, "Entry should be small positive: {}", entry);
        assert!(exit < 0.0 && exit > -1.0, "Exit should be small negative: {}", exit);
        assert!((entry - exit).abs() > 0.01, "Entry/Exit must differ: {} vs {}", entry, exit);
    }

    // =========================================================================
    // Story 1.4: SpreadMonitor Tests
    // =========================================================================

    #[test]
    fn test_spread_monitor_creation() {
        let monitor = SpreadMonitor::new("BTC-PERP", 0.30, 0.05);
        assert!(!monitor.has_both_orderbooks());
    }

    #[tokio::test]
    async fn test_spread_monitor_processes_orderbook_update() {
        // Create monitor with 0.0 thresholds (emit on any positive spread)
        let mut monitor = SpreadMonitor::new("ETH-PERP", 0.0, 0.0);
        
        // Create channels
        let (orderbook_tx, orderbook_rx) = mpsc::channel::<OrderbookUpdate>(10);
        let (opportunity_tx, mut opportunity_rx) = mpsc::channel::<SpreadOpportunity>(10);
        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        let shutdown_rx = shutdown_tx.subscribe();
        
        // Spawn monitor in background
        let handle = tokio::spawn(async move {
            monitor.run(orderbook_rx, opportunity_tx, shutdown_rx).await
        });
        
        // Send vest orderbook (ask=100.5, bid=100.0)
        let vest_update = OrderbookUpdate {
            symbol: "ETH-PERP".to_string(),
            exchange: "vest".to_string(),
            orderbook: make_orderbook(100.5, 100.0),
        };
        orderbook_tx.send(vest_update).await.unwrap();
        
        // Send paradex orderbook (ask=100.0, bid=99.0) - entry spread should be positive
        let paradex_update = OrderbookUpdate {
            symbol: "ETH-PERP".to_string(),
            exchange: "paradex".to_string(),
            orderbook: make_orderbook(100.0, 99.0),
        };
        orderbook_tx.send(paradex_update).await.unwrap();
        
        // Wait briefly for processing
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        
        // Should have received an opportunity
        let opportunity = opportunity_rx.try_recv().expect("Should receive spread opportunity");
        assert_eq!(opportunity.pair, "ETH-PERP");
        assert!(opportunity.spread_percent > 0.0, "Spread should be positive");
        
        // Shutdown
        shutdown_tx.send(()).unwrap();
        let _ = handle.await;
    }

    #[tokio::test]
    async fn test_spread_opportunity_emitted_on_spread_change() {
        // Use 0.0 threshold to emit on any positive spread
        let mut monitor = SpreadMonitor::new("BTC-PERP", 0.0, 0.0);
        
        let (orderbook_tx, orderbook_rx) = mpsc::channel::<OrderbookUpdate>(10);
        let (opportunity_tx, mut opportunity_rx) = mpsc::channel::<SpreadOpportunity>(10);
        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        let shutdown_rx = shutdown_tx.subscribe();
        
        let handle = tokio::spawn(async move {
            monitor.run(orderbook_rx, opportunity_tx, shutdown_rx).await
        });
        
        // Create orderbooks with positive entry spread
        // Vest Ask=105, Bid=104 | Paradex Ask=103, Bid=100
        // Entry = (105 - 100) / 102.5 * 100 = ~4.88%
        let vest_update = OrderbookUpdate {
            symbol: "BTC-PERP".to_string(),
            exchange: "vest".to_string(),
            orderbook: make_orderbook(105.0, 104.0),
        };
        let paradex_update = OrderbookUpdate {
            symbol: "BTC-PERP".to_string(),
            exchange: "paradex".to_string(),
            orderbook: make_orderbook(103.0, 100.0),
        };
        
        orderbook_tx.send(vest_update).await.unwrap();
        orderbook_tx.send(paradex_update).await.unwrap();
        
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        
        let opp = opportunity_rx.try_recv().expect("Opportunity should be emitted");
        assert!(opp.spread_percent > 4.0, "Expected large spread: {}", opp.spread_percent);
        assert_eq!(opp.direction, SpreadDirection::AOverB);
        
        shutdown_tx.send(()).unwrap();
        let _ = handle.await;
    }

    #[tokio::test]
    async fn test_spread_monitor_handles_missing_orderbook() {
        let mut monitor = SpreadMonitor::new("ETH-PERP", 0.0, 0.0);
        
        let (orderbook_tx, orderbook_rx) = mpsc::channel::<OrderbookUpdate>(10);
        let (opportunity_tx, mut opportunity_rx) = mpsc::channel::<SpreadOpportunity>(10);
        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        let shutdown_rx = shutdown_tx.subscribe();
        
        let handle = tokio::spawn(async move {
            monitor.run(orderbook_rx, opportunity_tx, shutdown_rx).await
        });
        
        // Send only vest orderbook - no paradex
        let vest_update = OrderbookUpdate {
            symbol: "ETH-PERP".to_string(),
            exchange: "vest".to_string(),
            orderbook: make_orderbook(100.5, 100.0),
        };
        orderbook_tx.send(vest_update).await.unwrap();
        
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        
        // Should NOT have received any opportunity (missing paradex)
        assert!(opportunity_rx.try_recv().is_err(), "No opportunity without both orderbooks");
        
        shutdown_tx.send(()).unwrap();
        let _ = handle.await;
    }

    #[test]
    fn test_spread_calculation_performance_2ms() {
        // NFR1: Spread calculation must complete in < 2ms
        let calc = SpreadCalculator::new("vest", "paradex");
        
        // Create orderbooks with 100+ levels each (realistic depth)
        let mut ob_a = Orderbook::new();
        let mut ob_b = Orderbook::new();
        
        for i in 0..100 {
            ob_a.asks.push(OrderbookLevel::new(42150.0 + (i as f64), 1.0 + (i as f64) * 0.1));
            ob_a.bids.push(OrderbookLevel::new(42140.0 - (i as f64), 1.0 + (i as f64) * 0.1));
            ob_b.asks.push(OrderbookLevel::new(42155.0 + (i as f64), 1.0 + (i as f64) * 0.1));
            ob_b.bids.push(OrderbookLevel::new(42135.0 - (i as f64), 1.0 + (i as f64) * 0.1));
        }
        
        // Single calculation should be <2ms
        let start = std::time::Instant::now();
        let result = calc.calculate_dual_spreads(&ob_a, &ob_b);
        let elapsed = start.elapsed();
        
        assert!(result.is_some(), "Spread calculation should succeed");
        assert!(
            elapsed.as_millis() < 2,
            "NFR1: Spread calculation took {:?} (must be <2ms)",
            elapsed
        );
        
        // Also test 1000 iterations stay under budget
        let start = std::time::Instant::now();
        for _ in 0..1000 {
            let _ = calc.calculate_dual_spreads(&ob_a, &ob_b);
        }
        let elapsed = start.elapsed();
        
        assert!(
            elapsed.as_millis() < 200,
            "1000 calculations took {:?} (avg should be <0.2ms)",
            elapsed
        );
    }

    // =========================================================================
    // Story 1.5: Threshold Detection Tests
    // =========================================================================

    #[test]
    fn test_spread_thresholds_creation() {
        let thresholds = SpreadThresholds::new(0.30, 0.05);
        assert!((thresholds.entry - 0.30).abs() < 0.0001);
        assert!((thresholds.exit - 0.05).abs() < 0.0001);
    }

    #[test]
    fn test_spread_thresholds_default() {
        let thresholds = SpreadThresholds::default();
        assert_eq!(thresholds.entry, 0.0);
        assert_eq!(thresholds.exit, 0.0);
    }

    #[test]
    fn test_spread_monitor_with_thresholds() {
        let thresholds = SpreadThresholds::new(0.30, 0.05);
        let monitor = SpreadMonitor::with_thresholds("BTC-PERP", thresholds);
        assert!(!monitor.has_both_orderbooks());
    }

    #[tokio::test]
    async fn test_spread_monitor_detects_entry_above_threshold() {
        // Entry threshold = 1.0%, spread will be ~1.5%
        let mut monitor = SpreadMonitor::new("BTC-PERP", 1.0, 0.0);
        
        let (orderbook_tx, orderbook_rx) = mpsc::channel::<OrderbookUpdate>(10);
        let (opportunity_tx, mut opportunity_rx) = mpsc::channel::<SpreadOpportunity>(10);
        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        let shutdown_rx = shutdown_tx.subscribe();
        
        let handle = tokio::spawn(async move {
            monitor.run(orderbook_rx, opportunity_tx, shutdown_rx).await
        });
        
        // Vest Ask=102, Bid=101 | Paradex Ask=101, Bid=100
        // Entry spread: (102 - 100) / 101 * 100 ≈ 1.98% > 1.0% threshold
        let vest_update = OrderbookUpdate {
            symbol: "BTC-PERP".to_string(),
            exchange: "vest".to_string(),
            orderbook: make_orderbook(102.0, 101.0),
        };
        let paradex_update = OrderbookUpdate {
            symbol: "BTC-PERP".to_string(),
            exchange: "paradex".to_string(),
            orderbook: make_orderbook(101.0, 100.0),
        };
        
        orderbook_tx.send(vest_update).await.unwrap();
        orderbook_tx.send(paradex_update).await.unwrap();
        
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        
        // Should emit opportunity since spread (1.98%) >= threshold (1.0%)
        let opp = opportunity_rx.try_recv().expect("Should emit opportunity above threshold");
        assert!(opp.spread_percent >= 1.0, "Spread {} should be >= 1.0%", opp.spread_percent);
        assert_eq!(opp.direction, SpreadDirection::AOverB);
        
        shutdown_tx.send(()).unwrap();
        let _ = handle.await;
    }

    #[tokio::test]
    async fn test_spread_monitor_ignores_entry_below_threshold() {
        // Entry threshold = 5.0%, Exit threshold = 10.0% (high to prevent exit triggers)
        // Spread will be ~1.98% (below entry threshold)
        let mut monitor = SpreadMonitor::new("ETH-PERP", 5.0, 10.0);
        
        let (orderbook_tx, orderbook_rx) = mpsc::channel::<OrderbookUpdate>(10);
        let (opportunity_tx, mut opportunity_rx) = mpsc::channel::<SpreadOpportunity>(10);
        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        let shutdown_rx = shutdown_tx.subscribe();
        
        let handle = tokio::spawn(async move {
            monitor.run(orderbook_rx, opportunity_tx, shutdown_rx).await
        });
        
        // Entry spread: (102 - 100) / 101 * 100 ≈ 1.98% < 5.0% threshold
        let vest_update = OrderbookUpdate {
            symbol: "ETH-PERP".to_string(),
            exchange: "vest".to_string(),
            orderbook: make_orderbook(102.0, 101.0),
        };
        let paradex_update = OrderbookUpdate {
            symbol: "ETH-PERP".to_string(),
            exchange: "paradex".to_string(),
            orderbook: make_orderbook(101.0, 100.0),
        };
        
        orderbook_tx.send(vest_update).await.unwrap();
        orderbook_tx.send(paradex_update).await.unwrap();
        
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        
        // Should NOT emit opportunity since spread (1.98%) < threshold (5.0%)
        assert!(opportunity_rx.try_recv().is_err(), "Should NOT emit opportunity below threshold");
        
        shutdown_tx.send(()).unwrap();
        let _ = handle.await;
    }

    #[tokio::test]
    async fn test_spread_monitor_detects_exit_above_threshold() {
        // Exit threshold = 0.5%, spread will be positive (unusual but testable)
        // Using reversed prices to get positive exit spread
        let mut monitor = SpreadMonitor::new("BTC-PERP", 10.0, 0.5);
        
        let (orderbook_tx, orderbook_rx) = mpsc::channel::<OrderbookUpdate>(10);
        let (opportunity_tx, mut opportunity_rx) = mpsc::channel::<SpreadOpportunity>(10);
        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        let shutdown_rx = shutdown_tx.subscribe();
        
        let handle = tokio::spawn(async move {
            monitor.run(orderbook_rx, opportunity_tx, shutdown_rx).await
        });
        
        // Vest Ask=99, Bid=101 (crossed) | Paradex Ask=99, Bid=97
        // Exit spread: (101 - 99) / 100 * 100 = 2.0% > 0.5% threshold
        let vest_update = OrderbookUpdate {
            symbol: "BTC-PERP".to_string(),
            exchange: "vest".to_string(),
            orderbook: make_orderbook(99.0, 101.0), // Crossed for positive exit
        };
        let paradex_update = OrderbookUpdate {
            symbol: "BTC-PERP".to_string(),
            exchange: "paradex".to_string(),
            orderbook: make_orderbook(99.0, 97.0),
        };
        
        orderbook_tx.send(vest_update).await.unwrap();
        orderbook_tx.send(paradex_update).await.unwrap();
        
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        
        // Should emit exit opportunity
        let opp = opportunity_rx.try_recv().expect("Should emit exit opportunity above threshold");
        assert_eq!(opp.direction, SpreadDirection::BOverA, "Exit direction should be BOverA");
        
        shutdown_tx.send(()).unwrap();
        let _ = handle.await;
    }

    #[tokio::test]
    async fn test_spread_monitor_ignores_exit_below_threshold() {
        // Exit threshold = 10.0%, typical exit spread is negative so won't trigger
        let mut monitor = SpreadMonitor::new("ETH-PERP", 0.0, 10.0);
        
        let (orderbook_tx, orderbook_rx) = mpsc::channel::<OrderbookUpdate>(10);
        let (opportunity_tx, mut opportunity_rx) = mpsc::channel::<SpreadOpportunity>(10);
        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        let shutdown_rx = shutdown_tx.subscribe();
        
        let handle = tokio::spawn(async move {
            monitor.run(orderbook_rx, opportunity_tx, shutdown_rx).await
        });
        
        // Normal market: exit spread is typically negative or small positive
        // Exit spread: (101 - 101.5) / 101.25 * 100 ≈ -0.49% < 10.0% threshold
        let vest_update = OrderbookUpdate {
            symbol: "ETH-PERP".to_string(),
            exchange: "vest".to_string(),
            orderbook: make_orderbook(102.0, 101.0),
        };
        let paradex_update = OrderbookUpdate {
            symbol: "ETH-PERP".to_string(),
            exchange: "paradex".to_string(),
            orderbook: make_orderbook(101.5, 100.0),
        };
        
        orderbook_tx.send(vest_update).await.unwrap();
        orderbook_tx.send(paradex_update).await.unwrap();
        
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        
        // Entry might trigger (entry spread ~2%), but exit should NOT trigger
        // Clear any entry opportunities
        let _ = opportunity_rx.try_recv();
        
        // Send another update to potentially trigger exit - but threshold is too high
        // Should NOT have additional exit opportunity
        assert!(opportunity_rx.try_recv().is_err(), "Should NOT emit exit opportunity below threshold");
        
        shutdown_tx.send(()).unwrap();
        let _ = handle.await;
    }

    #[tokio::test]
    async fn test_spread_monitor_different_entry_exit_thresholds() {
        // Entry = 1.0%, Exit = 2.0% - realistic config
        let mut monitor = SpreadMonitor::new("SOL-PERP", 1.0, 2.0);
        
        let (orderbook_tx, orderbook_rx) = mpsc::channel::<OrderbookUpdate>(10);
        let (opportunity_tx, mut opportunity_rx) = mpsc::channel::<SpreadOpportunity>(10);
        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        let shutdown_rx = shutdown_tx.subscribe();
        
        let handle = tokio::spawn(async move {
            monitor.run(orderbook_rx, opportunity_tx, shutdown_rx).await
        });
        
        // Entry spread: (105 - 100) / 102.5 * 100 ≈ 4.88% > 1.0% (triggers)
        // Exit spread: (104 - 103) / 103.5 * 100 ≈ 0.97% < 2.0% (doesn't trigger)
        let vest_update = OrderbookUpdate {
            symbol: "SOL-PERP".to_string(),
            exchange: "vest".to_string(),
            orderbook: make_orderbook(105.0, 104.0),
        };
        let paradex_update = OrderbookUpdate {
            symbol: "SOL-PERP".to_string(),
            exchange: "paradex".to_string(),
            orderbook: make_orderbook(103.0, 100.0),
        };
        
        orderbook_tx.send(vest_update).await.unwrap();
        orderbook_tx.send(paradex_update).await.unwrap();
        
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        
        // Should only get entry opportunity, not exit
        let opp = opportunity_rx.try_recv().expect("Should emit entry opportunity");
        assert_eq!(opp.direction, SpreadDirection::AOverB, "Should be entry direction");
        assert!(opp.spread_percent > 1.0, "Entry spread should be > 1.0%");
        
        // No exit opportunity
        assert!(opportunity_rx.try_recv().is_err(), "Exit below threshold should not emit");
        
        shutdown_tx.send(()).unwrap();
        let _ = handle.await;
    }
}
