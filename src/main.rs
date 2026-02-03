//! HFT Arbitrage Bot - MVP Entry Point
//!
//! This is a minimal implementation that:
//! 1. Loads configuration
//! 2. Connects to Vest and Paradex
//! 3. Subscribes to orderbooks
//! 4. Calculates spreads using VWAP
//! 5. Logs opportunities to console

use std::path::Path;
use std::sync::Arc;
use tokio::signal;
use tokio::sync::{broadcast, Mutex, mpsc};
use tracing::{info, error};
use hft_bot::config;
use hft_bot::core::state::{PositionState, StateManager};
use hft_bot::adapters::ExchangeAdapter;
use hft_bot::adapters::vest::{VestAdapter, VestConfig};
use hft_bot::adapters::paradex::{ParadexAdapter, ParadexConfig};
use hft_bot::core::channels::SpreadOpportunity;
use hft_bot::core::monitoring::{monitoring_task, MonitoringConfig};
use hft_bot::core::runtime::execution_task;
use hft_bot::core::execution::DeltaNeutralExecutor;
use hft_bot::core::position_monitor::{position_monitoring_task, PositionMonitoringConfig};


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
    info!("   Position Size: {} {}", bot.position_size, bot.pair);
    
    
    // Story 6.1 Task 1: Create adapters with real credentials
    info!("🔐 Initializing exchange adapters...");
    
    // Subtask 1.1: Create VestAdapter with credentials from .env
    let vest_config = VestConfig::from_env()
        .expect("VEST credentials must be configured in .env (VEST_PRIMARY_ADDR, VEST_PRIMARY_KEY, VEST_SIGNING_KEY)");
    let vest_adapter = VestAdapter::new(vest_config);
    info!("[INFO] Vest adapter initialized");
    
    // Subtask 1.2: Create ParadexAdapter with credentials from .env
    let paradex_config = ParadexConfig::from_env()
        .expect("PARADEX credentials must be configured in .env (PARADEX_PRIVATE_KEY, PARADEX_ACCOUNT_ADDRESS)");
    let paradex_adapter = ParadexAdapter::new(paradex_config);
    info!("[INFO] Paradex adapter initialized");
    
    // Subtask 1.3: Wrap adapters in Arc<Mutex<>> for shared ownership
    let vest = Arc::new(Mutex::new(vest_adapter));
    let paradex = Arc::new(Mutex::new(paradex_adapter));
    
    // Subtask 1.4: Create StateManager with Supabase config
    let supabase_config = hft_bot::config::SupabaseConfig {
        url: std::env::var("SUPABASE_URL")
            .unwrap_or_else(|_| "https://placeholder.supabase.co".to_string()),
        anon_key: std::env::var("SUPABASE_ANON_KEY")
            .unwrap_or_else(|_| "placeholder-key".to_string()),
        enabled: std::env::var("SUPABASE_ENABLED")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false),  // Disabled by default if not set
    };
    let state_manager = Arc::new(StateManager::new(supabase_config));

    
    // Task 2: Create channels for data pipeline
    
    // Subtask 2.1: Create spread_opportunity channel (Story 6.2)
    // Channel capacity of 1 ensures only one opportunity at a time
    // New opportunities are dropped if execution is in progress (see try_send in monitoring)
    let (opportunity_tx, opportunity_rx) = mpsc::channel::<SpreadOpportunity>(1);
    
    // Task 3: Connect to exchanges and restore state
    info!("🌐 Connecting to exchanges...");
    
    // Subtask 3.1: Connect to both exchanges (sequential for borrow checker)
    vest.lock().await.connect().await
        .expect("Failed to connect to Vest");
    info!("[INFO] Connected to Vest");
    
    paradex.lock().await.connect().await
        .expect("Failed to connect to Paradex");
    info!("[INFO] Connected to Paradex");

    
    // Subtask 3.2: Restore positions from Supabase
    match state_manager.load_positions().await {
        Ok(positions) => {
            if !positions.is_empty() {
                info!("[STATE] Restored {} positions from database", positions.len());
            } else {
                info!("[STATE] Restored 0 positions from database");
            }
        }
        Err(e) => {
            error!("[STATE] Failed to load positions: {}. Continuing with empty state.", e);
        }
    }
    
    // Subtask 3.3: Subscribe to orderbooks
    // Map symbols: BTC-PERP for Vest, BTC-USD-PERP for Paradex
    let vest_symbol = bot.pair.to_string();  // e.g., "BTC-PERP"
    let paradex_symbol = format!("{}-USD-PERP",
        bot.pair.to_string().split('-').next().unwrap_or("BTC")
    );  // e.g., "BTC-USD-PERP"
    
    vest.lock().await.subscribe_orderbook(&vest_symbol).await
        .expect("Failed to subscribe to Vest orderbook");
    paradex.lock().await.subscribe_orderbook(&paradex_symbol).await
        .expect("Failed to subscribe to Paradex orderbook");
    
    info!("[INFO] Subscribed to orderbooks: {}, {}", vest_symbol, paradex_symbol);

    // Create shutdown broadcast channel (moved up from Task 5 for Story 6.2)
    let (shutdown_tx, mut shutdown_rx) = broadcast::channel::<>(1);
    
    // IMPORTANT: Connect execution adapters BEFORE spawning monitoring task
    // This prevents race conditions where spreads are detected before executor is ready
    info!("🔌 Initializing execution adapters...");
    
    // Task 3 (Story 6.2): Create DeltaNeutralExecutor with separate adapter instances
    // Note: Execution adapters are separate from monitoring adapters
    // MVP: Use from_env() to create new instances with same credentials
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
        bot.position_size,  // Position size from config (e.g., 0.001 BTC)
        vest_symbol.clone(),
        paradex_symbol.clone(),
    );
    
    // Story 6.3: Create channel for new positions (execution -> monitoring)
    let (new_position_tx, new_position_rx) = mpsc::channel::<PositionState>(10);
    
    // Now spawn execution_task first (it will wait for opportunities from channel)
    let execution_shutdown = shutdown_tx.subscribe();
    let exec_state_manager = state_manager.clone();
    tokio::spawn(async move {
        execution_task(
            opportunity_rx,
            executor,
            exec_state_manager,
            Some(new_position_tx),  // Story 6.3: Send new positions to monitor
            execution_shutdown,
        ).await;
    });
    info!("[INFO] Execution task spawned");
    
    // NOW spawn monitoring task - execution is ready to receive opportunities
    let monitoring_vest = Arc::clone(&vest);
    let monitoring_paradex = Arc::clone(&paradex);
    let monitoring_tx = opportunity_tx.clone();
    let monitoring_vest_symbol = vest_symbol.clone();
    let monitoring_paradex_symbol = paradex_symbol.clone();
    let monitoring_config = MonitoringConfig {
        pair: bot.pair.to_string(),
        spread_entry: bot.spread_entry,
    };
    let monitoring_shutdown = shutdown_tx.subscribe();
    
    tokio::spawn(async move {
        monitoring_task(
            monitoring_vest,
            monitoring_paradex,
            monitoring_tx,
            monitoring_vest_symbol,
            monitoring_paradex_symbol,
            monitoring_config,
            monitoring_shutdown,
        ).await;
    });
    info!("[INFO] Monitoring task spawned");
    
    // Story 6.3: Spawn position_monitoring_task for automatic exit
    let monitor_config = PositionMonitoringConfig::new(
        bot.spread_exit,  // Exit threshold from config
        vest_symbol.clone(),
        paradex_symbol.clone(),
    );
    let monitor_vest = Arc::clone(&vest);
    let monitor_paradex = Arc::clone(&paradex);
    let monitor_state_manager = state_manager.clone();
    let monitor_shutdown = shutdown_tx.subscribe();
    tokio::spawn(async move {
        position_monitoring_task(
            monitor_vest,
            monitor_paradex,
            monitor_state_manager,
            new_position_rx,
            monitor_config,
            monitor_shutdown,
        ).await;
    });
    info!("[INFO] Position monitoring task spawned");
    
    info!("✅ Bot runtime started with automatic spread monitoring and execution");

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

    // Shutdown signal received - begin graceful shutdown
    tokio::select! {
        _ = shutdown_rx.recv() => {
            info!("[SHUTDOWN] Shutdown signal received in main task");
        }
    }

    // Task 5: Graceful shutdown integration
    // Subtask 5.1: Cancel pending orders (Story 4.6 pattern integration)
    info!("[SHUTDOWN] Cancelling pending orders...");
    if let Err(e) = cancel_pending_orders(
        Arc::clone(&state_manager),
        Arc::clone(&vest),
        Arc::clone(&paradex),
    ).await {
        error!("[SHUTDOWN] Failed to cancel pending orders: {}", e);
    }
    
    // Subtask 5.2: Disconnect from exchanges
    info!("[SHUTDOWN] Disconnecting from exchanges...");
    vest.lock().await.disconnect().await
        .unwrap_or_else(|e| error!("[SHUTDOWN] Failed to disconnect from Vest: {}", e));
    paradex.lock().await.disconnect().await
        .unwrap_or_else(|e| error!("[SHUTDOWN] Failed to disconnect from Paradex: {}", e));
    info!("[SHUTDOWN] Disconnected from exchanges");

    info!("[SHUTDOWN] Clean exit");

    Ok(())
}

// ============================================================================
// STORY 6.1 TASK 5.1: ORPHAN ORDER PROTECTION (SHUTDOWN INTEGRATION)
// ============================================================================

/// Cancel all pending orders during shutdown (Story 6.1 - Task 5.1)
///
/// # Integration Status
/// ✅ Story 6.1: Fully implemented shutdown integration
///
/// # Arguments
/// * `state_manager` - Shared state manager with pending order tracking
/// * `vest_adapter` - Vest exchange adapter (for cancel_order calls)
/// * `paradex_adapter` - Paradex exchange adapter (for cancel_order calls)
///
/// # Implementation Logic
/// 1. Get pending orders from StateManager
/// 2. If empty → log "[SHUTDOWN] Clean exit, no pending orders"
/// 3. If pending → log "[SHUTDOWN] Found N pending orders, cancelling..."
/// 4. Iterate and cancel each via adapter.cancel_order()
/// 5. Use 10s timeout to avoid hang if exchange is down
/// 6. On success: log "[SHUTDOWN] Cancelled N pending orders successfully"
/// 7. On timeout: log "[SHUTDOWN] Cancel timeout exceeded (10s) - cancelled M/N orders, proceeding with exit"
/// 8. On partial failure: log "[SHUTDOWN] Cancelled M/N orders, X errors"
///
/// # Returns
/// Ok(()) on success (including no orders to cancel)
/// Err on critical failure (logged but doesn't block shutdown)
async fn cancel_pending_orders(
    state_manager: Arc<StateManager>,
    vest_adapter: Arc<Mutex<VestAdapter>>,
    paradex_adapter: Arc<Mutex<ParadexAdapter>>,
) -> anyhow::Result<()> {
    use tokio::time::{timeout, Duration};
    
    // Get pending orders from StateManager
    let pending_orders = state_manager.get_pending_orders().await;
    
    if pending_orders.is_empty() {
        info!("[SHUTDOWN] Clean exit, no pending orders");
        return Ok(());
    }
    
    let total_orders = pending_orders.len();
    info!("[SHUTDOWN] Found {} pending orders, cancelling...", total_orders);
    
    let mut cancelled_count = 0;
    let mut error_count = 0;
    
    // 10s timeout for all cancellations
    let cancel_result = timeout(Duration::from_secs(10), async {
        for order in pending_orders {
            let cancel_result = match order.exchange.as_str() {
                "vest" => {
                    vest_adapter.lock().await.cancel_order(&order.order_id).await
                }
                "paradex" => {
                    paradex_adapter.lock().await.cancel_order(&order.order_id).await
                }
                unknown => {
                    error!("[SHUTDOWN] Unknown exchange: {}", unknown);
                    error_count += 1;
                    continue;
                }
            };
            
            match cancel_result {
                Ok(_) => {
                    info!("[SHUTDOWN] Cancelled order {} on {}", order.order_id, order.exchange);
                    cancelled_count += 1;
                }
                Err(e) => {
                    error!("[SHUTDOWN] Failed to cancel order {}: {}", order.order_id, e);
                    error_count += 1;
                }
            }
        }
    }).await;
    
    // Log final result
    match cancel_result {
        Ok(_) => {
            if error_count == 0 {
                info!("[SHUTDOWN] Cancelled {} pending orders successfully", cancelled_count);
            } else {
                info!("[SHUTDOWN] Cancelled {}/{} orders, {} errors", cancelled_count, total_orders, error_count);
            }
        }
        Err(_) => {
            info!("[SHUTDOWN] Cancel timeout exceeded (10s) - cancelled {}/{} orders, proceeding with exit", cancelled_count, total_orders);
        }
    }
    
    Ok(())
}
