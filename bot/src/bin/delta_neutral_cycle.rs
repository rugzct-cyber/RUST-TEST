//! Delta-Neutral Full Cycle Test
//! 1. Open positions (LONG Vest + SHORT Paradex)
//! 2. Verify positions are active via get_position
//! 3. Close both positions simultaneously
//!
//! # Logging (Story 5.1)
//! - Uses LOG_FORMAT env var: `json` (default) or `pretty`

use std::path::Path;
use hft_bot::adapters::paradex::{ParadexAdapter, ParadexConfig};
use hft_bot::adapters::vest::{VestAdapter, VestConfig};
use hft_bot::adapters::traits::ExchangeAdapter;
use hft_bot::adapters::types::{OrderRequest, OrderSide, OrderType, TimeInForce};
use hft_bot::config;
use tracing::{info, warn, error};
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    // Initialize logging (Story 5.1: JSON/Pretty configurable via LOG_FORMAT)
    config::init_logging();
    
    // Load config from config.yaml (same source of truth as main bot)
    let cfg = config::load_config(Path::new("config.yaml"))
        .expect("Failed to load config.yaml");
    let bot = &cfg.bots[0];
    let vest_pair = bot.pair.to_string();
    let paradex_pair = format!("{}-USD-PERP",
        vest_pair.split('-').next().unwrap_or("BTC"));
    let qty = bot.position_size;
    let leverage = bot.leverage as u32;
    
    info!("╔══════════════════════════════════════════════════════════╗");
    info!("║       DELTA-NEUTRAL FULL CYCLE TEST                      ║");
    info!("║  Open → Verify → Close                                   ║");
    info!("╚══════════════════════════════════════════════════════════╝");
    info!(vest = %vest_pair, paradex = %paradex_pair, qty = qty, leverage = leverage, "Test configuration");
    
    // =========================================================================
    // SETUP: Connect both adapters
    // =========================================================================
    info!("PHASE 0: Connecting adapters...");
    
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
    info!("Both adapters connected");

    // =========================================================================
    // PHASE 0.5: SET LEVERAGE
    // =========================================================================
    info!(leverage = leverage, "PHASE 0.5: Setting leverage on both exchanges...");
    
    let vest_lev_result = vest_adapter.set_leverage(&vest_pair, leverage).await;
    let paradex_lev_result = paradex_adapter.set_leverage(&paradex_pair, leverage).await;
    
    match &vest_lev_result {
        Ok(lev) => info!(exchange = "vest", leverage = lev, "Leverage set"),
        Err(e) => warn!(exchange = "vest", error = %e, "Set leverage failed (continuing)"),
    }
    match &paradex_lev_result {
        Ok(lev) => info!(exchange = "paradex", leverage = lev, "Leverage set"),
        Err(e) => warn!(exchange = "paradex", error = %e, "Set leverage failed (continuing)"),
    }

    // Subscribe to orderbooks for pricing
    let (vest_sub, paradex_sub) = tokio::join!(
        vest_adapter.subscribe_orderbook(&vest_pair),
        paradex_adapter.subscribe_orderbook(&paradex_pair)
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
    let vest_ask = vest_adapter.get_orderbook(&vest_pair)
        .and_then(|ob| ob.asks.first().map(|l| l.price))
        .unwrap_or(0.0);
    let paradex_bid = paradex_adapter.get_orderbook(&paradex_pair)
        .and_then(|ob| ob.bids.first().map(|l| l.price))
        .unwrap_or(0.0);
    
    info!(vest_ask = vest_ask, paradex_bid = paradex_bid, "Current prices");

    // =========================================================================
    // PHASE 1: OPEN DELTA-NEUTRAL POSITIONS
    // =========================================================================
    info!("PHASE 1: OPENING DELTA-NEUTRAL POSITIONS...");
    info!(exchange = "vest", side = "BUY", qty = qty, "Opening LONG");
    info!(exchange = "paradex", side = "SELL", qty = qty, "Opening SHORT");

    let vest_open_order = OrderRequest {
        client_order_id: format!("open-vest-{}", &Uuid::new_v4().to_string()[..6]),
        symbol: vest_pair.clone(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        price: Some(vest_ask * 1.002),  // Slightly above ask
        quantity: qty,
        time_in_force: TimeInForce::Ioc,
        reduce_only: false,
    };

    let paradex_open_order = OrderRequest {
        client_order_id: format!("open-pdx-{}", &Uuid::new_v4().to_string()[..6]),
        symbol: paradex_pair.clone(),
        side: OrderSide::Sell,
        order_type: OrderType::Market,
        price: None,
        quantity: qty,
        time_in_force: TimeInForce::Ioc,
        reduce_only: false,
    };

    let open_start = std::time::Instant::now();
    let (vest_open_result, paradex_open_result) = tokio::join!(
        vest_adapter.place_order(vest_open_order),
        paradex_adapter.place_order(paradex_open_order)
    );
    let open_latency = open_start.elapsed();

    info!(latency_ms = open_latency.as_millis() as u64, "Open latency");
    
    match &vest_open_result {
        Ok(resp) => info!(exchange = "vest", order_id = %resp.order_id, status = ?resp.status, "OPEN succeeded"),
        Err(e) => error!(exchange = "vest", error = %e, "OPEN failed"),
    }
    match &paradex_open_result {
        Ok(resp) => info!(exchange = "paradex", order_id = %resp.order_id, status = ?resp.status, "OPEN succeeded"),
        Err(e) => error!(exchange = "paradex", error = %e, "OPEN failed"),
    }

    // =========================================================================
    // PHASE 2: VERIFY POSITIONS
    // =========================================================================
    info!("PHASE 2: VERIFYING POSITIONS (waiting 2s for settlement)...");
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    
    let vest_pos = vest_adapter.get_position(&vest_pair).await?;
    let paradex_pos = paradex_adapter.get_position(&paradex_pair).await?;
    
    let vest_has_position = match &vest_pos {
        Some(pos) => {
            info!(exchange = "vest", side = %pos.side.to_uppercase(), qty = pos.quantity, 
                  entry_price = pos.entry_price, pnl = pos.unrealized_pnl, "Position verified");
            true
        }
        None => {
            warn!(exchange = "vest", "NO POSITION DETECTED");
            false
        }
    };
    
    let paradex_has_position = match &paradex_pos {
        Some(pos) => {
            info!(exchange = "paradex", side = %pos.side.to_uppercase(), qty = pos.quantity,
                  entry_price = pos.entry_price, pnl = pos.unrealized_pnl, "Position verified");
            true
        }
        None => {
            warn!(exchange = "paradex", "NO POSITION DETECTED");
            false
        }
    };

    if !vest_has_position && !paradex_has_position {
        warn!("No positions to close. Orders may not have filled.");
        info!("=== TEST COMPLETE ===");
        return Ok(());
    }

    // =========================================================================
    // PHASE 3: CLOSE POSITIONS
    // =========================================================================
    info!("PHASE 3: CLOSING POSITIONS...");

    // Get fresh prices for closing
    vest_adapter.sync_orderbooks().await;
    paradex_adapter.sync_orderbooks().await;
    
    let vest_bid = vest_adapter.get_orderbook(&vest_pair)
        .and_then(|ob| ob.bids.first().map(|l| l.price))
        .unwrap_or(0.0);

    let vest_close_future = async {
        if let Some(pos) = vest_pos {
            let close_side = if pos.side.to_lowercase() == "long" { OrderSide::Sell } else { OrderSide::Buy };
            let close_price = if close_side == OrderSide::Sell {
                vest_bid * 0.998
            } else {
                vest_ask * 1.002
            };
            let order = OrderRequest {
                client_order_id: format!("close-vest-{}", &Uuid::new_v4().to_string()[..6]),
                symbol: vest_pair.clone(),
                side: close_side,
                order_type: OrderType::Limit,
                price: Some(close_price),
                quantity: pos.quantity,
                time_in_force: TimeInForce::Ioc,
                reduce_only: true,
            };
            info!(exchange = "vest", side = %pos.side, qty = pos.quantity, price = close_price, "Closing position");
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
                symbol: paradex_pair.clone(),
                side: close_side,
                order_type: OrderType::Market,
                price: None,
                quantity: pos.quantity,
                time_in_force: TimeInForce::Ioc,
                reduce_only: true,
            };
            info!(exchange = "paradex", side = %pos.side, qty = pos.quantity, "Closing position");
            Some(paradex_adapter.place_order(order).await)
        } else {
            None
        }
    };

    let close_start = std::time::Instant::now();
    let (vest_close_result, paradex_close_result) = tokio::join!(vest_close_future, paradex_close_future);
    let close_latency = close_start.elapsed();

    info!(latency_ms = close_latency.as_millis() as u64, "Close latency");
    
    match vest_close_result {
        Some(Ok(resp)) => info!(exchange = "vest", order_id = %resp.order_id, "CLOSE succeeded"),
        Some(Err(e)) => error!(exchange = "vest", error = %e, "CLOSE failed"),
        None => info!(exchange = "vest", "CLOSE skipped (no position)"),
    }
    match paradex_close_result {
        Some(Ok(resp)) => info!(exchange = "paradex", order_id = %resp.order_id, "CLOSE succeeded"),
        Some(Err(e)) => error!(exchange = "paradex", error = %e, "CLOSE failed"),
        None => info!(exchange = "paradex", "CLOSE skipped (no position)"),
    }

    // =========================================================================
    // PHASE 4: FINAL VERIFICATION
    // =========================================================================
    info!("PHASE 4: FINAL POSITION CHECK...");
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    
    match vest_adapter.get_position(&vest_pair).await {
        Ok(Some(pos)) => warn!(exchange = "vest", side = %pos.side, qty = pos.quantity, "Position still open"),
        Ok(None) => info!(exchange = "vest", "Position closed"),
        Err(e) => error!(exchange = "vest", error = %e, "Error checking position"),
    }
    match paradex_adapter.get_position(&paradex_pair).await {
        Ok(Some(pos)) => warn!(exchange = "paradex", side = %pos.side, qty = pos.quantity, "Position still open"),
        Ok(None) => info!(exchange = "paradex", "Position closed"),
        Err(e) => error!(exchange = "paradex", error = %e, "Error checking position"),
    }

    info!("╔══════════════════════════════════════════════════════════╗");
    info!("║                    TEST COMPLETE                         ║");
    info!("╚══════════════════════════════════════════════════════════╝");
    Ok(())
}
