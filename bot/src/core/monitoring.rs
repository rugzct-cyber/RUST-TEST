//! Orderbook monitoring task for automatic spread detection
//!
//! This module provides the polling-based monitoring task that continuously
//! reads orderbooks from shared storage (lock-free), calculates spreads, and emits
//! SpreadOpportunity messages when thresholds are exceeded.
//!
//! # Architecture (V1 HFT optimized)
//! - Polls orderbooks every 25ms using `tokio::time::interval`
//! - Reads directly from SharedOrderbooks (RwLock) - NO Mutex locks!
//! - Calculates spreads using `SpreadCalculator`
//! - Emits `SpreadOpportunity` via watch channel to execution_task (always freshest)
//! - Shutdown-aware via broadcast receiver
//!
//! # Logging
//! - Uses structured SPREAD_DETECTED events for opportunity logging
//! - Events include entry_spread, spread_threshold, direction, pair

use std::sync::Arc;
use tokio::sync::{broadcast, watch};
use tokio::time::Duration;
use tracing::{debug, warn};

use crate::core::channels::SpreadOpportunity;
use crate::core::events::{
    current_timestamp_ms, format_pct, log_event, log_system_event, SystemEvent, TradingEvent,
};
use crate::core::spread::SpreadCalculator;

use crate::core::channels::{SharedOrderbooks, OrderbookNotify};
use crate::core::channels::SharedBestPrices;

/// Timeout for waiting on orderbook notifications before checking diagnostics (Axe 5)
/// If no orderbook update arrives within this window, log a "waiting" warning.
const NOTIFY_TIMEOUT_MS: u64 = 1000;

/// Log throttle — imported from channels (single source of truth)
use crate::core::channels::LOG_THROTTLE_POLLS;

/// Warning throttle interval - warn every N polls (~10 seconds at 25ms polling)
const WARN_THROTTLE_POLLS: u32 = 400;

/// Monitoring configuration for the task
#[derive(Clone, Debug)]
pub struct MonitoringConfig {
    /// Trading pair string (e.g., "BTC-PERP")
    pub pair: Arc<str>,
    /// Entry threshold in percentage (e.g., 0.30 = 0.30%)
    pub spread_entry: f64,
    /// Exit threshold in percentage (e.g., 0.05 = 0.05%)
    pub spread_exit: f64,
}

/// Lock-free monitoring task using AtomicBestPrices
///
/// # Hot-Path Design (Axe 1 optimization)
/// - Reads 4 `f64` prices from `AtomicU64` — NO lock, NO clone, NO allocation
/// - Calculates spread via `calculate_from_prices()` directly
/// - `SharedOrderbooks` kept only for the rare missing-data warning path
///
/// # Arguments
/// * `vest_best_prices` - Atomic best bid/ask from Vest WebSocket reader
/// * `paradex_best_prices` - Atomic best bid/ask from Paradex WebSocket reader
/// * `vest_orderbooks` - Vest SharedOrderbooks (only for connection-check warnings)
/// * `paradex_orderbooks` - Paradex SharedOrderbooks (only for connection-check warnings)
/// * `opportunity_tx` - Watch channel sender for spread opportunities (always keeps freshest)
/// * `vest_symbol` - Symbol for Vest orderbook (e.g., "BTC-PERP")
/// * `paradex_symbol` - Symbol for Paradex orderbook (e.g., "BTC-USD-PERP")
/// * `config` - Monitoring configuration with spread thresholds
/// * `shutdown_rx` - Broadcast receiver for shutdown signal
pub async fn monitoring_task(
    vest_best_prices: SharedBestPrices,
    paradex_best_prices: SharedBestPrices,
    vest_orderbooks: SharedOrderbooks,
    paradex_orderbooks: SharedOrderbooks,
    opportunity_tx: watch::Sender<Option<SpreadOpportunity>>,
    vest_symbol: String,
    paradex_symbol: String,
    config: MonitoringConfig,
    orderbook_notify: OrderbookNotify,
    mut shutdown_rx: broadcast::Receiver<()>,
) {
    log_system_event(&SystemEvent::task_started("monitoring"));

    let calculator = SpreadCalculator::new("vest", "paradex");
    let mut poll_count: u64 = 0;
    let mut warn_counter: u32 = 0;

    loop {
        tokio::select! {
            // Shutdown takes priority
            _ = shutdown_rx.recv() => {
                log_system_event(&SystemEvent::task_shutdown("monitoring", "shutdown_signal"));
                break;
            }
            // Wait for orderbook update notification (event-driven, Axe 5)
            result = tokio::time::timeout(
                Duration::from_millis(NOTIFY_TIMEOUT_MS),
                orderbook_notify.notified()
            ) => {
                poll_count += 1;
                let _timed_out = result.is_err();

                // HOT PATH: Read atomic best prices — zero lock, zero allocation
                let (vest_bid, vest_ask) = vest_best_prices.load();
                let (paradex_bid, paradex_ask) = paradex_best_prices.load();

                // Only calculate spread if both have valid prices
                if let Some(spread_result) = calculator.calculate_from_prices(
                    vest_bid, vest_ask, paradex_bid, paradex_ask
                ) {
                    // Log spread status periodically (every ~1 second)
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

                        let opportunity = SpreadOpportunity {
                            pair: config.pair.clone(),
                            dex_a: "vest",
                            dex_b: "paradex",
                            spread_percent: spread_result.spread_pct,
                            direction: spread_result.direction,
                            detected_at_ms: now_ms,
                            // Prices from atomics (already loaded above)
                            dex_a_ask: vest_ask,
                            dex_a_bid: vest_bid,
                            dex_b_ask: paradex_ask,
                            dex_b_bid: paradex_bid,
                        };

                        // Watch channel: always replaces with freshest opportunity
                        if opportunity_tx.is_closed() {
                            warn!(
                                event_type = "CHANNEL_CLOSED",
                                "Opportunity channel closed - executor may have crashed"
                            );
                            break;
                        }
                        opportunity_tx.send_replace(Some(opportunity));
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
                        debug!("Spread opportunity sent to execution task (watch)");
                    }
                } else {
                    // Atomic prices are 0.0 (no data yet) — use SharedOrderbooks for diagnostics
                    warn_counter += 1;
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
    use crate::adapters::types::{Orderbook, OrderbookLevel};
    use crate::core::channels::AtomicBestPrices;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use tokio::time::timeout;

    /// Helper: create AtomicBestPrices pre-loaded with the given bid/ask
    fn make_best_prices(bid: f64, ask: f64) -> SharedBestPrices {
        let bp = Arc::new(AtomicBestPrices::new());
        bp.store(bid, ask);
        bp
    }

    /// Helper: create an OrderbookNotify and spawn a background signaler
    /// that keeps poking the monitoring loop so tests don't wait 1s timeout.
    fn make_test_notify() -> OrderbookNotify {
        let n = Arc::new(tokio::sync::Notify::new());
        let handle = n.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(5)).await;
                handle.notify_waiters();
            }
        });
        n
    }

    #[tokio::test]
    async fn test_monitoring_task_shutdown() {
        let vest_orderbooks = Arc::new(RwLock::new(HashMap::new()));
        let paradex_orderbooks = Arc::new(RwLock::new(HashMap::new()));
        let vest_bp = Arc::new(AtomicBestPrices::new()); // uninitialized (0.0)
        let paradex_bp = Arc::new(AtomicBestPrices::new());
        let (opportunity_tx, _opportunity_rx) = watch::channel(None);
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

        let config = MonitoringConfig {
            pair: Arc::from("BTC-PERP"),
            spread_entry: 0.30,
            spread_exit: 0.05,
        };

        let handle = tokio::spawn(monitoring_task(
            vest_bp,
            paradex_bp,
            vest_orderbooks,
            paradex_orderbooks,
            opportunity_tx,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
            config,
            make_test_notify(),
            shutdown_rx,
        ));

        tokio::time::sleep(Duration::from_millis(50)).await;
        shutdown_tx.send(()).unwrap();

        let result = timeout(Duration::from_secs(1), handle).await;
        assert!(result.is_ok(), "Monitoring task should shutdown cleanly");
    }

    #[tokio::test]
    async fn test_monitoring_task_sends_opportunity_when_threshold_exceeded() {
        // Vest: ask=99.0, bid=98.5; Paradex: bid=100.5, ask=100.0
        // spread = (100.5-99)/99*100 = 1.515%
        let mut vest_books = HashMap::new();
        let mut vest_ob = Orderbook::new();
        vest_ob.asks.push(OrderbookLevel::new(99.0, 1.0));
        vest_ob.bids.push(OrderbookLevel::new(98.5, 1.0));
        vest_books.insert("BTC-PERP".to_string(), vest_ob);
        let vest_orderbooks = Arc::new(RwLock::new(vest_books));
        let vest_bp = make_best_prices(98.5, 99.0);

        let mut paradex_books = HashMap::new();
        let mut paradex_ob = Orderbook::new();
        paradex_ob.asks.push(OrderbookLevel::new(100.0, 1.0));
        paradex_ob.bids.push(OrderbookLevel::new(100.5, 1.0));
        paradex_books.insert("BTC-USD-PERP".to_string(), paradex_ob);
        let paradex_orderbooks = Arc::new(RwLock::new(paradex_books));
        let paradex_bp = make_best_prices(100.5, 100.0);

        let (opportunity_tx, mut opportunity_rx) = watch::channel(None);
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

        let config = MonitoringConfig {
            pair: Arc::from("BTC-PERP"),
            spread_entry: 0.30,
            spread_exit: 0.05,
        };

        tokio::spawn(monitoring_task(
            vest_bp,
            paradex_bp,
            vest_orderbooks,
            paradex_orderbooks,
            opportunity_tx,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
            config,
            make_test_notify(),
            shutdown_rx,
        ));

        let result = timeout(Duration::from_millis(500), opportunity_rx.changed()).await;
        let _ = shutdown_tx.send(());

        assert!(result.is_ok(), "Should receive opportunity within timeout");
        let opportunity = opportunity_rx.borrow_and_update().clone().expect("Should receive SpreadOpportunity");
        assert_eq!(&*opportunity.pair, "BTC-PERP");
        assert!(
            opportunity.spread_percent >= 0.30,
            "Spread should exceed threshold"
        );
    }

    #[tokio::test]
    async fn test_monitoring_task_no_opportunity_below_threshold() {
        let mut vest_books = HashMap::new();
        let mut vest_ob = Orderbook::new();
        vest_ob.asks.push(OrderbookLevel::new(100.1, 1.0));
        vest_ob.bids.push(OrderbookLevel::new(100.0, 1.0));
        vest_books.insert("BTC-PERP".to_string(), vest_ob);
        let vest_orderbooks = Arc::new(RwLock::new(vest_books));
        let vest_bp = make_best_prices(100.0, 100.1);

        let mut paradex_books = HashMap::new();
        let mut paradex_ob = Orderbook::new();
        paradex_ob.asks.push(OrderbookLevel::new(100.0, 1.0));
        paradex_ob.bids.push(OrderbookLevel::new(99.95, 1.0));
        paradex_books.insert("BTC-USD-PERP".to_string(), paradex_ob);
        let paradex_orderbooks = Arc::new(RwLock::new(paradex_books));
        let paradex_bp = make_best_prices(99.95, 100.0);

        let (opportunity_tx, mut opportunity_rx) = watch::channel(None);
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

        let config = MonitoringConfig {
            pair: Arc::from("BTC-PERP"),
            spread_entry: 0.30,
            spread_exit: 0.05,
        };

        tokio::spawn(monitoring_task(
            vest_bp,
            paradex_bp,
            vest_orderbooks,
            paradex_orderbooks,
            opportunity_tx,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
            config,
            make_test_notify(),
            shutdown_rx,
        ));

        let result = timeout(Duration::from_millis(200), opportunity_rx.changed()).await;
        let _ = shutdown_tx.send(());

        assert!(
            result.is_err(),
            "Should NOT receive opportunity when spread below threshold"
        );
    }

    #[tokio::test]
    async fn test_monitoring_task_threshold_boundary_just_above() {
        // bid_b=100.31, ask_a=100.0 => spread = 0.31% > 0.30%
        let mut vest_books = HashMap::new();
        let mut vest_ob = Orderbook::new();
        vest_ob.asks.push(OrderbookLevel::new(100.0, 1.0));
        vest_ob.bids.push(OrderbookLevel::new(99.5, 1.0));
        vest_books.insert("BTC-PERP".to_string(), vest_ob);
        let vest_orderbooks = Arc::new(RwLock::new(vest_books));
        let vest_bp = make_best_prices(99.5, 100.0);

        let mut paradex_books = HashMap::new();
        let mut paradex_ob = Orderbook::new();
        paradex_ob.asks.push(OrderbookLevel::new(101.0, 1.0));
        paradex_ob.bids.push(OrderbookLevel::new(100.31, 1.0));
        paradex_books.insert("BTC-USD-PERP".to_string(), paradex_ob);
        let paradex_orderbooks = Arc::new(RwLock::new(paradex_books));
        let paradex_bp = make_best_prices(100.31, 101.0);

        let (opportunity_tx, mut opportunity_rx) = watch::channel(None);
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

        let config = MonitoringConfig {
            pair: Arc::from("BTC-PERP"),
            spread_entry: 0.30,
            spread_exit: 0.05,
        };

        tokio::spawn(monitoring_task(
            vest_bp,
            paradex_bp,
            vest_orderbooks,
            paradex_orderbooks,
            opportunity_tx,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
            config,
            make_test_notify(),
            shutdown_rx,
        ));

        let result = timeout(Duration::from_millis(500), opportunity_rx.changed()).await;
        let _ = shutdown_tx.send(());

        assert!(result.is_ok(), "Should trigger at 0.31% (just above 0.30%)");
        let opp = opportunity_rx.borrow_and_update().clone().unwrap();
        assert!(opp.spread_percent >= 0.30, "Spread {} should be >= 0.30", opp.spread_percent);
    }

    #[tokio::test]
    async fn test_monitoring_task_threshold_just_below() {
        // bid_b=100.29, ask_a=100.0 => spread = 0.29% < 0.30%
        let mut vest_books = HashMap::new();
        let mut vest_ob = Orderbook::new();
        vest_ob.asks.push(OrderbookLevel::new(100.0, 1.0));
        vest_ob.bids.push(OrderbookLevel::new(99.5, 1.0));
        vest_books.insert("BTC-PERP".to_string(), vest_ob);
        let vest_orderbooks = Arc::new(RwLock::new(vest_books));
        let vest_bp = make_best_prices(99.5, 100.0);

        let mut paradex_books = HashMap::new();
        let mut paradex_ob = Orderbook::new();
        paradex_ob.asks.push(OrderbookLevel::new(101.0, 1.0));
        paradex_ob.bids.push(OrderbookLevel::new(100.29, 1.0));
        paradex_books.insert("BTC-USD-PERP".to_string(), paradex_ob);
        let paradex_orderbooks = Arc::new(RwLock::new(paradex_books));
        let paradex_bp = make_best_prices(100.29, 101.0);

        let (opportunity_tx, mut opportunity_rx) = watch::channel(None);
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

        let config = MonitoringConfig {
            pair: Arc::from("BTC-PERP"),
            spread_entry: 0.30,
            spread_exit: 0.05,
        };

        tokio::spawn(monitoring_task(
            vest_bp,
            paradex_bp,
            vest_orderbooks,
            paradex_orderbooks,
            opportunity_tx,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
            config,
            make_test_notify(),
            shutdown_rx,
        ));

        let result = timeout(Duration::from_millis(200), opportunity_rx.changed()).await;
        let _ = shutdown_tx.send(());

        assert!(result.is_err(), "Should NOT trigger at 0.29% (below 0.30%)");
    }

    #[tokio::test]
    async fn test_monitoring_task_missing_one_orderbook() {
        // Vest has prices, Paradex has none (atomics at 0.0)
        let mut vest_books = HashMap::new();
        let mut vest_ob = Orderbook::new();
        vest_ob.asks.push(OrderbookLevel::new(100.0, 1.0));
        vest_ob.bids.push(OrderbookLevel::new(99.0, 1.0));
        vest_books.insert("BTC-PERP".to_string(), vest_ob);
        let vest_orderbooks = Arc::new(RwLock::new(vest_books));
        let vest_bp = make_best_prices(99.0, 100.0);

        let paradex_orderbooks = Arc::new(RwLock::new(HashMap::new()));
        let paradex_bp = Arc::new(AtomicBestPrices::new()); // 0.0 = no data

        let (opportunity_tx, mut opportunity_rx) = watch::channel(None);
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

        let config = MonitoringConfig {
            pair: Arc::from("BTC-PERP"),
            spread_entry: 0.30,
            spread_exit: 0.05,
        };

        tokio::spawn(monitoring_task(
            vest_bp,
            paradex_bp,
            vest_orderbooks,
            paradex_orderbooks,
            opportunity_tx,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
            config,
            make_test_notify(),
            shutdown_rx,
        ));

        let result = timeout(Duration::from_millis(200), opportunity_rx.changed()).await;
        let _ = shutdown_tx.send(());

        assert!(result.is_err(), "Should timeout (no opportunity with missing orderbook)");
    }

    #[tokio::test]
    async fn test_monitoring_task_channel_full_no_panic() {
        let mut vest_books = HashMap::new();
        let mut vest_ob = Orderbook::new();
        vest_ob.asks.push(OrderbookLevel::new(99.0, 1.0));
        vest_ob.bids.push(OrderbookLevel::new(98.5, 1.0));
        vest_books.insert("BTC-PERP".to_string(), vest_ob);
        let vest_orderbooks = Arc::new(RwLock::new(vest_books));
        let vest_bp = make_best_prices(98.5, 99.0);

        let mut paradex_books = HashMap::new();
        let mut paradex_ob = Orderbook::new();
        paradex_ob.asks.push(OrderbookLevel::new(100.0, 1.0));
        paradex_ob.bids.push(OrderbookLevel::new(100.5, 1.0));
        paradex_books.insert("BTC-USD-PERP".to_string(), paradex_ob);
        let paradex_orderbooks = Arc::new(RwLock::new(paradex_books));
        let paradex_bp = make_best_prices(100.5, 100.0);

        let (opportunity_tx, _opportunity_rx) = watch::channel(None);
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

        let config = MonitoringConfig {
            pair: Arc::from("BTC-PERP"),
            spread_entry: 0.30,
            spread_exit: 0.05,
        };

        let handle = tokio::spawn(monitoring_task(
            vest_bp,
            paradex_bp,
            vest_orderbooks,
            paradex_orderbooks,
            opportunity_tx,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
            config,
            make_test_notify(),
            shutdown_rx,
        ));

        tokio::time::sleep(Duration::from_millis(200)).await;
        let _ = shutdown_tx.send(());

        let result = timeout(Duration::from_secs(1), handle).await;
        assert!(result.is_ok(), "Task should not panic (watch channel never overflows)");
    }

    #[tokio::test]
    async fn test_monitoring_task_channel_closed_breaks_loop() {
        let mut vest_books = HashMap::new();
        let mut vest_ob = Orderbook::new();
        vest_ob.asks.push(OrderbookLevel::new(99.0, 1.0));
        vest_ob.bids.push(OrderbookLevel::new(98.5, 1.0));
        vest_books.insert("BTC-PERP".to_string(), vest_ob);
        let vest_orderbooks = Arc::new(RwLock::new(vest_books));
        let vest_bp = make_best_prices(98.5, 99.0);

        let mut paradex_books = HashMap::new();
        let mut paradex_ob = Orderbook::new();
        paradex_ob.asks.push(OrderbookLevel::new(100.0, 1.0));
        paradex_ob.bids.push(OrderbookLevel::new(100.5, 1.0));
        paradex_books.insert("BTC-USD-PERP".to_string(), paradex_ob);
        let paradex_orderbooks = Arc::new(RwLock::new(paradex_books));
        let paradex_bp = make_best_prices(100.5, 100.0);

        let (opportunity_tx, opportunity_rx) = watch::channel(None);
        let (_shutdown_tx, shutdown_rx) = broadcast::channel(1);

        let config = MonitoringConfig {
            pair: Arc::from("BTC-PERP"),
            spread_entry: 0.30,
            spread_exit: 0.05,
        };

        drop(opportunity_rx);

        let handle = tokio::spawn(monitoring_task(
            vest_bp,
            paradex_bp,
            vest_orderbooks,
            paradex_orderbooks,
            opportunity_tx,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
            config,
            make_test_notify(),
            shutdown_rx,
        ));

        let result = timeout(Duration::from_secs(2), handle).await;
        assert!(result.is_ok(), "Task should exit when channel is closed");
    }

    #[tokio::test]
    async fn test_monitoring_task_opportunity_includes_prices() {
        let mut vest_books = HashMap::new();
        let mut vest_ob = Orderbook::new();
        vest_ob.asks.push(OrderbookLevel::new(42000.0, 1.0));
        vest_ob.bids.push(OrderbookLevel::new(41990.0, 1.0));
        vest_books.insert("BTC-PERP".to_string(), vest_ob);
        let vest_orderbooks = Arc::new(RwLock::new(vest_books));
        let vest_bp = make_best_prices(41990.0, 42000.0);

        let mut paradex_books = HashMap::new();
        let mut paradex_ob = Orderbook::new();
        paradex_ob.asks.push(OrderbookLevel::new(42010.0, 1.0));
        paradex_ob.bids.push(OrderbookLevel::new(42200.0, 1.0));
        paradex_books.insert("BTC-USD-PERP".to_string(), paradex_ob);
        let paradex_orderbooks = Arc::new(RwLock::new(paradex_books));
        let paradex_bp = make_best_prices(42200.0, 42010.0);

        let (opportunity_tx, mut opportunity_rx) = watch::channel(None);
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

        let config = MonitoringConfig {
            pair: Arc::from("BTC-PERP"),
            spread_entry: 0.10,
            spread_exit: 0.05,
        };

        tokio::spawn(monitoring_task(
            vest_bp,
            paradex_bp,
            vest_orderbooks,
            paradex_orderbooks,
            opportunity_tx,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
            config,
            make_test_notify(),
            shutdown_rx,
        ));

        let result = timeout(Duration::from_millis(500), opportunity_rx.changed()).await;
        let _ = shutdown_tx.send(());

        assert!(result.is_ok(), "Should receive opportunity");
        let opp = opportunity_rx.borrow_and_update().clone().unwrap();
        assert_eq!(opp.dex_a_ask, 42000.0, "dex_a_ask should be vest best ask");
        assert_eq!(opp.dex_a_bid, 41990.0, "dex_a_bid should be vest best bid");
        assert_eq!(opp.dex_b_ask, 42010.0, "dex_b_ask should be paradex best ask");
        assert_eq!(opp.dex_b_bid, 42200.0, "dex_b_bid should be paradex best bid");
    }
}
