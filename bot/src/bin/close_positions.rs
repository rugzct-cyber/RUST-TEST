//! Close Delta-Neutral Positions
//! Closes both Vest and Paradex positions simultaneously with reduce_only orders
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
    
    // Load pairs from config.yaml (same source of truth as main bot)
    let cfg = config::load_config(Path::new("config.yaml"))
        .expect("Failed to load config.yaml");
    let bot = &cfg.bots[0];
    let vest_pair = bot.pair.to_string();
    let paradex_pair = format!("{}-USD-PERP",
        vest_pair.split('-').next().unwrap_or("BTC"));
    
    info!(vest = %vest_pair, paradex = %paradex_pair, "=== CLOSING DELTA-NEUTRAL POSITIONS ===");
    
    // Load configs and connect
    let vest_config = VestConfig::from_env()?;
    let paradex_config = ParadexConfig::from_env()?;
    let mut vest_adapter = VestAdapter::new(vest_config);
    let mut paradex_adapter = ParadexAdapter::new(paradex_config);
    
    info!("Phase 1: Connecting...");
    let (vest_conn, paradex_conn) = tokio::join!(
        vest_adapter.connect(),
        paradex_adapter.connect()
    );
    vest_conn?;
    paradex_conn?;
    info!("Both adapters connected");

    // Check current positions
    info!("Phase 2: Checking current positions...");
    let vest_pos = vest_adapter.get_position(&vest_pair).await?;
    let paradex_pos = paradex_adapter.get_position(&paradex_pair).await?;
    
    match &vest_pos {
        Some(pos) => info!(exchange = "vest", side = %pos.side.to_uppercase(), qty = pos.quantity, entry_price = pos.entry_price, "Position found"),
        None => info!(exchange = "vest", "No position"),
    }
    match &paradex_pos {
        Some(pos) => info!(exchange = "paradex", side = %pos.side.to_uppercase(), qty = pos.quantity, entry_price = pos.entry_price, "Position found"),
        None => info!(exchange = "paradex", "No position"),
    }

    // Close positions with reduce_only MARKET orders
    info!("Phase 3: CLOSING POSITIONS IN PARALLEL...");
    
    let vest_close = async {
        if let Some(pos) = vest_pos {
            // Opposite side to close: LONG -> SELL, SHORT -> BUY
            let close_side = if pos.side.to_lowercase() == "long" { OrderSide::Sell } else { OrderSide::Buy };
            
            // Vest REQUIRES a limit price even for MARKET orders (slippage protection)
            // Use entry_price with 1% slippage tolerance
            let limit_price = if close_side == OrderSide::Buy {
                pos.entry_price * 1.01  // Buying to close short: allow 1% above
            } else {
                pos.entry_price * 0.99  // Selling to close long: allow 1% below
            };
            
            let order = OrderRequest {
                client_order_id: format!("close-vest-{}", &Uuid::new_v4().to_string()[..6]),
                symbol: vest_pair.clone(),
                side: close_side,
                order_type: OrderType::Market,
                price: Some(limit_price),  // Vest requires this!
                quantity: pos.quantity,
                time_in_force: TimeInForce::Ioc,
                reduce_only: true,
            };
            info!(exchange = "vest", side = %pos.side, qty = pos.quantity, limit_price = limit_price, "Closing position (reduce_only)");
            vest_adapter.place_order(order).await
        } else {
            info!(exchange = "vest", "No position to close");
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
                symbol: paradex_pair.clone(),
                side: close_side,
                order_type: OrderType::Market,
                price: None,
                quantity: pos.quantity,
                time_in_force: TimeInForce::Ioc,
                reduce_only: true,
            };
            info!(exchange = "paradex", side = %pos.side, qty = pos.quantity, "Closing position (reduce_only)");
            paradex_adapter.place_order(order).await
        } else {
            info!(exchange = "paradex", "No position to close");
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

    info!(latency_ms = elapsed.as_millis() as u64, "Phase 4: CLOSE RESULTS");
    
    match vest_result {
        Ok(resp) if resp.order_id != "none" => {
            info!(exchange = "vest", order_id = %resp.order_id, "Closed successfully");
        }
        Ok(_) => info!(exchange = "vest", "Skipped (no position)"),
        Err(e) => error!(exchange = "vest", error = %e, "CLOSE FAILED"),
    }
    
    match paradex_result {
        Ok(resp) if resp.order_id != "none" => {
            info!(exchange = "paradex", order_id = %resp.order_id, "Closed successfully");
        }
        Ok(_) => info!(exchange = "paradex", "Skipped (no position)"),
        Err(e) => error!(exchange = "paradex", error = %e, "CLOSE FAILED"),
    }

    // Final position check
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    info!("Phase 5: Final position check...");
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

    info!("=== CLOSE COMPLETE ===");
    Ok(())
}
