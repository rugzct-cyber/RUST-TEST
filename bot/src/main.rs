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
use tokio::sync::{broadcast, mpsc};
use tracing::{error, info, warn};
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;
// ratatui accessed via full path in spawn block for Terminal
use hft_bot::adapters::paradex::{ParadexAdapter, ParadexConfig};
use hft_bot::adapters::vest::{VestAdapter, VestConfig};
use hft_bot::adapters::ExchangeAdapter;
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
        leverage = %format!("{}x", bot.leverage),
        position_size = %format!("{} {}", bot.position_size, bot.pair),
        "Active bot configuration"
    );

    // Create adapters with real credentials
    info!(
        event_type = "ADAPTER_INIT",
        "Initializing exchange adapters"
    );

    // Subtask 1.1: Create VestAdapter with credentials from .env
    let vest_config = VestConfig::from_env()
        .expect("VEST credentials must be configured in .env (VEST_PRIMARY_ADDR, VEST_PRIMARY_KEY, VEST_SIGNING_KEY)");
    let mut vest_adapter = VestAdapter::new(vest_config);
    info!(
        event_type = "ADAPTER_INIT",
        exchange = "vest",
        "Adapter initialized"
    );

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

    // Subtask 1.2: Create ParadexAdapter with credentials from .env
    let paradex_config = ParadexConfig::from_env()
        .expect("PARADEX credentials must be configured in .env (PARADEX_PRIVATE_KEY, PARADEX_ACCOUNT_ADDRESS)");
    let mut paradex_adapter = ParadexAdapter::new(paradex_config);
    paradex_adapter.set_usdc_rate_cache(std::sync::Arc::clone(&usdc_rate_cache));
    info!(
        event_type = "ADAPTER_INIT",
        exchange = "paradex",
        "Adapter initialized with USDC conversion"
    );

    // Task 2: Create channels for data pipeline

    // Create spread_opportunity channel
    let (opportunity_tx, opportunity_rx) = mpsc::channel::<SpreadOpportunity>(1);

    // Task 3: Connect to exchanges
    info!(event_type = "CONNECTION", "Connecting to exchanges");

    vest_adapter
        .connect()
        .await
        .expect("Failed to connect to Vest");
    info!(event_type = "CONNECTION", exchange = "vest", "Connected");

    paradex_adapter
        .connect()
        .await
        .expect("Failed to connect to Paradex");
    info!(event_type = "CONNECTION", exchange = "paradex", "Connected");

    // Subtask 3.3: Subscribe to orderbooks
    let vest_symbol = bot.pair.to_string(); // e.g., "BTC-PERP"
    let paradex_symbol = format!(
        "{}-USD-PERP",
        bot.pair.to_string().split('-').next().unwrap_or("BTC")
    ); // e.g., "BTC-USD-PERP"

    vest_adapter
        .subscribe_orderbook(&vest_symbol)
        .await
        .expect("Failed to subscribe to Vest orderbook");
    paradex_adapter
        .subscribe_orderbook(&paradex_symbol)
        .await
        .expect("Failed to subscribe to Paradex orderbook");

    // Subscribe to Paradex order confirmations via WebSocket
    paradex_adapter
        .subscribe_orders(&paradex_symbol)
        .await
        .expect("Failed to subscribe to Paradex order channel");

    info!(
        event_type = "SUBSCRIPTION",
        vest_symbol = %vest_symbol,
        paradex_symbol = %paradex_symbol,
        "Subscribed to orderbooks"
    );
    info!(
        event_type = "SUBSCRIPTION",
        channel = "orders",
        exchange = "paradex",
        "Subscribed to order confirmations"
    );

    // DEBUG: Check current positions at startup to verify entry prices
    info!(
        event_type = "STARTUP_CHECK",
        "Checking current positions at startup..."
    );
    if let Ok(Some(vest_pos)) = vest_adapter.get_position(&vest_symbol).await {
        info!(
            event_type = "STARTUP_POSITION",
            exchange = "vest",
            entry_price = vest_pos.entry_price,
            side = %vest_pos.side,
            quantity = vest_pos.quantity,
            "Vest position found at startup"
        );
    } else {
        info!(
            event_type = "STARTUP_POSITION",
            exchange = "vest",
            "No position"
        );
    }
    if let Ok(Some(paradex_pos)) = paradex_adapter.get_position(&paradex_symbol).await {
        info!(
            event_type = "STARTUP_POSITION",
            exchange = "paradex",
            entry_price = paradex_pos.entry_price,
            side = %paradex_pos.side,
            quantity = paradex_pos.quantity,
            "Paradex position found at startup"
        );
    } else {
        info!(
            event_type = "STARTUP_POSITION",
            exchange = "paradex",
            "No position"
        );
    }

    // Get SharedOrderbooks for lock-free monitoring (NO Mutex!)
    let vest_shared_orderbooks = vest_adapter.get_shared_orderbooks();
    let paradex_shared_orderbooks = paradex_adapter.get_shared_orderbooks();
    // Get AtomicBestPrices for hot-path monitoring (zero lock, zero allocation)
    let vest_best_prices = vest_adapter.get_shared_best_prices();
    let paradex_best_prices = paradex_adapter.get_shared_best_prices();
    // Create shared Notify for event-driven monitoring (Axe 5)
    let orderbook_notify: hft_bot::core::OrderbookNotify = Arc::new(tokio::sync::Notify::new());
    vest_adapter.set_orderbook_notify(orderbook_notify.clone());
    paradex_adapter.set_orderbook_notify(orderbook_notify.clone());
    info!(
        event_type = "RUNTIME",
        "SharedOrderbooks + AtomicBestPrices + OrderbookNotify configured for event-driven monitoring"
    );

    // Create shutdown broadcast channel
    let (shutdown_tx, mut shutdown_rx) = broadcast::channel::<()>(1);

    // Initialize execution adapters (separate from monitoring adapters)
    info!(
        event_type = "ADAPTER_INIT",
        "Initializing execution adapters"
    );

    let execution_vest_config =
        VestConfig::from_env().expect("VEST credentials must be configured for execution adapter");
    let mut execution_vest = VestAdapter::new(execution_vest_config);
    execution_vest
        .connect()
        .await
        .expect("Failed to connect Vest execution adapter");
    info!(
        event_type = "CONNECTION",
        adapter = "vest_execution",
        "Connected"
    );

    let execution_paradex_config = ParadexConfig::from_env()
        .expect("PARADEX credentials must be configured for execution adapter");
    let mut execution_paradex = ParadexAdapter::new(execution_paradex_config);
    execution_paradex
        .connect()
        .await
        .expect("Failed to connect Paradex execution adapter");
    info!(
        event_type = "CONNECTION",
        adapter = "paradex_execution",
        "Connected"
    );

    // Set leverage on both execution adapters (from config)
    let target_leverage = bot.leverage as u32;
    info!(event_type = "LEVERAGE_SETUP", leverage = %format!("{}x", target_leverage), "Setting leverage on execution adapters");

    match execution_vest
        .set_leverage(&vest_symbol, target_leverage)
        .await
    {
        Ok(lev) => info!(
            event_type = "LEVERAGE_SETUP",
            exchange = "vest",
            leverage = lev,
            "Leverage configured"
        ),
        Err(e) => {
            warn!(event_type = "LEVERAGE_SETUP", exchange = "vest", error = %e, "Failed to set leverage (continuing)")
        }
    }

    match execution_paradex
        .set_leverage(&paradex_symbol, target_leverage)
        .await
    {
        Ok(lev) => info!(
            event_type = "LEVERAGE_SETUP",
            exchange = "paradex",
            leverage = lev,
            "Leverage configured"
        ),
        Err(e) => {
            warn!(event_type = "LEVERAGE_SETUP", exchange = "paradex", error = %e, "Failed to set leverage (continuing)")
        }
    }

    let executor = DeltaNeutralExecutor::new(
        execution_vest,
        execution_paradex,
        bot.position_size,
        vest_symbol.clone(),
        paradex_symbol.clone(),
    );

    // Spawn execution_task (V1: with exit monitoring)
    let execution_shutdown = shutdown_tx.subscribe();
    let exit_spread_target = bot.spread_exit;
    let exec_vest_best_prices = vest_best_prices.clone();
    let exec_paradex_best_prices = paradex_best_prices.clone();
    let exec_vest_symbol = vest_symbol.clone();
    let exec_paradex_symbol = paradex_symbol.clone();
    let exec_tui_state = app_state.clone();
    let exec_orderbook_notify = orderbook_notify.clone();
    tokio::spawn(async move {
        execution_task(
            opportunity_rx,
            executor,
            exec_vest_best_prices,
            exec_paradex_best_prices,
            exec_vest_symbol,
            exec_paradex_symbol,
            execution_shutdown,
            exit_spread_target,
            exec_tui_state,
            exec_orderbook_notify,
        )
        .await;
    });
    info!(
        event_type = "RUNTIME",
        task = "execution",
        "Task spawned (V1 HFT mode with exit monitoring)"
    );

    // Spawn monitoring task (using SharedOrderbooks - NO Mutex!)
    let monitoring_tx = opportunity_tx.clone();
    let monitoring_vest_symbol = vest_symbol.clone();
    let monitoring_paradex_symbol = paradex_symbol.clone();
    let monitoring_config = MonitoringConfig {
        pair: bot.pair.to_string(),
        spread_entry: bot.spread_entry,
        spread_exit: bot.spread_exit,
    };
    let monitoring_shutdown = shutdown_tx.subscribe();

    tokio::spawn(async move {
        monitoring_task(
            vest_best_prices.clone(),
            paradex_best_prices.clone(),
            vest_shared_orderbooks.clone(),
            paradex_shared_orderbooks.clone(),
            monitoring_tx,
            monitoring_vest_symbol,
            monitoring_paradex_symbol,
            monitoring_config,
            orderbook_notify,
            monitoring_shutdown,
        )
        .await;
    });
    info!(
        event_type = "RUNTIME",
        task = "monitoring",
        mode = "event-driven",
        "Task spawned (event-driven, Axe 5)"
    );

    // Spawn TUI render task (if TUI mode enabled)
    if let Some(ref state) = app_state {
        let tui_state = Arc::clone(state);
        let tui_shutdown = shutdown_tx.subscribe();
        let tui_shutdown_tx = shutdown_tx.clone();
        // Clone SharedOrderbooks for TUI data feed
        let tui_vest_orderbooks = vest_adapter.get_shared_orderbooks();
        let tui_paradex_orderbooks = paradex_adapter.get_shared_orderbooks();
        let tui_vest_symbol = vest_symbol.clone();
        let tui_paradex_symbol = paradex_symbol.clone();

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
                let vest_ob_data = tui_vest_orderbooks
                    .read()
                    .await
                    .get(&tui_vest_symbol)
                    .cloned();
                let paradex_ob_data = tui_paradex_orderbooks
                    .read()
                    .await
                    .get(&tui_paradex_symbol)
                    .cloned();

                if let (Some(vest_ob), Some(paradex_ob)) = (vest_ob_data, paradex_ob_data) {
                    if let Ok(mut state) = tui_state.try_lock() {
                        // Update prices
                        state.update_prices(
                            vest_ob.best_bid().unwrap_or(0.0),
                            vest_ob.best_ask().unwrap_or(0.0),
                            paradex_ob.best_bid().unwrap_or(0.0),
                            paradex_ob.best_ask().unwrap_or(0.0),
                        );

                        // Calculate live entry/exit spreads
                        state.update_live_spreads();

                        // Determine best direction for header display
                        let entry_spread = state.live_entry_spread;
                        let best_dir = if state.vest_best_ask > 0.0 && state.paradex_best_ask > 0.0
                        {
                            let a_over_b = (state.paradex_best_bid - state.vest_best_ask)
                                / state.vest_best_ask;
                            let b_over_a = (state.vest_best_bid - state.paradex_best_ask)
                                / state.paradex_best_ask;
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
    vest_adapter.disconnect().await.unwrap_or_else(
        |e| warn!(event_type = "BOT_SHUTDOWN", error = %e, "Failed to disconnect from Vest"),
    );
    paradex_adapter.disconnect().await.unwrap_or_else(
        |e| warn!(event_type = "BOT_SHUTDOWN", error = %e, "Failed to disconnect from Paradex"),
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
