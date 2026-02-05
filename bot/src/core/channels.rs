//! Simplified channel bundle for MVP
//!
//! Minimal inter-task communication without complex dependencies.

use tokio::sync::{broadcast, mpsc};

// Import SpreadDirection from spread module to avoid duplication (CR-H1 fix)
pub use super::spread::SpreadDirection;

// Import OrderbookUpdate for orderbook streaming channel (Story 1.3)
use crate::adapters::types::OrderbookUpdate;

/// Default channel capacity for bounded channels
pub const DEFAULT_CHANNEL_CAPACITY: usize = 100;

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

/// Signal to close an open position (Story 6.3: Exit Monitoring)
#[derive(Debug, Clone)]
pub struct ExitSignal {
    /// Current exit spread when condition was detected
    pub exit_spread: f64,
    /// Timestamp when exit condition was detected
    pub detected_at_ms: u64,
}

/// Shared position state for coordination between executor and monitoring (Story 6.3)
/// 
/// Uses atomics for lock-free read/write across tasks.
#[derive(Debug)]
pub struct PositionState {
    /// True if a position is currently open
    pub is_open: std::sync::atomic::AtomicBool,
    /// Entry direction: 0=none, 1=AOverB (long Vest), 2=BOverA (long Paradex)
    pub entry_direction: std::sync::atomic::AtomicU8,
}

impl PositionState {
    pub fn new() -> Self {
        Self {
            is_open: std::sync::atomic::AtomicBool::new(false),
            entry_direction: std::sync::atomic::AtomicU8::new(0),
        }
    }
}

impl Default for PositionState {
    fn default() -> Self {
        Self::new()
    }
}


/// Bundle of all inter-task communication channels
#[derive(Debug)]
pub struct ChannelBundle {
    /// SpreadCalculator -> Executor: spread opportunities
    pub opportunity_tx: mpsc::Sender<SpreadOpportunity>,
    pub opportunity_rx: mpsc::Receiver<SpreadOpportunity>,
    
    /// Adapters -> SpreadCalculator: orderbook updates (Story 1.3)
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
        assert_eq!(received.exchange, "vest"); // Story 1.4: Verify exchange field
        assert_eq!(received.orderbook.bids.len(), 1);
        assert_eq!(received.orderbook.asks.len(), 1);
        assert_eq!(received.orderbook.bids[0].price, 100.0);
        assert_eq!(received.orderbook.asks[0].price, 101.0);
    }
}
