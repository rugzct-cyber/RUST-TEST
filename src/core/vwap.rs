//! VWAP (Volume-Weighted Average Price) orderbook calculation module
//!
//! Calculates realistic execution prices based on orderbook depth,
//! handling partial fills across multiple levels.
//!
//! Performance target: <2ms for calculation (NFR2 compliance)

use crate::adapters::types::{Orderbook, OrderSide};
use serde::{Deserialize, Serialize};

/// Result of VWAP calculation through orderbook depth
#[derive(Debug, Clone, Serialize, Deserialize)]
#[must_use]
pub struct VwapResult {
    /// Volume-weighted average price across consumed levels
    pub vwap_price: f64,
    /// Total quantity that can be filled
    pub total_quantity: f64,
    /// Slippage from best price in basis points (1 bp = 0.01%)
    pub slippage_bps: f64,
    /// Number of orderbook levels consumed to fill quantity
    pub levels_consumed: usize,
    /// Best price for reference (best ask for Buy, best bid for Sell)
    pub best_price: f64,
}

impl VwapResult {
    /// Check if slippage is acceptable (threshold in basis points)
    /// Story 3.5 threshold: 10 bps (0.10%)
    #[inline]
    pub fn is_slippage_acceptable(&self, threshold_bps: f64) -> bool {
        self.slippage_bps.abs() <= threshold_bps
    }
}

/// Calculate VWAP from orderbook depth for a given quantity and side
///
/// # Arguments
/// * `orderbook` - The orderbook to calculate from
/// * `side` - Buy (consume asks) or Sell (consume bids)
/// * `target_quantity` - The quantity to fill
///
/// # Returns
/// * `Some(VwapResult)` if sufficient depth exists
/// * `None` if orderbook is empty or insufficient depth
///
/// # Performance
/// This function is optimized for <2ms latency (NFR2).
/// Uses inline attributes and avoids allocations in the hot path.
#[inline]
#[must_use]
pub fn calculate_vwap(
    orderbook: &Orderbook,
    side: OrderSide,
    target_quantity: f64,
) -> Option<VwapResult> {
    // Validate target_quantity - must be positive
    // Returns None for zero or negative to prevent division by zero
    if target_quantity <= 0.0 {
        return None;
    }

    // Select correct side of orderbook
    // Buy: consume asks (lowest first)
    // Sell: consume bids (highest first)
    let levels = match side {
        OrderSide::Buy => &orderbook.asks,
        OrderSide::Sell => &orderbook.bids,
    };

    if levels.is_empty() {
        return None;
    }

    let best_price = levels[0].price;
    let mut remaining = target_quantity;
    let mut weighted_sum = 0.0;
    let mut filled_quantity = 0.0;
    let mut levels_consumed = 0;

    for level in levels.iter() {
        if remaining <= 0.0 {
            break;
        }

        // Partial fill at this level
        let fill_qty = remaining.min(level.quantity);
        weighted_sum += level.price * fill_qty;
        filled_quantity += fill_qty;
        remaining -= fill_qty;
        levels_consumed += 1;
    }

    // Insufficient depth - couldn't fill full quantity
    if remaining > 0.0 {
        return None;
    }

    let vwap_price = weighted_sum / filled_quantity;
    let slippage_bps = calculate_slippage_bps(vwap_price, best_price, side);

    Some(VwapResult {
        vwap_price,
        total_quantity: filled_quantity,
        slippage_bps,
        levels_consumed,
        best_price,
    })
}

/// Calculate slippage in basis points
///
/// For Buy: positive slippage means paying more than best ask
/// For Sell: positive slippage means receiving less than best bid
#[inline]
#[must_use]
fn calculate_slippage_bps(vwap: f64, best: f64, side: OrderSide) -> f64 {
    if best == 0.0 {
        return 0.0;
    }
    match side {
        OrderSide::Buy => (vwap - best) / best * 10_000.0,
        OrderSide::Sell => (best - vwap) / best * 10_000.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::types::OrderbookLevel;

    fn make_orderbook(asks: Vec<(f64, f64)>, bids: Vec<(f64, f64)>) -> Orderbook {
        let mut ob = Orderbook::new();
        ob.asks = asks
            .into_iter()
            .map(|(p, q)| OrderbookLevel::new(p, q))
            .collect();
        ob.bids = bids
            .into_iter()
            .map(|(p, q)| OrderbookLevel::new(p, q))
            .collect();
        ob
    }

    // =========================================================================
    // Task 6.1: Test VWAP with exact single level fill
    // =========================================================================

    #[test]
    fn test_vwap_single_level_exact_fill() {
        // Buy 1.0 BTC from single ask level at 42000
        let ob = make_orderbook(
            vec![(42000.0, 2.0)], // Ask: 2 BTC at 42000
            vec![(41900.0, 2.0)], // Bid
        );

        let result = calculate_vwap(&ob, OrderSide::Buy, 1.0).unwrap();

        assert_eq!(result.vwap_price, 42000.0);
        assert_eq!(result.total_quantity, 1.0);
        assert_eq!(result.slippage_bps, 0.0); // No slippage for single level
        assert_eq!(result.levels_consumed, 1);
        assert_eq!(result.best_price, 42000.0);
    }

    #[test]
    fn test_vwap_single_level_full_consume() {
        // Buy exactly the available quantity
        let ob = make_orderbook(
            vec![(42000.0, 1.0)], // Ask: exactly 1 BTC
            vec![(41900.0, 1.0)],
        );

        let result = calculate_vwap(&ob, OrderSide::Buy, 1.0).unwrap();

        assert_eq!(result.vwap_price, 42000.0);
        assert_eq!(result.total_quantity, 1.0);
        assert_eq!(result.levels_consumed, 1);
    }

    // =========================================================================
    // Task 6.2: Test VWAP with partial level fill (multi-level)
    // =========================================================================

    #[test]
    fn test_vwap_multi_level_partial_fill() {
        // Buy 3.0 BTC across multiple levels
        // Level 1: 1.0 @ 42000 → cost = 42000
        // Level 2: 1.5 @ 42050 → cost = 63075
        // Level 3: 0.5 @ 42100 (partial) → cost = 21050
        // Total: 3.0 BTC, Total cost = 126,125
        // VWAP = 126,125 / 3.0 = 42,041.67
        let ob = make_orderbook(
            vec![(42000.0, 1.0), (42050.0, 1.5), (42100.0, 2.0)],
            vec![(41900.0, 5.0)],
        );

        let result = calculate_vwap(&ob, OrderSide::Buy, 3.0).unwrap();

        // Verify VWAP is between best and worst prices consumed
        assert!(result.vwap_price > 42000.0);
        assert!(result.vwap_price < 42100.0);
        assert_eq!(result.total_quantity, 3.0);
        assert_eq!(result.levels_consumed, 3);
        // Slippage should be positive (paying more than best)
        assert!(result.slippage_bps > 0.0);
    }

    #[test]
    fn test_vwap_exact_calculation() {
        // Precise calculation test
        // Buy 2.5 BTC:
        // Level 1: 1.0 @ 100.0 → cost = 100.0
        // Level 2: 1.5 @ 110.0 (partial of 2.0) → cost = 165.0
        // Total cost = 265.0, VWAP = 265.0 / 2.5 = 106.0
        let ob = make_orderbook(
            vec![(100.0, 1.0), (110.0, 2.0)],
            vec![(90.0, 5.0)],
        );

        let result = calculate_vwap(&ob, OrderSide::Buy, 2.5).unwrap();

        assert!((result.vwap_price - 106.0).abs() < 0.0001);
        assert_eq!(result.total_quantity, 2.5);
        assert_eq!(result.levels_consumed, 2);
        // Slippage: (106 - 100) / 100 * 10000 = 600 bps
        assert!((result.slippage_bps - 600.0).abs() < 0.1);
    }

    // =========================================================================
    // Task 6.3: Test insufficient depth returns None/error
    // =========================================================================

    #[test]
    fn test_vwap_insufficient_depth_returns_none() {
        let ob = make_orderbook(
            vec![(42000.0, 1.0)], // Only 1 BTC available
            vec![(41900.0, 1.0)],
        );

        // Try to buy 5 BTC - insufficient
        let result = calculate_vwap(&ob, OrderSide::Buy, 5.0);
        assert!(result.is_none());
    }

    #[test]
    fn test_vwap_empty_asks_returns_none() {
        let ob = make_orderbook(
            vec![], // No asks
            vec![(41900.0, 1.0)],
        );

        let result = calculate_vwap(&ob, OrderSide::Buy, 1.0);
        assert!(result.is_none());
    }

    #[test]
    fn test_vwap_empty_bids_returns_none() {
        let ob = make_orderbook(
            vec![(42000.0, 1.0)],
            vec![], // No bids
        );

        let result = calculate_vwap(&ob, OrderSide::Sell, 1.0);
        assert!(result.is_none());
    }

    #[test]
    fn test_vwap_empty_orderbook_returns_none() {
        let ob = make_orderbook(vec![], vec![]);

        assert!(calculate_vwap(&ob, OrderSide::Buy, 1.0).is_none());
        assert!(calculate_vwap(&ob, OrderSide::Sell, 1.0).is_none());
    }

    // =========================================================================
    // Task 6.4: Test slippage calculation correctness (known values)
    // =========================================================================

    #[test]
    fn test_slippage_calculation_buy() {
        // Buy across levels with known slippage
        // VWAP = 105, best = 100
        // Slippage = (105 - 100) / 100 * 10000 = 500 bps
        let ob = make_orderbook(
            vec![(100.0, 0.5), (110.0, 0.5)],
            vec![(90.0, 1.0)],
        );

        let result = calculate_vwap(&ob, OrderSide::Buy, 1.0).unwrap();
        
        // VWAP = (0.5 * 100 + 0.5 * 110) / 1.0 = 105
        assert!((result.vwap_price - 105.0).abs() < 0.0001);
        // Slippage = (105 - 100) / 100 * 10000 = 500 bps
        assert!((result.slippage_bps - 500.0).abs() < 0.1);
    }

    #[test]
    fn test_slippage_calculation_sell() {
        // Sell across levels with known slippage
        // VWAP = 95, best = 100
        // Slippage = (100 - 95) / 100 * 10000 = 500 bps (positive = worse)
        let ob = make_orderbook(
            vec![(110.0, 1.0)],
            vec![(100.0, 0.5), (90.0, 0.5)],
        );

        let result = calculate_vwap(&ob, OrderSide::Sell, 1.0).unwrap();

        // VWAP = (0.5 * 100 + 0.5 * 90) / 1.0 = 95
        assert!((result.vwap_price - 95.0).abs() < 0.0001);
        // Slippage = (100 - 95) / 100 * 10000 = 500 bps
        assert!((result.slippage_bps - 500.0).abs() < 0.1);
    }

    #[test]
    fn test_slippage_acceptable_threshold() {
        let result = VwapResult {
            vwap_price: 42010.0,
            total_quantity: 1.0,
            slippage_bps: 9.5, // 9.5 bps
            levels_consumed: 2,
            best_price: 42000.0,
        };

        assert!(result.is_slippage_acceptable(10.0)); // 10 bps threshold
        assert!(!result.is_slippage_acceptable(5.0)); // 5 bps threshold
    }

    #[test]
    fn test_zero_slippage_single_level() {
        // Single level fill = zero slippage
        let ob = make_orderbook(
            vec![(42000.0, 10.0)],
            vec![(41900.0, 10.0)],
        );

        let buy_result = calculate_vwap(&ob, OrderSide::Buy, 5.0).unwrap();
        assert_eq!(buy_result.slippage_bps, 0.0);

        let sell_result = calculate_vwap(&ob, OrderSide::Sell, 5.0).unwrap();
        assert_eq!(sell_result.slippage_bps, 0.0);
    }

    // =========================================================================
    // Task 6.5: Test Buy vs Sell side iteration (asks vs bids)
    // =========================================================================

    #[test]
    fn test_vwap_sell_side_uses_bids() {
        // Sell 1.0 BTC to bids (highest first)
        let ob = make_orderbook(
            vec![(42100.0, 2.0)],
            vec![(42000.0, 2.0)], // Best bid = 42000
        );

        let result = calculate_vwap(&ob, OrderSide::Sell, 1.0).unwrap();

        assert_eq!(result.vwap_price, 42000.0);
        assert_eq!(result.best_price, 42000.0);
        assert_eq!(result.slippage_bps, 0.0);
    }

    #[test]
    fn test_vwap_buy_side_uses_asks() {
        // Buy 1.0 BTC from asks (lowest first)
        let ob = make_orderbook(
            vec![(42100.0, 2.0)], // Best ask = 42100
            vec![(42000.0, 2.0)],
        );

        let result = calculate_vwap(&ob, OrderSide::Buy, 1.0).unwrap();

        assert_eq!(result.vwap_price, 42100.0);
        assert_eq!(result.best_price, 42100.0);
        assert_eq!(result.slippage_bps, 0.0);
    }

    #[test]
    fn test_buy_and_sell_different_results() {
        // Same orderbook, different results for buy vs sell
        let ob = make_orderbook(
            vec![(105.0, 1.0), (110.0, 1.0)], // Asks
            vec![(100.0, 1.0), (95.0, 1.0)],  // Bids
        );

        let buy_result = calculate_vwap(&ob, OrderSide::Buy, 1.5).unwrap();
        let sell_result = calculate_vwap(&ob, OrderSide::Sell, 1.5).unwrap();

        // Buy should have higher VWAP (consuming asks)
        assert!(buy_result.vwap_price > sell_result.vwap_price);
        assert_eq!(buy_result.best_price, 105.0);
        assert_eq!(sell_result.best_price, 100.0);
    }

    // =========================================================================
    // Task 6.6: Performance benchmark: 10k VWAP calcs < 2ms
    // =========================================================================

    #[test]
    fn test_vwap_performance_benchmark() {
        let ob = make_orderbook(
            (0..10).map(|i| (42000.0 + i as f64 * 10.0, 1.0)).collect(),
            (0..10).map(|i| (41990.0 - i as f64 * 10.0, 1.0)).collect(),
        );

        let start = std::time::Instant::now();
        for _ in 0..10_000 {
            let _ = calculate_vwap(&ob, OrderSide::Buy, 5.0);
        }
        let elapsed = start.elapsed();

        // 10k VWAP calcs should complete in <2ms (NFR2)
        assert!(
            elapsed.as_millis() < 2,
            "Performance: 10k calcs took {:?}",
            elapsed
        );
    }

    #[test]
    fn test_vwap_performance_with_sell() {
        let ob = make_orderbook(
            (0..10).map(|i| (42000.0 + i as f64 * 10.0, 1.0)).collect(),
            (0..10).map(|i| (41990.0 - i as f64 * 10.0, 1.0)).collect(),
        );

        let start = std::time::Instant::now();
        for _ in 0..10_000 {
            let _ = calculate_vwap(&ob, OrderSide::Sell, 5.0);
        }
        let elapsed = start.elapsed();

        assert!(
            elapsed.as_millis() < 2,
            "Performance: 10k sell calcs took {:?}",
            elapsed
        );
    }

    // =========================================================================
    // Additional edge case tests
    // =========================================================================

    #[test]
    fn test_vwap_zero_quantity_returns_none() {
        let ob = make_orderbook(
            vec![(42000.0, 2.0)],
            vec![(41900.0, 2.0)],
        );

        // Zero quantity request - returns None (invalid input)
        let result = calculate_vwap(&ob, OrderSide::Buy, 0.0);
        assert!(result.is_none(), "Zero quantity should return None");
    }

    #[test]
    fn test_vwap_negative_quantity_returns_none() {
        let ob = make_orderbook(
            vec![(42000.0, 2.0)],
            vec![(41900.0, 2.0)],
        );

        // Negative quantity request - returns None (invalid input)
        let result = calculate_vwap(&ob, OrderSide::Buy, -1.0);
        assert!(result.is_none(), "Negative quantity should return None");
    }

    #[test]
    fn test_vwap_very_small_quantity() {
        let ob = make_orderbook(
            vec![(42000.0, 0.00001)],
            vec![(41900.0, 0.00001)],
        );

        let result = calculate_vwap(&ob, OrderSide::Buy, 0.000001).unwrap();
        assert_eq!(result.levels_consumed, 1);
    }
}
