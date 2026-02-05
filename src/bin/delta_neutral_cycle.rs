//! Delta-Neutral Full Cycle Test
//! 1. Open positions (LONG Vest + SHORT Paradex)
//! 2. Verify positions are active via get_position
//! 3. Close both positions simultaneously
//!
//! # Logging (Story 5.1)
//! - Uses LOG_FORMAT env var: `json` (default) or `pretty`

use hft_bot::adapters::paradex::{ParadexAdapter, ParadexConfig};
use hft_bot::adapters::vest::{VestAdapter, VestConfig};
use hft_bot::adapters::traits::ExchangeAdapter;
use hft_bot::adapters::types::{OrderRequest, OrderSide, OrderType, TimeInForce};
use hft_bot::config;
use uuid::Uuid;

const VEST_PAIR: &str = "BTC-PERP";
const PARADEX_PAIR: &str = "BTC-USD-PERP";
const BTC_QTY: f64 = 0.005;     // User requested 0.005 BTC
const LEVERAGE: u32 = 50;       // 50x leverage

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    // Initialize logging (Story 5.1: JSON/Pretty configurable via LOG_FORMAT)
    config::init_logging();
    
    let log = |msg: &str| println!("{}", msg);
    
    log("╔══════════════════════════════════════════════════════════╗");
    log("║       DELTA-NEUTRAL FULL CYCLE TEST                      ║");
    log("║  Open → Verify → Close                                   ║");
    log("╚══════════════════════════════════════════════════════════╝");
    
    // =========================================================================
    // SETUP: Connect both adapters
    // =========================================================================
    log("\n📡 PHASE 0: Connecting adapters...");
    
    let vest_config = VestConfig::from_env()?;
    let paradex_config = ParadexConfig::from_env()?;
    let mut vest_adapter = VestAdapter::new(vest_config);
    let mut paradex_adapter = ParadexAdapter::new(paradex_config);
    
    let (vest_conn, paradex_conn) = tokio::join!(
        vest_adapter.connect(),
        paradex_adapter.connect()
    );
    vest_conn?;
    paradex_conn?;
    log("   ✅ Both adapters connected");

    // =========================================================================
    // PHASE 0.5: SET LEVERAGE (50x on both exchanges)
    // =========================================================================
    log(&format!("\n⚙️  PHASE 0.5: Setting leverage to {}x on both exchanges...", LEVERAGE));
    
    let vest_lev_result = vest_adapter.set_leverage(VEST_PAIR, LEVERAGE).await;
    let paradex_lev_result = paradex_adapter.set_leverage(PARADEX_PAIR, LEVERAGE).await;
    
    match &vest_lev_result {
        Ok(lev) => log(&format!("   Vest: ✅ Leverage set to {}x", lev)),
        Err(e) => log(&format!("   Vest: ⚠️  Set leverage failed: {} (continuing...)", e)),
    }
    match &paradex_lev_result {
        Ok(lev) => log(&format!("   Paradex: ✅ Leverage set to {}x", lev)),
        Err(e) => log(&format!("   Paradex: ⚠️  Set leverage failed: {} (continuing...)", e)),
    }

    // Subscribe to orderbooks for pricing
    let (vest_sub, paradex_sub) = tokio::join!(
        vest_adapter.subscribe_orderbook(VEST_PAIR),
        paradex_adapter.subscribe_orderbook(PARADEX_PAIR)
    );
    vest_sub?;
    paradex_sub?;
    
    // Wait for orderbook data
    for _ in 0..3 {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        vest_adapter.sync_orderbooks().await;
        paradex_adapter.sync_orderbooks().await;
    }

    // Get current prices
    let vest_ask = vest_adapter.get_orderbook(VEST_PAIR)
        .and_then(|ob| ob.asks.first().map(|l| l.price))
        .unwrap_or(0.0);
    let paradex_bid = paradex_adapter.get_orderbook(PARADEX_PAIR)
        .and_then(|ob| ob.bids.first().map(|l| l.price))
        .unwrap_or(0.0);
    
    log(&format!("   Vest ask: ${:.2} | Paradex bid: ${:.2}", vest_ask, paradex_bid));

    // =========================================================================
    // PHASE 1: OPEN DELTA-NEUTRAL POSITIONS
    // =========================================================================
    log("\n🚀 PHASE 1: OPENING DELTA-NEUTRAL POSITIONS...");
    log(&format!("   Vest: BUY {} BTC (LONG)", BTC_QTY));
    log(&format!("   Paradex: SELL {} BTC (SHORT)", BTC_QTY));

    let vest_open_order = OrderRequest {
        client_order_id: format!("open-vest-{}", &Uuid::new_v4().to_string()[..6]),
        symbol: VEST_PAIR.to_string(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        price: Some(vest_ask * 1.002),  // Slightly above ask
        quantity: BTC_QTY,
        time_in_force: TimeInForce::Ioc,
        reduce_only: false,
    };

    let paradex_open_order = OrderRequest {
        client_order_id: format!("open-pdx-{}", &Uuid::new_v4().to_string()[..6]),
        symbol: PARADEX_PAIR.to_string(),
        side: OrderSide::Sell,
        order_type: OrderType::Market,
        price: None,
        quantity: BTC_QTY,
        time_in_force: TimeInForce::Ioc,
        reduce_only: false,
    };

    let open_start = std::time::Instant::now();
    let (vest_open_result, paradex_open_result) = tokio::join!(
        vest_adapter.place_order(vest_open_order),
        paradex_adapter.place_order(paradex_open_order)
    );
    let open_latency = open_start.elapsed();

    log(&format!("\n   ⏱️  Open latency: {}ms", open_latency.as_millis()));
    
    match &vest_open_result {
        Ok(resp) => log(&format!("   Vest OPEN: ✅ Order ID: {} | Status: {:?}", resp.order_id, resp.status)),
        Err(e) => log(&format!("   Vest OPEN: ❌ {}", e)),
    }
    match &paradex_open_result {
        Ok(resp) => log(&format!("   Paradex OPEN: ✅ Order ID: {} | Status: {:?}", resp.order_id, resp.status)),
        Err(e) => log(&format!("   Paradex OPEN: ❌ {}", e)),
    }

    // =========================================================================
    // PHASE 2: VERIFY POSITIONS
    // =========================================================================
    log("\n🔍 PHASE 2: VERIFYING POSITIONS (waiting 2s for settlement)...");
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    
    let vest_pos = vest_adapter.get_position(VEST_PAIR).await?;
    let paradex_pos = paradex_adapter.get_position(PARADEX_PAIR).await?;
    
    log("\n   Position Status:");
    let vest_has_position = match &vest_pos {
        Some(pos) => {
            log(&format!("   Vest:    ✅ {} {} BTC @ ${:.2} (PnL: ${:.2})", 
                pos.side.to_uppercase(), pos.quantity, pos.entry_price, pos.unrealized_pnl));
            true
        }
        None => {
            log("   Vest:    ⚠️  NO POSITION DETECTED");
            false
        }
    };
    
    let paradex_has_position = match &paradex_pos {
        Some(pos) => {
            log(&format!("   Paradex: ✅ {} {} BTC @ ${:.2} (PnL: ${:.2})", 
                pos.side.to_uppercase(), pos.quantity, pos.entry_price, pos.unrealized_pnl));
            true
        }
        None => {
            log("   Paradex: ⚠️  NO POSITION DETECTED");
            false
        }
    };

    if !vest_has_position && !paradex_has_position {
        log("\n   ⚠️  No positions to close. Orders may not have filled.");
        log("=== TEST COMPLETE ===");
        return Ok(());
    }

    // =========================================================================
    // PHASE 3: CLOSE POSITIONS
    // =========================================================================
    log("\n🔒 PHASE 3: CLOSING POSITIONS...");

    // Get fresh prices for closing
    vest_adapter.sync_orderbooks().await;
    paradex_adapter.sync_orderbooks().await;
    
    let vest_bid = vest_adapter.get_orderbook(VEST_PAIR)
        .and_then(|ob| ob.bids.first().map(|l| l.price))
        .unwrap_or(0.0);

    let vest_close_future = async {
        if let Some(pos) = vest_pos {
            let close_side = if pos.side.to_lowercase() == "long" { OrderSide::Sell } else { OrderSide::Buy };
            // For SELL: use bid price * 0.998 (aggressive, cross the spread)
            // For BUY: use ask price * 1.002
            let close_price = if close_side == OrderSide::Sell {
                vest_bid * 0.998
            } else {
                vest_ask * 1.002
            };
            let order = OrderRequest {
                client_order_id: format!("close-vest-{}", &Uuid::new_v4().to_string()[..6]),
                symbol: VEST_PAIR.to_string(),
                side: close_side,
                order_type: OrderType::Limit,  // LIMIT order for Vest
                price: Some(close_price),
                quantity: pos.quantity,
                time_in_force: TimeInForce::Ioc,
                reduce_only: true,
            };
            log(&format!("   Vest: Closing {} {} BTC @ ${:.2}", pos.side, pos.quantity, close_price));
            Some(vest_adapter.place_order(order).await)
        } else {
            None
        }
    };

    let paradex_close_future = async {
        if let Some(pos) = paradex_pos {
            let close_side = if pos.side.to_lowercase() == "short" { OrderSide::Buy } else { OrderSide::Sell };
            let order = OrderRequest {
                client_order_id: format!("close-pdx-{}", &Uuid::new_v4().to_string()[..6]),
                symbol: PARADEX_PAIR.to_string(),
                side: close_side,
                order_type: OrderType::Market,
                price: None,
                quantity: pos.quantity,
                time_in_force: TimeInForce::Ioc,
                reduce_only: true,
            };
            log(&format!("   Paradex: Closing {} {} BTC", pos.side, pos.quantity));
            Some(paradex_adapter.place_order(order).await)
        } else {
            None
        }
    };

    let close_start = std::time::Instant::now();
    let (vest_close_result, paradex_close_result) = tokio::join!(vest_close_future, paradex_close_future);
    let close_latency = close_start.elapsed();

    log(&format!("\n   ⏱️  Close latency: {}ms", close_latency.as_millis()));
    
    match vest_close_result {
        Some(Ok(resp)) => log(&format!("   Vest CLOSE: ✅ Order ID: {}", resp.order_id)),
        Some(Err(e)) => log(&format!("   Vest CLOSE: ❌ {}", e)),
        None => log("   Vest CLOSE: ⏭️  Skipped (no position)"),
    }
    match paradex_close_result {
        Some(Ok(resp)) => log(&format!("   Paradex CLOSE: ✅ Order ID: {}", resp.order_id)),
        Some(Err(e)) => log(&format!("   Paradex CLOSE: ❌ {}", e)),
        None => log("   Paradex CLOSE: ⏭️  Skipped (no position)"),
    }

    // =========================================================================
    // PHASE 4: FINAL VERIFICATION
    // =========================================================================
    log("\n✅ PHASE 4: FINAL POSITION CHECK...");
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    
    match vest_adapter.get_position(VEST_PAIR).await {
        Ok(Some(pos)) => log(&format!("   Vest: ⚠️  Still have {} {} BTC", pos.side, pos.quantity)),
        Ok(None) => log("   Vest: ✅ Position closed"),
        Err(e) => log(&format!("   Vest: Error - {}", e)),
    }
    match paradex_adapter.get_position(PARADEX_PAIR).await {
        Ok(Some(pos)) => log(&format!("   Paradex: ⚠️  Still have {} {} BTC", pos.side, pos.quantity)),
        Ok(None) => log("   Paradex: ✅ Position closed"),
        Err(e) => log(&format!("   Paradex: Error - {}", e)),
    }

    log("\n╔══════════════════════════════════════════════════════════╗");
    log("║                    TEST COMPLETE                         ║");
    log("╚══════════════════════════════════════════════════════════╝");
    Ok(())
}
