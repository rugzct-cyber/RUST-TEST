//! Runtime execution tasks (Story 2.3, Story 6.2, Story 7.3, Story 5.3)
//!
//! This module provides the async task loops for the execution pipeline.
//! The execution task consumes SpreadOpportunity messages and triggers
//! delta-neutral trades.
//!
//! V1 HFT Mode: No persistence (Supabase removed for latency optimization)
//!
//! # Logging (Story 5.3)
//! - Uses structured trading events (TRADE_ENTRY, TRADE_EXIT, POSITION_MONITORING)
//! - Distinct entry_spread vs exit_spread fields for slippage analysis

use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use tokio::time::{interval, Duration};
use tracing::{debug, error, info};

use crate::adapters::ExchangeAdapter;
use crate::core::channels::SpreadOpportunity;
use crate::core::events::{TradingEvent, SystemEvent, log_event, log_system_event, calculate_latency_ms, format_pct};
use crate::core::execution::DeltaNeutralExecutor;
use crate::core::spread::{SpreadCalculator, SpreadDirection};

// TUI State type for optional TUI updates
use crate::tui::app::AppState as TuiState;
use std::sync::Mutex as StdMutex;

use crate::core::channels::SharedOrderbooks;

// =============================================================================
// Constants
// =============================================================================

/// Exit monitoring polling interval in milliseconds (25ms for V1 HFT)
const EXIT_POLL_INTERVAL_MS: u64 = 25;

/// Delay after trade entry to let exchange APIs settle before verifying positions (milliseconds)
const API_SETTLE_DELAY_MS: u64 = 500;

/// Log throttle — imported from channels (single source of truth)
use super::channels::LOG_THROTTLE_POLLS;

// =============================================================================
// Helper Functions
// =============================================================================

/// Drain all pending messages from a channel and log if any were drained
fn drain_channel<T>(rx: &mut mpsc::Receiver<T>, context: &str) {
    let mut drained = 0;
    while rx.try_recv().is_ok() {
        drained += 1;
    }
    if drained > 0 {
        debug!("Drained {} stale messages from {}", drained, context);
    }
}

/// Exit monitoring result with exit fill prices for PnL calculation
struct ExitResult {
    exit_spread: f64,
    vest_exit_price: f64,
    paradex_exit_price: f64,
}

/// Exit monitoring loop - polls orderbooks until exit condition or shutdown
/// 
/// Returns: (poll_count, Option<ExitResult>) - None for shutdown, Some for normal exit
#[allow(clippy::too_many_arguments)]
async fn exit_monitoring_loop<V, P>(
    executor: &DeltaNeutralExecutor<V, P>,
    vest_orderbooks: SharedOrderbooks,
    paradex_orderbooks: SharedOrderbooks,
    vest_symbol: &str,
    paradex_symbol: &str,
    pair: &str,
    entry_spread: f64,
    exit_spread_target: f64,
    direction: SpreadDirection,
    shutdown_rx: &mut broadcast::Receiver<()>,
) -> (u64, Option<ExitResult>)
where
    V: ExchangeAdapter + Send + Sync,
    P: ExchangeAdapter + Send + Sync,
{
    let mut exit_interval = interval(Duration::from_millis(EXIT_POLL_INTERVAL_MS));
    let mut poll_count: u64 = 0;
    
    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                log_system_event(&SystemEvent::task_shutdown("exit_monitoring", "shutdown_signal"));
                return (poll_count, None);
            }
            _ = exit_interval.tick() => {
                poll_count += 1;
                
                // Read orderbooks
                let vest_ob = vest_orderbooks.read().await.get(vest_symbol).cloned();
                let paradex_ob = paradex_orderbooks.read().await.get(paradex_symbol).cloned();
                
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
                    
                    // Log POSITION_MONITORING event (throttled - every ~1 second)
                    if poll_count % LOG_THROTTLE_POLLS == 0 {
                        let event = TradingEvent::position_monitoring(
                            pair,
                            entry_spread,    // entry_spread (original)
                            exit_spread,     // exit_spread (current)
                            exit_spread_target,
                            poll_count,
                        );
                        log_event(&event);
                    }
                    
                    // DEBUG: Log near-exit conditions at INFO level
                    if exit_spread >= exit_spread_target - 0.02 {
                        info!(
                            event_type = "EXIT_CHECK",
                            exit_spread = %format!("{:.4}", exit_spread),
                            target = %format!("{:.4}", exit_spread_target),
                            condition = %format!("{} >= {} = {}", exit_spread, exit_spread_target, exit_spread >= exit_spread_target),
                            "Near exit threshold"
                        );
                    }
                    
                    if exit_spread >= exit_spread_target {
                        // Calculate profit
                        let profit = entry_spread + exit_spread;
                        
                        // Log TRADE_EXIT event (Story 5.3)
                        let event = TradingEvent::trade_exit(
                            pair,
                            entry_spread,    // entry_spread
                            exit_spread,     // exit_spread
                            exit_spread_target,
                            profit,
                            poll_count,
                        );
                        log_event(&event);
                        
                        // Close the position
                        match executor.close_position(exit_spread, vest_bid, vest_ask).await {
                            Ok(close_result) => {
                                // Log POSITION_CLOSED event
                                let closed_event = TradingEvent::position_closed(
                                    pair,
                                    entry_spread,
                                    exit_spread,
                                    profit,
                                );
                                log_event(&closed_event);
                                
                                // Return exit fill prices for real-price PnL calculation
                                return (poll_count, Some(ExitResult {
                                    exit_spread,
                                    vest_exit_price: close_result.long_fill_price,
                                    paradex_exit_price: close_result.short_fill_price,
                                }));
                            }
                            Err(e) => {
                                error!(
                                    event_type = "ORDER_FAILED",
                                    error = ?e,
                                    "Failed to close position"
                                );
                            }
                        }
                        
                        return (poll_count, Some(ExitResult {
                            exit_spread,
                            vest_exit_price: 0.0,
                            paradex_exit_price: 0.0,
                        }));
                    }
                } else {
                    debug!(
                        event_type = "POSITION_MONITORING",
                        poll = poll_count,
                        "Missing orderbook data"
                    );
                }
            }
        }
    }
}

// =============================================================================
// Functions
// =============================================================================

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
#[allow(clippy::too_many_arguments)]
pub async fn execution_task<V, P>(
    mut opportunity_rx: mpsc::Receiver<SpreadOpportunity>,
    executor: DeltaNeutralExecutor<V, P>,
    vest_orderbooks: SharedOrderbooks,
    paradex_orderbooks: SharedOrderbooks,
    vest_symbol: String,
    paradex_symbol: String,
    mut shutdown_rx: broadcast::Receiver<()>,
    exit_spread_target: f64,
    tui_state: Option<Arc<StdMutex<TuiState>>>,
) where
    V: ExchangeAdapter + Send + Sync,
    P: ExchangeAdapter + Send + Sync,
{
    log_system_event(&SystemEvent::task_started("execution"));
    
    // Track execution statistics
    let mut execution_count: u64 = 0;

    loop {
        tokio::select! {
            // Shutdown takes priority
            _ = shutdown_rx.recv() => {
                log_system_event(&SystemEvent::task_shutdown("execution", "shutdown_signal"));
                break;
            }
            // Process incoming opportunities (AtomicBool guard prevents duplicates)
            Some(opportunity) = opportunity_rx.recv() => {
                let spread_pct = opportunity.spread_percent;
                let pair = opportunity.pair.clone();
                
                execution_count += 1;
                debug!(
                    pair = %pair,
                    spread = %format_pct(spread_pct),
                    direction = ?opportunity.direction,
                    execution_number = execution_count,
                    "Processing spread opportunity #{}", execution_count
                );

                // Ensure adapters are ready (refresh JWT if expired)
                if let Err(e) = executor.ensure_ready().await {
                    error!(event_type = "TRADE_FAILED", error = ?e, "Failed to prepare adapters - skipping opportunity");
                    continue;
                }

                match executor.execute_delta_neutral(opportunity.clone()).await {
                    Ok(result) => {
                        if result.success {
                            // Calculate latency from detection to execution
                            let latency = calculate_latency_ms(opportunity.detected_at_ms);
                            let direction_str = format!("{:?}", opportunity.direction);
                            
                            // Log TRADE_ENTRY event (Story 5.3)
                            let event = TradingEvent::trade_entry(
                                &pair,
                                spread_pct,
                                exit_spread_target, // entry threshold
                                &direction_str,
                                &result.long_exchange,
                                &result.short_exchange,
                                latency,
                                result.long_fill_price,
                                result.short_fill_price,
                            );
                            log_event(&event);
                            
                            // Verify positions on both exchanges and get entry prices
                            // Add small delay to let Vest API update entry price
                            tokio::time::sleep(tokio::time::Duration::from_millis(API_SETTLE_DELAY_MS)).await;
                            let (vest_entry, paradex_entry) = executor.verify_positions(spread_pct, exit_spread_target).await;
                            
                            // Update TUI state with entry prices from position data
                            if let Some(ref tui) = tui_state {
                                if let Ok(mut state) = tui.try_lock() {
                                    // Calculate actual entry spread from real entry prices
                                    let actual_spread = match (vest_entry, paradex_entry) {
                                        (Some(v), Some(p)) if v > 0.0 && p > 0.0 => {
                                            ((v - p).abs() / v.max(p)) * 100.0
                                        }
                                        _ => spread_pct, // fallback to detected spread
                                    };
                                    
                                    state.record_entry(
                                        actual_spread,
                                        opportunity.direction,
                                        vest_entry.unwrap_or(0.0),
                                        paradex_entry.unwrap_or(0.0),
                                    );
                                }
                            }
                            
                            // Start exit monitoring via extracted function
                            let entry_direction = executor.get_entry_direction();
                            
                            if let Some(direction) = entry_direction {
                                debug!(
                                    event_type = "POSITION_OPENED",
                                    direction = ?direction,
                                    exit_target = %format_pct(exit_spread_target),
                                    "Starting exit monitoring"
                                );
                                
                                let (poll_count, maybe_exit_result) = exit_monitoring_loop(
                                    &executor,
                                    vest_orderbooks.clone(),
                                    paradex_orderbooks.clone(),
                                    &vest_symbol,
                                    &paradex_symbol,
                                    &pair,
                                    spread_pct,
                                    exit_spread_target,
                                    direction,
                                    &mut shutdown_rx,
                                ).await;
                                
                                log_system_event(&SystemEvent::task_stopped("exit_monitoring"));
                                
                                // Update TUI trade history (AC1: record_exit after close_position)
                                if let (Some(exit_result), Some(ref tui)) = (maybe_exit_result, &tui_state) {
                                    if let Ok(mut state) = tui.try_lock() {
                                        let position_size = executor.get_default_quantity();
                                        
                                        // PnL from real prices: profit on each leg
                                        // Vest leg PnL: depends on whether vest was long or short
                                        // Paradex leg PnL: depends on whether paradex was long or short
                                        let pnl_usd = match direction {
                                            SpreadDirection::AOverB => {
                                                // Vest=Long, Paradex=Short
                                                // Long PnL = (exit - entry) * qty
                                                // Short PnL = (entry - exit) * qty
                                                let vest_entry = state.entry_vest_price.unwrap_or(0.0);
                                                let paradex_entry = state.entry_paradex_price.unwrap_or(0.0);
                                                let long_pnl = (exit_result.vest_exit_price - vest_entry) * position_size;
                                                let short_pnl = (paradex_entry - exit_result.paradex_exit_price) * position_size;
                                                long_pnl + short_pnl
                                            }
                                            SpreadDirection::BOverA => {
                                                // Vest=Short, Paradex=Long
                                                let vest_entry = state.entry_vest_price.unwrap_or(0.0);
                                                let paradex_entry = state.entry_paradex_price.unwrap_or(0.0);
                                                let short_pnl = (vest_entry - exit_result.vest_exit_price) * position_size;
                                                let long_pnl = (exit_result.paradex_exit_price - paradex_entry) * position_size;
                                                long_pnl + short_pnl
                                            }
                                        };
                                        
                                        let latency_ms = poll_count * EXIT_POLL_INTERVAL_MS;
                                        state.record_exit(exit_result.exit_spread, pnl_usd, latency_ms);
                                    }
                                }
                            } else {
                                error!(event_type = "ORDER_FAILED", "No entry direction found after successful trade");
                            }
                        } else {
                            error!(
                                event_type = "ORDER_FAILED",
                                latency_ms = result.execution_latency_ms,
                                long_success = %result.long_order.is_success(),
                                short_success = %result.short_order.is_success(),
                                "Delta-neutral trade partially failed"
                            );
                        }
                        
                        // Drain any stale opportunities that accumulated during execution
                        drain_channel(&mut opportunity_rx, "opportunity queue");
                    }
                    Err(e) => {
                        // Check if it's a "position already open" error
                        let err_msg = format!("{:?}", e);
                        if err_msg.contains("Position already open") {
                            debug!(event_type = "TRADE_SKIPPED", "Position already open - draining queue");
                            drain_channel(&mut opportunity_rx, "opportunity queue");
                        } else {
                            error!(event_type = "ORDER_FAILED", error = ?e, "Delta-neutral execution error");
                        }
                    }
                }
            }
        }
    }

    log_system_event(&SystemEvent::task_stopped("execution"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tokio::sync::RwLock;
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
                None,   // No TUI state in tests
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

        // Give time for entry (including 500ms API delay) + exit monitoring to process
        tokio::time::sleep(Duration::from_millis(800)).await;

        // Shutdown
        let _ = shutdown_tx.send(());

        // Wait for task to complete (longer timeout for exit processing)
        let result = timeout(Duration::from_secs(5), handle).await;
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
                None,   // No TUI state in tests
            ).await;
        });

        // Send shutdown immediately
        let _ = shutdown_tx.send(());

        // Task should terminate quickly
        let result = timeout(Duration::from_secs(1), handle).await;
        assert!(result.is_ok(), "Task should shutdown gracefully");
    }

    #[tokio::test]
    async fn test_exit_monitoring_loop_exits_on_spread_condition() {
        // Create mock executor
        let vest = RuntimeMockAdapter::new("vest");
        let paradex = RuntimeMockAdapter::new("paradex");
        let executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
        );
        
        // Create SharedOrderbooks with prices that produce exit_spread >= target
        // For AOverB: exit_spread = (vest_bid - paradex_ask) / paradex_ask * 100
        // vest_bid = 42000, paradex_ask = 42000 => spread = 0%
        // Target = -0.05%, so 0% >= -0.05% triggers exit
        let mut vest_books = HashMap::new();
        vest_books.insert("BTC-PERP".to_string(), Orderbook {
            bids: vec![crate::adapters::types::OrderbookLevel::new(42000.0, 1.0)],
            asks: vec![crate::adapters::types::OrderbookLevel::new(42001.0, 1.0)],
            timestamp: 1706000000000,
        });
        let mut paradex_books = HashMap::new();
        paradex_books.insert("BTC-USD-PERP".to_string(), Orderbook {
            bids: vec![crate::adapters::types::OrderbookLevel::new(42000.0, 1.0)],
            asks: vec![crate::adapters::types::OrderbookLevel::new(42000.0, 1.0)], // Same as vest_bid
            timestamp: 1706000000000,
        });
        let vest_orderbooks: SharedOrderbooks = Arc::new(RwLock::new(vest_books));
        let paradex_orderbooks: SharedOrderbooks = Arc::new(RwLock::new(paradex_books));
        
        let (_shutdown_tx, mut shutdown_rx) = broadcast::channel(1);
        
        // Call the extracted function directly
        let result = timeout(Duration::from_secs(2), exit_monitoring_loop(
            &executor,
            vest_orderbooks,
            paradex_orderbooks,
            "BTC-PERP",
            "BTC-USD-PERP",
            "BTC-PERP",
            0.35,       // entry_spread
            -0.05,      // exit_spread_target
            SpreadDirection::AOverB,
            &mut shutdown_rx,
        )).await;
        
        // Should complete (not timeout) and return poll_count >= 1 with Some(exit_spread)
        assert!(result.is_ok(), "Exit monitoring should complete on exit condition");
        let (poll_count, exit_spread) = result.unwrap();
        assert!(poll_count >= 1, "Should have polled at least once");
        assert!(exit_spread.is_some(), "Should return Some(exit_spread) on normal exit");
    }

    #[tokio::test]
    async fn test_exit_monitoring_loop_responds_to_shutdown() {
        // Create mock executor
        let vest = RuntimeMockAdapter::new("vest");
        let paradex = RuntimeMockAdapter::new("paradex");
        let executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
        );
        
        // Create orderbooks with prices that will NEVER trigger exit
        // vest_bid = 40000, paradex_ask = 42000 => spread = -4.76% (never >= -0.05%)
        let mut vest_books = HashMap::new();
        vest_books.insert("BTC-PERP".to_string(), Orderbook {
            bids: vec![crate::adapters::types::OrderbookLevel::new(40000.0, 1.0)],
            asks: vec![crate::adapters::types::OrderbookLevel::new(40001.0, 1.0)],
            timestamp: 1706000000000,
        });
        let mut paradex_books = HashMap::new();
        paradex_books.insert("BTC-USD-PERP".to_string(), Orderbook {
            bids: vec![crate::adapters::types::OrderbookLevel::new(42000.0, 1.0)],
            asks: vec![crate::adapters::types::OrderbookLevel::new(42000.0, 1.0)],
            timestamp: 1706000000000,
        });
        let vest_orderbooks: SharedOrderbooks = Arc::new(RwLock::new(vest_books));
        let paradex_orderbooks: SharedOrderbooks = Arc::new(RwLock::new(paradex_books));
        
        let (shutdown_tx, mut shutdown_rx) = broadcast::channel(1);
        
        // Spawn the monitoring loop in background
        let handle = tokio::spawn(async move {
            exit_monitoring_loop(
                &executor,
                vest_orderbooks,
                paradex_orderbooks,
                "BTC-PERP",
                "BTC-USD-PERP",
                "BTC-PERP",
                0.35,
                -0.05,
                SpreadDirection::AOverB,
                &mut shutdown_rx,
            ).await
        });
        
        // Give it a moment to start polling
        tokio::time::sleep(Duration::from_millis(50)).await;
        
        // Send shutdown
        let _ = shutdown_tx.send(());
        
        // Should complete quickly
        let result = timeout(Duration::from_secs(1), handle).await;
        assert!(result.is_ok(), "Exit monitoring should respond to shutdown");
        let (_, exit_spread) = result.unwrap().unwrap();
        assert!(exit_spread.is_none(), "Should return None exit_spread on shutdown (AC3)");
    }

    #[tokio::test]
    async fn test_exit_monitoring_loop_b_over_a_direction() {
        // Create mock executor
        let vest = RuntimeMockAdapter::new("vest");
        let paradex = RuntimeMockAdapter::new("paradex");
        let executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
        );
        
        // For BOverA: exit_spread = (paradex_bid - vest_ask) / vest_ask * 100
        // paradex_bid = 42000, vest_ask = 42000 => spread = 0%
        // Target = -0.05%, so 0% >= -0.05% triggers exit
        let mut vest_books = HashMap::new();
        vest_books.insert("BTC-PERP".to_string(), Orderbook {
            bids: vec![crate::adapters::types::OrderbookLevel::new(42000.0, 1.0)],
            asks: vec![crate::adapters::types::OrderbookLevel::new(42000.0, 1.0)],
            timestamp: 1706000000000,
        });
        let mut paradex_books = HashMap::new();
        paradex_books.insert("BTC-USD-PERP".to_string(), Orderbook {
            bids: vec![crate::adapters::types::OrderbookLevel::new(42000.0, 1.0)],
            asks: vec![crate::adapters::types::OrderbookLevel::new(42001.0, 1.0)],
            timestamp: 1706000000000,
        });
        let vest_orderbooks: SharedOrderbooks = Arc::new(RwLock::new(vest_books));
        let paradex_orderbooks: SharedOrderbooks = Arc::new(RwLock::new(paradex_books));
        
        let (_shutdown_tx, mut shutdown_rx) = broadcast::channel(1);
        
        // Call with BOverA direction
        let result = timeout(Duration::from_secs(2), exit_monitoring_loop(
            &executor,
            vest_orderbooks,
            paradex_orderbooks,
            "BTC-PERP",
            "BTC-USD-PERP",
            "BTC-PERP",
            0.35,
            -0.05,
            SpreadDirection::BOverA,  // <-- Testing BOverA
            &mut shutdown_rx,
        )).await;
        
        assert!(result.is_ok(), "Exit monitoring should complete on exit condition");
        let (poll_count, exit_spread) = result.unwrap();
        assert!(poll_count >= 1, "Should have polled at least once");
        assert!(exit_spread.is_some(), "Should return Some(exit_spread) on normal exit");
    }
}
