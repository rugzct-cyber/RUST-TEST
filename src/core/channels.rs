//! Simplified channel bundle for MVP
//!
//! Minimal inter-task communication without complex dependencies.

use tokio::sync::{broadcast, mpsc};

// Import SpreadDirection from spread module to avoid duplication (CR-H1 fix)
pub use super::spread::SpreadDirection;

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
}

/// Bundle of all inter-task communication channels
#[derive(Debug)]
pub struct ChannelBundle {
    /// SpreadCalculator -> Executor: spread opportunities
    pub opportunity_tx: mpsc::Sender<SpreadOpportunity>,
    pub opportunity_rx: mpsc::Receiver<SpreadOpportunity>,
    
    /// Shutdown broadcast: main -> all tasks
    pub shutdown_tx: broadcast::Sender<()>,
}

impl ChannelBundle {
    pub fn new(capacity: usize) -> Self {
        let (opportunity_tx, opportunity_rx) = mpsc::channel(capacity);
        let (shutdown_tx, _) = broadcast::channel(1);
        
        Self {
            opportunity_tx,
            opportunity_rx,
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

    #[test]
    fn test_channel_bundle_creation() {
        let bundle = ChannelBundle::new(50);
        assert!(!bundle.opportunity_tx.is_closed());
    }

    #[tokio::test]
    async fn test_shutdown_signal() {
        let bundle = ChannelBundle::default();
        let mut rx = bundle.subscribe_shutdown();
        
        assert!(bundle.shutdown_tx.send(()).is_ok());
        assert!(rx.recv().await.is_ok());
    }
}
