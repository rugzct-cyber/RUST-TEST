//! Runtime execution tasks (Story 2.3, Story 6.2, Story 7.3)
//!
//! This module provides the async task loops for the execution pipeline.
//! The execution task consumes SpreadOpportunity messages and triggers
//! delta-neutral trades.
//!
//! V1 HFT Mode: No persistence (Supabase removed for latency optimization)

use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::{broadcast, mpsc, RwLock};
use tokio::time::{interval, Duration};
use tracing::{debug, error, info};

use crate::adapters::{ExchangeAdapter, Orderbook};
use crate::core::channels::SpreadOpportunity;
use crate::core::execution::DeltaNeutralExecutor;
use crate::core::spread::{SpreadCalculator, SpreadDirection};

/// Type alias for shared orderbooks
pub type SharedOrderbooks = Arc<RwLock<HashMap<String, Orderbook>>>;

/// Execution task that processes spread opportunities (Story 6.2, Story 7.3)
///
/// Listens for `SpreadOpportunity` messages on the channel and executes
/// delta-neutral trades. After entry, polls orderbooks for exit condition.
/// V1 HFT mode - no persistence for maximum speed.
///
/// # Arguments
/// * `opportunity_rx` - Receiver for spread opportunities
/// * `executor` - The DeltaNeutralExecutor for trade execution
/// * `vest_orderbooks` - Shared orderbooks for Vest (for exit monitoring)
/// * `paradex_orderbooks` - Shared orderbooks for Paradex (for exit monitoring)
/// * `vest_symbol` - Symbol on Vest exchange
/// * `paradex_symbol` - Symbol on Paradex exchange
/// * `shutdown_rx` - Broadcast receiver for shutdown signal
/// * `exit_spread_target` - Target spread for position exit (from config, e.g. -0.10)
pub async fn execution_task<V, P>(
    mut opportunity_rx: mpsc::Receiver<SpreadOpportunity>,
    executor: DeltaNeutralExecutor<V, P>,
    vest_orderbooks: SharedOrderbooks,
    paradex_orderbooks: SharedOrderbooks,
    vest_symbol: String,
    paradex_symbol: String,
    mut shutdown_rx: broadcast::Receiver<()>,
    exit_spread_target: f64,
) where
    V: ExchangeAdapter + Send + Sync,
    P: ExchangeAdapter + Send + Sync,
{
    info!("Execution task started (V1 HFT Mode with exit monitoring)");
    
    // Track execution statistics
    let mut execution_count: u64 = 0;

    loop {
        tokio::select! {
            // Shutdown takes priority
            _ = shutdown_rx.recv() => {
                info!(total_executions = execution_count, "Execution task shutting down");
                break;
            }
            // Process incoming opportunities (AtomicBool guard prevents duplicates)
            Some(opportunity) = opportunity_rx.recv() => {
                let spread_pct = opportunity.spread_percent;
                let pair = opportunity.pair.clone();
                
                execution_count += 1;
                info!(
                    pair = %pair,
                    spread = %format!("{:.4}%", spread_pct),
                    direction = ?opportunity.direction,
                    execution_number = execution_count,
                    "Processing spread opportunity #{}", execution_count
                );

                // Ensure adapters are ready (refresh JWT if expired)
                if let Err(e) = executor.ensure_ready().await {
                    error!(error = ?e, "[TRADE] Failed to prepare adapters - skipping opportunity");
                    continue;
                }

                match executor.execute_delta_neutral(opportunity).await {
                    Ok(result) => {
                        if result.success {
                            // Log successful trade
                            info!(
                                spread = %format!("{:.4}%", spread_pct),
                                latency_ms = result.execution_latency_ms,
                                long = %result.long_exchange,
                                short = %result.short_exchange,
                                execution_number = execution_count,
                                "[TRADE] Auto-executed"
                            );
                            
                            info!(
                                pair = %pair,
                                entry_spread = %format!("{:.4}%", spread_pct),
                                "[TRADE] Position opened - starting exit monitoring"
                            );
                            
                            // Verify positions on both exchanges
                            executor.verify_positions(spread_pct, exit_spread_target).await;
                            
                            // ============================================================
                            // EXIT MONITORING LOOP (Option A: polling in execution_task)
                            // ============================================================
                            let entry_direction = executor.get_entry_direction();
                            
                            if let Some(direction) = entry_direction {
                                info!(
                                    direction = ?direction,
                                    exit_target = %format!("{:.4}%", exit_spread_target),
                                    "[EXIT-MONITOR] Started"
                                );
                                
                                let mut exit_interval = interval(Duration::from_millis(25));
                                let mut poll_count: u64 = 0;
                                
                                'exit_loop: loop {
                                    tokio::select! {
                                        // Check shutdown
                                        _ = shutdown_rx.recv() => {
                                            info!("[EXIT-MONITOR] Shutdown received - exiting without closing");
                                            break 'exit_loop;
                                        }
                                        _ = exit_interval.tick() => {
                                            poll_count += 1;
                                            
                                            // Read orderbooks
                                            let vest_ob = vest_orderbooks.read().await.get(&vest_symbol).cloned();
                                            let paradex_ob = paradex_orderbooks.read().await.get(&paradex_symbol).cloned();
                                            
                                            if let (Some(v_ob), Some(p_ob)) = (vest_ob, paradex_ob) {
                                                // Get best prices
                                                let vest_bid = v_ob.best_bid().unwrap_or(0.0);
                                                let vest_ask = v_ob.best_ask().unwrap_or(0.0);
                                                let paradex_bid = p_ob.best_bid().unwrap_or(0.0);
                                                let paradex_ask = p_ob.best_ask().unwrap_or(0.0);
                                                
                                                // Calculate exit spread based on entry direction
                                                let exit_spread = match direction {
                                                    SpreadDirection::AOverB => {
                                                        // Entry: Long Vest, Short Paradex
                                                        // Exit: Sell Vest (bid), Buy Paradex (ask)
                                                        SpreadCalculator::calculate_exit_spread(vest_bid, paradex_ask)
                                                    }
                                                    SpreadDirection::BOverA => {
                                                        // Entry: Long Paradex, Short Vest
                                                        // Exit: Sell Paradex (bid), Buy Vest (ask)
                                                        SpreadCalculator::calculate_exit_spread(paradex_bid, vest_ask)
                                                    }
                                                };
                                                
                                                // DEBUG log every poll
                                                debug!(
                                                    poll = poll_count,
                                                    exit_spread = %format!("{:.4}%", exit_spread),
                                                    target = %format!("{:.4}%", exit_spread_target),
                                                    vest_bid = vest_bid,
                                                    paradex_ask = paradex_ask,
                                                    "[EXIT-MONITOR] Polling"
                                                );
                                                
                                                // Check exit condition: exit_spread >= exit_spread_target
                                                // e.g. exit_spread = -0.08% >= -0.10% (target) => true
                                                if exit_spread >= exit_spread_target {
                                                    info!(
                                                        exit_spread = %format!("{:.4}%", exit_spread),
                                                        target = %format!("{:.4}%", exit_spread_target),
                                                        polls = poll_count,
                                                        "[EXIT] Condition met - closing position"
                                                    );
                                                    
                                                    // Close the position
                                                    match executor.close_position(exit_spread).await {
                                                        Ok(_) => {
                                                            let profit = spread_pct + exit_spread;
                                                            info!(
                                                                entry = %format!("{:.4}%", spread_pct),
                                                                exit = %format!("{:.4}%", exit_spread),
                                                                profit = %format!("{:.4}%", profit),
                                                                "[EXIT] Position closed successfully"
                                                            );
                                                        }
                                                        Err(e) => {
                                                            error!(error = ?e, "[EXIT] Failed to close position");
                                                        }
                                                    }
                                                    
                                                    break 'exit_loop;
                                                }
                                            } else {
                                                debug!(poll = poll_count, "[EXIT-MONITOR] Missing orderbook data");
                                            }
                                        }
                                    }
                                }
                                
                                info!(total_polls = poll_count, "[EXIT-MONITOR] Stopped");
                            } else {
                                error!("[EXIT-MONITOR] No entry direction found after successful trade!");
                            }
                        } else {
                            error!(
                                latency_ms = result.execution_latency_ms,
                                long_success = %result.long_order.is_success(),
                                short_success = %result.short_order.is_success(),
                                "[TRADE] Delta-neutral trade partially failed"
                            );
                        }
                        
                        // Drain any stale opportunities that accumulated during execution
                        let mut drained = 0;
                        while opportunity_rx.try_recv().is_ok() {
                            drained += 1;
                        }
                        if drained > 0 {
                            debug!("Drained {} stale opportunities after execution", drained);
                        }
                    }
                    Err(e) => {
                        // Check if it's a "position already open" error
                        let err_msg = format!("{:?}", e);
                        if err_msg.contains("Position already open") {
                            debug!("[TRADE] Position already open - draining queue");
                            while opportunity_rx.try_recv().is_ok() {}
                        } else {
                            error!(error = ?e, "[TRADE] Delta-neutral execution error");
                        }
                    }
                }
            }
        }
    }

    info!("Execution task stopped");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::ExchangeResult;
    use crate::adapters::types::{OrderRequest, OrderResponse, OrderStatus, Orderbook, PositionInfo};
    use crate::core::spread::SpreadDirection;
    use async_trait::async_trait;
    use tokio::time::{timeout, Duration};

    /// Simple mock adapter for runtime tests
    struct RuntimeMockAdapter {
        name: &'static str,
    }

    impl RuntimeMockAdapter {
        fn new(name: &'static str) -> Self {
            Self { name }
        }
    }

    #[async_trait]
    impl ExchangeAdapter for RuntimeMockAdapter {
        async fn connect(&mut self) -> ExchangeResult<()> { Ok(()) }
        async fn disconnect(&mut self) -> ExchangeResult<()> { Ok(()) }
        async fn subscribe_orderbook(&mut self, _symbol: &str) -> ExchangeResult<()> { Ok(()) }
        async fn unsubscribe_orderbook(&mut self, _symbol: &str) -> ExchangeResult<()> { Ok(()) }
        
        async fn place_order(&self, order: OrderRequest) -> ExchangeResult<OrderResponse> {
            Ok(OrderResponse {
                order_id: format!("{}-{}", self.name, order.client_order_id),
                client_order_id: order.client_order_id,
                status: OrderStatus::Filled,
                filled_quantity: order.quantity,
                avg_price: Some(42000.0),
            })
        }
        
        async fn cancel_order(&self, _order_id: &str) -> ExchangeResult<()> { Ok(()) }
        fn get_orderbook(&self, _symbol: &str) -> Option<&Orderbook> { None }
        fn is_connected(&self) -> bool { true }
        fn is_stale(&self) -> bool { false }
        async fn sync_orderbooks(&mut self) {}
        async fn reconnect(&mut self) -> ExchangeResult<()> { Ok(()) }
        async fn get_position(&self, _symbol: &str) -> ExchangeResult<Option<PositionInfo>> { Ok(None) }
        fn exchange_name(&self) -> &'static str { self.name }
    }

    #[tokio::test]
    async fn test_execution_task_processes_opportunity() {
        let (opportunity_tx, opportunity_rx) = mpsc::channel(10);
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

        let vest = RuntimeMockAdapter::new("vest");
        let paradex = RuntimeMockAdapter::new("paradex");
        let executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
        );

        // Create SharedOrderbooks with data that triggers exit (spread = 0 >= -0.05)
        let mut vest_books = HashMap::new();
        vest_books.insert("BTC-PERP".to_string(), Orderbook {
            bids: vec![crate::adapters::types::OrderbookLevel::new(42000.0, 1.0)],
            asks: vec![crate::adapters::types::OrderbookLevel::new(42001.0, 1.0)],
            timestamp: 1706000000000,
        });
        let mut paradex_books = HashMap::new();
        paradex_books.insert("BTC-USD-PERP".to_string(), Orderbook {
            bids: vec![crate::adapters::types::OrderbookLevel::new(42000.0, 1.0)],
            asks: vec![crate::adapters::types::OrderbookLevel::new(42001.0, 1.0)],  // Same as vest_bid => spread ~0%
            timestamp: 1706000000000,
        });
        let vest_orderbooks: SharedOrderbooks = Arc::new(RwLock::new(vest_books));
        let paradex_orderbooks: SharedOrderbooks = Arc::new(RwLock::new(paradex_books));

        // Spawn the execution task (V1: with exit monitoring)
        let handle = tokio::spawn(async move {
            execution_task(
                opportunity_rx,
                executor,
                vest_orderbooks,
                paradex_orderbooks,
                "BTC-PERP".to_string(),
                "BTC-USD-PERP".to_string(),
                shutdown_rx,
                -0.05,  // exit_spread_target: exit when spread >= -0.05%
            ).await;
        });

        // Send an opportunity
        let opportunity = SpreadOpportunity {
            pair: "BTC-PERP".to_string(),
            dex_a: "vest".to_string(),
            dex_b: "paradex".to_string(),
            spread_percent: 0.35,
            direction: SpreadDirection::AOverB,
            detected_at_ms: 1706000000000,
            dex_a_ask: 42000.0,
            dex_a_bid: 41990.0,
            dex_b_ask: 42005.0,
            dex_b_bid: 41985.0,
        };
        opportunity_tx.send(opportunity).await.unwrap();

        // Give time for entry + exit monitoring to process
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Shutdown
        let _ = shutdown_tx.send(());

        // Wait for task to complete (longer timeout for exit processing)
        let result = timeout(Duration::from_secs(2), handle).await;
        assert!(result.is_ok(), "Task should complete on shutdown");
    }

    #[tokio::test]
    async fn test_execution_task_shutdown() {
        let (_opportunity_tx, opportunity_rx) = mpsc::channel::<SpreadOpportunity>(10);
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

        let vest = RuntimeMockAdapter::new("vest");
        let paradex = RuntimeMockAdapter::new("paradex");
        let executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
        );

        // Create empty SharedOrderbooks for test
        let vest_orderbooks: SharedOrderbooks = Arc::new(RwLock::new(HashMap::new()));
        let paradex_orderbooks: SharedOrderbooks = Arc::new(RwLock::new(HashMap::new()));

        // V1: with exit monitoring
        let handle = tokio::spawn(async move {
            execution_task(
                opportunity_rx,
                executor,
                vest_orderbooks,
                paradex_orderbooks,
                "BTC-PERP".to_string(),
                "BTC-USD-PERP".to_string(),
                shutdown_rx,
                -0.05,  // exit_spread_target
            ).await;
        });

        // Send shutdown immediately
        let _ = shutdown_tx.send(());

        // Task should terminate quickly
        let result = timeout(Duration::from_secs(1), handle).await;
        assert!(result.is_ok(), "Task should shutdown gracefully");
    }
}
