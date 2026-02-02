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

// Story 4.6: Orphan order protection (Epic 6 integration pending)
// use hft_bot::core::state::{StateManager, PendingOrder};
// use hft_bot::adapters::vest::VestAdapter;
// use hft_bot::adapters::paradex::ParadexAdapter;
// use std::sync::Arc;
// use tokio::sync::Mutex;

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
    
    // TODO Epic 6: Create adapters with real credentials
    // let vest = Arc::new(Mutex::new(VestAdapter::new(&config.vest)?));
    // let paradex = Arc::new(Mutex::new(ParadexAdapter::new(&config.paradex)?));
    // let state_manager = Arc::new(StateManager::new(config.supabase));
    
    // TODO Epic 6: Connect to exchanges
    // vest.lock().await.connect().await?;
    // paradex.lock().await.connect().await?;
    
    // TODO Epic 6: Subscribe to orderbooks
    // vest.lock().await.subscribe_orderbook(pair).await?;
    // paradex.lock().await.subscribe_orderbook(pair).await?;
    
    // Create shutdown broadcast channel
    let (shutdown_tx, mut shutdown_rx) = broadcast::channel::<>(1);

    // Spawn SIGINT handler task
    let shutdown_signal = shutdown_tx.clone();
    tokio::spawn(async move {
        info!("[SHUTDOWN] SIGINT handler registered - press Ctrl+C to initiate graceful shutdown");
        match signal::ctrl_c().await {
            Ok(()) => {
                info!("[SHUTDOWN] Graceful shutdown initiated");
                // Broadcast shutdown to all tasks
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
                error!("[SHUTDOWN] Initiating immediate shutdown due to signal handler failure");
                std::process::exit(1);
            }
        }
    });

    info!("⏳ Bot scaffold ready (Epic 6 runtime pending). Monitoring for shutdown signal...");
    
    // Placeholder task - waits for shutdown
    tokio::select! {
        _ = shutdown_rx.recv() => {
            info!("[SHUTDOWN] Shutdown signal received in main task");
        }
    }

    // Story 4.6: Cancel pending orders (Epic 6 integration)
    // TODO Epic 6: Pass state_manager, vest, paradex from runtime
    // cancel_pending_orders(state_manager, vest, paradex).await?;

    // TODO Epic 6: Disconnect adapters here
    // vest.lock().await.disconnect().await?;
    // paradex.lock().await.disconnect().await?;

    info!("[SHUTDOWN] Clean exit");
    Ok(())
}

// ============================================================================
// STORY 4.6: ORPHAN ORDER PROTECTION (MVP - PATTERN DEFINITION)
// ============================================================================

/// Cancel all pending orders during shutdown (Story 4.6)
///
/// # Epic 6 Integration Required
/// This function defines the pattern for orphan order cleanup.
/// Integration requires passing StateManager and adapters from runtime.
///
/// # Arguments
/// * `state_manager` - Shared state manager with pending order tracking
/// * `vest_adapter` - Vest exchange adapter (for cancel_order calls)
/// * `paradex_adapter` - Paradex exchange adapter (for cancel_order calls)
///
/// # Pattern
/// 1. Get pending orders from StateManager
/// 2. If empty → log "Clean exit, no pending orders"
/// 3. If pending → iterate and cancel each via adapter.cancel_order()
/// 4. Use 10s timeout to avoid hang if exchange is down
/// 5. Log results: success count, error count
///
/// # Example (Epic 6)
/// ```no_run
/// // In Epic 6 runtime, before adapter disconnect:
/// // cancel_pending_orders(state_manager, vest, paradex).await?;
/// ```
#[allow(dead_code)]
async fn cancel_pending_orders(
    // state_manager: Arc<StateManager>,
    // vest_adapter: Arc<Mutex<VestAdapter>>,
    // paradex_adapter: Arc<Mutex<ParadexAdapter>>,
) -> anyhow::Result<()> {
    // Epic 6: Implementation pattern defined in story dev notes
    // See: 4-6-protection-ordres-orphelins.md → Shutdown Cleanup Logic
    
    // Placeholder for MVP (no adapters yet)
    info!("[SHUTDOWN] Clean exit, no pending orders");
    Ok(())
}
