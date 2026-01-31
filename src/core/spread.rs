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
        // Get best prices from both orderbooks (Task 4.2, 4.3)
        let ask_a = orderbook_a.best_ask()?;
        let bid_a = orderbook_a.best_bid()?;
        let ask_b = orderbook_b.best_ask()?;
        let bid_b = orderbook_b.best_bid()?;

        // Calculate both directions (Task 2.3)
        let spread_a_over_b = Self::raw_spread(ask_a, bid_b);
        let spread_b_over_a = Self::raw_spread(ask_b, bid_a);

        // Return the better opportunity
        let timestamp_ms = current_time_ms();

        if spread_a_over_b >= spread_b_over_a {
            Some(SpreadResult {
                spread_pct: spread_a_over_b,
                direction: SpreadDirection::AOverB,
                ask_price: ask_a,
                bid_price: bid_b,
                midpoint: (ask_a + bid_b) / 2.0, // Task 2.4
                timestamp_ms,
            })
        } else {
            Some(SpreadResult {
                spread_pct: spread_b_over_a,
                direction: SpreadDirection::BOverA,
                ask_price: ask_b,
                bid_price: bid_a,
                midpoint: (ask_b + bid_a) / 2.0, // Task 2.4
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
}

impl SpreadMonitor {
    /// Create a new SpreadMonitor for a trading pair
    pub fn new(pair: impl Into<String>) -> Self {
        Self {
            calculator: SpreadCalculator::new("vest", "paradex"),
            vest_orderbook: None,
            paradex_orderbook: None,
            pair: pair.into(),
        }
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
                
                // Emit opportunity if entry spread is positive (buy on vest, sell on paradex)
                if entry_spread > 0.0 {
                    info!(
                        pair = %self.pair,
                        spread = %format!("{:.4}%", entry_spread),
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
                    };
                    
                    tx.send(opportunity).await.map_err(|_| SpreadMonitorError::ChannelClosed)?;
                }
                
                // Emit opportunity if exit spread is positive (sell on vest, buy back on paradex)
                if exit_spread > 0.0 {
                    info!(
                        pair = %self.pair,
                        spread = %format!("{:.4}%", exit_spread),
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

        // A ask (100) > B bid (98.5): spread = (100-98.5) / 99.25 * 100 ≈ 1.51%
        assert!(result.spread_pct > 1.5, "Spread should be > 1.5%, got {}", result.spread_pct);
        assert!(result.spread_pct < 1.52, "Spread should be < 1.52%, got {}", result.spread_pct);
        assert_eq!(result.direction, SpreadDirection::AOverB);
        assert_eq!(result.ask_price, 100.0);
        assert_eq!(result.bid_price, 98.5);
    }

    #[test]
    fn test_spread_calculation_exact_values() {
        let calc = SpreadCalculator::new("vest", "paradex");
        // Simple case: ask=102, bid=100 -> spread = (102-100)/101 * 100 = 1.9801...%
        let ob_a = make_orderbook(102.0, 101.0);
        let ob_b = make_orderbook(101.5, 100.0);

        let result = calc.calculate(&ob_a, &ob_b).unwrap();

        // A ask (102) - B bid (100) = 2, midpoint = 101, spread = 2/101*100 ≈ 1.98%
        let expected = (102.0 - 100.0) / 101.0 * 100.0;
        assert!((result.spread_pct - expected).abs() < 0.0001);
    }

    // =========================================================================
    // Task 6.2: Test both spread directions (A>B, B>A)
    // =========================================================================

    #[test]
    fn test_spread_direction_a_over_b() {
        let calc = SpreadCalculator::new("vest", "paradex");
        // A has higher ask, B has lower bid -> A over B opportunity
        let ob_a = make_orderbook(105.0, 104.0);
        let ob_b = make_orderbook(103.0, 102.0);

        let result = calc.calculate(&ob_a, &ob_b).unwrap();
        
        // A ask (105) - B bid (102) = 3 -> A over B
        // B ask (103) - A bid (104) = -1 -> no opportunity
        assert_eq!(result.direction, SpreadDirection::AOverB);
        assert!(result.spread_pct > 0.0);
    }

    #[test]
    fn test_spread_direction_b_over_a() {
        let calc = SpreadCalculator::new("vest", "paradex");
        // B has higher ask, A has lower bid -> B over A opportunity
        let ob_a = make_orderbook(98.0, 97.0);
        let ob_b = make_orderbook(100.0, 99.0);

        let result = calc.calculate(&ob_a, &ob_b).unwrap();

        // A ask (98) - B bid (99) = -1 -> no opportunity in A->B
        // B ask (100) - A bid (97) = 3 -> B over A opportunity
        assert_eq!(result.direction, SpreadDirection::BOverA);
        assert!(result.spread_pct > 0.0);
        assert_eq!(result.ask_price, 100.0); // B's ask
        assert_eq!(result.bid_price, 97.0);  // A's bid
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
        // A ask < B bid: negative spread in A->B direction
        let ob_a = make_orderbook(98.0, 97.0);
        let ob_b = make_orderbook(99.0, 98.5);

        let result = calc.calculate(&ob_a, &ob_b).unwrap();

        // A ask (98) - B bid (98.5) = -0.5 -> negative in A->B
        // B ask (99) - A bid (97) = +2 -> positive in B->A
        // Calculator should return B->A as the better direction
        assert_eq!(result.direction, SpreadDirection::BOverA);
        assert!(result.spread_pct > 0.0);
    }

    #[test]
    fn test_spread_negative_both_directions() {
        let calc = SpreadCalculator::new("vest", "paradex");
        // Both directions have negative spread (crossed market within each exchange)
        // A: ask=100, bid=101 (crossed)
        // B: ask=100, bid=101 (crossed)
        // This is an edge case with crossed markets
        let mut ob_a = Orderbook::new();
        ob_a.asks.push(OrderbookLevel::new(100.0, 1.0));
        ob_a.bids.push(OrderbookLevel::new(101.0, 1.0)); // bid > ask (crossed)

        let mut ob_b = Orderbook::new();
        ob_b.asks.push(OrderbookLevel::new(100.0, 1.0));
        ob_b.bids.push(OrderbookLevel::new(101.0, 1.0)); // bid > ask (crossed)

        let result = calc.calculate(&ob_a, &ob_b).unwrap();

        // Both A ask (100) - B bid (101) = -1 and B ask (100) - A bid (101) = -1
        // Both spreads are negative, but calculator returns the "better" (less negative) one
        assert!(result.spread_pct < 0.0, "Both directions negative, spread should be < 0");
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
        let monitor = SpreadMonitor::new("BTC-PERP");
        assert!(!monitor.has_both_orderbooks());
    }

    #[tokio::test]
    async fn test_spread_monitor_processes_orderbook_update() {
        // Create monitor
        let mut monitor = SpreadMonitor::new("ETH-PERP");
        
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
        let mut monitor = SpreadMonitor::new("BTC-PERP");
        
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
        let mut monitor = SpreadMonitor::new("ETH-PERP");
        
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
}

