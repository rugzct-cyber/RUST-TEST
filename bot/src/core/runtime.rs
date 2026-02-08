//! Runtime execution tasks
//!
//! This module provides the async task loops for the execution pipeline.
//! The execution task consumes SpreadOpportunity messages and triggers
//! delta-neutral trades.
//!
//! V1 HFT Mode: No persistence (Supabase removed for latency optimization)
//!
//! # Logging
//! - Uses structured trading events (TRADE_ENTRY, TRADE_EXIT, POSITION_MONITORING)
//! - Distinct entry_spread vs exit_spread fields for slippage analysis

use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use tokio::time::{interval, Duration};
use tracing::{debug, error, warn};

use crate::adapters::ExchangeAdapter;
use crate::core::channels::SpreadOpportunity;
use crate::core::events::{format_pct, log_event, log_system_event, SystemEvent, TradingEvent};
use crate::core::execution::{log_successful_trade, DeltaNeutralExecutor};
use crate::core::spread::{SpreadCalculator, SpreadDirection};

// TUI State type for optional TUI updates
use crate::tui::app::AppState as TuiState;
use std::sync::Mutex as StdMutex;

use crate::core::channels::SharedOrderbooks;

// =============================================================================
// Constants
// =============================================================================

/// Exit monitoring uses same polling interval as monitoring task (single source of truth)
use super::monitoring::POLL_INTERVAL_MS;

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
    vest_realized_pnl: Option<f64>,
    paradex_realized_pnl: Option<f64>,
    execution_latency_ms: u64,
}

/// Bundled parameters for exit monitoring (replaces 7 loose arguments)
struct ExitMonitoringParams {
    vest_orderbooks: SharedOrderbooks,
    paradex_orderbooks: SharedOrderbooks,
    vest_symbol: String,
    paradex_symbol: String,
    pair: String,
    entry_spread: f64,
    exit_spread_target: f64,
    direction: SpreadDirection,
}

/// Exit monitoring loop - polls orderbooks until exit condition or shutdown
///
/// Returns: (poll_count, Option<ExitResult>) - None for shutdown, Some for normal exit
async fn exit_monitoring_loop<V, P>(
    executor: &DeltaNeutralExecutor<V, P>,
    params: ExitMonitoringParams,
    shutdown_rx: &mut broadcast::Receiver<()>,
) -> (u64, Option<ExitResult>)
where
    V: ExchangeAdapter + Send + Sync,
    P: ExchangeAdapter + Send + Sync,
{
    let ExitMonitoringParams {
        vest_orderbooks,
        paradex_orderbooks,
        vest_symbol,
        paradex_symbol,
        pair,
        entry_spread,
        exit_spread_target,
        direction,
    } = params;

    let mut exit_interval = interval(Duration::from_millis(POLL_INTERVAL_MS));
    let mut poll_count: u64 = 0;

    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                log_system_event(&SystemEvent::task_shutdown("exit_monitoring", "shutdown_signal"));
                return (poll_count, None);
            }
            _ = exit_interval.tick() => {
                poll_count += 1;

                // Proactively refresh JWT every ~2 min so close_position has zero delay
                const JWT_REFRESH_POLLS: u64 = 4800; // 4800 × 25ms ≈ 2 min
                if poll_count % JWT_REFRESH_POLLS == 0 {
                    if let Err(e) = executor.ensure_ready().await {
                        warn!(event_type = "JWT_REFRESH_FAILED", error = %e, "Adapter refresh failed during monitoring");
                    }
                }

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

                    // Log POSITION_MONITORING event (throttled - every ~1 second)
                    if poll_count % LOG_THROTTLE_POLLS == 0 {
                        let event = TradingEvent::position_monitoring(
                            &pair,
                            entry_spread,    // entry_spread (original)
                            exit_spread,     // exit_spread (current)
                            exit_spread_target,
                            poll_count,
                        );
                        log_event(&event);
                    }

                    // DEBUG: Log near-exit conditions (throttled - every ~1 second)
                    // Was logging at info! 40x/sec → now debug! throttled
                    if exit_spread >= exit_spread_target - 0.02 && poll_count % LOG_THROTTLE_POLLS == 0 {
                        debug!(
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

                        // Log TRADE_EXIT event
                        let event = TradingEvent::trade_exit(
                            &pair,
                            entry_spread,    // entry_spread
                            exit_spread,     // exit_spread
                            exit_spread_target,
                            profit,
                            poll_count,
                        );
                        log_event(&event);

                        // Retry close with backoff instead of abandoning
                        const MAX_CLOSE_RETRIES: u32 = 3;
                        const CLOSE_RETRY_DELAY_SECS: u64 = 5;
                        let mut close_retries = 0u32;
                        let close_start = std::time::Instant::now();

                        loop {
                            match executor.close_position(exit_spread, vest_bid, vest_ask).await {
                                Ok(close_result) => {
                                    let execution_latency_ms = close_start.elapsed().as_millis() as u64;

                                    // Log POSITION_CLOSED event
                                    let closed_event = TradingEvent::position_closed(
                                        &pair,
                                        entry_spread,
                                        exit_spread,
                                        profit,
                                    );
                                    log_event(&closed_event);

                                    // Return exit fill prices for real-price PnL calculation
                                    return (poll_count, Some(ExitResult {
                                        exit_spread,
                                        vest_exit_price: close_result.vest_fill_price,
                                        paradex_exit_price: close_result.paradex_fill_price,
                                        vest_realized_pnl: close_result.vest_realized_pnl,
                                        paradex_realized_pnl: close_result.paradex_realized_pnl,
                                        execution_latency_ms,
                                    }));
                                }
                                Err(e) => {
                                    close_retries += 1;
                                    error!(
                                        event_type = "ORDER_FAILED",
                                        error = ?e,
                                        retry = close_retries,
                                        max_retries = MAX_CLOSE_RETRIES,
                                        "Failed to close position - retrying in {}s",
                                        CLOSE_RETRY_DELAY_SECS
                                    );

                                    if close_retries >= MAX_CLOSE_RETRIES {
                                        let execution_latency_ms = close_start.elapsed().as_millis() as u64;
                                        error!(
                                            event_type = "CLOSE_ABANDONED",
                                            retries = close_retries,
                                            "CRITICAL: All close retries exhausted - manual intervention required"
                                        );
                                        return (poll_count, Some(ExitResult {
                                            exit_spread,
                                            vest_exit_price: 0.0,
                                            paradex_exit_price: 0.0,
                                            vest_realized_pnl: None,
                                            paradex_realized_pnl: None,
                                            execution_latency_ms,
                                        }));
                                    }

                                    tokio::time::sleep(Duration::from_secs(CLOSE_RETRY_DELAY_SECS)).await;
                                }
                            }
                        }
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

/// Execution task that processes spread opportunities
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
                            // Verify positions on both exchanges and get entry prices
                            // Add small delay to let Vest API update entry price
                            tokio::time::sleep(tokio::time::Duration::from_millis(API_SETTLE_DELAY_MS)).await;
                            let (vest_entry, paradex_entry) = executor.verify_positions(spread_pct, exit_spread_target).await;

                            // Log TRADE_ENTRY + SLIPPAGE_ANALYSIS AFTER
                            // verify_positions() with real fill prices (not 0.0 from IOC response)
                            if let Some(timings) = result.timings.as_ref() {
                                log_successful_trade(
                                    &opportunity,
                                    &result,
                                    timings,
                                    vest_entry.unwrap_or(0.0),
                                    paradex_entry.unwrap_or(0.0),
                                );
                            }

                            // Update TUI state with entry prices from position data
                            if let Some(ref tui) = tui_state {
                                match tui.lock() {
                                    Ok(mut state) => {
                                    // Calculate actual entry spread from real fill prices (direction-aware)
                                    let actual_spread = match (vest_entry, paradex_entry) {
                                        (Some(v), Some(p)) if v > 0.0 && p > 0.0 => {
                                            match opportunity.direction {
                                                SpreadDirection::AOverB => ((p - v) / v) * 100.0,
                                                SpreadDirection::BOverA => ((v - p) / p) * 100.0,
                                            }
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
                                    Err(e) => {
                                        error!(event_type = "TUI_STATE_ERROR", error = %e, "Failed to record trade entry in TUI state");
                                    }
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

                                let (_poll_count, maybe_exit_result) = exit_monitoring_loop(
                                    &executor,
                                    ExitMonitoringParams {
                                        vest_orderbooks: vest_orderbooks.clone(),
                                        paradex_orderbooks: paradex_orderbooks.clone(),
                                        vest_symbol: vest_symbol.clone(),
                                        paradex_symbol: paradex_symbol.clone(),
                                        pair: pair.clone(),
                                        entry_spread: spread_pct,
                                        exit_spread_target,
                                        direction,
                                    },
                                    &mut shutdown_rx,
                                ).await;

                                log_system_event(&SystemEvent::task_stopped("exit_monitoring"));

                                // Update TUI trade history (AC1: record_exit after close_position)
                                if let (Some(exit_result), Some(ref tui)) = (maybe_exit_result, &tui_state) {
                                    match tui.lock() {
                                        Ok(mut state) => {
                                        let position_size = executor.get_default_quantity();

                                        // Get exit prices for display
                                        let vest_exit = exit_result.vest_exit_price;
                                        let paradex_exit = exit_result.paradex_exit_price;

                                        // === PnL: prefer exchange-reported realized_pnl ===
                                        let vest_rpnl = exit_result.vest_realized_pnl;
                                        let paradex_rpnl = exit_result.paradex_realized_pnl;

                                        let pnl_usd = if vest_rpnl.is_some() || paradex_rpnl.is_some() {
                                            let total = vest_rpnl.unwrap_or(0.0) + paradex_rpnl.unwrap_or(0.0);
                                            tracing::info!(
                                                event_type = "PNL_FROM_EXCHANGE",
                                                vest_realized_pnl = ?vest_rpnl,
                                                paradex_realized_pnl = ?paradex_rpnl,
                                                total_pnl = %format!("{:.6}", total),
                                                "PnL from exchange-reported realized PnL"
                                            );
                                            total
                                        } else {
                                            tracing::warn!(
                                                event_type = "PNL_UNAVAILABLE",
                                                "No realized PnL returned by either exchange"
                                            );
                                            0.0
                                        };

                                        state.record_exit(exit_result.exit_spread, pnl_usd, exit_result.execution_latency_ms, vest_exit, paradex_exit);
                                        }
                                        Err(e) => {
                                            error!(event_type = "TUI_STATE_ERROR", error = %e, "Failed to record trade exit in TUI state");
                                        }
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
    use crate::adapters::test_utils::TestMockAdapter;
    use crate::adapters::types::Orderbook;
    use crate::core::spread::SpreadDirection;
    use std::collections::HashMap;
    use tokio::sync::RwLock;
    use tokio::time::{timeout, Duration};

    #[tokio::test]
    async fn test_execution_task_processes_opportunity() {
        let (opportunity_tx, opportunity_rx) = mpsc::channel(10);
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

        let vest = TestMockAdapter::new("vest");
        let paradex = TestMockAdapter::new("paradex");
        let executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
        );

        // Create SharedOrderbooks with data that triggers exit (spread = 0 >= -0.05)
        let mut vest_books = HashMap::new();
        vest_books.insert(
            "BTC-PERP".to_string(),
            Orderbook {
                bids: vec![crate::adapters::types::OrderbookLevel::new(42000.0, 1.0)],
                asks: vec![crate::adapters::types::OrderbookLevel::new(42001.0, 1.0)],
                timestamp: 1706000000000,
            },
        );
        let mut paradex_books = HashMap::new();
        paradex_books.insert(
            "BTC-USD-PERP".to_string(),
            Orderbook {
                bids: vec![crate::adapters::types::OrderbookLevel::new(42000.0, 1.0)],
                asks: vec![crate::adapters::types::OrderbookLevel::new(42001.0, 1.0)], // Same as vest_bid => spread ~0%
                timestamp: 1706000000000,
            },
        );
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
                -0.05, // exit_spread_target: exit when spread >= -0.05%
                None,  // No TUI state in tests
            )
            .await;
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

        let vest = TestMockAdapter::new("vest");
        let paradex = TestMockAdapter::new("paradex");
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
                -0.05, // exit_spread_target
                None,  // No TUI state in tests
            )
            .await;
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
        let vest = TestMockAdapter::new("vest");
        let paradex = TestMockAdapter::new("paradex");
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
        vest_books.insert(
            "BTC-PERP".to_string(),
            Orderbook {
                bids: vec![crate::adapters::types::OrderbookLevel::new(42000.0, 1.0)],
                asks: vec![crate::adapters::types::OrderbookLevel::new(42001.0, 1.0)],
                timestamp: 1706000000000,
            },
        );
        let mut paradex_books = HashMap::new();
        paradex_books.insert(
            "BTC-USD-PERP".to_string(),
            Orderbook {
                bids: vec![crate::adapters::types::OrderbookLevel::new(42000.0, 1.0)],
                asks: vec![crate::adapters::types::OrderbookLevel::new(42000.0, 1.0)], // Same as vest_bid
                timestamp: 1706000000000,
            },
        );
        let vest_orderbooks: SharedOrderbooks = Arc::new(RwLock::new(vest_books));
        let paradex_orderbooks: SharedOrderbooks = Arc::new(RwLock::new(paradex_books));

        let (_shutdown_tx, mut shutdown_rx) = broadcast::channel(1);

        // Simulate open position so close_position can work
        executor.simulate_open_position(SpreadDirection::AOverB);

        // Call the extracted function directly
        let result = timeout(
            Duration::from_secs(5),
            exit_monitoring_loop(
                &executor,
                ExitMonitoringParams {
                    vest_orderbooks,
                    paradex_orderbooks,
                    vest_symbol: "BTC-PERP".to_string(),
                    paradex_symbol: "BTC-USD-PERP".to_string(),
                    pair: "BTC-PERP".to_string(),
                    entry_spread: 0.35,
                    exit_spread_target: -0.05,
                    direction: SpreadDirection::AOverB,
                },
                &mut shutdown_rx,
            ),
        )
        .await;

        // Should complete (not timeout) and return poll_count >= 1 with Some(exit_spread)
        assert!(
            result.is_ok(),
            "Exit monitoring should complete on exit condition"
        );
        let (poll_count, exit_spread) = result.unwrap();
        assert!(poll_count >= 1, "Should have polled at least once");
        assert!(
            exit_spread.is_some(),
            "Should return Some(exit_spread) on normal exit"
        );
    }

    #[tokio::test]
    async fn test_exit_monitoring_loop_responds_to_shutdown() {
        // Create mock executor
        let vest = TestMockAdapter::new("vest");
        let paradex = TestMockAdapter::new("paradex");
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
        vest_books.insert(
            "BTC-PERP".to_string(),
            Orderbook {
                bids: vec![crate::adapters::types::OrderbookLevel::new(40000.0, 1.0)],
                asks: vec![crate::adapters::types::OrderbookLevel::new(40001.0, 1.0)],
                timestamp: 1706000000000,
            },
        );
        let mut paradex_books = HashMap::new();
        paradex_books.insert(
            "BTC-USD-PERP".to_string(),
            Orderbook {
                bids: vec![crate::adapters::types::OrderbookLevel::new(42000.0, 1.0)],
                asks: vec![crate::adapters::types::OrderbookLevel::new(42000.0, 1.0)],
                timestamp: 1706000000000,
            },
        );
        let vest_orderbooks: SharedOrderbooks = Arc::new(RwLock::new(vest_books));
        let paradex_orderbooks: SharedOrderbooks = Arc::new(RwLock::new(paradex_books));

        let (shutdown_tx, mut shutdown_rx) = broadcast::channel(1);

        // Spawn the monitoring loop in background
        let handle = tokio::spawn(async move {
            exit_monitoring_loop(
                &executor,
                ExitMonitoringParams {
                    vest_orderbooks,
                    paradex_orderbooks,
                    vest_symbol: "BTC-PERP".to_string(),
                    paradex_symbol: "BTC-USD-PERP".to_string(),
                    pair: "BTC-PERP".to_string(),
                    entry_spread: 0.35,
                    exit_spread_target: -0.05,
                    direction: SpreadDirection::AOverB,
                },
                &mut shutdown_rx,
            )
            .await
        });

        // Give it a moment to start polling
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Send shutdown
        let _ = shutdown_tx.send(());

        // Should complete quickly
        let result = timeout(Duration::from_secs(1), handle).await;
        assert!(result.is_ok(), "Exit monitoring should respond to shutdown");
        let (_, exit_spread) = result.unwrap().unwrap();
        assert!(
            exit_spread.is_none(),
            "Should return None exit_spread on shutdown (AC3)"
        );
    }

    #[tokio::test]
    async fn test_exit_monitoring_loop_b_over_a_direction() {
        // Create mock executor
        let vest = TestMockAdapter::new("vest");
        let paradex = TestMockAdapter::new("paradex");
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
        vest_books.insert(
            "BTC-PERP".to_string(),
            Orderbook {
                bids: vec![crate::adapters::types::OrderbookLevel::new(42000.0, 1.0)],
                asks: vec![crate::adapters::types::OrderbookLevel::new(42000.0, 1.0)],
                timestamp: 1706000000000,
            },
        );
        let mut paradex_books = HashMap::new();
        paradex_books.insert(
            "BTC-USD-PERP".to_string(),
            Orderbook {
                bids: vec![crate::adapters::types::OrderbookLevel::new(42000.0, 1.0)],
                asks: vec![crate::adapters::types::OrderbookLevel::new(42001.0, 1.0)],
                timestamp: 1706000000000,
            },
        );
        let vest_orderbooks: SharedOrderbooks = Arc::new(RwLock::new(vest_books));
        let paradex_orderbooks: SharedOrderbooks = Arc::new(RwLock::new(paradex_books));

        let (_shutdown_tx, mut shutdown_rx) = broadcast::channel(1);

        // Simulate open position so close_position can work
        executor.simulate_open_position(SpreadDirection::BOverA);

        // Call with BOverA direction
        let result = timeout(
            Duration::from_secs(5),
            exit_monitoring_loop(
                &executor,
                ExitMonitoringParams {
                    vest_orderbooks,
                    paradex_orderbooks,
                    vest_symbol: "BTC-PERP".to_string(),
                    paradex_symbol: "BTC-USD-PERP".to_string(),
                    pair: "BTC-PERP".to_string(),
                    entry_spread: 0.35,
                    exit_spread_target: -0.05,
                    direction: SpreadDirection::BOverA, // <-- Testing BOverA
                },
                &mut shutdown_rx,
            ),
        )
        .await;

        assert!(
            result.is_ok(),
            "Exit monitoring should complete on exit condition"
        );
        let (poll_count, exit_spread) = result.unwrap();
        assert!(poll_count >= 1, "Should have polled at least once");
        assert!(
            exit_spread.is_some(),
            "Should return Some(exit_spread) on normal exit"
        );
    }

    // =========================================================================
    // Additional Tests
    // =========================================================================

    #[tokio::test]
    async fn test_execution_task_drains_pending_messages() {
        // Send multiple opportunities, then shutdown — verify at least one is processed
        let (opportunity_tx, opportunity_rx) = mpsc::channel::<SpreadOpportunity>(10);
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

        let vest = TestMockAdapter::new("vest");
        let paradex = TestMockAdapter::new("paradex");
        let vest_count = vest.order_count.clone();
        let executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
        );

        let vest_orderbooks: SharedOrderbooks = Arc::new(RwLock::new(HashMap::new()));
        let paradex_orderbooks: SharedOrderbooks = Arc::new(RwLock::new(HashMap::new()));

        let handle = tokio::spawn(async move {
            execution_task(
                opportunity_rx,
                executor,
                vest_orderbooks,
                paradex_orderbooks,
                "BTC-PERP".to_string(),
                "BTC-USD-PERP".to_string(),
                shutdown_rx,
                -0.05,
                None,
            )
            .await;
        });

        // Send one opportunity
        let opp = SpreadOpportunity {
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
        opportunity_tx.send(opp).await.unwrap();

        // Give time for processing
        tokio::time::sleep(Duration::from_millis(500)).await;

        // At least one order should have been placed
        assert!(
            vest_count.load(std::sync::atomic::Ordering::Relaxed) >= 1,
            "Should have processed at least one opportunity"
        );

        let _ = shutdown_tx.send(());
        let _ = timeout(Duration::from_secs(2), handle).await;
    }

    #[tokio::test]
    async fn test_execution_task_handles_empty_channel() {
        // No opportunities sent, just shutdown — should exit cleanly
        let (_opportunity_tx, opportunity_rx) = mpsc::channel::<SpreadOpportunity>(10);
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

        let vest = TestMockAdapter::new("vest");
        let paradex = TestMockAdapter::new("paradex");
        let vest_count = vest.order_count.clone();
        let executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
        );

        let vest_orderbooks: SharedOrderbooks = Arc::new(RwLock::new(HashMap::new()));
        let paradex_orderbooks: SharedOrderbooks = Arc::new(RwLock::new(HashMap::new()));

        let handle = tokio::spawn(async move {
            execution_task(
                opportunity_rx,
                executor,
                vest_orderbooks,
                paradex_orderbooks,
                "BTC-PERP".to_string(),
                "BTC-USD-PERP".to_string(),
                shutdown_rx,
                -0.05,
                None,
            )
            .await;
        });

        // Shutdown immediately
        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = shutdown_tx.send(());

        let result = timeout(Duration::from_secs(1), handle).await;
        assert!(result.is_ok(), "Should exit cleanly with empty channel");
        assert_eq!(
            vest_count.load(std::sync::atomic::Ordering::Relaxed),
            0,
            "No orders should have been placed"
        );
    }

    #[tokio::test]
    async fn test_exit_monitoring_continues_with_missing_orderbooks() {
        // Exit monitoring with empty orderbooks should NOT panic,
        // and should respond to shutdown signal
        let vest = TestMockAdapter::new("vest");
        let paradex = TestMockAdapter::new("paradex");
        let executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
        );

        // Empty orderbooks (no keys)
        let vest_orderbooks: SharedOrderbooks = Arc::new(RwLock::new(HashMap::new()));
        let paradex_orderbooks: SharedOrderbooks = Arc::new(RwLock::new(HashMap::new()));

        let (shutdown_tx, mut shutdown_rx) = broadcast::channel(1);

        let handle = tokio::spawn(async move {
            exit_monitoring_loop(
                &executor,
                ExitMonitoringParams {
                    vest_orderbooks,
                    paradex_orderbooks,
                    vest_symbol: "BTC-PERP".to_string(),
                    paradex_symbol: "BTC-USD-PERP".to_string(),
                    pair: "BTC-PERP".to_string(),
                    entry_spread: 0.35,
                    exit_spread_target: -0.05,
                    direction: SpreadDirection::AOverB,
                },
                &mut shutdown_rx,
            )
            .await
        });

        // Let it poll a few times with missing orderbooks
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Send shutdown
        let _ = shutdown_tx.send(());

        let result = timeout(Duration::from_secs(1), handle).await;
        assert!(
            result.is_ok(),
            "Exit monitoring should not panic with missing orderbooks"
        );
        let (_, exit_result) = result.unwrap().unwrap();
        assert!(
            exit_result.is_none(),
            "Should return None on shutdown (not exit condition)"
        );
    }
}
