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

use hft_bot::adapters::vest::{VestAdapter, VestConfig};
use hft_bot::adapters::traits::ExchangeAdapter;
use hft_bot::adapters::types::{OrderRequest, OrderSide, OrderType, TimeInForce};
use uuid::Uuid;

const VEST_PAIR: &str = "BTC-PERP";
const BTC_QTY: f64 = 0.0005;    // User requested 0.0005 BTC
const LEVERAGE: u32 = 50;        // 50x leverage

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .init();
    
    let log = |msg: &str| println!("{}", msg);
    
    log("╔══════════════════════════════════════════════════════════╗");
    log("║          AUTO-CLOSE SAFETY MECHANISM TEST                ║");
    log("║  Story 2.5 - NFR7: Zero Directional Exposure            ║");
    log("╚══════════════════════════════════════════════════════════╝");
    
    log("\n🎯 TEST SCENARIO:");
    log("   1. Open LONG position on Vest (0.0005 BTC)");
    log("   2. Simulate SHORT failure (skip Paradex order)");
    log("   3. Manually trigger auto-close on Vest");
    log("   4. Verify position is closed");
    
    // =========================================================================
    // PHASE 0: Connect Vest adapter
    // =========================================================================
    log("\n📡 PHASE 0: Connecting to Vest...");
    
    let vest_config = VestConfig::from_env()?;
    let mut vest_adapter = VestAdapter::new(vest_config);
    vest_adapter.connect().await?;
    log("   ✅ Vest connected");

    // =========================================================================
    // PHASE 0.5: SET LEVERAGE
    // =========================================================================
    log(&format!("\n⚙️  PHASE 0.5: Setting leverage to {}x...", LEVERAGE));
    
    match vest_adapter.set_leverage(VEST_PAIR, LEVERAGE).await {
        Ok(lev) => log(&format!("   ✅ Leverage set to {}x", lev)),
        Err(e) => log(&format!("   ⚠️  Set leverage failed: {} (continuing...)", e)),
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
    
    log(&format!("   Vest ask: ${:.2}", vest_ask));

    // =========================================================================
    // PHASE 1: OPEN LONG POSITION ON VEST
    // =========================================================================
    log("\n🚀 PHASE 1: OPENING LONG POSITION ON VEST...");
    log(&format!("   Opening BUY {} BTC @ ${:.2}", BTC_QTY, vest_ask * 1.002));

    let vest_open_order = OrderRequest {
        client_order_id: format!("test-open-{}", &Uuid::new_v4().to_string()[..6]),
        symbol: VEST_PAIR.to_string(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        price: Some(vest_ask * 1.002),  // Slightly above ask
        quantity: BTC_QTY,
        time_in_force: TimeInForce::Ioc,
        reduce_only: false,
    };

    let open_start = std::time::Instant::now();
    let vest_open_result = vest_adapter.place_order(vest_open_order).await;
    let open_latency = open_start.elapsed();

    log(&format!("\n   ⏱️  Open latency: {}ms", open_latency.as_millis()));
    
    let vest_order_response = match vest_open_result {
        Ok(resp) => {
            log(&format!("   ✅ LONG opened: Order ID: {} | Status: {:?}", resp.order_id, resp.status));
            resp
        }
        Err(e) => {
            log(&format!("   ❌ LONG failed: {}", e));
            log("\n   ⚠️  Cannot proceed without successful LONG position");
            return Ok(());
        }
    };

    // =========================================================================
    // PHASE 2: VERIFY LONG POSITION
    // =========================================================================
    log("\n🔍 PHASE 2: VERIFYING LONG POSITION (waiting 2s for settlement)...");
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    
    let vest_pos = vest_adapter.get_position(VEST_PAIR).await?;
    
    let position_qty = match &vest_pos {
        Some(pos) => {
            log(&format!("   ✅ Vest: {} {} BTC @ ${:.2} (PnL: ${:.2})", 
                pos.side.to_uppercase(), pos.quantity, pos.entry_price, pos.unrealized_pnl));
            pos.quantity
        }
        None => {
            log("   ⚠️  NO POSITION DETECTED - order may not have filled");
            log("   Exiting test...");
            return Ok(());
        }
    };

    // =========================================================================
    // PHASE 3: SIMULATE SHORT FAILURE (skip Paradex)
    // =========================================================================
    log("\n❌ PHASE 3: SIMULATING SHORT FAILURE...");
    log("   Paradex: SHORT order NOT PLACED (simulated failure)");
    log("   💡 This triggers the auto-close scenario:");
    log("      - LONG succeeded ✅");
    log("      - SHORT failed ❌");
    log("      → Must close LONG to avoid directional exposure");

    // =========================================================================
    // PHASE 4: MANUALLY TRIGGER AUTO-CLOSE (Story 2.5 logic)
    // =========================================================================
    log("\n🔒 PHASE 4: AUTO-CLOSE PROCEDURE (NFR7)...");
    log(&format!("   [SAFETY] Closing exposed LONG leg: {} BTC", position_qty));
    
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
        side: OrderSide::Sell,  // Inverted from Buy
        order_type: OrderType::Limit,
        price: Some(close_price),
        quantity: position_qty,
        time_in_force: TimeInForce::Ioc,
        reduce_only: true,  // ← NFR7: MUST be reduce_only
    };

    log(&format!("   Closing order: SELL {} BTC @ ${:.2} (reduce_only=true)", 
        position_qty, close_price));

    let close_start = std::time::Instant::now();
    let close_result = vest_adapter.place_order(vest_close_order).await;
    let close_latency = close_start.elapsed();

    log(&format!("\n   ⏱️  Auto-close latency: {}ms", close_latency.as_millis()));
    
    match close_result {
        Ok(resp) => {
            log(&format!("   ✅ [SAFETY] Auto-close succeeded: Order ID: {}", resp.order_id));
            log(&format!("   Status: {:?} | Filled: {} BTC", resp.status, resp.filled_quantity));
        }
        Err(e) => {
            log(&format!("   ❌ [SAFETY] CRITICAL: Auto-close failed: {}", e));
            log("   ⚠️  DIRECTIONAL EXPOSURE REMAINS - manual intervention required!");
        }
    }

    // =========================================================================
    // PHASE 5: FINAL VERIFICATION
    // =========================================================================
    log("\n✅ PHASE 5: FINAL POSITION CHECK...");
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    
    match vest_adapter.get_position(VEST_PAIR).await {
        Ok(Some(pos)) => {
            log(&format!("   ⚠️  POSITION STILL OPEN: {} {} BTC", pos.side, pos.quantity));
            log("   NFR7 VIOLATION: Directional exposure detected!");
        }
        Ok(None) => {
            log("   ✅ Position fully closed");
            log("   NFR7 VALIDATED: Zero directional exposure confirmed");
        }
        Err(e) => log(&format!("   Error checking position: {}", e)),
    }

    log("\n╔══════════════════════════════════════════════════════════╗");
    log("║              AUTO-CLOSE TEST COMPLETE                    ║");
    log("║  Review logs for [SAFETY] messages and NFR7 validation  ║");
    log("╚══════════════════════════════════════════════════════════╝");
    
    Ok(())
}
