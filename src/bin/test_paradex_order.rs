//! Paradex Position Test
//! Tests: open position -> check position -> close with reduce_only

use hft_bot::adapters::paradex::{ParadexAdapter, ParadexConfig};
use hft_bot::adapters::traits::ExchangeAdapter;
use hft_bot::adapters::types::{OrderRequest, OrderSide, OrderType, TimeInForce};
use uuid::Uuid;

const PARADEX_PAIR: &str = "BTC-USD-PERP";
const BTC_QTY: f64 = 0.0002;  // ~$15 at $77k (may be below $100 min notional)

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    // Enable tracing to see API responses
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .init();
    
    let log = |msg: &str| println!("{}", msg);
    
    log("=== PARADEX POSITION TEST ===");
    log(&format!("Testing with {} BTC", BTC_QTY));
    
    let config = ParadexConfig::from_env()?;
    log(&format!("Config loaded (production={})", config.production));

    let mut adapter = ParadexAdapter::new(config);
    
    log("\n1. Connecting...");
    adapter.connect().await?;
    log("   Connected!");

    log(&format!("\n2. Subscribing to {}", PARADEX_PAIR));
    adapter.subscribe_orderbook(PARADEX_PAIR).await?;
    
    // Wait for orderbook
    for i in 0..5 {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        adapter.sync_orderbooks().await;
        if adapter.get_orderbook(PARADEX_PAIR).map_or(false, |ob| !ob.bids.is_empty()) {
            log(&format!("   Got orderbook at {}s", i + 1));
            break;
        }
    }

    let orderbook = adapter.get_orderbook(PARADEX_PAIR)
        .ok_or("No orderbook data")?.clone();
    let best_bid = orderbook.bids.first().map(|l| l.price).unwrap_or(0.0);
    let best_ask = orderbook.asks.first().map(|l| l.price).unwrap_or(0.0);
    log(&format!("   Bid: ${:.2} | Ask: ${:.2}", best_bid, best_ask));

    // Check existing position first
    log("\n3. Checking existing position...");
    match adapter.get_position(PARADEX_PAIR).await {
        Ok(Some(pos)) => {
            log(&format!("   Existing position: {} {} BTC @ ${:.2}", 
                pos.side.to_uppercase(), pos.quantity, pos.entry_price));
            log(&format!("   PnL: ${:.2}", pos.unrealized_pnl));
        }
        Ok(None) => log("   No existing position"),
        Err(e) => log(&format!("   Error checking position: {}", e)),
    }

    // Open position with aggressive limit order at best ask
    log("\n4. Opening LONG position...");
    let limit_price = (best_ask * 10.0).ceil() / 10.0;  // Round up to 0.1
    let open_order = OrderRequest {
        client_order_id: format!("open_{}", &Uuid::new_v4().to_string()[..6]),
        symbol: PARADEX_PAIR.to_string(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        price: Some(limit_price),  // Aggressive limit at ask
        quantity: BTC_QTY,
        time_in_force: TimeInForce::Ioc,
        reduce_only: false,  // Opening new position
    };
    
    log(&format!("   LIMIT BUY {} BTC @ ${:.1}", BTC_QTY, limit_price));
    match adapter.place_order(open_order).await {
        Ok(resp) => {
            log(&format!("   Order ID: {}", resp.order_id));
            log(&format!("   Status: {:?}", resp.status));
            log(&format!("   Filled: {} BTC @ ${:.2}", 
                resp.filled_quantity, resp.avg_price.unwrap_or(0.0)));
        }
        Err(e) => {
            log(&format!("   FAILED: {}", e));
            adapter.disconnect().await?;
            return Ok(());
        }
    }

    // Wait for position to be reflected
    log("\n   Waiting 2s for settlement...");
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // Check position after opening
    log("\n5. Checking position after open...");
    let position = match adapter.get_position(PARADEX_PAIR).await {
        Ok(Some(pos)) => {
            log(&format!("   Position: {} {} BTC @ ${:.2}", 
                pos.side.to_uppercase(), pos.quantity, pos.entry_price));
            log(&format!("   PnL: ${:.2}", pos.unrealized_pnl));
            Some(pos)
        }
        Ok(None) => {
            log("   No position found (order may not have filled)");
            None
        }
        Err(e) => {
            log(&format!("   Error: {}", e));
            None
        }
    };

    // Close position with reduce_only
    if let Some(pos) = position {
        log("\n6. Closing position with REDUCE_ONLY...");
        let close_price = (best_bid * 10.0).floor() / 10.0;  // Round down to 0.1
        let close_order = OrderRequest {
            client_order_id: format!("close_{}", &Uuid::new_v4().to_string()[..6]),
            symbol: PARADEX_PAIR.to_string(),
            side: OrderSide::Sell,  // Opposite side to close
            order_type: OrderType::Limit,
            price: Some(close_price),  // Aggressive limit at bid
            quantity: pos.quantity,  // Close full position
            time_in_force: TimeInForce::Ioc,
            reduce_only: true,  // REDUCE ONLY - just closes position
        };
        
        log(&format!("   LIMIT SELL {} BTC @ ${:.1} (reduce_only=true)", pos.quantity, close_price));
        match adapter.place_order(close_order).await {
            Ok(resp) => {
                log(&format!("   Order ID: {}", resp.order_id));
                log(&format!("   Status: {:?}", resp.status));
                log(&format!("   Filled: {} BTC @ ${:.2}", 
                    resp.filled_quantity, resp.avg_price.unwrap_or(0.0)));
            }
            Err(e) => log(&format!("   FAILED: {}", e)),
        }

        // Final position check
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        log("\n7. Final position check...");
        match adapter.get_position(PARADEX_PAIR).await {
            Ok(Some(pos)) => {
                log(&format!("   Still have position: {} {} BTC", 
                    pos.side.to_uppercase(), pos.quantity));
            }
            Ok(None) => log("   Position closed successfully!"),
            Err(e) => log(&format!("   Error: {}", e)),
        }
    }

    adapter.disconnect().await?;
    log("\n=== TEST COMPLETE ===");
    Ok(())
}
