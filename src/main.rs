//! HFT Arbitrage Bot - MVP Entry Point
//!
//! This is a minimal implementation that:
//! 1. Loads configuration
//! 2. Connects to Vest and Paradex
//! 3. Subscribes to orderbooks
//! 4. Calculates spreads using VWAP
//! 5. Logs opportunities to console

use std::path::Path;
use tokio::signal;
use tokio::sync::broadcast;
use tracing::{info, error};
use hft_bot::config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load environment variables from .env file (if it exists)
    dotenvy::dotenv().ok();
    
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    info!("🚀 HFT Arbitrage Bot MVP starting...");
    
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
    info!("   Capital: ${}", bot.capital);
    
    // TODO: Create adapters with real credentials
    // let vest = VestAdapter::new(&config.vest)?;
    // let paradex = ParadexAdapter::new(&config.paradex)?;
    
    // TODO: Connect to exchanges
    // vest.connect().await?;
    // paradex.connect().await?;
    
    // TODO: Subscribe to orderbooks
    // vest.subscribe_orderbook(pair).await?;
    // paradex.subscribe_orderbook(pair).await?;
    
    // Create shutdown broadcast channel
    let (shutdown_tx, mut shutdown_rx) = broadcast::channel::<()>(1);

    // Spawn SIGINT handler task
    let shutdown_signal = shutdown_tx.clone();
    tokio::spawn(async move {
        match signal::ctrl_c().await {
            Ok(()) => {
                info!("[SHUTDOWN] Graceful shutdown initiated");
                // Broadcast shutdown to all tasks
                let _ = shutdown_signal.send(());
            }
            Err(err) => {
                eprintln!("Failed to listen for Ctrl+C signal: {}", err);
            }
        }
    });

    info!("⏳ MVP scaffold ready. Press Ctrl+C to test graceful shutdown.");
    
    // Placeholder task - waits for shutdown
    tokio::select! {
        _ = shutdown_rx.recv() => {
            info!("[SHUTDOWN] Shutdown signal received in main task");
        }
    }

    // TODO Epic 6: Disconnect adapters here
    // vest_adapter.disconnect().await?;
    // paradex_adapter.disconnect().await?;

    info!("[SHUTDOWN] Clean exit");
    Ok(())
}
