//! Orderbook monitoring task for automatic spread detection (Story 6.2)
//!
//! This module provides the polling-based monitoring task that continuously
//! polls orderbooks from both exchanges, calculates spreads, and emits
//! SpreadOpportunity messages when thresholds are exceeded.
//!
//! # Architecture
//! - Polls orderbooks every 100ms using `tokio::time::interval`
//! - Calculates spreads using `SpreadCalculator`
//! - Emits `SpreadOpportunity` via mpsc channel to execution_task
//! - Shutdown-aware via broadcast receiver

use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, Mutex};
use tokio::time::{interval, Duration};
use tracing::{debug, info, warn, error};

use crate::adapters::ExchangeAdapter;
use crate::core::channels::SpreadOpportunity;
use crate::core::spread::SpreadCalculator;

/// Polling interval for orderbook monitoring (100ms)
pub const POLL_INTERVAL_MS: u64 = 100;

/// Monitoring configuration for the task
#[derive(Clone, Debug)]
pub struct MonitoringConfig {
    /// Trading pair string (e.g., "BTC-PERP")
    pub pair: String,
    /// Entry threshold in percentage (e.g., 0.30 = 0.30%)
    pub spread_entry: f64,
}

/// Monitoring task that polls orderbooks and detects spread opportunities
///
/// # Story 6.2 Implementation
/// - Polls orderbooks from both exchanges every 100ms
/// - Calculates spread using SpreadCalculator
/// - Emits SpreadOpportunity when spread >= entry threshold
/// - Responds to shutdown signal for graceful termination
///
/// # Type Parameters
/// * `V` - Vest exchange adapter implementing ExchangeAdapter
/// * `P` - Paradex exchange adapter implementing ExchangeAdapter
///
/// # Arguments
/// * `vest` - Vest adapter wrapped in Arc<Mutex<>>
/// * `paradex` - Paradex adapter wrapped in Arc<Mutex<>>
/// * `opportunity_tx` - Channel to send spread opportunities to execution_task
/// * `vest_symbol` - Symbol for Vest orderbook (e.g., "BTC-PERP")
/// * `paradex_symbol` - Symbol for Paradex orderbook (e.g., "BTC-USD-PERP")
/// * `config` - Monitoring configuration with spread thresholds
/// * `shutdown_rx` - Broadcast receiver for shutdown signal
pub async fn monitoring_task<V, P>(
    vest: Arc<Mutex<V>>,
    paradex: Arc<Mutex<P>>,
    opportunity_tx: mpsc::Sender<SpreadOpportunity>,
    vest_symbol: String,
    paradex_symbol: String,
    config: MonitoringConfig,
    mut shutdown_rx: broadcast::Receiver<()>,
) where
    V: ExchangeAdapter + Send,
    P: ExchangeAdapter + Send,
{
    info!("Monitoring task started");
    
    let calculator = SpreadCalculator::new("vest", "paradex");
    let mut poll_interval = interval(Duration::from_millis(POLL_INTERVAL_MS));
    
    loop {
        tokio::select! {
            // Shutdown takes priority
            _ = shutdown_rx.recv() => {
                info!("Monitoring task shutting down");
                break;
            }
            // Poll orderbooks on interval
            _ = poll_interval.tick() => {
                // Get orderbooks from both adapters (clone to release lock quickly)
                let vest_ob = {
                    let guard = vest.lock().await;
                    guard.get_orderbook(&vest_symbol).cloned()
                };
                
                let paradex_ob = {
                    let guard = paradex.lock().await;
                    guard.get_orderbook(&paradex_symbol).cloned()
                };
                
                // Only calculate spread if both orderbooks are available
                if let (Some(vest_orderbook), Some(paradex_orderbook)) = (vest_ob, paradex_ob) {
                    if let Some(spread_result) = calculator.calculate(&vest_orderbook, &paradex_orderbook) {
                        debug!(
                            spread = %format!("{:.4}%", spread_result.spread_pct),
                            direction = ?spread_result.direction,
                            "Spread calculated"
                        );
                        
                        // Check if spread exceeds entry threshold
                        if spread_result.spread_pct >= config.spread_entry {
                            info!(
                                spread = %format!("{:.4}%", spread_result.spread_pct),
                                threshold = %format!("{:.4}%", config.spread_entry),
                                "[TRADE] Spread opportunity detected"
                            );
                            
                            // Create timestamp for opportunity
                            let now_ms = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_millis() as u64)
                                .unwrap_or(0);
                            
                            let opportunity = SpreadOpportunity {
                                pair: config.pair.clone(),
                                dex_a: "vest".to_string(),
                                dex_b: "paradex".to_string(),
                                spread_percent: spread_result.spread_pct,
                                direction: spread_result.direction,
                                detected_at_ms: now_ms,
                            };
                            
                            // Non-blocking send to avoid blocking monitoring loop
                            match opportunity_tx.try_send(opportunity) {
                                Ok(_) => {
                                    debug!("Spread opportunity sent to execution task");
                                }
                                Err(mpsc::error::TrySendError::Full(_)) => {
                                    warn!("Opportunity channel full, dropping opportunity");
                                }
                                Err(mpsc::error::TrySendError::Closed(_)) => {
                                    error!("Opportunity channel closed, shutting down monitoring");
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    
    info!("Monitoring task stopped");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::ExchangeAdapter;
    use crate::adapters::types::{Orderbook, OrderbookLevel, OrderRequest, OrderResponse, OrderStatus, PositionInfo};
    use crate::adapters::ExchangeResult;
    use async_trait::async_trait;
    use tokio::time::timeout;
    
    /// Mock adapter for monitoring tests
    struct MockMonitoringAdapter {
        name: &'static str,
        orderbook: Option<Orderbook>,
    }
    
    impl MockMonitoringAdapter {
        fn new(name: &'static str) -> Self {
            Self { name, orderbook: None }
        }
        
        fn with_orderbook(name: &'static str, best_ask: f64, best_bid: f64) -> Self {
            let mut ob = Orderbook::new();
            ob.asks.push(OrderbookLevel::new(best_ask, 1.0));
            ob.bids.push(OrderbookLevel::new(best_bid, 1.0));
            Self { name, orderbook: Some(ob) }
        }
    }
    
    #[async_trait]
    impl ExchangeAdapter for MockMonitoringAdapter {
        async fn connect(&mut self) -> ExchangeResult<()> { Ok(()) }
        async fn disconnect(&mut self) -> ExchangeResult<()> { Ok(()) }
        async fn subscribe_orderbook(&mut self, _symbol: &str) -> ExchangeResult<()> { Ok(()) }
        async fn unsubscribe_orderbook(&mut self, _symbol: &str) -> ExchangeResult<()> { Ok(()) }
        async fn place_order(&self, _order: OrderRequest) -> ExchangeResult<OrderResponse> {
            Ok(OrderResponse {
                order_id: "mock-order".to_string(),
                client_order_id: "client-order".to_string(),
                status: OrderStatus::Filled,
                filled_quantity: 0.01,
                avg_price: Some(42000.0),
            })
        }
        async fn cancel_order(&self, _order_id: &str) -> ExchangeResult<()> { Ok(()) }
        fn get_orderbook(&self, _symbol: &str) -> Option<&Orderbook> { self.orderbook.as_ref() }
        fn is_connected(&self) -> bool { true }
        fn is_stale(&self) -> bool { false }
        async fn sync_orderbooks(&mut self) {}
        async fn reconnect(&mut self) -> ExchangeResult<()> { Ok(()) }
        async fn get_position(&self, _symbol: &str) -> ExchangeResult<Option<PositionInfo>> { Ok(None) }
        fn exchange_name(&self) -> &'static str { self.name }
    }
    
    #[tokio::test]
    async fn test_monitoring_task_shutdown() {
        let vest = Arc::new(Mutex::new(MockMonitoringAdapter::new("vest")));
        let paradex = Arc::new(Mutex::new(MockMonitoringAdapter::new("paradex")));
        let (opportunity_tx, _opportunity_rx) = mpsc::channel(10);
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
        
        let config = MonitoringConfig {
            pair: "BTC-PERP".to_string(),
            spread_entry: 0.30,
        };
        
        // Spawn monitoring task
        let handle = tokio::spawn(monitoring_task(
            vest,
            paradex,
            opportunity_tx,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
            config,
            shutdown_rx,
        ));
        
        // Wait a bit then send shutdown
        tokio::time::sleep(Duration::from_millis(50)).await;
        shutdown_tx.send(()).unwrap();
        
        // Task should complete cleanly
        let result = timeout(Duration::from_secs(1), handle).await;
        assert!(result.is_ok(), "Monitoring task should shutdown cleanly");
    }
    
    #[tokio::test]
    async fn test_monitoring_task_sends_opportunity_when_threshold_exceeded() {
        // Create mock adapters with orderbooks that create a spread > threshold
        // Vest: ask=100.5, bid=100.0
        // Paradex: ask=100.0, bid=99.5
        // Spread A>B: (100.5 - 99.5) / 100 * 100 = 1.0% (> 0.30% threshold)
        let vest = Arc::new(Mutex::new(MockMonitoringAdapter::with_orderbook("vest", 100.5, 100.0)));
        let paradex = Arc::new(Mutex::new(MockMonitoringAdapter::with_orderbook("paradex", 100.0, 99.5)));
        let (opportunity_tx, mut opportunity_rx) = mpsc::channel(10);
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
        
        let config = MonitoringConfig {
            pair: "BTC-PERP".to_string(),
            spread_entry: 0.30,  // 0.30% threshold
        };
        
        // Spawn monitoring task
        tokio::spawn(monitoring_task(
            vest,
            paradex,
            opportunity_tx,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
            config,
            shutdown_rx,
        ));
        
        // Wait for opportunity to be received
        let result = timeout(Duration::from_millis(500), opportunity_rx.recv()).await;
        
        // Shutdown monitoring task
        let _ = shutdown_tx.send(());
        
        // Verify opportunity was received
        assert!(result.is_ok(), "Should receive opportunity within timeout");
        let opportunity = result.unwrap().expect("Should receive SpreadOpportunity");
        assert_eq!(opportunity.pair, "BTC-PERP");
        assert!(opportunity.spread_percent >= 0.30, "Spread should exceed threshold");
    }
    
    #[tokio::test]
    async fn test_monitoring_task_no_opportunity_below_threshold() {
        // Create orderbooks with spread below threshold
        // Vest: ask=100.1, bid=100.0
        // Paradex: ask=100.0, bid=99.9
        // Spread: very small, below 0.30%
        let vest = Arc::new(Mutex::new(MockMonitoringAdapter::with_orderbook("vest", 100.1, 100.0)));
        let paradex = Arc::new(Mutex::new(MockMonitoringAdapter::with_orderbook("paradex", 100.0, 99.95)));
        let (opportunity_tx, mut opportunity_rx) = mpsc::channel(10);
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
        
        let config = MonitoringConfig {
            pair: "BTC-PERP".to_string(),
            spread_entry: 0.30,  // 0.30% threshold
        };
        
        // Spawn monitoring task
        tokio::spawn(monitoring_task(
            vest,
            paradex,
            opportunity_tx,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
            config,
            shutdown_rx,
        ));
        
        // Wait briefly - no opportunity should arrive
        let result = timeout(Duration::from_millis(300), opportunity_rx.recv()).await;
        
        // Shutdown monitoring task
        let _ = shutdown_tx.send(());
        
        // Should timeout because no opportunity exceeds threshold
        assert!(result.is_err(), "Should NOT receive opportunity when spread below threshold");
    }
}
