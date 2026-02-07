//! Test Auto-Close Mechanism (Story 2.5 NFR7)
//!
//! This binary tests the auto-close safety mechanism by:
//! 1. Opening a LONG position on Vest (0.0005 BTC)
//! 2. Simulating a SHORT failure on Paradex
//! 3. Verifying that the LONG position is automatically closed via reduce_only
//!
//! Expected behavior:
//! - Vest LONG opens successfully
//! - Paradex SHORT simulated failure (we skip the order)
//! - Auto-close mechanism triggers
//! - Vest position is closed with reduce_only=true
//! - Logs show [SAFETY] messages
//!
//! # Logging (Story 5.1)
//! - Uses LOG_FORMAT env var: `json` (default) or `pretty`

use hft_bot::adapters::vest::{VestAdapter, VestConfig};
use hft_bot::adapters::traits::ExchangeAdapter;
use hft_bot::adapters::types::{OrderRequest, OrderSide, OrderType, TimeInForce};
use hft_bot::config;
use tracing::{info, warn, error};
use uuid::Uuid;

const VEST_PAIR: &str = "BTC-PERP";
const BTC_QTY: f64 = 0.0005;    // User requested 0.0005 BTC
const LEVERAGE: u32 = 50;        // 50x leverage

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    // Initialize logging (Story 5.1: JSON/Pretty configurable via LOG_FORMAT)
    config::init_logging();
    
    info!("╔══════════════════════════════════════════════════════════╗");
    info!("║          AUTO-CLOSE SAFETY MECHANISM TEST                ║");
    info!("║  Story 2.5 - NFR7: Zero Directional Exposure            ║");
    info!("╚══════════════════════════════════════════════════════════╝");
    
    info!("TEST SCENARIO:");
    info!("  1. Open LONG position on Vest (0.0005 BTC)");
    info!("  2. Simulate SHORT failure (skip Paradex order)");
    info!("  3. Manually trigger auto-close on Vest");
    info!("  4. Verify position is closed");
    
    // =========================================================================
    // PHASE 0: Connect Vest adapter
    // =========================================================================
    info!("PHASE 0: Connecting to Vest...");
    
    let vest_config = VestConfig::from_env()?;
    let mut vest_adapter = VestAdapter::new(vest_config);
    vest_adapter.connect().await?;
    info!(exchange = "vest", "Connected");

    // =========================================================================
    // PHASE 0.5: SET LEVERAGE
    // =========================================================================
    info!(leverage = LEVERAGE, "PHASE 0.5: Setting leverage...");
    
    match vest_adapter.set_leverage(VEST_PAIR, LEVERAGE).await {
        Ok(lev) => info!(leverage = lev, "Leverage set"),
        Err(e) => warn!(error = %e, "Set leverage failed (continuing)"),
    }

    // Subscribe to orderbook and wait for data
    vest_adapter.subscribe_orderbook(VEST_PAIR).await?;
    
    for _ in 0..3 {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        vest_adapter.sync_orderbooks().await;
    }

    // Get current ask price
    let vest_ask = vest_adapter.get_orderbook(VEST_PAIR)
        .and_then(|ob| ob.asks.first().map(|l| l.price))
        .unwrap_or(0.0);
    
    info!(vest_ask = vest_ask, "Current ask price");

    // =========================================================================
    // PHASE 1: OPEN LONG POSITION ON VEST
    // =========================================================================
    info!("PHASE 1: OPENING LONG POSITION ON VEST...");
    let limit_price = vest_ask * 1.002;
    info!(side = "BUY", qty = BTC_QTY, price = limit_price, "Opening position");

    let vest_open_order = OrderRequest {
        client_order_id: format!("test-open-{}", &Uuid::new_v4().to_string()[..6]),
        symbol: VEST_PAIR.to_string(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        price: Some(limit_price),
        quantity: BTC_QTY,
        time_in_force: TimeInForce::Ioc,
        reduce_only: false,
    };

    let open_start = std::time::Instant::now();
    let vest_open_result = vest_adapter.place_order(vest_open_order).await;
    let open_latency = open_start.elapsed();

    info!(latency_ms = open_latency.as_millis() as u64, "Open latency");
    
    let vest_order_response = match vest_open_result {
        Ok(resp) => {
            info!(order_id = %resp.order_id, status = ?resp.status, "LONG opened");
            resp
        }
        Err(e) => {
            error!(error = %e, "LONG failed");
            warn!("Cannot proceed without successful LONG position");
            return Ok(());
        }
    };

    // =========================================================================
    // PHASE 2: VERIFY LONG POSITION
    // =========================================================================
    info!("PHASE 2: VERIFYING LONG POSITION (waiting 2s for settlement)...");
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    
    let vest_pos = vest_adapter.get_position(VEST_PAIR).await?;
    
    let position_qty = match &vest_pos {
        Some(pos) => {
            info!(side = %pos.side.to_uppercase(), qty = pos.quantity, 
                  entry_price = pos.entry_price, pnl = pos.unrealized_pnl, "Position verified");
            pos.quantity
        }
        None => {
            warn!("NO POSITION DETECTED - order may not have filled");
            info!("Exiting test...");
            return Ok(());
        }
    };

    // =========================================================================
    // PHASE 3: SIMULATE SHORT FAILURE (skip Paradex)
    // =========================================================================
    info!("PHASE 3: SIMULATING SHORT FAILURE...");
    info!("Paradex: SHORT order NOT PLACED (simulated failure)");
    info!("Auto-close scenario: LONG succeeded, SHORT failed → must close LONG");

    // =========================================================================
    // PHASE 4: MANUALLY TRIGGER AUTO-CLOSE (Story 2.5 logic)
    // =========================================================================
    info!("PHASE 4: AUTO-CLOSE PROCEDURE (NFR7)...");
    info!(qty = position_qty, "[SAFETY] Closing exposed LONG leg");
    
    // Get fresh bid price for closing
    vest_adapter.sync_orderbooks().await;
    let vest_bid = vest_adapter.get_orderbook(VEST_PAIR)
        .and_then(|ob| ob.bids.first().map(|l| l.price))
        .unwrap_or(0.0);

    // Create closing order (inverts Buy → Sell, sets reduce_only=true)
    let close_price = vest_bid * 0.998;  // Aggressive price to ensure fill
    let vest_close_order = OrderRequest {
        client_order_id: format!("auto-close-{}-{}", 
            vest_order_response.order_id, 
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ),
        symbol: VEST_PAIR.to_string(),
        side: OrderSide::Sell,
        order_type: OrderType::Limit,
        price: Some(close_price),
        quantity: position_qty,
        time_in_force: TimeInForce::Ioc,
        reduce_only: true,  // ← NFR7: MUST be reduce_only
    };

    info!(side = "SELL", qty = position_qty, price = close_price, reduce_only = true, "Closing order");

    let close_start = std::time::Instant::now();
    let close_result = vest_adapter.place_order(vest_close_order).await;
    let close_latency = close_start.elapsed();

    info!(latency_ms = close_latency.as_millis() as u64, "Auto-close latency");
    
    match close_result {
        Ok(resp) => {
            info!(order_id = %resp.order_id, status = ?resp.status, filled = resp.filled_quantity, 
                  "[SAFETY] Auto-close succeeded");
        }
        Err(e) => {
            error!(error = %e, "[SAFETY] CRITICAL: Auto-close failed");
            error!("DIRECTIONAL EXPOSURE REMAINS - manual intervention required!");
        }
    }

    // =========================================================================
    // PHASE 5: FINAL VERIFICATION
    // =========================================================================
    info!("PHASE 5: FINAL POSITION CHECK...");
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    
    match vest_adapter.get_position(VEST_PAIR).await {
        Ok(Some(pos)) => {
            error!(side = %pos.side, qty = pos.quantity, "POSITION STILL OPEN - NFR7 VIOLATION");
        }
        Ok(None) => {
            info!("Position fully closed");
            info!("NFR7 VALIDATED: Zero directional exposure confirmed");
        }
        Err(e) => error!(error = %e, "Error checking position"),
    }

    info!("╔══════════════════════════════════════════════════════════╗");
    info!("║              AUTO-CLOSE TEST COMPLETE                    ║");
    info!("║  Review logs for [SAFETY] messages and NFR7 validation  ║");
    info!("╚══════════════════════════════════════════════════════════╝");
    
    Ok(())
}
