//! Simple Paradex Order Test
//! Tests order placement with minimal output

use hft_bot::adapters::paradex::{ParadexAdapter, ParadexConfig};
use hft_bot::adapters::traits::ExchangeAdapter;
use hft_bot::adapters::types::{OrderRequest, OrderSide, OrderType, TimeInForce};
use uuid::Uuid;

const PARADEX_PAIR: &str = "BTC-USD-PERP";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    
    // Simple console log
    let log = |msg: &str| println!("{}", msg);
    
    log("=== PARADEX ORDER TEST ===");
    
    let config = ParadexConfig::from_env()?;
    log(&format!("Config loaded (production={})", config.production));

    let mut adapter = ParadexAdapter::new(config);
    
    log("Connecting...");
    adapter.connect().await?;
    log("Connected!");

    log(&format!("Subscribing to {}", PARADEX_PAIR));
    adapter.subscribe_orderbook(PARADEX_PAIR).await?;
    
    log("Waiting for orderbook (10s)...");
    for i in 0..10 {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        adapter.sync_orderbooks().await;
        if let Some(ob) = adapter.get_orderbook(PARADEX_PAIR) {
            if !ob.bids.is_empty() {
                log(&format!("Got orderbook at {}s", i + 1));
                break;
            }
        }
    }

    let orderbook = adapter.get_orderbook(PARADEX_PAIR)
        .ok_or("No orderbook data")?;
    let orderbook = orderbook.clone();

    let best_ask = orderbook.asks.first().map(|l| l.price).unwrap_or(0.0);
    let best_bid = orderbook.bids.first().map(|l| l.price).unwrap_or(0.0);
    log(&format!("Bid: {:.2} | Ask: {:.2}", best_bid, best_ask));

    // Calculate tiny order
    let btc_qty = (10.0 / best_ask * 10000.0).floor() / 10000.0; // $10 worth  
    let limit_price = (best_bid * 0.9 * 10.0).floor() / 10.0; // Round to 0.1 (Paradex requirement)
    
    log(&format!("Order: BUY {} BTC @ ${:.2}", btc_qty, limit_price));
    
    let order = OrderRequest {
        client_order_id: format!("t{}", &Uuid::new_v4().to_string()[..6]),
        symbol: PARADEX_PAIR.to_string(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        price: Some(limit_price),
        quantity: btc_qty,
        time_in_force: TimeInForce::Gtc,
        reduce_only: false,
    };

    log("Placing order...");
    match adapter.place_order(order).await {
        Ok(resp) => {
            log(&format!("SUCCESS! Order ID: {}", resp.order_id));
            log(&format!("Status: {:?}", resp.status));
            
            // Cancel it
            log("Cancelling...");
            match adapter.cancel_order(&resp.order_id).await {
                Ok(()) => log("Cancelled!"),
                Err(e) => log(&format!("Cancel error: {}", e)),
            }
        }
        Err(e) => {
            log(&format!("ORDER FAILED: {}", e));
        }
    }

    adapter.disconnect().await?;
    log("Done!");
    Ok(())
}
