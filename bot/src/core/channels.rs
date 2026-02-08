//! Simplified channel bundle for MVP
//!
//! Minimal inter-task communication without complex dependencies.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use std::sync::atomic::{AtomicU64, Ordering};

use crate::adapters::Orderbook;

/// Type alias for shared orderbooks used across all modules
///
/// Single source of truth — imported by adapters, runtime, and monitoring.
pub type SharedOrderbooks = Arc<RwLock<HashMap<String, Orderbook>>>;

/// Lock-free best bid/ask storage for hot-path monitoring
///
/// Stores best bid and best ask as `AtomicU64` (f64 bit patterns).
/// The WebSocket reader writes to this on every orderbook update,
/// and the monitoring task reads it without any lock or allocation.
///
/// # Safety
/// Uses `f64::to_bits()` / `f64::from_bits()` for lossless atomic storage.
/// `Ordering::Release` on store, `Ordering::Acquire` on load ensures
/// the monitoring task always sees a consistent (bid, ask) pair.
pub struct AtomicBestPrices {
    best_bid_bits: AtomicU64,
    best_ask_bits: AtomicU64,
}

impl AtomicBestPrices {
    /// Create with initial values of 0.0 (invalid — no data yet)
    pub fn new() -> Self {
        Self {
            best_bid_bits: AtomicU64::new(0f64.to_bits()),
            best_ask_bits: AtomicU64::new(0f64.to_bits()),
        }
    }

    /// Store best bid and ask prices (called by WebSocket reader)
    #[inline]
    pub fn store(&self, bid: f64, ask: f64) {
        self.best_bid_bits.store(bid.to_bits(), Ordering::Release);
        self.best_ask_bits.store(ask.to_bits(), Ordering::Release);
    }

    /// Load best bid and ask prices (called by monitoring/exit loop)
    #[inline]
    pub fn load(&self) -> (f64, f64) {
        let bid = f64::from_bits(self.best_bid_bits.load(Ordering::Acquire));
        let ask = f64::from_bits(self.best_ask_bits.load(Ordering::Acquire));
        (bid, ask)
    }

    /// Check if prices have been initialized (both > 0.0)
    #[inline]
    pub fn is_valid(&self) -> bool {
        let (bid, ask) = self.load();
        bid > 0.0 && ask > 0.0
    }
}

/// Type alias for shared atomic best prices
pub type SharedBestPrices = Arc<AtomicBestPrices>;

/// Shared notification for event-driven monitoring (Axe 5)
///
/// Adapters call `notify_waiters()` after writing new prices to `AtomicBestPrices`.
/// The monitoring loop `notified().await`s instead of polling on a fixed interval.
pub type OrderbookNotify = Arc<tokio::sync::Notify>;

// Import SpreadDirection from spread module to avoid duplication (CR-H1 fix)
pub use super::spread::SpreadDirection;

/// Log throttle interval — log every N polls (~1 second at 25ms polling)
/// Single source of truth for runtime and monitoring tasks.
pub const LOG_THROTTLE_POLLS: u64 = 40;

/// Simple spread opportunity for MVP
#[derive(Debug, Clone)]
pub struct SpreadOpportunity {
    pub pair: String,
    pub dex_a: String,
    pub dex_b: String,
    pub spread_percent: f64,
    pub direction: SpreadDirection,
    pub detected_at_ms: u64,
    /// Best ask price on DEX A (buy price)
    pub dex_a_ask: f64,
    /// Best bid price on DEX A (sell price)
    pub dex_a_bid: f64,
    /// Best ask price on DEX B (buy price)
    pub dex_b_ask: f64,
    /// Best bid price on DEX B (sell price)
    pub dex_b_bid: f64,
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::types::{Orderbook, OrderbookLevel, OrderbookUpdate};
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn test_orderbook_channel_send_receive() {
        let (tx, mut rx) = mpsc::channel::<OrderbookUpdate>(10);

        // Create a test orderbook update
        let update = OrderbookUpdate {
            symbol: "BTC-PERP".to_string(),
            exchange: "vest".to_string(),
            orderbook: Orderbook {
                bids: vec![OrderbookLevel::new(100.0, 1.0)],
                asks: vec![OrderbookLevel::new(101.0, 1.0)],
                timestamp: 1234567890,
            },
        };

        // Send through channel
        tx.send(update.clone()).await.unwrap();

        // Receive and verify
        let received = rx.recv().await.unwrap();
        assert_eq!(received.symbol, "BTC-PERP");
        assert_eq!(received.exchange, "vest");
        assert_eq!(received.orderbook.bids.len(), 1);
        assert_eq!(received.orderbook.asks.len(), 1);
        assert_eq!(received.orderbook.bids[0].price, 100.0);
        assert_eq!(received.orderbook.asks[0].price, 101.0);
    }
}
