//! Simplified channel bundle for MVP
//!
//! Minimal inter-task communication without complex dependencies.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, RwLock};

use crate::adapters::Orderbook;

/// Type alias for shared orderbooks used across all modules
///
/// Single source of truth — imported by adapters, runtime, and monitoring.
pub type SharedOrderbooks = Arc<RwLock<HashMap<String, Orderbook>>>;

// Import SpreadDirection from spread module to avoid duplication (CR-H1 fix)
pub use super::spread::SpreadDirection;

// Import OrderbookUpdate for orderbook streaming channel
use crate::adapters::types::OrderbookUpdate;

/// Default channel capacity for bounded channels
pub const DEFAULT_CHANNEL_CAPACITY: usize = 100;

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

/// Bundle of all inter-task communication channels
#[derive(Debug)]
pub struct ChannelBundle {
    /// SpreadCalculator -> Executor: spread opportunities
    pub opportunity_tx: mpsc::Sender<SpreadOpportunity>,
    pub opportunity_rx: mpsc::Receiver<SpreadOpportunity>,

    /// Adapters -> SpreadCalculator: orderbook updates
    pub orderbook_tx: mpsc::Sender<OrderbookUpdate>,
    pub orderbook_rx: mpsc::Receiver<OrderbookUpdate>,

    /// Shutdown broadcast: main -> all tasks
    pub shutdown_tx: broadcast::Sender<()>,
}

impl ChannelBundle {
    pub fn new(capacity: usize) -> Self {
        let (opportunity_tx, opportunity_rx) = mpsc::channel(capacity);
        let (orderbook_tx, orderbook_rx) = mpsc::channel(capacity);
        let (shutdown_tx, _) = broadcast::channel(1);

        Self {
            opportunity_tx,
            opportunity_rx,
            orderbook_tx,
            orderbook_rx,
            shutdown_tx,
        }
    }

    pub fn subscribe_shutdown(&self) -> broadcast::Receiver<()> {
        self.shutdown_tx.subscribe()
    }
}

impl Default for ChannelBundle {
    fn default() -> Self {
        Self::new(DEFAULT_CHANNEL_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::types::{Orderbook, OrderbookLevel};

    #[test]
    fn test_channel_bundle_creation() {
        let bundle = ChannelBundle::new(50);
        assert!(!bundle.opportunity_tx.is_closed());
        assert!(!bundle.orderbook_tx.is_closed());
    }

    #[tokio::test]
    async fn test_shutdown_signal() {
        let bundle = ChannelBundle::default();
        let mut rx = bundle.subscribe_shutdown();

        assert!(bundle.shutdown_tx.send(()).is_ok());
        assert!(rx.recv().await.is_ok());
    }

    #[tokio::test]
    async fn test_orderbook_channel_send_receive() {
        let bundle = ChannelBundle::new(10);
        let mut rx = bundle.orderbook_rx;
        let tx = bundle.orderbook_tx;

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
        assert_eq!(received.exchange, "vest"); // Verify exchange field
        assert_eq!(received.orderbook.bids.len(), 1);
        assert_eq!(received.orderbook.asks.len(), 1);
        assert_eq!(received.orderbook.bids[0].price, 100.0);
        assert_eq!(received.orderbook.asks[0].price, 101.0);
    }
}
