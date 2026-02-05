//! Paradex Position Test
//! Tests: open position -> check position -> close with reduce_only
//!
//! # Logging (Story 5.1)
//! - Uses LOG_FORMAT env var: `json` (default) or `pretty`

use hft_bot::adapters::paradex::{ParadexAdapter, ParadexConfig};
use hft_bot::adapters::traits::ExchangeAdapter;
use hft_bot::adapters::types::{OrderRequest, OrderSide, OrderType, TimeInForce};
use hft_bot::config;
use uuid::Uuid;

const PARADEX_PAIR: &str = "BTC-USD-PERP";
const BTC_QTY: f64 = 0.0002;  // ~$15 at $77k (may be below $100 min notional)

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    // Initialize logging (Story 5.1: JSON/Pretty configurable via LOG_FORMAT)
    config::init_logging();
    
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
    
    // Subscribe to order confirmations via WebSocket (Story 7.1)
    adapter.subscribe_orders(PARADEX_PAIR).await?;
    log("   Subscribed to orderbook and order confirmations (WS)");
    
    // Wait for orderbook
    for i in 0..5 {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        adapter.sync_orderbooks().await;
        if adapter.get_orderbook(PARADEX_PAIR).is_some_and(|ob| !ob.bids.is_empty()) {
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

    // Open position with MARKET order (taker)
    log("\n4. Opening LONG position...");
    let open_order = OrderRequest {
        client_order_id: format!("open_{}", &Uuid::new_v4().to_string()[..6]),
        symbol: PARADEX_PAIR.to_string(),
        side: OrderSide::Buy,
        order_type: OrderType::Market,  // MARKET order = taker
        price: None,  // No price for market orders
        quantity: BTC_QTY,
        time_in_force: TimeInForce::Ioc,  // IOC is standard for market
        reduce_only: false,
    };
    
    log(&format!("   MARKET BUY {} BTC", BTC_QTY));
    let start = std::time::Instant::now();
    match adapter.place_order(open_order).await {
        Ok(resp) => {
            let elapsed = start.elapsed();
            log(&format!("   ⏱️  Order latency: {} ms", elapsed.as_millis()));
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

    // Close position with reduce_only MARKET order
    if let Some(pos) = position {
        log("\n6. Closing position with REDUCE_ONLY MARKET...");
        let close_order = OrderRequest {
            client_order_id: format!("close_{}", &Uuid::new_v4().to_string()[..6]),
            symbol: PARADEX_PAIR.to_string(),
            side: OrderSide::Sell,  // Opposite side to close
            order_type: OrderType::Market,  // MARKET = taker, immediate fill
            price: None,
            quantity: pos.quantity,  // Close full position
            time_in_force: TimeInForce::Ioc,
            reduce_only: true,  // REDUCE ONLY - just closes position
        };
        
        log(&format!("   MARKET SELL {} BTC (reduce_only=true)", pos.quantity));
        let start = std::time::Instant::now();
        match adapter.place_order(close_order).await {
            Ok(resp) => {
                let elapsed = start.elapsed();
                log(&format!("   ⏱️  Order latency: {} ms", elapsed.as_millis()));
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
