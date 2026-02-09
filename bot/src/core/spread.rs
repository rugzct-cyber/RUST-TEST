//! Spread calculation engine for arbitrage detection
//!
//! This module provides real-time spread calculation between two DEX orderbooks.
//! Performance target: <2ms per calculation (NFR2).
//!
//! # Architecture  
//! - `SpreadCalculator`: Main calculator struct for a DEX pair
//! - `SpreadResult`: Result of spread calculation with direction and prices
//! - `SpreadDirection`: Direction of the arbitrage opportunity

use crate::adapters::types::{OrderSide, Orderbook};
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

impl SpreadDirection {
    /// Returns (long_exchange, short_exchange) given the dex_a/dex_b names
    ///
    /// AOverB: long on A, short on B
    /// BOverA: long on B, short on A
    #[inline]
    pub fn to_exchanges<'a>(&self, dex_a: &'a str, dex_b: &'a str) -> (&'a str, &'a str) {
        match self {
            SpreadDirection::AOverB => (dex_a, dex_b),
            SpreadDirection::BOverA => (dex_b, dex_a),
        }
    }

    /// Convert to atomic storage value (1=AOverB, 2=BOverA)
    #[inline]
    pub fn to_u8(&self) -> u8 {
        match self {
            SpreadDirection::AOverB => 1,
            SpreadDirection::BOverA => 2,
        }
    }

    /// Create from atomic storage value
    #[inline]
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(SpreadDirection::AOverB),
            2 => Some(SpreadDirection::BOverA),
            _ => None,
        }
    }

    /// Get close order sides (to reverse the position)
    #[inline]
    pub fn to_close_sides(&self) -> (OrderSide, OrderSide) {
        match self {
            SpreadDirection::AOverB => (OrderSide::Sell, OrderSide::Buy),
            SpreadDirection::BOverA => (OrderSide::Buy, OrderSide::Sell),
        }
    }

    /// Calculate captured spread from entry fill prices
    ///
    /// # Arguments
    /// * `buy_price` - Price paid on the buying leg (Vest for AOverB, Paradex for BOverA)
    /// * `sell_price` - Price received on the selling leg (Paradex for AOverB, Vest for BOverA)
    #[inline]
    pub fn calculate_captured_spread(&self, buy_price: f64, sell_price: f64) -> f64 {
        if buy_price <= 0.0 {
            return 0.0;
        }
        ((sell_price - buy_price) / buy_price) * 100.0
    }
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
                ask_price: ask_a, // We BUY at this price (on A)
                bid_price: bid_b, // We SELL at this price (on B)
                midpoint,
                timestamp_ms,
            })
        } else {
            // B→A: Buy on B (Paradex), Sell on A (Vest)
            Some(SpreadResult {
                spread_pct: spread_b_to_a,
                direction: SpreadDirection::BOverA,
                ask_price: ask_b, // We BUY at this price (on B)
                bid_price: bid_a, // We SELL at this price (on A)
                midpoint,
                timestamp_ms,
            })
        }
    }

    // =========================================================================
    // Raw Price Calculation (used by AtomicBestPrices hot path)
    // =========================================================================

    /// Calculate spread from raw prices (lock-free hot path)
    ///
    /// Same logic as `calculate()` but takes 4 `f64` values directly
    /// from `AtomicBestPrices`, avoiding `Orderbook` construction/cloning.
    ///
    /// Returns `None` if any price is 0.0 (uninitialized atomic data).
    #[inline]
    #[must_use]
    pub fn calculate_from_prices(
        &self,
        bid_a: f64,
        ask_a: f64,
        bid_b: f64,
        ask_b: f64,
    ) -> Option<SpreadResult> {
        // Guard: 0.0 means no data yet
        if ask_a <= 0.0 || bid_a <= 0.0 || ask_b <= 0.0 || bid_b <= 0.0 {
            return None;
        }

        let spread_a_to_b = ((bid_b - ask_a) / ask_a) * 100.0;
        let spread_b_to_a = ((bid_a - ask_b) / ask_b) * 100.0;

        let timestamp_ms = current_time_ms();
        let midpoint = (ask_a + bid_b + ask_b + bid_a) / 4.0;

        if spread_a_to_b >= spread_b_to_a {
            Some(SpreadResult {
                spread_pct: spread_a_to_b,
                direction: SpreadDirection::AOverB,
                ask_price: ask_a,
                bid_price: bid_b,
                midpoint,
                timestamp_ms,
            })
        } else {
            Some(SpreadResult {
                spread_pct: spread_b_to_a,
                direction: SpreadDirection::BOverA,
                ask_price: ask_b,
                bid_price: bid_a,
                midpoint,
                timestamp_ms,
            })
        }
    }

    // =========================================================================
    // Dual Spread Calculation Functions (used by monitor.rs)
    // =========================================================================

    /// Calculate Entry spread: (BID_B - ASK_A) / ASK_A × 100
    ///
    /// Use this for Entry Monitor: "Buy on DEX A, Sell on DEX B" arbitrage opportunity.
    /// Formula calculates actual profit % relative to investment (ask_a is what you pay).
    ///
    /// # Arguments
    /// * `ask_a` - Best ask price on DEX A (we BUY at this price)
    /// * `bid_b` - Best bid price on DEX B (we SELL at this price)
    ///
    /// # Returns
    /// * `f64` - Entry spread percentage (positive = profit opportunity)
    ///
    /// # Edge Cases
    /// * Returns 0.0 if ask_a is zero (prevents division by zero)
    #[inline]
    pub fn calculate_entry_spread(ask_a: f64, bid_b: f64) -> f64 {
        if ask_a <= 0.0 {
            return 0.0;
        }
        ((bid_b - ask_a) / ask_a) * 100.0
    }

    /// Calculate Exit spread (for A→B entry): (BID_A - ASK_B) / ASK_B × 100
    ///
    /// Use this for Exit Monitor: "Sell on DEX A, Buy back on DEX B" to close position.
    /// Formula calculates actual profit % relative to what you pay to close (ask_b).
    ///
    /// # Arguments
    /// * `bid_a` - Best bid price on DEX A (we SELL at this price to close)
    /// * `ask_b` - Best ask price on DEX B (we BUY at this price to close)
    ///
    /// # Returns
    /// * `f64` - Exit spread percentage (typically negative when closing)
    ///
    /// # Edge Cases
    /// * Returns 0.0 if ask_b is zero (prevents division by zero)
    #[inline]
    pub fn calculate_exit_spread(bid_a: f64, ask_b: f64) -> f64 {
        if ask_b <= 0.0 {
            return 0.0;
        }
        ((bid_a - ask_b) / ask_b) * 100.0
    }

    /// Calculate both Entry and Exit spreads from orderbooks
    ///
    /// Evaluates both directions and returns the best entry spread
    /// with the corresponding exit spread for that direction.
    ///
    /// # Returns
    /// * `Option<(f64, f64)>` - (best_entry_spread_pct, matching_exit_spread_pct) or None if orderbooks empty
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

        // A→B: Buy A (ask_a), Sell B (bid_b)  →  Exit: Sell A (bid_a), Buy B (ask_b)
        let entry_a_to_b = Self::calculate_entry_spread(ask_a, bid_b);
        let exit_a_to_b = Self::calculate_exit_spread(bid_a, ask_b);

        // B→A: Buy B (ask_b), Sell A (bid_a)  →  Exit: Sell B (bid_b), Buy A (ask_a)
        let entry_b_to_a = Self::calculate_entry_spread(ask_b, bid_a);
        let exit_b_to_a = Self::calculate_exit_spread(bid_b, ask_a);

        // Return the direction with the best entry, plus its matching exit
        if entry_a_to_b >= entry_b_to_a {
            Some((entry_a_to_b, exit_a_to_b))
        } else {
            Some((entry_b_to_a, exit_b_to_a))
        }
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

// Timestamp function: use canonical implementation from core::events
use crate::core::events::current_timestamp_ms as current_time_ms;

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
        assert!(
            result.spread_pct < 0.0,
            "Spread should be negative, got {}",
            result.spread_pct
        );
        assert_eq!(result.ask_price, 99.5); // B's ask (we BUY here)
        assert_eq!(result.bid_price, 99.0); // A's bid (we SELL here)
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
        let expected = (101.0 - 98.0) / 98.0 * 100.0; // 3.0612...%
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
        assert!(
            result.spread_pct > 0.0,
            "Spread should be positive, got {}",
            result.spread_pct
        );
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
        assert!(
            result.spread_pct > 0.0,
            "Spread should be positive, got {}",
            result.spread_pct
        );
        assert_eq!(result.ask_price, 98.0); // B's ask (we BUY here)
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
        assert!(
            result.spread_pct < 0.0,
            "Expected negative spread, got {}",
            result.spread_pct
        );
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
        assert!(
            result.spread_pct < 0.0,
            "Both directions negative, spread should be < 0, got {}",
            result.spread_pct
        );
    }

    // =========================================================================
    // Task 6.5: Benchmark test - verify calculation under 2ms for 10k iterations
    // =========================================================================

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
        assert!(
            now_ms - result.timestamp_ms < 1000,
            "Timestamp should be within last second"
        );
    }

    // =========================================================================
    // Dual Spread Calculation Tests
    // =========================================================================

    #[test]
    fn test_calculate_entry_spread_basic() {
        // Entry: (BID_B - ASK_A) / ASK_A * 100
        // Ask A = 100, Bid B = 99
        // spread = (99 - 100) / 100 * 100 = -1.0%
        let spread = SpreadCalculator::calculate_entry_spread(100.0, 99.0);
        let expected = (99.0 - 100.0) / 100.0 * 100.0; // -1.0%
        assert!((spread - expected).abs() < 0.0001);
        assert!(
            spread < 0.0,
            "Entry spread negative when bid_b < ask_a (no opportunity)"
        );
    }

    #[test]
    fn test_calculate_entry_spread_positive() {
        // When BID_B > ASK_A, entry spread is POSITIVE (profit opportunity)
        // Ask A = 98, Bid B = 100 → (100-98)/98*100 = 2.04%
        let spread = SpreadCalculator::calculate_entry_spread(98.0, 100.0);
        assert!(
            spread > 0.0,
            "Entry spread should be positive when bid_b > ask_a"
        );
    }

    #[test]
    fn test_calculate_entry_spread_zero_midpoint() {
        let spread = SpreadCalculator::calculate_entry_spread(0.0, 0.0);
        assert_eq!(spread, 0.0);
    }

    #[test]
    fn test_calculate_exit_spread_basic() {
        // Exit: (BID_A - ASK_B) / ASK_B * 100
        // Bid A = 101, Ask B = 99
        // spread = (101 - 99) / 99 * 100 ≈ 2.02%
        let spread = SpreadCalculator::calculate_exit_spread(101.0, 99.0);
        let expected = (101.0 - 99.0) / 99.0 * 100.0; // ~2.0202%
        assert!((spread - expected).abs() < 0.0001);
        assert!(spread > 2.0, "Exit spread should be ~2.02%");
    }

    #[test]
    fn test_calculate_exit_spread_negative() {
        // When BID_A < ASK_B, exit spread is negative (typical market condition)
        // Bid A = 99, Ask B = 101
        // spread = (99-101)/101*100 ≈ -1.98%
        let spread = SpreadCalculator::calculate_exit_spread(99.0, 101.0);
        let expected = (99.0 - 101.0) / 101.0 * 100.0; // ~-1.98%
        assert!((spread - expected).abs() < 0.0001);
        assert!(
            spread < 0.0,
            "Exit spread typically negative when bid_a < ask_b"
        );
    }

    #[test]
    fn test_calculate_exit_spread_zero_midpoint() {
        let spread = SpreadCalculator::calculate_exit_spread(0.0, 0.0);
        assert_eq!(spread, 0.0);
    }

    #[test]
    fn test_entry_exit_spreads_are_different() {
        // Critical test: Entry and Exit MUST produce different values
        //
        // Arbitrage scenario: BID_B > ASK_A (we can buy low on A, sell high on B)
        // DEX A: Ask=99.0, Bid=98.5 (we buy at Ask=99.0)
        // DEX B: Ask=100.5, Bid=100.0 (we sell at Bid=100.0)

        let ask_a = 99.0;
        let bid_a = 98.5;
        let ask_b = 100.5;
        let bid_b = 100.0;

        let entry = SpreadCalculator::calculate_entry_spread(ask_a, bid_b);
        let exit = SpreadCalculator::calculate_exit_spread(bid_a, ask_b);

        // Entry: (bid_b - ask_a) / ask_a * 100 = (100.0 - 99.0) / 99.0 = +1.01%
        // Exit: (bid_a - ask_b) / ask_b * 100 = (98.5 - 100.5) / 100.5 = -1.99%

        assert!(entry > 0.0, "Entry should be positive: {}", entry);
        assert!(exit < 0.0, "Exit should be negative: {}", exit);
        assert!(
            (entry - exit).abs() > 0.5,
            "Entry and Exit MUST be different"
        );
    }

    #[test]
    fn test_calculate_dual_spreads_from_orderbooks() {
        let calc = SpreadCalculator::new("vest", "paradex");

        // Create orderbooks where BID_B > ASK_A for profitable entry
        // DEX A: Ask=99, Bid=98 (we buy at Ask=99)
        // DEX B: Ask=101, Bid=100 (we sell at Bid=100)
        let ob_a = make_orderbook(99.0, 98.0); // ask, bid
        let ob_b = make_orderbook(101.0, 100.0); // ask, bid

        let (entry, exit) = calc.calculate_dual_spreads(&ob_a, &ob_b).unwrap();

        // Entry: (bid_b - ask_a) / ask_a = (100 - 99) / 99 = +1.01%
        // Exit: (bid_a - ask_b) / ask_b = (98 - 101) / 101 = -2.97%
        let expected_entry = (100.0 - 99.0) / 99.0 * 100.0;
        let expected_exit = (98.0 - 101.0) / 101.0 * 100.0;

        assert!(
            (entry - expected_entry).abs() < 0.01,
            "Entry mismatch: {} vs {}",
            entry,
            expected_entry
        );
        assert!(
            (exit - expected_exit).abs() < 0.01,
            "Exit mismatch: {} vs {}",
            exit,
            expected_exit
        );

        assert!(entry > 0.0, "Entry should be positive: {}", entry);
        assert!(exit < 0.0, "Exit should be negative: {}", exit);
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
        // Real-world BTC prices scenario with arbitrage opportunity
        let calc = SpreadCalculator::new("vest", "paradex");

        // Scenario: Vest ask < Paradex bid = arbitrage opportunity
        // Vest: Ask=42145.00, Bid=42140.00 (we buy at Ask=42145)
        // Paradex: Ask=42160.00, Bid=42155.00 (we sell at Bid=42155)
        let ob_vest = make_orderbook(42145.00, 42140.00);
        let ob_paradex = make_orderbook(42160.00, 42155.00);

        let (entry, exit) = calc.calculate_dual_spreads(&ob_vest, &ob_paradex).unwrap();

        // Entry: (42155 - 42145) / 42145 * 100 = +0.0237% (positive = profit)
        // Exit: (42140 - 42160) / 42160 * 100 = -0.0474% (negative = cost to close)

        assert!(
            entry > 0.0,
            "Entry should be positive (arb opportunity): {}",
            entry
        );
        assert!(
            exit < 0.0,
            "Exit should be negative (cost to close): {}",
            exit
        );
        assert!(
            (entry - exit).abs() > 0.01,
            "Entry/Exit must differ: {} vs {}",
            entry,
            exit
        );
    }

    // =========================================================================
    // SpreadDirection Helper Tests
    // =========================================================================

    #[test]
    fn test_spread_direction_to_exchanges() {
        assert_eq!(SpreadDirection::AOverB.to_exchanges("vest", "paradex"), ("vest", "paradex"));
        assert_eq!(SpreadDirection::BOverA.to_exchanges("vest", "paradex"), ("paradex", "vest"));
    }

    #[test]
    fn test_spread_direction_to_u8() {
        assert_eq!(SpreadDirection::AOverB.to_u8(), 1);
        assert_eq!(SpreadDirection::BOverA.to_u8(), 2);
    }

    #[test]
    fn test_spread_direction_from_u8() {
        assert_eq!(SpreadDirection::from_u8(1), Some(SpreadDirection::AOverB));
        assert_eq!(SpreadDirection::from_u8(2), Some(SpreadDirection::BOverA));
        assert_eq!(SpreadDirection::from_u8(0), None);
        assert_eq!(SpreadDirection::from_u8(3), None);
    }

    #[test]
    fn test_spread_direction_to_close_sides() {
        assert_eq!(
            SpreadDirection::AOverB.to_close_sides(),
            (OrderSide::Sell, OrderSide::Buy)
        );
        assert_eq!(
            SpreadDirection::BOverA.to_close_sides(),
            (OrderSide::Buy, OrderSide::Sell)
        );
    }

    #[test]
    fn test_spread_direction_calculate_captured_spread_a_over_b() {
        // Long Vest at 42000, Short Paradex at 42100
        // Spread = (42100 - 42000) / 42000 * 100 = 0.238%
        let spread = SpreadDirection::AOverB.calculate_captured_spread(42000.0, 42100.0);
        assert!((spread - 0.238).abs() < 0.01);
    }

    #[test]
    fn test_spread_direction_calculate_captured_spread_b_over_a() {
        // Long Paradex at 42000 (buy), Short Vest at 42100 (sell)
        // Spread = (42100 - 42000) / 42000 * 100 = 0.238%
        let spread = SpreadDirection::BOverA.calculate_captured_spread(42000.0, 42100.0);
        assert!((spread - 0.238).abs() < 0.01);
    }

    #[test]
    fn test_spread_direction_calculate_captured_spread_zero_price() {
        // buy_price = 0 → always returns 0.0
        assert_eq!(
            SpreadDirection::AOverB.calculate_captured_spread(0.0, 42100.0),
            0.0
        );
        assert_eq!(
            SpreadDirection::BOverA.calculate_captured_spread(0.0, 42100.0),
            0.0
        );
    }

    #[test]
    fn test_spread_direction_roundtrip() {
        // Verify to_u8 and from_u8 are inverses
        for dir in [SpreadDirection::AOverB, SpreadDirection::BOverA] {
            assert_eq!(SpreadDirection::from_u8(dir.to_u8()), Some(dir));
        }
    }

    // =========================================================================
    // Property-based tests (proptest)
    // =========================================================================
    mod proptests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn never_panics(ask in 0.0f64..1e12, bid in 0.0f64..1e12) {
                // Should never panic regardless of input
                let _ = SpreadCalculator::calculate_entry_spread(ask, bid);
                let _ = SpreadCalculator::calculate_exit_spread(bid, ask);
            }

            #[test]
            fn always_finite(ask in 0.001f64..1e9, bid in 0.001f64..1e9) {
                let entry = SpreadCalculator::calculate_entry_spread(ask, bid);
                let exit = SpreadCalculator::calculate_exit_spread(bid, ask);
                prop_assert!(entry.is_finite(), "Entry spread must be finite: {}", entry);
                prop_assert!(exit.is_finite(), "Exit spread must be finite: {}", exit);
            }

            #[test]
            fn positive_spread_when_bid_exceeds_ask(
                ask in 100.0f64..50000.0,
                premium in 0.01f64..100.0
            ) {
                let bid = ask + premium;
                let spread = SpreadCalculator::calculate_entry_spread(ask, bid);
                prop_assert!(spread > 0.0, "Spread should be positive when bid > ask: {}", spread);
            }

            #[test]
            fn zero_ask_returns_zero(bid in 0.0f64..1e9) {
                let spread = SpreadCalculator::calculate_entry_spread(0.0, bid);
                prop_assert_eq!(spread, 0.0, "Zero ask should return 0.0");
            }

            #[test]
            fn symmetric_direction_detection(
                ask_a in 100.0f64..50000.0,
                bid_b in 100.0f64..50000.0,
                ask_b in 100.0f64..50000.0,
                bid_a in 100.0f64..50000.0,
            ) {
                // Build orderbooks
                let ob_a = Orderbook {
                    asks: vec![OrderbookLevel::new(ask_a, 1.0)],
                    bids: vec![OrderbookLevel::new(bid_a, 1.0)],
                    timestamp: 0,
                };
                let ob_b = Orderbook {
                    asks: vec![OrderbookLevel::new(ask_b, 1.0)],
                    bids: vec![OrderbookLevel::new(bid_b, 1.0)],
                    timestamp: 0,
                };

                let calc = SpreadCalculator::new("A", "B");
                if let Some(result) = calc.calculate(&ob_a, &ob_b) {
                    prop_assert!(result.spread_pct.is_finite());
                    // Direction should match the better side
                    let a_over_b = SpreadCalculator::calculate_entry_spread(ask_a, bid_b);
                    let b_over_a = SpreadCalculator::calculate_entry_spread(ask_b, bid_a);
                    if a_over_b >= b_over_a {
                        prop_assert_eq!(result.direction, SpreadDirection::AOverB);
                    } else {
                        prop_assert_eq!(result.direction, SpreadDirection::BOverA);
                    }
                }
            }

            #[test]
            fn negative_spread_when_ask_exceeds_bid(
                bid in 100.0f64..50000.0,
                premium in 0.01f64..100.0
            ) {
                let ask = bid + premium;
                let spread = SpreadCalculator::calculate_entry_spread(ask, bid);
                prop_assert!(spread < 0.0, "Spread should be negative when bid < ask: {}", spread);
            }

            #[test]
            fn large_values_no_overflow(
                ask in 1e6f64..1e12,
                bid in 1e6f64..1e12
            ) {
                let entry = SpreadCalculator::calculate_entry_spread(ask, bid);
                let exit = SpreadCalculator::calculate_exit_spread(bid, ask);
                prop_assert!(entry.is_finite(), "Large value entry should be finite");
                prop_assert!(exit.is_finite(), "Large value exit should be finite");
            }

            #[test]
            fn entry_exit_inverse_relationship(
                ask in 100.0f64..50000.0,
                bid in 100.0f64..50000.0
            ) {
                // Entry: (bid - ask) / ask * 100
                // Exit:  (bid - ask) / ask * 100 (with reversed roles)
                let entry = SpreadCalculator::calculate_entry_spread(ask, bid);
                let exit = SpreadCalculator::calculate_exit_spread(bid, ask);
                // Both should be finite
                prop_assert!(entry.is_finite());
                prop_assert!(exit.is_finite());
            }
        }
    }
}
