//! HFT Arbitrage Bot - V1 Entry Point (Story 7.3, Story 5.3, Story 5.1)
//!
//! This is a lean HFT implementation that:
//! 1. Loads configuration
//! 2. Connects to Vest and Paradex
//! 3. Subscribes to orderbooks
//! 4. Calculates spreads using VWAP (lock-free shared orderbooks)
//! 5. Executes delta-neutral trades
//!
//! V1 HFT Mode: No persistence, no Mutex locks for minimum latency
//!
//! # Logging (Story 5.1, 5.3)
//! - Uses structured JSON output (configurable via LOG_FORMAT env var)
//! - Uses structured BOT_STARTED/BOT_SHUTDOWN events
//! - Removes legacy [TAG] prefixes, uses event_type fields instead
//!
//! # TUI Mode
//! - Set LOG_FORMAT=tui to enable terminal UI
//! - Press 'q' or Ctrl+C to quit

use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::signal;
use tokio::sync::{broadcast, mpsc};
use tracing::{info, error, warn};
// Note: tracing_subscriber will be used for TuiLayer composition in future release
use hft_bot::config;
use hft_bot::adapters::ExchangeAdapter;
use hft_bot::adapters::vest::{VestAdapter, VestConfig};
use hft_bot::adapters::paradex::{ParadexAdapter, ParadexConfig};
use hft_bot::core::channels::SpreadOpportunity;
use hft_bot::core::events::{TradingEvent, log_event, format_pct};
use hft_bot::core::monitoring::{monitoring_task, MonitoringConfig};
use hft_bot::core::runtime::execution_task;
use hft_bot::core::execution::DeltaNeutralExecutor;
use hft_bot::tui::AppState;
// Note: TuiLayer will be used for TUI rendering in future release
#[allow(unused_imports)]
use hft_bot::tui::TuiLayer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load environment variables from .env file (if it exists)
    dotenvy::dotenv().ok();
    
    // Check if TUI mode is requested (V1: detection only, full TUI spawn in future release)
    let _tui_mode = config::is_tui_mode();
    
    // AppState for TUI (wrapped in Option for conditional use)
    // V1: Infrastructure in place, TUI render loop to be added
    let _app_state: Option<Arc<Mutex<AppState>>> = if _tui_mode {
        // Will be initialized after config is loaded
        None
    } else {
        None
    };
    
    // Initialize logging (Story 5.1: JSON/Pretty/TUI configurable via LOG_FORMAT)
    // Note: TUI mode skips this, initialized later with TuiLayer
    config::init_logging();

    // Log BOT_STARTED event (Story 5.3)
    let started_event = TradingEvent::bot_started();
    log_event(&started_event);
    
    // Load configuration from YAML
    info!(event_type = "CONFIG", "Loading configuration from config.yaml");
    let config = match config::load_config(Path::new("config.yaml")) {
        Ok(cfg) => {
            let pairs: Vec<String> = cfg.bots.iter()
                .map(|b| b.pair.to_string())
                .collect();
            info!(
                event_type = "CONFIG",
                pairs = ?pairs,
                bot_count = cfg.bots.len(),
                "Configuration loaded"
            );
            cfg
        }
        Err(e) => {
            error!(event_type = "CONFIG", error = %e, "Configuration failed");
            std::process::exit(1);
        }
    };

    // Access first bot for MVP single-pair mode
    let bot = &config.bots[0];
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
    
    
    // Story 6.1 Task 1: Create adapters with real credentials
    info!(event_type = "ADAPTER_INIT", "Initializing exchange adapters");
    
    // Subtask 1.1: Create VestAdapter with credentials from .env
    let vest_config = VestConfig::from_env()
        .expect("VEST credentials must be configured in .env (VEST_PRIMARY_ADDR, VEST_PRIMARY_KEY, VEST_SIGNING_KEY)");
    let mut vest_adapter = VestAdapter::new(vest_config);
    info!(event_type = "ADAPTER_INIT", exchange = "vest", "Adapter initialized");
    
    // Initialize Pyth USDC rate cache for USD→USDC price conversion
    let usdc_rate_cache = std::sync::Arc::new(hft_bot::core::UsdcRateCache::new());
    hft_bot::core::spawn_rate_refresh_task(
        std::sync::Arc::clone(&usdc_rate_cache),
        reqwest::Client::new(),
    );
    info!(event_type = "PYTH_INIT", "USDC rate cache initialized with 15-minute refresh");
    
    // Subtask 1.2: Create ParadexAdapter with credentials from .env
    let paradex_config = ParadexConfig::from_env()
        .expect("PARADEX credentials must be configured in .env (PARADEX_PRIVATE_KEY, PARADEX_ACCOUNT_ADDRESS)");
    let mut paradex_adapter = ParadexAdapter::new(paradex_config);
    paradex_adapter.set_usdc_rate_cache(std::sync::Arc::clone(&usdc_rate_cache));
    info!(event_type = "ADAPTER_INIT", exchange = "paradex", "Adapter initialized with USDC conversion");

    
    // Task 2: Create channels for data pipeline
    
    // Subtask 2.1: Create spread_opportunity channel (Story 6.2)
    let (opportunity_tx, opportunity_rx) = mpsc::channel::<SpreadOpportunity>(1);
    
    // Task 3: Connect to exchanges
    info!(event_type = "CONNECTION", "Connecting to exchanges");
    
    vest_adapter.connect().await
        .expect("Failed to connect to Vest");
    info!(event_type = "CONNECTION", exchange = "vest", "Connected");
    
    paradex_adapter.connect().await
        .expect("Failed to connect to Paradex");
    info!(event_type = "CONNECTION", exchange = "paradex", "Connected");

    
    // Subtask 3.3: Subscribe to orderbooks
    let vest_symbol = bot.pair.to_string();  // e.g., "BTC-PERP"
    let paradex_symbol = format!("{}-USD-PERP",
        bot.pair.to_string().split('-').next().unwrap_or("BTC")
    );  // e.g., "BTC-USD-PERP"
    
    vest_adapter.subscribe_orderbook(&vest_symbol).await
        .expect("Failed to subscribe to Vest orderbook");
    paradex_adapter.subscribe_orderbook(&paradex_symbol).await
        .expect("Failed to subscribe to Paradex orderbook");
    
    // Story 7.1: Subscribe to Paradex order confirmations via WebSocket
    paradex_adapter.subscribe_orders(&paradex_symbol).await
        .expect("Failed to subscribe to Paradex order channel");
    
    info!(
        event_type = "SUBSCRIPTION",
        vest_symbol = %vest_symbol,
        paradex_symbol = %paradex_symbol,
        "Subscribed to orderbooks"
    );
    info!(event_type = "SUBSCRIPTION", channel = "orders", exchange = "paradex", "Subscribed to order confirmations");

    // Story 7.3: Get SharedOrderbooks for lock-free monitoring (NO Mutex!)
    let vest_shared_orderbooks = vest_adapter.get_shared_orderbooks();
    let paradex_shared_orderbooks = paradex_adapter.get_shared_orderbooks();
    info!(event_type = "RUNTIME", "SharedOrderbooks extracted for lock-free monitoring");

    // Create shutdown broadcast channel
    let (shutdown_tx, mut shutdown_rx) = broadcast::channel::<()>(1);
    
    // Initialize execution adapters (separate from monitoring adapters)
    info!(event_type = "ADAPTER_INIT", "Initializing execution adapters");
    
    let execution_vest_config = VestConfig::from_env()
        .expect("VEST credentials must be configured for execution adapter");
    let mut execution_vest = VestAdapter::new(execution_vest_config);
    execution_vest.connect().await
        .expect("Failed to connect Vest execution adapter");
    info!(event_type = "CONNECTION", adapter = "vest_execution", "Connected");
    
    let execution_paradex_config = ParadexConfig::from_env()
        .expect("PARADEX credentials must be configured for execution adapter");
    let mut execution_paradex = ParadexAdapter::new(execution_paradex_config);
    execution_paradex.connect().await
        .expect("Failed to connect Paradex execution adapter");
    info!(event_type = "CONNECTION", adapter = "paradex_execution", "Connected");
    
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
    let exec_vest_orderbooks = vest_shared_orderbooks.clone();
    let exec_paradex_orderbooks = paradex_shared_orderbooks.clone();
    let exec_vest_symbol = vest_symbol.clone();
    let exec_paradex_symbol = paradex_symbol.clone();
    tokio::spawn(async move {
        execution_task(
            opportunity_rx,
            executor,
            exec_vest_orderbooks,
            exec_paradex_orderbooks,
            exec_vest_symbol,
            exec_paradex_symbol,
            execution_shutdown,
            exit_spread_target,
        ).await;
    });
    info!(event_type = "RUNTIME", task = "execution", "Task spawned (V1 HFT mode with exit monitoring)");
    
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
            vest_shared_orderbooks,
            paradex_shared_orderbooks,
            monitoring_tx,
            monitoring_vest_symbol,
            monitoring_paradex_symbol,
            monitoring_config,
            monitoring_shutdown,
        ).await;
    });
    info!(event_type = "RUNTIME", task = "monitoring", polling_ms = 25, "Task spawned (lock-free)");
    
    info!(event_type = "RUNTIME", "Bot runtime started (V1 HFT - no persistence, no Mutex locks)");

    // Spawn SIGINT handler task
    let shutdown_signal = shutdown_tx.clone();
    tokio::spawn(async move {
        info!(event_type = "BOT_SHUTDOWN", "SIGINT handler registered - press Ctrl+C to initiate graceful shutdown");
        match signal::ctrl_c().await {
            Ok(()) => {
                // Log BOT_SHUTDOWN event (Story 5.3)
                let shutdown_event = TradingEvent::bot_shutdown();
                log_event(&shutdown_event);
                
                match shutdown_signal.send(()) {
                    Ok(n) => {
                        info!(event_type = "BOT_SHUTDOWN", receivers = n, "Shutdown signal broadcast");
                    }
                    Err(_) => {
                        error!(event_type = "BOT_SHUTDOWN", "CRITICAL: Failed to broadcast shutdown - no receivers!");
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
    vest_adapter.disconnect().await
        .unwrap_or_else(|e| warn!(event_type = "BOT_SHUTDOWN", error = %e, "Failed to disconnect from Vest"));
    paradex_adapter.disconnect().await
        .unwrap_or_else(|e| warn!(event_type = "BOT_SHUTDOWN", error = %e, "Failed to disconnect from Paradex"));
    info!(event_type = "BOT_SHUTDOWN", "Disconnected from exchanges");

    info!(event_type = "BOT_SHUTDOWN", "Clean exit");

    Ok(())
}
