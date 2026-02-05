//! Close Delta-Neutral Positions
//! Closes both Vest and Paradex positions simultaneously with reduce_only orders
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    // Initialize logging (Story 5.1: JSON/Pretty configurable via LOG_FORMAT)
    config::init_logging();
    
    let log = |msg: &str| println!("{}", msg);
    
    log("=== CLOSING DELTA-NEUTRAL POSITIONS ===");
    
    // Load configs and connect
    let vest_config = VestConfig::from_env()?;
    let paradex_config = ParadexConfig::from_env()?;
    let mut vest_adapter = VestAdapter::new(vest_config);
    let mut paradex_adapter = ParadexAdapter::new(paradex_config);
    
    log("\n1. Connecting...");
    let (vest_conn, paradex_conn) = tokio::join!(
        vest_adapter.connect(),
        paradex_adapter.connect()
    );
    vest_conn?;
    paradex_conn?;
    log("   Both connected!");

    // Check current positions
    log("\n2. Checking current positions...");
    let vest_pos = vest_adapter.get_position(VEST_PAIR).await?;
    let paradex_pos = paradex_adapter.get_position(PARADEX_PAIR).await?;
    
    match &vest_pos {
        Some(pos) => log(&format!("   Vest: {} {} BTC @ ${:.2}", pos.side.to_uppercase(), pos.quantity, pos.entry_price)),
        None => log("   Vest: No position"),
    }
    match &paradex_pos {
        Some(pos) => log(&format!("   Paradex: {} {} BTC @ ${:.2}", pos.side.to_uppercase(), pos.quantity, pos.entry_price)),
        None => log("   Paradex: No position"),
    }

    // Close positions with reduce_only MARKET orders
    log("\n3. CLOSING POSITIONS IN PARALLEL...");
    
    let vest_close = async {
        if let Some(pos) = vest_pos {
            // Opposite side to close: LONG -> SELL, SHORT -> BUY
            let close_side = if pos.side.to_lowercase() == "long" { OrderSide::Sell } else { OrderSide::Buy };
            let order = OrderRequest {
                client_order_id: format!("close-vest-{}", &Uuid::new_v4().to_string()[..6]),
                symbol: VEST_PAIR.to_string(),
                side: close_side,
                order_type: OrderType::Market,
                price: None,
                quantity: pos.quantity,
                time_in_force: TimeInForce::Ioc,
                reduce_only: true,
            };
            log(&format!("   Vest: Closing {} {} BTC (reduce_only=true)", pos.side, pos.quantity));
            vest_adapter.place_order(order).await
        } else {
            log("   Vest: No position to close");
            Ok(hft_bot::adapters::types::OrderResponse {
                order_id: "none".to_string(),
                client_order_id: "none".to_string(),
                status: hft_bot::adapters::types::OrderStatus::Cancelled,
                filled_quantity: 0.0,
                avg_price: None,
            })
        }
    };

    let paradex_close = async {
        if let Some(pos) = paradex_pos {
            // Opposite side to close: SHORT -> BUY, LONG -> SELL  
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
            log(&format!("   Paradex: Closing {} {} BTC (reduce_only=true)", pos.side, pos.quantity));
            paradex_adapter.place_order(order).await
        } else {
            log("   Paradex: No position to close");
            Ok(hft_bot::adapters::types::OrderResponse {
                order_id: "none".to_string(),
                client_order_id: "none".to_string(),
                status: hft_bot::adapters::types::OrderStatus::Cancelled,
                filled_quantity: 0.0,
                avg_price: None,
            })
        }
    };

    let start = std::time::Instant::now();
    let (vest_result, paradex_result) = tokio::join!(vest_close, paradex_close);
    let elapsed = start.elapsed();

    log(&format!("\n4. CLOSE RESULTS ({}ms):", elapsed.as_millis()));
    
    match vest_result {
        Ok(resp) if resp.order_id != "none" => {
            log(&format!("   Vest: ✅ Closed - Order ID: {}", resp.order_id));
        }
        Ok(_) => log("   Vest: ⏭️  Skipped (no position)"),
        Err(e) => log(&format!("   Vest: ❌ FAILED: {}", e)),
    }
    
    match paradex_result {
        Ok(resp) if resp.order_id != "none" => {
            log(&format!("   Paradex: ✅ Closed - Order ID: {}", resp.order_id));
        }
        Ok(_) => log("   Paradex: ⏭️  Skipped (no position)"),
        Err(e) => log(&format!("   Paradex: ❌ FAILED: {}", e)),
    }

    // Final position check
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    log("\n5. Final position check...");
    match vest_adapter.get_position(VEST_PAIR).await {
        Ok(Some(pos)) => log(&format!("   Vest: Still have {} {} BTC", pos.side, pos.quantity)),
        Ok(None) => log("   Vest: ✅ Position closed"),
        Err(e) => log(&format!("   Vest: Error - {}", e)),
    }
    match paradex_adapter.get_position(PARADEX_PAIR).await {
        Ok(Some(pos)) => log(&format!("   Paradex: Still have {} {} BTC", pos.side, pos.quantity)),
        Ok(None) => log("   Paradex: ✅ Position closed"),
        Err(e) => log(&format!("   Paradex: Error - {}", e)),
    }

    log("\n=== CLOSE COMPLETE ===");
    Ok(())
}
