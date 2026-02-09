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
use tokio::sync::{broadcast, watch};
use tokio::time::Duration;
use tracing::{debug, error, warn};

use crate::adapters::ExchangeAdapter;
use crate::core::channels::SpreadOpportunity;
use crate::core::events::{format_pct, log_event, log_system_event, SystemEvent, TradingEvent};
use crate::core::execution::{log_successful_trade, DeltaNeutralExecutor};
use crate::core::spread::{SpreadCalculator, SpreadDirection};

// TUI State type for optional TUI updates
use crate::tui::app::AppState as TuiState;
use std::sync::Mutex as StdMutex;

use crate::core::channels::SharedBestPrices;
use crate::core::channels::OrderbookNotify;

// =============================================================================
// Constants
// =============================================================================

/// Exit monitoring timeout — how long to wait for a Notify before checking anyway
const EXIT_NOTIFY_TIMEOUT_MS: u64 = 100;

/// Delay between position verification retries (milliseconds)
const API_SETTLE_DELAY_MS: u64 = 200;
/// Maximum retries for position verification after trade entry
const VERIFY_POSITION_RETRIES: u32 = 3;

/// Log throttle — imported from channels (single source of truth)
use super::channels::LOG_THROTTLE_POLLS;

// =============================================================================
// Helper Functions
// =============================================================================

/// Drain is no longer needed with watch channel (auto-replaces stale values)

/// Exit monitoring result with exit fill prices for PnL calculation
struct ExitResult {
    exit_spread: f64,
    vest_exit_price: f64,
    paradex_exit_price: f64,
    vest_realized_pnl: Option<f64>,
    paradex_realized_pnl: Option<f64>,
    execution_latency_ms: u64,
}

// =============================================================================
// Functions
// =============================================================================

/// Execution task that processes spread opportunities with quadratic scaling-in
///
/// Listens for `SpreadOpportunity` messages and executes delta-neutral trades
/// across multiple scaling layers. After initial entry, runs a hybrid monitoring
/// loop that simultaneously watches for exit condition AND fills deeper layers
/// when spread widens further.
///
/// V1 exit: all-at-once close when exit threshold is hit.
#[allow(clippy::too_many_arguments)]
pub async fn execution_task<V, P>(
    mut opportunity_rx: watch::Receiver<Option<SpreadOpportunity>>,
    mut executor: DeltaNeutralExecutor<V, P>,
    vest_best_prices: SharedBestPrices,
    paradex_best_prices: SharedBestPrices,
    _vest_symbol: String,
    _paradex_symbol: String,
    mut shutdown_rx: broadcast::Receiver<()>,
    exit_spread_target: f64,
    tui_state: Option<Arc<StdMutex<TuiState>>>,
    orderbook_notify: OrderbookNotify,
    // Scaling parameters
    spread_entry: f64,
    spread_entry_max: f64,
    total_position_size: f64,
) where
    V: ExchangeAdapter + Send + Sync,
    P: ExchangeAdapter + Send + Sync,
{
    use crate::core::scaling::calculate_entry_layers;

    log_system_event(&SystemEvent::task_started("execution"));

    // Resolve effective spread_entry_max (0.0 means not configured → single layer)
    let effective_max = if spread_entry_max > 0.0 {
        spread_entry_max
    } else {
        spread_entry
    };

    // Pre-compute scaling layers
    let num_layers = if (effective_max - spread_entry).abs() < 1e-10 { 1 } else { 5 };
    let layers = calculate_entry_layers(spread_entry, effective_max, total_position_size, num_layers);

    tracing::info!(
        event_type = "SCALING_CONFIG",
        num_layers = num_layers,
        spread_min = %format_pct(spread_entry),
        spread_max = %format_pct(effective_max),
        total_size = %format!("{:.6}", total_position_size),
        layer_sizes = %layers.iter().map(|l| format!("{:.6}", l.quantity)).collect::<Vec<_>>().join(", "),
        layer_triggers = %layers.iter().map(|l| format!("{:.4}%", l.spread_trigger)).collect::<Vec<_>>().join(", "),
        "Quadratic scaling layers computed"
    );

    // Track execution statistics
    let mut execution_count: u64 = 0;

    // =========================================================================
    // POSITION RECOVERY: Check for existing positions before entering main loop
    // =========================================================================
    let mut recovered = false;
    if let Some((direction, quantity)) = executor.recover_position().await {
        // We have an existing position — jump directly to hybrid monitoring
        let filled_layers = layers.len(); // assume all layers filled (recovery = full position)

        // Update TUI with recovered position
        if let Some(ref tui) = tui_state {
            if let Ok(mut state) = tui.lock() {
                // Use 0.0 for entry spread since we don't know the original
                state.record_entry(0.0, direction, 0.0, 0.0);
            }
        }

        // === HYBRID MONITORING LOOP (recovered) ===
        let mut poll_count: u64 = 0;
        let mut last_jwt_refresh = std::time::Instant::now();
        let mut exit_result_final: Option<ExitResult> = None;
        let mut shutdown_triggered = false;

        tracing::info!(
            event_type = "RECOVERY_MONITORING",
            direction = ?direction,
            quantity = %format!("{:.6}", quantity),
            exit_target = %format_pct(exit_spread_target),
            "Starting exit monitoring for recovered position"
        );

        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    log_system_event(&SystemEvent::task_shutdown("hybrid_monitoring", "shutdown_signal"));
                    shutdown_triggered = true;
                    break;
                }
                _ = tokio::time::timeout(
                    Duration::from_millis(EXIT_NOTIFY_TIMEOUT_MS),
                    orderbook_notify.notified()
                ) => {
                    poll_count += 1;

                    // JWT refresh every ~2 min
                    const JWT_REFRESH_INTERVAL_SECS: u64 = 120;
                    if last_jwt_refresh.elapsed().as_secs() >= JWT_REFRESH_INTERVAL_SECS {
                        last_jwt_refresh = std::time::Instant::now();
                        if let Err(e) = executor.ensure_ready().await {
                            warn!(event_type = "JWT_REFRESH_FAILED", error = %e, "Adapter refresh failed");
                        }
                    }

                    // Read live prices
                    let (vest_bid, vest_ask) = vest_best_prices.load();
                    let (paradex_bid, paradex_ask) = paradex_best_prices.load();

                    if vest_bid == 0.0 || paradex_bid == 0.0 {
                        if poll_count % LOG_THROTTLE_POLLS == 0 {
                            debug!(event_type = "EXIT_CHECK", "Waiting for orderbook data...");
                        }
                        continue;
                    }

                    // Calculate exit spread
                    let exit_spread = match direction {
                        SpreadDirection::AOverB => {
                            SpreadCalculator::calculate_exit_spread(vest_bid, paradex_ask)
                        }
                        SpreadDirection::BOverA => {
                            SpreadCalculator::calculate_exit_spread(paradex_bid, vest_ask)
                        }
                    };

                    // Update TUI live exit spread
                    if let Some(ref tui) = tui_state {
                        if let Ok(mut state) = tui.lock() {
                            state.live_exit_spread = exit_spread;
                            state.position_polls += 1;
                        }
                    }

                    // Check exit condition
                    if exit_spread >= exit_spread_target {
                        tracing::info!(
                            event_type = "EXIT_TRIGGERED",
                            exit_spread = %format!("{:.4}", exit_spread),
                            target = %format!("{:.4}", exit_spread_target),
                            "Exit condition met for recovered position"
                        );

                        let close_start = std::time::Instant::now();

                        loop {
                            match executor.close_position(exit_spread, vest_bid, vest_ask, paradex_bid, paradex_ask).await {
                                Ok(close_result) => {
                                    let execution_latency_ms = close_start.elapsed().as_millis() as u64;
                                    let closed_event = TradingEvent::position_closed(
                                        "recovered", 0.0, exit_spread, 0.0,
                                    );
                                    log_event(&closed_event);

                                    exit_result_final = Some(ExitResult {
                                        exit_spread,
                                        vest_exit_price: close_result.vest_fill_price,
                                        paradex_exit_price: close_result.paradex_fill_price,
                                        vest_realized_pnl: close_result.vest_realized_pnl,
                                        paradex_realized_pnl: close_result.paradex_realized_pnl,
                                        execution_latency_ms,
                                    });
                                    break;
                                }
                                Err(e) => {
                                    error!(event_type = "CLOSE_FAILED", error = ?e, "Close failed, retrying...");
                                    tokio::time::sleep(Duration::from_millis(500)).await;
                                    continue;
                                }
                            }
                        }
                        break; // Exit the monitoring loop
                    }

                    // Near-exit debug log (throttled)
                    if exit_spread >= exit_spread_target - 0.02 && poll_count % LOG_THROTTLE_POLLS == 0 {
                        debug!(
                            event_type = "EXIT_CHECK",
                            exit_spread = %format!("{:.4}", exit_spread),
                            target = %format!("{:.4}", exit_spread_target),
                            "Near exit threshold (recovered)"
                        );
                    }
                }
            }
        }

        if shutdown_triggered {
            return;
        }

        log_system_event(&SystemEvent::task_stopped("hybrid_monitoring"));

        // Update TUI trade history
        if let (Some(exit_result), Some(ref tui)) = (exit_result_final, &tui_state) {
            if let Ok(mut state) = tui.lock() {
                let vest_rpnl = exit_result.vest_realized_pnl;
                let paradex_rpnl = exit_result.paradex_realized_pnl;
                let pnl_usd = if vest_rpnl.is_some() || paradex_rpnl.is_some() {
                    let total = vest_rpnl.unwrap_or(0.0) + paradex_rpnl.unwrap_or(0.0);
                    tracing::info!(
                        event_type = "PNL_FROM_EXCHANGE",
                        vest_realized_pnl = ?vest_rpnl,
                        paradex_realized_pnl = ?paradex_rpnl,
                        total_pnl = %format!("{:.6}", total),
                        "PnL from exchange-reported realized PnL (recovered)"
                    );
                    total
                } else {
                    tracing::warn!(event_type = "PNL_UNAVAILABLE", "No realized PnL returned");
                    0.0
                };
                state.record_exit(exit_result.exit_spread, pnl_usd, exit_result.execution_latency_ms, exit_result.vest_exit_price, exit_result.paradex_exit_price);
            }
        }

        recovered = true;
        tracing::info!(event_type = "RECOVERY_COMPLETE", "Recovered position closed — resuming normal operation");
    }

    loop {
        tokio::select! {
            // Shutdown takes priority
            _ = shutdown_rx.recv() => {
                log_system_event(&SystemEvent::task_shutdown("execution", "shutdown_signal"));
                break;
            }
            // Process incoming opportunities (watch: always freshest)
            Ok(_) = opportunity_rx.changed() => {
                let opportunity = match opportunity_rx.borrow_and_update().clone() {
                    Some(opp) => opp,
                    None => continue, // Initial None value, skip
                };
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

                // === MULTI-LAYER ENTRY ===
                // Fill all eligible layers based on current spread
                let mut filled_layers: usize = 0;
                let mut total_filled_qty: f64 = 0.0;
                let mut first_layer_result = None;

                // Fill layers whose spread_trigger <= current_spread
                while filled_layers < layers.len()
                    && spread_pct >= layers[filled_layers].spread_trigger
                {
                    let layer = &layers[filled_layers];
                    let is_first = filled_layers == 0;

                    match executor.execute_delta_neutral_with_quantity(
                        opportunity.clone(),
                        layer.quantity,
                        layer.index,
                        is_first,
                    ).await {
                        Ok(result) => {
                            if result.success {
                                total_filled_qty += layer.quantity;
                                executor.set_quantity(total_filled_qty);

                                if is_first {
                                    first_layer_result = Some(result);
                                }
                                filled_layers += 1;
                            } else {
                                warn!(
                                    event_type = "LAYER_PARTIAL_FAIL",
                                    layer = layer.index,
                                    filled_layers = filled_layers,
                                    "Layer execution returned success=false, stopping entry"
                                );
                                break;
                            }
                        }
                        Err(e) => {
                            let err_msg = format!("{:?}", e);
                            if err_msg.contains("Position already open") {
                                debug!(event_type = "TRADE_SKIPPED", "Position already open");
                            } else {
                                error!(
                                    event_type = "LAYER_ERROR",
                                    layer = layer.index,
                                    error = ?e,
                                    "Layer entry failed"
                                );
                            }
                            break;
                        }
                    }
                }

                // Skip if no layers filled
                if filled_layers == 0 {
                    continue;
                }

                tracing::info!(
                    event_type = "SCALING_ENTRY_COMPLETE",
                    filled_layers = filled_layers,
                    total_layers = layers.len(),
                    total_filled_qty = %format!("{:.6}", total_filled_qty),
                    spread = %format_pct(spread_pct),
                    "Initial layer fill complete"
                );

                // Verify positions with retry
                let mut vest_entry = None;
                let mut paradex_entry = None;
                for attempt in 0..VERIFY_POSITION_RETRIES {
                    tokio::time::sleep(tokio::time::Duration::from_millis(API_SETTLE_DELAY_MS)).await;
                    let (v, p) = executor.verify_positions(spread_pct, exit_spread_target).await;
                    if vest_entry.is_none() { vest_entry = v; }
                    if paradex_entry.is_none() { paradex_entry = p; }
                    if vest_entry.is_some() && paradex_entry.is_some() {
                        break;
                    }
                    if attempt < VERIFY_POSITION_RETRIES - 1 {
                        debug!(
                            event_type = "VERIFY_RETRY",
                            attempt = attempt + 1,
                            vest_ok = vest_entry.is_some(),
                            paradex_ok = paradex_entry.is_some(),
                            "Position not yet propagated, retrying"
                        );
                    }
                }

                // Log TRADE_ENTRY + SLIPPAGE_ANALYSIS with real fill prices
                if let Some(ref result) = first_layer_result {
                    if let Some(timings) = result.timings.as_ref() {
                        log_successful_trade(
                            &opportunity,
                            result,
                            timings,
                            vest_entry.unwrap_or(0.0),
                            paradex_entry.unwrap_or(0.0),
                        );
                    }
                }

                // Update TUI state with entry prices
                if let Some(ref tui) = tui_state {
                    match tui.lock() {
                        Ok(mut state) => {
                            let actual_spread = match (vest_entry, paradex_entry) {
                                (Some(v), Some(p)) if v > 0.0 && p > 0.0 => {
                                    match opportunity.direction {
                                        SpreadDirection::AOverB => ((p - v) / v) * 100.0,
                                        SpreadDirection::BOverA => ((v - p) / p) * 100.0,
                                    }
                                }
                                _ => spread_pct,
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

                // Start hybrid monitoring: exit condition + deeper layer entries
                let entry_direction = executor.get_entry_direction();

                if let Some(direction) = entry_direction {
                    debug!(
                        event_type = "POSITION_OPENED",
                        direction = ?direction,
                        filled_layers = filled_layers,
                        remaining_layers = layers.len() - filled_layers,
                        exit_target = %format_pct(exit_spread_target),
                        "Starting hybrid exit/layer monitoring"
                    );

                    // === HYBRID MONITORING LOOP ===
                    // Monitors for both exit condition AND deeper layer entries
                    let mut poll_count: u64 = 0;
                    let mut last_jwt_refresh = std::time::Instant::now();
                    let mut exit_result_final: Option<ExitResult> = None;
                    let mut shutdown_triggered = false;

                    loop {
                        tokio::select! {
                            _ = shutdown_rx.recv() => {
                                log_system_event(&SystemEvent::task_shutdown("hybrid_monitoring", "shutdown_signal"));
                                shutdown_triggered = true;
                                break;
                            }
                            _ = tokio::time::timeout(
                                Duration::from_millis(EXIT_NOTIFY_TIMEOUT_MS),
                                orderbook_notify.notified()
                            ) => {
                                poll_count += 1;

                                // JWT refresh every ~2 min
                                const JWT_REFRESH_INTERVAL_SECS: u64 = 120;
                                if last_jwt_refresh.elapsed().as_secs() >= JWT_REFRESH_INTERVAL_SECS {
                                    last_jwt_refresh = std::time::Instant::now();
                                    if let Err(e) = executor.ensure_ready().await {
                                        warn!(event_type = "JWT_REFRESH_FAILED", error = %e, "Adapter refresh failed");
                                    }
                                }

                                let (vest_bid, vest_ask) = vest_best_prices.load();
                                let (paradex_bid, paradex_ask) = paradex_best_prices.load();

                                if vest_bid <= 0.0 || vest_ask <= 0.0 || paradex_bid <= 0.0 || paradex_ask <= 0.0 {
                                    if poll_count % LOG_THROTTLE_POLLS == 0 {
                                        debug!(event_type = "POSITION_MONITORING", poll = poll_count, "Missing orderbook data");
                                    }
                                    continue;
                                }

                                // Calculate exit spread
                                let exit_spread = match direction {
                                    SpreadDirection::AOverB => SpreadCalculator::calculate_exit_spread(vest_bid, paradex_ask),
                                    SpreadDirection::BOverA => SpreadCalculator::calculate_exit_spread(paradex_bid, vest_ask),
                                };

                                // Calculate entry spread for layer checks
                                let entry_spread_live = match direction {
                                    SpreadDirection::AOverB => SpreadCalculator::calculate_entry_spread(vest_ask, paradex_bid),
                                    SpreadDirection::BOverA => SpreadCalculator::calculate_entry_spread(paradex_ask, vest_bid),
                                };

                                // POSITION_MONITORING log (throttled)
                                if poll_count % LOG_THROTTLE_POLLS == 0 {
                                    let event = TradingEvent::position_monitoring(
                                        &pair, spread_pct, exit_spread, exit_spread_target, poll_count,
                                    );
                                    log_event(&event);
                                }

                                // === CHECK 1: Exit condition ===
                                if exit_spread >= exit_spread_target {
                                    let profit = spread_pct + exit_spread;
                                    let event = TradingEvent::trade_exit(
                                        &pair, spread_pct, exit_spread, exit_spread_target, profit, poll_count,
                                    );
                                    log_event(&event);

                                    // Close entire position
                                    const MAX_CLOSE_RETRIES: u32 = 3;
                                    const CLOSE_RETRY_DELAY_SECS: u64 = 5;
                                    let mut close_retries = 0u32;
                                    let close_start = std::time::Instant::now();

                                    loop {
                                        match executor.close_position(exit_spread, vest_bid, vest_ask, paradex_bid, paradex_ask).await {
                                            Ok(close_result) => {
                                                let execution_latency_ms = close_start.elapsed().as_millis() as u64;
                                                let closed_event = TradingEvent::position_closed(
                                                    &pair, spread_pct, exit_spread, profit,
                                                );
                                                log_event(&closed_event);
                                                exit_result_final = Some(ExitResult {
                                                    exit_spread,
                                                    vest_exit_price: close_result.vest_fill_price,
                                                    paradex_exit_price: close_result.paradex_fill_price,
                                                    vest_realized_pnl: close_result.vest_realized_pnl,
                                                    paradex_realized_pnl: close_result.paradex_realized_pnl,
                                                    execution_latency_ms,
                                                });
                                                break;
                                            }
                                            Err(e) => {
                                                close_retries += 1;
                                                error!(
                                                    event_type = "ORDER_FAILED", error = ?e,
                                                    retry = close_retries, max_retries = MAX_CLOSE_RETRIES,
                                                    "Failed to close - retrying in {}s", CLOSE_RETRY_DELAY_SECS
                                                );
                                                if close_retries >= MAX_CLOSE_RETRIES {
                                                    let execution_latency_ms = close_start.elapsed().as_millis() as u64;
                                                    error!(event_type = "CLOSE_ABANDONED", retries = close_retries, "CRITICAL: manual intervention required");
                                                    exit_result_final = Some(ExitResult {
                                                        exit_spread, vest_exit_price: 0.0, paradex_exit_price: 0.0,
                                                        vest_realized_pnl: None, paradex_realized_pnl: None, execution_latency_ms,
                                                    });
                                                    break;
                                                }
                                                tokio::time::sleep(Duration::from_secs(CLOSE_RETRY_DELAY_SECS)).await;
                                            }
                                        }
                                    }
                                    break; // Exit hybrid loop
                                }

                                // === CHECK 2: Deeper layer entries ===
                                if filled_layers < layers.len() {
                                    while filled_layers < layers.len()
                                        && entry_spread_live >= layers[filled_layers].spread_trigger
                                    {
                                        let layer = &layers[filled_layers];
                                        tracing::info!(
                                            event_type = "LAYER_TRIGGER",
                                            layer = layer.index,
                                            spread = %format_pct(entry_spread_live),
                                            trigger = %format_pct(layer.spread_trigger),
                                            "Deeper layer triggered during monitoring"
                                        );

                                        // Build a fresh opportunity with live prices
                                        let live_opp = SpreadOpportunity {
                                            pair: pair.clone(),
                                            dex_a: "vest",
                                            dex_b: "paradex",
                                            spread_percent: entry_spread_live,
                                            direction,
                                            detected_at_ms: crate::core::events::current_timestamp_ms(),
                                            dex_a_ask: vest_ask,
                                            dex_a_bid: vest_bid,
                                            dex_b_ask: paradex_ask,
                                            dex_b_bid: paradex_bid,
                                        };

                                        match executor.execute_delta_neutral_with_quantity(
                                            live_opp, layer.quantity, layer.index, false,
                                        ).await {
                                            Ok(result) if result.success => {
                                                total_filled_qty += layer.quantity;
                                                executor.set_quantity(total_filled_qty);
                                                filled_layers += 1;
                                                tracing::info!(
                                                    event_type = "LAYER_FILLED_DEEP",
                                                    layer = layer.index,
                                                    total_filled_qty = %format!("{:.6}", total_filled_qty),
                                                    filled_layers = filled_layers,
                                                    "Deeper layer filled"
                                                );

                                                // Re-query exchange positions for updated average entry prices
                                                tokio::time::sleep(tokio::time::Duration::from_millis(API_SETTLE_DELAY_MS)).await;
                                                let (v_entry, p_entry) = executor.verify_positions(entry_spread_live, exit_spread_target).await;

                                                // Update TUI with new average entry prices
                                                if let Some(ref tui) = tui_state {
                                                    if let Ok(mut state) = tui.lock() {
                                                        let ve = v_entry.unwrap_or(0.0);
                                                        let pe = p_entry.unwrap_or(0.0);
                                                        let actual_spread = if ve > 0.0 && pe > 0.0 {
                                                            match direction {
                                                                SpreadDirection::AOverB => ((pe - ve) / ve) * 100.0,
                                                                SpreadDirection::BOverA => ((ve - pe) / pe) * 100.0,
                                                            }
                                                        } else {
                                                            entry_spread_live
                                                        };
                                                        state.record_entry(actual_spread, direction, ve, pe);
                                                        tracing::debug!(
                                                            event_type = "TUI_ENTRY_UPDATE",
                                                            layer = layer.index,
                                                            vest_avg = %format!("{:.2}", ve),
                                                            paradex_avg = %format!("{:.2}", pe),
                                                            actual_spread = %format!("{:.4}", actual_spread),
                                                            "Updated TUI entry prices after deeper layer"
                                                        );
                                                    }
                                                }
                                            }
                                            Ok(_) => {
                                                warn!(event_type = "LAYER_PARTIAL_FAIL", layer = layer.index, "Deeper layer failed");
                                                break; // Stop trying more layers
                                            }
                                            Err(e) => {
                                                error!(event_type = "LAYER_ERROR", layer = layer.index, error = ?e, "Deeper layer error");
                                                break;
                                            }
                                        }
                                    }
                                }

                                // Near-exit debug log (throttled)
                                if exit_spread >= exit_spread_target - 0.02 && poll_count % LOG_THROTTLE_POLLS == 0 {
                                    debug!(
                                        event_type = "EXIT_CHECK",
                                        exit_spread = %format!("{:.4}", exit_spread),
                                        target = %format!("{:.4}", exit_spread_target),
                                        filled_layers = filled_layers,
                                        "Near exit threshold"
                                    );
                                }
                            }
                        }
                    }

                    if shutdown_triggered {
                        break;
                    }

                    log_system_event(&SystemEvent::task_stopped("hybrid_monitoring"));

                    // Update TUI trade history
                    if let (Some(exit_result), Some(ref tui)) = (exit_result_final, &tui_state) {
                        match tui.lock() {
                            Ok(mut state) => {
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
                                    tracing::warn!(event_type = "PNL_UNAVAILABLE", "No realized PnL returned");
                                    0.0
                                };
                                state.record_exit(exit_result.exit_spread, pnl_usd, exit_result.execution_latency_ms, exit_result.vest_exit_price, exit_result.paradex_exit_price);
                            }
                            Err(e) => {
                                error!(event_type = "TUI_STATE_ERROR", error = %e, "Failed to record trade exit in TUI state");
                            }
                        }
                    }
                } else {
                    error!(event_type = "ORDER_FAILED", "No entry direction found after successful trade");
                }

                // Watch channel auto-replaces stale data — no drain needed
            }
        }
    }

    log_system_event(&SystemEvent::task_stopped("execution"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::test_utils::TestMockAdapter;
    use crate::core::channels::AtomicBestPrices;
    use crate::core::spread::SpreadDirection;
    use tokio::time::{timeout, Duration};

    /// Helper: create SharedBestPrices pre-loaded with the given bid/ask
    fn make_best_prices(bid: f64, ask: f64) -> SharedBestPrices {
        let bp = Arc::new(AtomicBestPrices::new());
        bp.store(bid, ask);
        bp
    }

    #[tokio::test]
    async fn test_execution_task_processes_opportunity() {
        let (opportunity_tx, opportunity_rx) = watch::channel(None);
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

        // Create SharedBestPrices with data that triggers exit (spread = 0 >= -0.05)
        let vest_bp = make_best_prices(42000.0, 42001.0);
        let paradex_bp = make_best_prices(42000.0, 42001.0); // Same as vest => spread ~0%

        // Spawn the execution task (V1: with exit monitoring)
        let handle = tokio::spawn(async move {
            execution_task(
                opportunity_rx,
                executor,
                vest_bp,
                paradex_bp,
                "BTC-PERP".to_string(),
                "BTC-USD-PERP".to_string(),
                shutdown_rx,
                -0.05, // exit_spread_target: exit when spread >= -0.05%
                None,  // No TUI state in tests
                Arc::new(tokio::sync::Notify::new()),
                0.35, // spread_entry
                0.35, // spread_entry_max (single layer)
                0.01, // total_position_size
            )
            .await;
        });

        // Send an opportunity
        let opportunity = SpreadOpportunity {
            pair: Arc::from("BTC-PERP"),
            dex_a: "vest",
            dex_b: "paradex",
            spread_percent: 0.35,
            direction: SpreadDirection::AOverB,
            detected_at_ms: 1706000000000,
            dex_a_ask: 42000.0,
            dex_a_bid: 41990.0,
            dex_b_ask: 42005.0,
            dex_b_bid: 41985.0,
        };
        opportunity_tx.send_replace(Some(opportunity));

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
        let (_opportunity_tx, opportunity_rx) = watch::channel(None::<SpreadOpportunity>);
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

        // Create empty SharedBestPrices for test
        let vest_bp = Arc::new(AtomicBestPrices::new());
        let paradex_bp = Arc::new(AtomicBestPrices::new());

        // V1: with exit monitoring
        let handle = tokio::spawn(async move {
            execution_task(
                opportunity_rx,
                executor,
                vest_bp,
                paradex_bp,
                "BTC-PERP".to_string(),
                "BTC-USD-PERP".to_string(),
                shutdown_rx,
                -0.05, // exit_spread_target
                None,  // No TUI state in tests
                Arc::new(tokio::sync::Notify::new()),
                0.35, // spread_entry
                0.35, // spread_entry_max (single layer)
                0.01, // total_position_size
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
        let mut executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
        );

        // Create SharedBestPrices with prices that produce exit_spread >= target
        // For AOverB: exit_spread = (vest_bid - paradex_ask) / paradex_ask * 100
        // vest_bid = 42000, paradex_ask = 42000 => spread = 0%
        // Target = -0.05%, so 0% >= -0.05% triggers exit
        let vest_bp = make_best_prices(42000.0, 42001.0);
        let paradex_bp = make_best_prices(42000.0, 42000.0); // ask same as vest_bid

        let (opportunity_tx, opportunity_rx) = watch::channel(None);
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

        // Simulate open position so close_position can work
        executor.simulate_open_position(SpreadDirection::AOverB);

        let handle = tokio::spawn(async move {
            execution_task(
                opportunity_rx,
                executor,
                vest_bp,
                paradex_bp,
                "BTC-PERP".to_string(),
                "BTC-USD-PERP".to_string(),
                shutdown_rx,
                -0.05, // exit_spread_target
                None,  // No TUI state in tests
                Arc::new(tokio::sync::Notify::new()),
                0.35, // spread_entry
                0.35, // spread_entry_max (single layer)
                0.01, // total_position_size
            )
            .await;
        });

        // Send an opportunity to trigger the monitoring loop
        let opportunity = SpreadOpportunity {
            pair: Arc::from("BTC-PERP"),
            dex_a: "vest",
            dex_b: "paradex",
            spread_percent: 0.35,
            direction: SpreadDirection::AOverB,
            detected_at_ms: 1706000000000,
            dex_a_ask: 42000.0,
            dex_a_bid: 41990.0,
            dex_b_ask: 42005.0,
            dex_b_bid: 41985.0,
        };
        opportunity_tx.send_replace(Some(opportunity));

        // Give time for entry and exit condition to be met
        tokio::time::sleep(Duration::from_millis(800)).await;

        // Shutdown to ensure the task cleans up
        let _ = shutdown_tx.send(());

        // Should complete (not timeout)
        let result = timeout(Duration::from_secs(5), handle).await;
        assert!(
            result.is_ok(),
            "Execution task should complete on exit condition"
        );
    }

    #[tokio::test]
    async fn test_execution_task_responds_to_shutdown_during_monitoring() {
        // Execution task should shut down cleanly even if spread never hits exit target
        let (opportunity_tx, opportunity_rx) = watch::channel(None::<SpreadOpportunity>);
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

        // Prices that will NEVER trigger exit
        let vest_bp = make_best_prices(40000.0, 40001.0);
        let paradex_bp = make_best_prices(42000.0, 42000.0);

        let handle = tokio::spawn(async move {
            execution_task(
                opportunity_rx,
                executor,
                vest_bp,
                paradex_bp,
                "BTC-PERP".to_string(),
                "BTC-USD-PERP".to_string(),
                shutdown_rx,
                -0.05,
                None,
                Arc::new(tokio::sync::Notify::new()),
                0.35,
                0.35,
                0.01,
            )
            .await;
        });

        // Send opportunity to enter the monitoring phase
        let opp = SpreadOpportunity {
            pair: Arc::from("BTC-PERP"),
            dex_a: "vest",
            dex_b: "paradex",
            spread_percent: 0.35,
            direction: SpreadDirection::AOverB,
            detected_at_ms: 1706000000000,
            dex_a_ask: 40001.0,
            dex_a_bid: 40000.0,
            dex_b_ask: 42000.0,
            dex_b_bid: 42000.0,
        };
        opportunity_tx.send_replace(Some(opp));

        // Give it time to enter monitoring
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Send shutdown
        let _ = shutdown_tx.send(());

        let result = timeout(Duration::from_secs(2), handle).await;
        assert!(result.is_ok(), "Execution task should respond to shutdown");
    }

    #[tokio::test]
    async fn test_execution_task_b_over_a_direction() {
        // Test that execution task works with BOverA spread direction
        let (opportunity_tx, opportunity_rx) = watch::channel(None::<SpreadOpportunity>);
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

        // For BOverA: entry should work, exit immediately triggers
        let vest_bp = make_best_prices(42000.0, 42000.0);
        let paradex_bp = make_best_prices(42000.0, 42001.0);

        let handle = tokio::spawn(async move {
            execution_task(
                opportunity_rx,
                executor,
                vest_bp,
                paradex_bp,
                "BTC-PERP".to_string(),
                "BTC-USD-PERP".to_string(),
                shutdown_rx,
                -0.05,
                None,
                Arc::new(tokio::sync::Notify::new()),
                0.35,
                0.35,
                0.01,
            )
            .await;
        });

        let opp = SpreadOpportunity {
            pair: Arc::from("BTC-PERP"),
            dex_a: "vest",
            dex_b: "paradex",
            spread_percent: 0.35,
            direction: SpreadDirection::BOverA,
            detected_at_ms: 1706000000000,
            dex_a_ask: 42000.0,
            dex_a_bid: 42000.0,
            dex_b_ask: 42001.0,
            dex_b_bid: 42000.0,
        };
        opportunity_tx.send_replace(Some(opp));

        tokio::time::sleep(Duration::from_millis(800)).await;
        let _ = shutdown_tx.send(());

        let result = timeout(Duration::from_secs(5), handle).await;
        assert!(result.is_ok(), "Should complete with BOverA direction");
    }

    // =========================================================================
    // Additional Tests
    // =========================================================================

    #[tokio::test]
    async fn test_execution_task_drains_pending_messages() {
        // Send multiple opportunities, then shutdown — verify at least one is processed
        let (opportunity_tx, opportunity_rx) = watch::channel(None::<SpreadOpportunity>);
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

        let vest_bp = Arc::new(AtomicBestPrices::new());
        let paradex_bp = Arc::new(AtomicBestPrices::new());

        let handle = tokio::spawn(async move {
            execution_task(
                opportunity_rx,
                executor,
                vest_bp,
                paradex_bp,
                "BTC-PERP".to_string(),
                "BTC-USD-PERP".to_string(),
                shutdown_rx,
                -0.05,
                None,
                Arc::new(tokio::sync::Notify::new()),
                0.35,
                0.35,
                0.01,
            )
            .await;
        });

        // Send one opportunity
        let opp = SpreadOpportunity {
            pair: Arc::from("BTC-PERP"),
            dex_a: "vest",
            dex_b: "paradex",
            spread_percent: 0.35,
            direction: SpreadDirection::AOverB,
            detected_at_ms: 1706000000000,
            dex_a_ask: 42000.0,
            dex_a_bid: 41990.0,
            dex_b_ask: 42005.0,
            dex_b_bid: 41985.0,
        };
        opportunity_tx.send_replace(Some(opp));

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
        let (_opportunity_tx, opportunity_rx) = watch::channel(None::<SpreadOpportunity>);
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

        let vest_bp = Arc::new(AtomicBestPrices::new());
        let paradex_bp = Arc::new(AtomicBestPrices::new());

        let handle = tokio::spawn(async move {
            execution_task(
                opportunity_rx,
                executor,
                vest_bp,
                paradex_bp,
                "BTC-PERP".to_string(),
                "BTC-USD-PERP".to_string(),
                shutdown_rx,
                -0.05,
                None,
                Arc::new(tokio::sync::Notify::new()),
                0.35,
                0.35,
                0.01,
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
    async fn test_execution_task_handles_missing_orderbooks() {
        // Execution task should not panic with missing orderbook data
        let (opportunity_tx, opportunity_rx) = watch::channel(None::<SpreadOpportunity>);
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

        // Empty best prices (0.0)
        let vest_bp = Arc::new(AtomicBestPrices::new());
        let paradex_bp = Arc::new(AtomicBestPrices::new());

        let handle = tokio::spawn(async move {
            execution_task(
                opportunity_rx,
                executor,
                vest_bp,
                paradex_bp,
                "BTC-PERP".to_string(),
                "BTC-USD-PERP".to_string(),
                shutdown_rx,
                -0.05,
                None,
                Arc::new(tokio::sync::Notify::new()),
                0.35,
                0.35,
                0.01,
            )
            .await;
        });

        // Send an opportunity with the empty orderbooks
        let opp = SpreadOpportunity {
            pair: Arc::from("BTC-PERP"),
            dex_a: "vest",
            dex_b: "paradex",
            spread_percent: 0.35,
            direction: SpreadDirection::AOverB,
            detected_at_ms: 1706000000000,
            dex_a_ask: 42000.0,
            dex_a_bid: 41990.0,
            dex_b_ask: 42005.0,
            dex_b_bid: 41985.0,
        };
        opportunity_tx.send_replace(Some(opp));

        // Let it poll with missing orderbooks, then shutdown
        tokio::time::sleep(Duration::from_millis(200)).await;
        let _ = shutdown_tx.send(());

        let result = timeout(Duration::from_secs(2), handle).await;
        assert!(
            result.is_ok(),
            "Should not panic with missing orderbooks"
        );
    }
}
