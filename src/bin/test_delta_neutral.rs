//! Delta-Neutral Execution Test
//! Opens LONG on Vest + SHORT on Paradex in parallel
//! Uses IOC LIMIT orders with aggressive prices (crosses spread)

use hft_bot::adapters::paradex::{ParadexAdapter, ParadexConfig};
use hft_bot::adapters::vest::{VestAdapter, VestConfig};
use hft_bot::adapters::traits::ExchangeAdapter;
use hft_bot::adapters::types::{OrderRequest, OrderSide, OrderType, TimeInForce};
use uuid::Uuid;

const VEST_PAIR: &str = "BTC-PERP";
const PARADEX_PAIR: &str = "BTC-USD-PERP";
const BTC_QTY: f64 = 0.0002;  // ~$15 at $77k - meets min notional

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .init();
    
    let log = |msg: &str| println!("{}", msg);
    
    log("=== DELTA-NEUTRAL EXECUTION TEST ===");
    log(&format!("Qty: {} BTC | Vest: {} | Paradex: {}", BTC_QTY, VEST_PAIR, PARADEX_PAIR));
    
    // Load configs
    log("\n1. Loading configs...");
    let vest_config = VestConfig::from_env()?;
    let paradex_config = ParadexConfig::from_env()?;
    log(&format!("   Vest (production={})", vest_config.production));
    log(&format!("   Paradex (production={})", paradex_config.production));

    let mut vest_adapter = VestAdapter::new(vest_config);
    let mut paradex_adapter = ParadexAdapter::new(paradex_config);
    
    // Connect both adapters in parallel
    log("\n2. Connecting adapters in parallel...");
    let start = std::time::Instant::now();
    let (vest_conn, paradex_conn) = tokio::join!(
        vest_adapter.connect(),
        paradex_adapter.connect()
    );
    vest_conn?;
    paradex_conn?;
    log(&format!("   Both connected in {}ms", start.elapsed().as_millis()));

    // Subscribe to orderbooks
    log("\n3. Subscribing to orderbooks...");
    let (vest_sub, paradex_sub) = tokio::join!(
        vest_adapter.subscribe_orderbook(VEST_PAIR),
        paradex_adapter.subscribe_orderbook(PARADEX_PAIR)
    );
    vest_sub?;
    paradex_sub?;
    
    // Wait for orderbook data
    log("   Waiting for orderbook data...");
    for i in 0..5 {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        vest_adapter.sync_orderbooks().await;
        paradex_adapter.sync_orderbooks().await;
        
        let vest_ob = vest_adapter.get_orderbook(VEST_PAIR);
        let paradex_ob = paradex_adapter.get_orderbook(PARADEX_PAIR);
        
        if vest_ob.is_some_and(|ob| !ob.bids.is_empty()) && 
           paradex_ob.is_some_and(|ob| !ob.bids.is_empty()) {
            log(&format!("   Got both orderbooks at {}s", i + 1));
            break;
        }
    }

    // Get prices from orderbooks
    let vest_ob = vest_adapter.get_orderbook(VEST_PAIR)
        .ok_or("No Vest orderbook")?.clone();
    let paradex_ob = paradex_adapter.get_orderbook(PARADEX_PAIR)
        .ok_or("No Paradex orderbook")?.clone();

    let vest_bid = vest_ob.bids.first().map(|l| l.price).unwrap_or(0.0);
    let vest_ask = vest_ob.asks.first().map(|l| l.price).unwrap_or(0.0);
    let paradex_bid = paradex_ob.bids.first().map(|l| l.price).unwrap_or(0.0);
    let paradex_ask = paradex_ob.asks.first().map(|l| l.price).unwrap_or(0.0);
    
    log(&format!("   Vest:    Bid: ${:.2} | Ask: ${:.2}", vest_bid, vest_ask));
    log(&format!("   Paradex: Bid: ${:.2} | Ask: ${:.2}", paradex_bid, paradex_ask));

    // Create IOC LIMIT orders with aggressive prices
    // For BUY: use ask price (cross spread for immediate fill)
    // For SELL: use bid price (cross spread for immediate fill)
    
    log("\n4. EXECUTING DELTA-NEUTRAL TRADE...");
    log(&format!("   LONG on Vest: BUY {} {} @ ${:.2} (IOC LIMIT)", BTC_QTY, VEST_PAIR, vest_ask));
    log(&format!("   SHORT on Paradex: SELL {} {} @ ${:.2} (IOC LIMIT)", BTC_QTY, PARADEX_PAIR, paradex_bid));
    
    let vest_order = OrderRequest {
        client_order_id: format!("dn-vest-{}", &Uuid::new_v4().to_string()[..6]),
        symbol: VEST_PAIR.to_string(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,  // LIMIT order
        price: Some(vest_ask * 1.001), // Slightly above ask to ensure fill
        quantity: BTC_QTY,
        time_in_force: TimeInForce::Ioc,
        reduce_only: false,
    };

    let paradex_order = OrderRequest {
        client_order_id: format!("dn-pdx-{}", &Uuid::new_v4().to_string()[..6]),
        symbol: PARADEX_PAIR.to_string(),
        side: OrderSide::Sell,
        order_type: OrderType::Market,  // Paradex supports Market
        price: None,
        quantity: BTC_QTY,
        time_in_force: TimeInForce::Ioc,
        reduce_only: false,
    };

    // Execute both orders in parallel
    let start = std::time::Instant::now();
    let (vest_result, paradex_result) = tokio::join!(
        vest_adapter.place_order(vest_order),
        paradex_adapter.place_order(paradex_order)
    );
    let total_elapsed = start.elapsed();

    log(&format!("\n5. EXECUTION RESULT ({}ms total):", total_elapsed.as_millis()));
    
    // Vest result
    log(&format!("   Vest (LONG):"));
    match vest_result {
        Ok(resp) => {
            log(&format!("      ✅ Order ID: {}", resp.order_id));
            log(&format!("      Status: {:?}", resp.status));
            log(&format!("      Filled: {} @ ${:.2}", resp.filled_quantity, resp.avg_price.unwrap_or(0.0)));
        }
        Err(e) => log(&format!("      ❌ FAILED: {}", e)),
    }

    // Paradex result
    log(&format!("   Paradex (SHORT):"));
    match paradex_result {
        Ok(resp) => {
            log(&format!("      ✅ Order ID: {}", resp.order_id));
            log(&format!("      Status: {:?}", resp.status));
            log(&format!("      Filled: {} @ ${:.2}", resp.filled_quantity, resp.avg_price.unwrap_or(0.0)));
        }
        Err(e) => log(&format!("      ❌ FAILED: {}", e)),
    }

    // Check final positions
    log("\n6. Checking final positions...");
    match vest_adapter.get_position(VEST_PAIR).await {
        Ok(Some(pos)) => log(&format!("   Vest: {} {} BTC @ ${:.2}", pos.side.to_uppercase(), pos.quantity, pos.entry_price)),
        Ok(None) => log("   Vest: No position"),
        Err(e) => log(&format!("   Vest position error: {}", e)),
    }
    match paradex_adapter.get_position(PARADEX_PAIR).await {
        Ok(Some(pos)) => log(&format!("   Paradex: {} {} BTC @ ${:.2}", pos.side.to_uppercase(), pos.quantity, pos.entry_price)),
        Ok(None) => log("   Paradex: No position"),
        Err(e) => log(&format!("   Paradex position error: {}", e)),
    }

    log("\n=== TEST COMPLETE ===");
    Ok(())
}
