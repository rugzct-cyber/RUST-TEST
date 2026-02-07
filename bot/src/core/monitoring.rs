//! Orderbook monitoring task for automatic spread detection (Story 6.2, 7.3, 5.3)
//!
//! This module provides the polling-based monitoring task that continuously
//! reads orderbooks from shared storage (lock-free), calculates spreads, and emits
//! SpreadOpportunity messages when thresholds are exceeded.
//!
//! # Architecture (V1 HFT optimized - Story 7.3)
//! - Polls orderbooks every 25ms using `tokio::time::interval`
//! - Reads directly from SharedOrderbooks (RwLock) - NO Mutex locks!
//! - Calculates spreads using `SpreadCalculator`
//! - Emits `SpreadOpportunity` via mpsc channel to execution_task
//! - Shutdown-aware via broadcast receiver
//!
//! # Logging (Story 5.3)
//! - Uses structured SPREAD_DETECTED events for opportunity logging
//! - Events include entry_spread, spread_threshold, direction, pair

use tokio::sync::{broadcast, mpsc};
use tokio::time::{interval, Duration};
use tracing::{debug, warn, error};

use crate::core::channels::SpreadOpportunity;
use crate::core::events::{TradingEvent, SystemEvent, log_event, log_system_event, current_timestamp_ms, format_pct};
use crate::core::spread::SpreadCalculator;

use crate::core::channels::SharedOrderbooks;

/// Polling interval for orderbook monitoring (25ms for V1 HFT)
pub const POLL_INTERVAL_MS: u64 = 25;

/// Log throttle — imported from channels (single source of truth)
use crate::core::channels::LOG_THROTTLE_POLLS;

/// Warning throttle interval - warn every N polls (~10 seconds at 25ms polling)
const WARN_THROTTLE_POLLS: u32 = 400;

/// Monitoring configuration for the task
#[derive(Clone, Debug)]
pub struct MonitoringConfig {
    /// Trading pair string (e.g., "BTC-PERP")
    pub pair: String,
    /// Entry threshold in percentage (e.g., 0.30 = 0.30%)
    pub spread_entry: f64,
    /// Exit threshold in percentage (e.g., 0.05 = 0.05%)
    pub spread_exit: f64,
}

/// Lock-free monitoring task that reads shared orderbooks directly
///
/// # Story 7.3 Optimization: Lock-Free Design
/// - NO Mutex locks - reads directly from Arc<RwLock<...>>
/// - Polls every 25ms (4x faster than original 100ms)
/// - WebSocket handlers write to SharedOrderbooks asynchronously
///
/// # Arguments
/// * `vest_orderbooks` - Vest shared orderbooks (Arc<RwLock<...>>)
/// * `paradex_orderbooks` - Paradex shared orderbooks (Arc<RwLock<...>>)
/// * `opportunity_tx` - Channel to send spread opportunities to execution_task
/// * `vest_symbol` - Symbol for Vest orderbook (e.g., "BTC-PERP")
/// * `paradex_symbol` - Symbol for Paradex orderbook (e.g., "BTC-USD-PERP")
/// * `config` - Monitoring configuration with spread thresholds
/// * `shutdown_rx` - Broadcast receiver for shutdown signal
pub async fn monitoring_task(
    vest_orderbooks: SharedOrderbooks,
    paradex_orderbooks: SharedOrderbooks,
    opportunity_tx: mpsc::Sender<SpreadOpportunity>,
    vest_symbol: String,
    paradex_symbol: String,
    config: MonitoringConfig,
    mut shutdown_rx: broadcast::Receiver<()>,
) {
    log_system_event(&SystemEvent::task_started("monitoring"));
    
    let calculator = SpreadCalculator::new("vest", "paradex");
    let mut poll_interval = interval(Duration::from_millis(POLL_INTERVAL_MS));
    let mut poll_count: u64 = 0;
    let mut warn_counter: u32 = 0;
    
    loop {
        tokio::select! {
            // Shutdown takes priority
            _ = shutdown_rx.recv() => {
                log_system_event(&SystemEvent::task_shutdown("monitoring", "shutdown_signal"));
                break;
            }
            // Poll orderbooks on interval
            _ = poll_interval.tick() => {
                poll_count += 1;
                
                // Read directly from shared storage - NO Mutex contention!
                let vest_ob = {
                    let books = vest_orderbooks.read().await;
                    books.get(&vest_symbol).cloned()
                };
                
                let paradex_ob = {
                    let books = paradex_orderbooks.read().await;
                    books.get(&paradex_symbol).cloned()
                };
                
                // Only calculate spread if both orderbooks are available
                if let (Some(vest_orderbook), Some(paradex_orderbook)) = (vest_ob, paradex_ob) {
                    if let Some(spread_result) = calculator.calculate(&vest_orderbook, &paradex_orderbook) {
                        // Log spread status periodically (every ~1 second)
                        // Uses debug level for non-opportunity monitoring
                        if poll_count % LOG_THROTTLE_POLLS == 0 {
                            debug!(
                                event_type = "SPREAD_MONITORING",
                                entry_spread = %format_pct(spread_result.spread_pct),
                                spread_threshold = %format_pct(config.spread_entry),
                                direction = ?spread_result.direction,
                                "Monitoring spread"
                            );
                        }
                        
                        // Only send opportunity if threshold exceeded
                        if spread_result.spread_pct >= config.spread_entry {
                            let now_ms = current_timestamp_ms();
                            
                            // Log SPREAD_DETECTED event (structured, throttled to reduce noise)
                            // Only log once per ~2 seconds to avoid flooding
                            if poll_count % LOG_THROTTLE_POLLS == 0 {
                                let direction_str = format!("{:?}", spread_result.direction);
                                let event = TradingEvent::spread_detected(
                                    &config.pair,
                                    spread_result.spread_pct,
                                    config.spread_entry,
                                    &direction_str,
                                );
                                log_event(&event);
                            }
                            
                            let opportunity = SpreadOpportunity {
                                pair: config.pair.clone(),
                                dex_a: "vest".to_string(),
                                dex_b: "paradex".to_string(),
                                spread_percent: spread_result.spread_pct,
                                direction: spread_result.direction,
                                detected_at_ms: now_ms,
                                // Include orderbook prices for order placement
                                dex_a_ask: vest_orderbook.best_ask().unwrap_or(0.0),
                                dex_a_bid: vest_orderbook.best_bid().unwrap_or(0.0),
                                dex_b_ask: paradex_orderbook.best_ask().unwrap_or(0.0),
                                dex_b_bid: paradex_orderbook.best_bid().unwrap_or(0.0),
                            };
                            
                            // Non-blocking send to avoid blocking monitoring loop
                            match opportunity_tx.try_send(opportunity) {
                                Ok(_) => {
                                    debug!("Spread opportunity sent to execution task");
                                }
                                Err(mpsc::error::TrySendError::Full(_)) => {
                                    // Silent drop: channel full means position is open or executor is busy
                                    // This is expected behavior, no need to log
                                }
                                Err(mpsc::error::TrySendError::Closed(_)) => {
                                    error!("Opportunity channel closed, shutting down monitoring");
                                    break;
                                }
                            }
                        }
                    }
                } else {
                    // Log when orderbooks are missing (helps debug connection issues)
                    warn_counter += 1;
                    // Only warn every 400 iterations (~10 seconds at 25ms) to avoid spam
                    if warn_counter % WARN_THROTTLE_POLLS == 0 {
                        let vest_has_ob = vest_orderbooks.read().await.contains_key(&vest_symbol);
                        let paradex_has_ob = paradex_orderbooks.read().await.contains_key(&paradex_symbol);
                        warn!(
                            vest_ob = vest_has_ob,
                            paradex_ob = paradex_has_ob,
                            "Waiting for orderbooks"
                        );
                    }
                }
            }
        }
    }
    
    log_system_event(&SystemEvent::task_stopped("monitoring"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::collections::HashMap;
    use tokio::sync::RwLock;
    use crate::adapters::types::{Orderbook, OrderbookLevel};
    use tokio::time::timeout;
    
    #[tokio::test]
    async fn test_monitoring_task_shutdown() {
        let vest_orderbooks = Arc::new(RwLock::new(HashMap::new()));
        let paradex_orderbooks = Arc::new(RwLock::new(HashMap::new()));
        let (opportunity_tx, _opportunity_rx) = mpsc::channel(10);
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
        
        let config = MonitoringConfig {
            pair: "BTC-PERP".to_string(),
            spread_entry: 0.30,
            spread_exit: 0.05,
        };
        
        // Spawn monitoring task
        let handle = tokio::spawn(monitoring_task(
            vest_orderbooks,
            paradex_orderbooks,
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
        // Create orderbooks with spread > threshold
        // Vest: ask=99.0, bid=98.5
        // Paradex: bid=100.5 (> ask_A so spread = (100.5-99)/99*100 = 1.515%)
        let mut vest_books = HashMap::new();
        let mut vest_ob = Orderbook::new();
        vest_ob.asks.push(OrderbookLevel::new(99.0, 1.0));
        vest_ob.bids.push(OrderbookLevel::new(98.5, 1.0));
        vest_books.insert("BTC-PERP".to_string(), vest_ob);
        let vest_orderbooks = Arc::new(RwLock::new(vest_books));
        
        let mut paradex_books = HashMap::new();
        let mut paradex_ob = Orderbook::new();
        paradex_ob.asks.push(OrderbookLevel::new(100.0, 1.0));
        paradex_ob.bids.push(OrderbookLevel::new(100.5, 1.0));
        paradex_books.insert("BTC-USD-PERP".to_string(), paradex_ob);
        let paradex_orderbooks = Arc::new(RwLock::new(paradex_books));
        
        let (opportunity_tx, mut opportunity_rx) = mpsc::channel(10);
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
        
        let config = MonitoringConfig {
            pair: "BTC-PERP".to_string(),
            spread_entry: 0.30,  // 0.30% threshold
            spread_exit: 0.05,
        };
        
        // Spawn monitoring task
        tokio::spawn(monitoring_task(
            vest_orderbooks,
            paradex_orderbooks,
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
        let mut vest_books = HashMap::new();
        let mut vest_ob = Orderbook::new();
        vest_ob.asks.push(OrderbookLevel::new(100.1, 1.0));
        vest_ob.bids.push(OrderbookLevel::new(100.0, 1.0));
        vest_books.insert("BTC-PERP".to_string(), vest_ob);
        let vest_orderbooks = Arc::new(RwLock::new(vest_books));
        
        let mut paradex_books = HashMap::new();
        let mut paradex_ob = Orderbook::new();
        paradex_ob.asks.push(OrderbookLevel::new(100.0, 1.0));
        paradex_ob.bids.push(OrderbookLevel::new(99.95, 1.0));
        paradex_books.insert("BTC-USD-PERP".to_string(), paradex_ob);
        let paradex_orderbooks = Arc::new(RwLock::new(paradex_books));
        
        let (opportunity_tx, mut opportunity_rx) = mpsc::channel(10);
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
        
        let config = MonitoringConfig {
            pair: "BTC-PERP".to_string(),
            spread_entry: 0.30,  // 0.30% threshold
            spread_exit: 0.05,
        };
        
        // Spawn monitoring task
        tokio::spawn(monitoring_task(
            vest_orderbooks,
            paradex_orderbooks,
            opportunity_tx,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
            config,
            shutdown_rx,
        ));
        
        // Wait briefly - no opportunity should arrive
        let result = timeout(Duration::from_millis(200), opportunity_rx.recv()).await;
        
        // Shutdown monitoring task
        let _ = shutdown_tx.send(());
        
        // Should timeout because no opportunity exceeds threshold
        assert!(result.is_err(), "Should NOT receive opportunity when spread below threshold");
    }
}
