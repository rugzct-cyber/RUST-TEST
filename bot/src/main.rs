//! HFT Arbitrage Bot - V1 Entry Point
//!
//! This is a lean HFT implementation that:
//! 1. Loads configuration
//! 2. Connects to Vest and Paradex
//! 3. Subscribes to orderbooks
//! 4. Calculates spreads (lock-free shared orderbooks)
//! 5. Executes delta-neutral trades
//!
//! V1 HFT Mode: No persistence, no Mutex locks for minimum latency
//!
//! # Logging
//! - Uses structured JSON output (configurable via LOG_FORMAT env var)
//! - Uses structured BOT_STARTED/BOT_SHUTDOWN events
//! - Removes legacy [TAG] prefixes, uses event_type fields instead
//!
//! # TUI Mode
//! - Set LOG_FORMAT=tui to enable terminal UI
//! - Press 'q' or Ctrl+C to quit

use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::io::stdout;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::signal;
use tokio::sync::{broadcast, watch};
use tracing::{error, info, warn};
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;
// ratatui accessed via full path in spawn block for Terminal
use hft_bot::adapters::{create_adapter, resolve_symbol, ExchangeAdapter};
use hft_bot::config;
use hft_bot::core::channels::SpreadOpportunity;
use hft_bot::core::events::{format_pct, log_event, TradingEvent};
use hft_bot::core::execution::DeltaNeutralExecutor;
use hft_bot::core::monitoring::{monitoring_task, MonitoringConfig};
use hft_bot::core::runtime::execution_task;
use hft_bot::tui::{AppState, TuiLayer};

/// Restore terminal to normal state
fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(stdout(), LeaveAlternateScreen);
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load environment variables from .env file (if it exists)
    dotenvy::dotenv().ok();

    // Check if TUI mode is requested
    let tui_mode = config::is_tui_mode();

    // Set up panic hook to restore terminal on crash (for TUI mode)
    if tui_mode {
        let original_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic_info| {
            restore_terminal();
            original_hook(panic_info);
        }));
    }

    // Load config FIRST (before subscriber) to avoid log loss in TUI mode.
    // In TUI mode, the subscriber isn't ready until after AppState is created from config.
    // So we load config silently, init subscriber, then log everything.
    let config = match config::load_config(Path::new("config.yaml")) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("FATAL: Failed to load config.yaml: {}", e);
            std::process::exit(1);
        }
    };
    let bot = config
        .bots
        .first()
        .expect("config.yaml must have at least one bot entry");

    // Initialize logging - TUI mode uses TuiLayer, others use normal logging
    if !tui_mode {
        config::init_logging();
        // Log BOT_STARTED event
        let started_event = TradingEvent::bot_started();
        log_event(&started_event);
    }

    // TUI Mode: Initialize AppState, terminal, and tracing subscriber
    let app_state: Option<Arc<Mutex<AppState>>> = if tui_mode {
        // Create AppState with config values
        let state = Arc::new(Mutex::new(AppState::new(
            bot.pair.to_string(),
            bot.spread_entry,
            bot.spread_exit,
            bot.position_size,
            bot.leverage as u32,
            bot.dex_a.to_string(),
            bot.dex_b.to_string(),
        )));

        // Initialize terminal in raw mode with alternate screen
        enable_raw_mode().expect("Failed to enable raw mode");
        execute!(stdout(), EnterAlternateScreen).expect("Failed to enter alternate screen");

        // Set up tracing subscriber with TuiLayer
        let tui_layer = TuiLayer::new(Arc::clone(&state));
        let env_filter =
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tui_layer)
            .init();

        info!(event_type = "TUI", "Terminal UI initialized");

        Some(state)
    } else {
        None
    };

    // Now that subscriber is ready (both TUI and non-TUI), log config details
    info!(
        event_type = "CONFIG",
        pairs = ?config.bots.iter().map(|b| b.pair.to_string()).collect::<Vec<_>>(),
        bot_count = config.bots.len(),
        "Configuration loaded"
    );
    info!(
        event_type = "CONFIG",
        bot_id = %bot.id,
        pair = %bot.pair,
        dex_a = %bot.dex_a,
        dex_b = %bot.dex_b,
        spread_entry = %format_pct(bot.spread_entry),
        spread_exit = %format_pct(bot.spread_exit),
        exit_confirm_ticks = bot.exit_confirm_ticks,
        entry_confirm_ticks = bot.entry_confirm_ticks,
        slippage_buffer_pct = %format!("{:.2}%", bot.slippage_buffer_pct * 100.0),
        leverage = %format!("{}x", bot.leverage),
        position_size = %format!("{} {}", bot.position_size, bot.pair),
        "Active bot configuration"
    );

    // Create adapters dynamically from config
    let dex_a_name = bot.dex_a.to_string();
    let dex_b_name = bot.dex_b.to_string();
    info!(
        event_type = "ADAPTER_INIT",
        dex_a = %dex_a_name,
        dex_b = %dex_b_name,
        "Initializing exchange adapters"
    );

    let mut dex_a_adapter = create_adapter(&dex_a_name)
        .unwrap_or_else(|e| panic!("Failed to create {} adapter: {}", dex_a_name, e));
    info!(event_type = "ADAPTER_INIT", exchange = %dex_a_name, "Adapter initialized");

    // Initialize Pyth USDC rate cache for USD→USDC price conversion
    let usdc_rate_cache = std::sync::Arc::new(hft_bot::core::UsdcRateCache::new());
    let pyth_handle = hft_bot::core::spawn_rate_refresh_task(
        std::sync::Arc::clone(&usdc_rate_cache),
        reqwest::Client::new(),
    );
    // Monitor Pyth task — warn if it exits unexpectedly (panic or error)
    tokio::spawn(async move {
        match pyth_handle.await {
            Ok(()) => warn!(
                event_type = "PYTH_STOPPED",
                "Pyth rate refresh task exited — USDC rate may become stale"
            ),
            Err(e) => {
                error!(event_type = "PYTH_PANIC", error = %e, "Pyth rate refresh task PANICKED — USDC rate frozen")
            }
        }
    });
    info!(
        event_type = "PYTH_INIT",
        "USDC rate cache initialized with 15-minute refresh"
    );

    let mut dex_b_adapter = create_adapter(&dex_b_name)
        .unwrap_or_else(|e| panic!("Failed to create {} adapter: {}", dex_b_name, e));
    // Set USDC rate cache on Paradex adapters (if either dex is Paradex)
    if let Some(paradex) = dex_a_adapter.as_paradex_mut() {
        paradex.set_usdc_rate_cache(std::sync::Arc::clone(&usdc_rate_cache));
    }
    if let Some(paradex) = dex_b_adapter.as_paradex_mut() {
        paradex.set_usdc_rate_cache(std::sync::Arc::clone(&usdc_rate_cache));
    }
    info!(event_type = "ADAPTER_INIT", exchange = %dex_b_name, "Adapter initialized");

    // Create channels for data pipeline
    let (opportunity_tx, opportunity_rx) = watch::channel(None::<SpreadOpportunity>);

    // Connect to exchanges
    info!(event_type = "CONNECTION", "Connecting to exchanges");

    dex_a_adapter
        .connect()
        .await
        .unwrap_or_else(|e| panic!("Failed to connect to {}: {}", dex_a_name, e));
    info!(event_type = "CONNECTION", exchange = %dex_a_name, "Connected");

    dex_b_adapter
        .connect()
        .await
        .unwrap_or_else(|e| panic!("Failed to connect to {}: {}", dex_b_name, e));
    info!(event_type = "CONNECTION", exchange = %dex_b_name, "Connected");

    // Resolve exchange-specific symbols from base pair
    let base = bot.pair.base();
    let dex_a_symbol = resolve_symbol(&dex_a_name, base);
    let dex_b_symbol = resolve_symbol(&dex_b_name, base);

    dex_a_adapter
        .subscribe_orderbook(&dex_a_symbol)
        .await
        .unwrap_or_else(|e| panic!("Failed to subscribe to {} orderbook: {}", dex_a_name, e));
    dex_b_adapter
        .subscribe_orderbook(&dex_b_symbol)
        .await
        .unwrap_or_else(|e| panic!("Failed to subscribe to {} orderbook: {}", dex_b_name, e));

    // Subscribe to order confirmations (no-op for exchanges that don't support it)
    dex_a_adapter
        .subscribe_orders(&dex_a_symbol)
        .await
        .unwrap_or_else(|e| warn!(event_type = "SUBSCRIPTION", exchange = %dex_a_name, error = %e, "Order subscription not available"));
    dex_b_adapter
        .subscribe_orders(&dex_b_symbol)
        .await
        .unwrap_or_else(|e| warn!(event_type = "SUBSCRIPTION", exchange = %dex_b_name, error = %e, "Order subscription not available"));

    info!(
        event_type = "SUBSCRIPTION",
        dex_a_symbol = %dex_a_symbol,
        dex_b_symbol = %dex_b_symbol,
        "Subscribed to orderbooks"
    );

    // Check current positions at startup
    info!(event_type = "STARTUP_CHECK", "Checking current positions at startup...");
    for (adapter, name, symbol) in [
        (&dex_a_adapter as &dyn ExchangeAdapter, dex_a_name.as_str(), dex_a_symbol.as_str()),
        (&dex_b_adapter as &dyn ExchangeAdapter, dex_b_name.as_str(), dex_b_symbol.as_str()),
    ] {
        match adapter.get_position(symbol).await {
            Ok(Some(pos)) => info!(
                event_type = "STARTUP_POSITION",
                exchange = %name,
                entry_price = pos.entry_price,
                side = %pos.side,
                quantity = pos.quantity,
                "Position found at startup"
            ),
            _ => info!(event_type = "STARTUP_POSITION", exchange = %name, "No position"),
        }
    }

    // Get shared data from monitoring adapters
    let dex_a_shared_orderbooks = dex_a_adapter.get_shared_orderbooks();
    let dex_b_shared_orderbooks = dex_b_adapter.get_shared_orderbooks();
    let dex_a_best_prices = dex_a_adapter.get_shared_best_prices();
    let dex_b_best_prices = dex_b_adapter.get_shared_best_prices();
    // Create shared Notify for event-driven monitoring
    let orderbook_notify: hft_bot::core::OrderbookNotify = Arc::new(tokio::sync::Notify::new());
    dex_a_adapter.set_orderbook_notify(orderbook_notify.clone());
    dex_b_adapter.set_orderbook_notify(orderbook_notify.clone());
    info!(
        event_type = "RUNTIME",
        "SharedOrderbooks + AtomicBestPrices + OrderbookNotify configured for event-driven monitoring"
    );

    // Create shutdown broadcast channel
    let (shutdown_tx, mut shutdown_rx) = broadcast::channel::<()>(1);

    // Initialize execution adapters (separate from monitoring adapters)
    info!(event_type = "ADAPTER_INIT", "Initializing execution adapters");

    let mut exec_dex_a = create_adapter(&dex_a_name)
        .unwrap_or_else(|e| panic!("Failed to create {} execution adapter: {}", dex_a_name, e));
    // Set USDC rate cache on Paradex execution adapters
    if let Some(paradex) = exec_dex_a.as_paradex_mut() {
        paradex.set_usdc_rate_cache(std::sync::Arc::clone(&usdc_rate_cache));
    }
    exec_dex_a
        .connect()
        .await
        .unwrap_or_else(|e| panic!("Failed to connect {} execution adapter: {}", dex_a_name, e));
    info!(event_type = "CONNECTION", adapter = %format!("{}_execution", dex_a_name), "Connected");

    let mut exec_dex_b = create_adapter(&dex_b_name)
        .unwrap_or_else(|e| panic!("Failed to create {} execution adapter: {}", dex_b_name, e));
    if let Some(paradex) = exec_dex_b.as_paradex_mut() {
        paradex.set_usdc_rate_cache(std::sync::Arc::clone(&usdc_rate_cache));
    }
    exec_dex_b
        .connect()
        .await
        .unwrap_or_else(|e| panic!("Failed to connect {} execution adapter: {}", dex_b_name, e));
    info!(event_type = "CONNECTION", adapter = %format!("{}_execution", dex_b_name), "Connected");

    // Set leverage on both execution adapters (from config)
    let target_leverage = bot.leverage as u32;
    info!(event_type = "LEVERAGE_SETUP", leverage = %format!("{}x", target_leverage), "Setting leverage on execution adapters");

    for (adapter, name, symbol) in [
        (&exec_dex_a as &dyn ExchangeAdapter, dex_a_name.as_str(), dex_a_symbol.as_str()),
        (&exec_dex_b as &dyn ExchangeAdapter, dex_b_name.as_str(), dex_b_symbol.as_str()),
    ] {
        match adapter.set_leverage(symbol, target_leverage).await {
            Ok(lev) => info!(event_type = "LEVERAGE_SETUP", exchange = %name, leverage = lev, "Leverage configured"),
            Err(e) => warn!(event_type = "LEVERAGE_SETUP", exchange = %name, error = %e, "Failed to set leverage (continuing)"),
        }
    }

    let executor = DeltaNeutralExecutor::new(
        exec_dex_a,
        exec_dex_b,
        bot.position_size,
        dex_a_symbol.clone(),
        dex_b_symbol.clone(),
        dex_a_name.clone(),
        dex_b_name.clone(),
        bot.slippage_buffer_pct,
    );

    // Spawn execution_task (V1: with exit monitoring)
    // Clone reader_alive flags from monitoring adapters for disconnect detection
    let dex_a_alive = dex_a_adapter.get_reader_alive();
    let dex_b_alive = dex_b_adapter.get_reader_alive();
    let execution_shutdown = shutdown_tx.subscribe();
    let exit_spread_target = bot.spread_exit;
    let exec_dex_a_best_prices = dex_a_best_prices.clone();
    let exec_dex_b_best_prices = dex_b_best_prices.clone();
    let exec_dex_a_symbol = dex_a_symbol.clone();
    let exec_dex_b_symbol = dex_b_symbol.clone();
    let exec_tui_state = app_state.clone();
    let exec_orderbook_notify = orderbook_notify.clone();
    let exec_spread_entry = bot.spread_entry;
    let exec_spread_entry_max = bot.spread_entry_max;
    let exec_position_size = bot.position_size;
    let exec_exit_confirm_ticks = bot.exit_confirm_ticks;
    let exec_dex_a_alive = dex_a_alive.clone();
    let exec_dex_b_alive = dex_b_alive.clone();
    tokio::spawn(async move {
        execution_task(
            opportunity_rx,
            executor,
            exec_dex_a_best_prices,
            exec_dex_b_best_prices,
            exec_dex_a_symbol,
            exec_dex_b_symbol,
            execution_shutdown,
            exit_spread_target,
            exec_tui_state,
            exec_orderbook_notify,
            exec_spread_entry,
            exec_spread_entry_max,
            exec_position_size,
            exec_exit_confirm_ticks,
            exec_dex_a_alive,
            exec_dex_b_alive,
        )
        .await;
    });
    info!(
        event_type = "RUNTIME",
        task = "execution",
        "Task spawned (V1 HFT mode with exit monitoring)"
    );

    // Spawn monitoring task
    let monitoring_dex_a_symbol = dex_a_symbol.clone();
    let monitoring_dex_b_symbol = dex_b_symbol.clone();
    let monitoring_config = MonitoringConfig {
        pair: Arc::from(bot.pair.to_string().as_str()),
        spread_entry: bot.spread_entry,
        spread_exit: bot.spread_exit,
        entry_confirm_ticks: bot.entry_confirm_ticks,
    };
    let monitoring_shutdown = shutdown_tx.subscribe();

    tokio::spawn(async move {
        monitoring_task(
            dex_a_best_prices.clone(),
            dex_b_best_prices.clone(),
            dex_a_shared_orderbooks.clone(),
            dex_b_shared_orderbooks.clone(),
            opportunity_tx,
            monitoring_dex_a_symbol,
            monitoring_dex_b_symbol,
            monitoring_config,
            orderbook_notify,
            monitoring_shutdown,
            dex_a_alive.clone(),
            dex_b_alive.clone(),
        )
        .await;
    });
    info!(
        event_type = "RUNTIME",
        task = "monitoring",
        mode = "event-driven",
        "Task spawned (event-driven)"
    );

    // Spawn TUI render task (if TUI mode enabled)
    if let Some(ref state) = app_state {
        let tui_state = Arc::clone(state);
        let tui_shutdown = shutdown_tx.subscribe();
        let tui_shutdown_tx = shutdown_tx.clone();
        let tui_dex_a_orderbooks = dex_a_adapter.get_shared_orderbooks();
        let tui_dex_b_orderbooks = dex_b_adapter.get_shared_orderbooks();
        let tui_dex_a_symbol = dex_a_symbol.clone();
        let tui_dex_b_symbol = dex_b_symbol.clone();

        tokio::spawn(async move {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(stdout()))
                    .expect("Failed to create terminal");

            let mut shutdown_rx = tui_shutdown;
            let mut event_stream = crossterm::event::EventStream::new();

            loop {
                // Check for shutdown
                if shutdown_rx.try_recv().is_ok() {
                    break;
                }

                // Update AppState with latest orderbook data
                // Clone data out of each RwLock individually to minimize lock hold time
                let dex_a_ob_data = tui_dex_a_orderbooks
                    .read()
                    .await
                    .get(&tui_dex_a_symbol)
                    .cloned();
                let dex_b_ob_data = tui_dex_b_orderbooks
                    .read()
                    .await
                    .get(&tui_dex_b_symbol)
                    .cloned();

                if let (Some(dex_a_ob), Some(dex_b_ob)) = (dex_a_ob_data, dex_b_ob_data) {
                    if let Ok(mut state) = tui_state.try_lock() {
                        state.update_prices(
                            dex_a_ob.best_bid().unwrap_or(0.0),
                            dex_a_ob.best_ask().unwrap_or(0.0),
                            dex_b_ob.best_bid().unwrap_or(0.0),
                            dex_b_ob.best_ask().unwrap_or(0.0),
                        );
                        state.update_live_spreads();

                        let entry_spread = state.live_entry_spread;
                        let best_dir = if state.dex_a_best_ask > 0.0 && state.dex_b_best_ask > 0.0
                        {
                            let a_over_b = (state.dex_b_best_bid - state.dex_a_best_ask)
                                / state.dex_a_best_ask;
                            let b_over_a = (state.dex_a_best_bid - state.dex_b_best_ask)
                                / state.dex_b_best_ask;
                            if a_over_b >= b_over_a {
                                Some(hft_bot::core::spread::SpreadDirection::AOverB)
                            } else {
                                Some(hft_bot::core::spread::SpreadDirection::BOverA)
                            }
                        } else {
                            None
                        };
                        state.update_spread(entry_spread, best_dir);
                    }
                }

                // Handle keyboard events (async, non-blocking)
                match hft_bot::tui::event::handle_events_async(
                    &tui_state,
                    &tui_shutdown_tx,
                    &mut event_stream,
                )
                .await
                {
                    hft_bot::tui::event::EventResult::Quit => break,
                    hft_bot::tui::event::EventResult::Continue => {}
                }

                // Render UI
                if let Ok(state) = tui_state.lock() {
                    let _ = terminal.draw(|frame| {
                        hft_bot::tui::ui::draw(frame, &state);
                    });
                }

                // 50ms tick rate (event timeout provides 50ms pacing, add small yield)
                tokio::time::sleep(Duration::from_millis(50)).await;
            }

            // Restore terminal on exit
            restore_terminal();
        });
        info!(
            event_type = "RUNTIME",
            task = "tui",
            tick_ms = 100,
            "TUI render task spawned"
        );
    }

    info!(
        event_type = "RUNTIME",
        "Bot runtime started (V1 HFT - no persistence, no Mutex locks)"
    );

    // Spawn SIGINT handler task
    let shutdown_signal = shutdown_tx.clone();
    tokio::spawn(async move {
        info!(
            event_type = "BOT_SHUTDOWN",
            "SIGINT handler registered - press Ctrl+C to initiate graceful shutdown"
        );
        match signal::ctrl_c().await {
            Ok(()) => {
                // Log BOT_SHUTDOWN event
                let shutdown_event = TradingEvent::bot_shutdown();
                log_event(&shutdown_event);

                match shutdown_signal.send(()) {
                    Ok(n) => {
                        info!(
                            event_type = "BOT_SHUTDOWN",
                            receivers = n,
                            "Shutdown signal broadcast"
                        );
                    }
                    Err(_) => {
                        error!(
                            event_type = "BOT_SHUTDOWN",
                            "CRITICAL: Failed to broadcast shutdown - no receivers!"
                        );
                        std::process::exit(1);
                    }
                }
            }
            Err(err) => {
                error!(event_type = "BOT_SHUTDOWN", error = %err, "CRITICAL: Failed to register Ctrl+C handler");
                std::process::exit(1);
            }
        }
    });

    // Wait for shutdown signal
    tokio::select! {
        _ = shutdown_rx.recv() => {
            info!(event_type = "BOT_SHUTDOWN", "Shutdown signal received in main task");
        }
    }

    // Graceful shutdown - disconnect from exchanges
    info!(event_type = "BOT_SHUTDOWN", "Disconnecting from exchanges");
    dex_a_adapter.disconnect().await.unwrap_or_else(
        |e| warn!(event_type = "BOT_SHUTDOWN", exchange = %dex_a_name, error = %e, "Failed to disconnect"),
    );
    dex_b_adapter.disconnect().await.unwrap_or_else(
        |e| warn!(event_type = "BOT_SHUTDOWN", exchange = %dex_b_name, error = %e, "Failed to disconnect"),
    );
    info!(event_type = "BOT_SHUTDOWN", "Disconnected from exchanges");

    // Restore terminal if TUI was active
    if tui_mode {
        restore_terminal();
        info!(event_type = "BOT_SHUTDOWN", "Terminal restored");
    }

    info!(event_type = "BOT_SHUTDOWN", "Clean exit");

    Ok(())
}
