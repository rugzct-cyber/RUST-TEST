//! HFT Arbitrage Bot - V1 Entry Point (Story 7.3: Supabase + Mutex removed)
//!
//! This is a lean HFT implementation that:
//! 1. Loads configuration
//! 2. Connects to Vest and Paradex
//! 3. Subscribes to orderbooks
//! 4. Calculates spreads using VWAP (lock-free shared orderbooks)
//! 5. Executes delta-neutral trades
//!
//! V1 HFT Mode: No persistence, no Mutex locks for minimum latency

use std::path::Path;
use tokio::signal;
use tokio::sync::{broadcast, mpsc};
use tracing::{info, error};
use hft_bot::config;
use hft_bot::adapters::ExchangeAdapter;
use hft_bot::adapters::vest::{VestAdapter, VestConfig};
use hft_bot::adapters::paradex::{ParadexAdapter, ParadexConfig};
use hft_bot::core::channels::SpreadOpportunity;
use hft_bot::core::monitoring::{monitoring_task, MonitoringConfig};
use hft_bot::core::runtime::execution_task;
use hft_bot::core::execution::DeltaNeutralExecutor;


#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load environment variables from .env file (if it exists)
    dotenvy::dotenv().ok();
    
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    info!("🚀 HFT Arbitrage Bot V1 starting (no persistence, lock-free mode)...");
    
    // Load configuration from YAML
    info!("📁 Loading configuration from config.yaml...");
    let config = match config::load_config(Path::new("config.yaml")) {
        Ok(cfg) => {
            let pairs: Vec<String> = cfg.bots.iter()
                .map(|b| b.pair.to_string())
                .collect();
            info!("[CONFIG] Loaded pairs: {:?}", pairs);
            info!("[INFO] Loaded {} bots from configuration", cfg.bots.len());
            cfg
        }
        Err(e) => {
            error!("[ERROR] Configuration failed: {}", e);
            std::process::exit(1);
        }
    };

    // Access first bot for MVP single-pair mode
    let bot = &config.bots[0];
    info!("📊 Active Bot Configuration:");
    info!("   ID: {}", bot.id);
    info!("   Pair: {}", bot.pair);
    info!("   DEX A: {}", bot.dex_a);
    info!("   DEX B: {}", bot.dex_b);
    info!("   Entry threshold: {}%", bot.spread_entry);
    info!("   Exit threshold: {}%", bot.spread_exit);
    info!("   Leverage: {}x", bot.leverage);
    info!("   Position Size: {} {}", bot.position_size, bot.pair);
    
    
    // Story 6.1 Task 1: Create adapters with real credentials
    info!("🔐 Initializing exchange adapters...");
    
    // Subtask 1.1: Create VestAdapter with credentials from .env
    let vest_config = VestConfig::from_env()
        .expect("VEST credentials must be configured in .env (VEST_PRIMARY_ADDR, VEST_PRIMARY_KEY, VEST_SIGNING_KEY)");
    let mut vest_adapter = VestAdapter::new(vest_config);
    info!("[INFO] Vest adapter initialized");
    
    // Subtask 1.2: Create ParadexAdapter with credentials from .env
    let paradex_config = ParadexConfig::from_env()
        .expect("PARADEX credentials must be configured in .env (PARADEX_PRIVATE_KEY, PARADEX_ACCOUNT_ADDRESS)");
    let mut paradex_adapter = ParadexAdapter::new(paradex_config);
    info!("[INFO] Paradex adapter initialized");

    
    // Task 2: Create channels for data pipeline
    
    // Subtask 2.1: Create spread_opportunity channel (Story 6.2)
    let (opportunity_tx, opportunity_rx) = mpsc::channel::<SpreadOpportunity>(1);
    
    // Task 3: Connect to exchanges
    info!("🌐 Connecting to exchanges...");
    
    vest_adapter.connect().await
        .expect("Failed to connect to Vest");
    info!("[INFO] Connected to Vest");
    
    paradex_adapter.connect().await
        .expect("Failed to connect to Paradex");
    info!("[INFO] Connected to Paradex");

    
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
    
    info!("[INFO] Subscribed to orderbooks: {}, {}", vest_symbol, paradex_symbol);
    info!("[INFO] Subscribed to Paradex order confirmations (WS)");

    // Story 7.3: Get SharedOrderbooks for lock-free monitoring (NO Mutex!)
    let vest_shared_orderbooks = vest_adapter.get_shared_orderbooks();
    let paradex_shared_orderbooks = paradex_adapter.get_shared_orderbooks();
    info!("[INFO] SharedOrderbooks extracted for lock-free monitoring");

    // Create shutdown broadcast channel
    let (shutdown_tx, mut shutdown_rx) = broadcast::channel::<()>(1);
    
    // Initialize execution adapters (separate from monitoring adapters)
    info!("🔌 Initializing execution adapters...");
    
    let execution_vest_config = VestConfig::from_env()
        .expect("VEST credentials must be configured for execution adapter");
    let mut execution_vest = VestAdapter::new(execution_vest_config);
    execution_vest.connect().await
        .expect("Failed to connect Vest execution adapter");
    info!("[INFO] Vest execution adapter connected");
    
    let execution_paradex_config = ParadexConfig::from_env()
        .expect("PARADEX credentials must be configured for execution adapter");
    let mut execution_paradex = ParadexAdapter::new(execution_paradex_config);
    execution_paradex.connect().await
        .expect("Failed to connect Paradex execution adapter");
    info!("[INFO] Paradex execution adapter connected");
    
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
    info!("[INFO] Execution task spawned (V1 HFT mode with exit monitoring)");
    
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
    info!("[INFO] Monitoring task spawned (lock-free, 25ms polling)");
    
    info!("✅ Bot runtime started (V1 HFT - no persistence, no Mutex locks)");

    // Spawn SIGINT handler task
    let shutdown_signal = shutdown_tx.clone();
    tokio::spawn(async move {
        info!("[SHUTDOWN] SIGINT handler registered - press Ctrl+C to initiate graceful shutdown");
        match signal::ctrl_c().await {
            Ok(()) => {
                info!("[SHUTDOWN] Graceful shutdown initiated");
                match shutdown_signal.send(()) {
                    Ok(n) => {
                        info!("[SHUTDOWN] Shutdown signal broadcast to {} receiver(s)", n);
                    }
                    Err(_) => {
                        error!("[SHUTDOWN] CRITICAL: Failed to broadcast shutdown - no receivers!");
                        std::process::exit(1);
                    }
                }
            }
            Err(err) => {
                error!("[SHUTDOWN] CRITICAL: Failed to register Ctrl+C handler: {}. Bot cannot be stopped gracefully!", err);
                std::process::exit(1);
            }
        }
    });

    // Wait for shutdown signal
    tokio::select! {
        _ = shutdown_rx.recv() => {
            info!("[SHUTDOWN] Shutdown signal received in main task");
        }
    }

    // Graceful shutdown - disconnect from exchanges
    info!("[SHUTDOWN] Disconnecting from exchanges...");
    vest_adapter.disconnect().await
        .unwrap_or_else(|e| error!("[SHUTDOWN] Failed to disconnect from Vest: {}", e));
    paradex_adapter.disconnect().await
        .unwrap_or_else(|e| error!("[SHUTDOWN] Failed to disconnect from Paradex: {}", e));
    info!("[SHUTDOWN] Disconnected from exchanges");

    info!("[SHUTDOWN] Clean exit");

    Ok(())
}
