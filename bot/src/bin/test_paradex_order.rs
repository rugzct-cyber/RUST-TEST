//! Paradex Position Test
//! Tests: open position -> check position -> close with reduce_only
//!
//! # Logging (Story 5.1)
//! - Uses LOG_FORMAT env var: `json` (default) or `pretty`

use hft_bot::adapters::paradex::{ParadexAdapter, ParadexConfig};
use hft_bot::adapters::traits::ExchangeAdapter;
use hft_bot::adapters::types::{OrderRequest, OrderSide, OrderType, TimeInForce};
use hft_bot::config;
use tracing::{info, warn, error};
use uuid::Uuid;

const PARADEX_PAIR: &str = "BTC-USD-PERP";
const BTC_QTY: f64 = 0.0002;  // ~$15 at $77k (may be below $100 min notional)

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    // Initialize logging (Story 5.1: JSON/Pretty configurable via LOG_FORMAT)
    config::init_logging();
    
    info!(pair = PARADEX_PAIR, qty = BTC_QTY, "=== PARADEX POSITION TEST ===");
    
    let config = ParadexConfig::from_env()?;
    info!(production = config.production, "Config loaded");

    let mut adapter = ParadexAdapter::new(config);
    
    info!("Phase 1: Connecting...");
    adapter.connect().await?;
    info!("Connected");

    info!(pair = PARADEX_PAIR, "Phase 2: Subscribing to orderbook");
    adapter.subscribe_orderbook(PARADEX_PAIR).await?;
    
    // Subscribe to order confirmations via WebSocket (Story 7.1)
    adapter.subscribe_orders(PARADEX_PAIR).await?;
    info!("Subscribed to orderbook and order confirmations (WS)");
    
    // Wait for orderbook
    for i in 0..5 {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        adapter.sync_orderbooks().await;
        if adapter.get_orderbook(PARADEX_PAIR).is_some_and(|ob| !ob.bids.is_empty()) {
            info!(elapsed_s = i + 1, "Got orderbook");
            break;
        }
    }

    let orderbook = adapter.get_orderbook(PARADEX_PAIR)
        .ok_or("No orderbook data")?.clone();
    let best_bid = orderbook.bids.first().map(|l| l.price).unwrap_or(0.0);
    let best_ask = orderbook.asks.first().map(|l| l.price).unwrap_or(0.0);
    info!(bid = best_bid, ask = best_ask, "Orderbook prices");

    // Check existing position first
    info!("Phase 3: Checking existing position...");
    match adapter.get_position(PARADEX_PAIR).await {
        Ok(Some(pos)) => {
            info!(side = %pos.side.to_uppercase(), qty = pos.quantity, 
                  entry_price = pos.entry_price, pnl = pos.unrealized_pnl, "Existing position");
        }
        Ok(None) => info!("No existing position"),
        Err(e) => warn!(error = %e, "Error checking position"),
    }

    // Open position with MARKET order (taker)
    info!("Phase 4: Opening LONG position...");
    let open_order = OrderRequest {
        client_order_id: format!("open_{}", &Uuid::new_v4().to_string()[..6]),
        symbol: PARADEX_PAIR.to_string(),
        side: OrderSide::Buy,
        order_type: OrderType::Market,
        price: None,
        quantity: BTC_QTY,
        time_in_force: TimeInForce::Ioc,
        reduce_only: false,
    };
    
    info!(side = "BUY", qty = BTC_QTY, order_type = "MARKET", "Placing order");
    let start = std::time::Instant::now();
    match adapter.place_order(open_order).await {
        Ok(resp) => {
            let elapsed = start.elapsed();
            info!(latency_ms = elapsed.as_millis() as u64, order_id = %resp.order_id, 
                  status = ?resp.status, filled = resp.filled_quantity, 
                  avg_price = resp.avg_price.unwrap_or(0.0), "Order placed");
        }
        Err(e) => {
            error!(error = %e, "Order FAILED");
            adapter.disconnect().await?;
            return Ok(());
        }
    }

    // Wait for position to be reflected
    info!("Waiting 2s for settlement...");
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // Check position after opening
    info!("Phase 5: Checking position after open...");
    let position = match adapter.get_position(PARADEX_PAIR).await {
        Ok(Some(pos)) => {
            info!(side = %pos.side.to_uppercase(), qty = pos.quantity,
                  entry_price = pos.entry_price, pnl = pos.unrealized_pnl, "Position found");
            Some(pos)
        }
        Ok(None) => {
            warn!("No position found (order may not have filled)");
            None
        }
        Err(e) => {
            error!(error = %e, "Error checking position");
            None
        }
    };

    // Close position with reduce_only MARKET order
    if let Some(pos) = position {
        info!("Phase 6: Closing position with REDUCE_ONLY MARKET...");
        let close_order = OrderRequest {
            client_order_id: format!("close_{}", &Uuid::new_v4().to_string()[..6]),
            symbol: PARADEX_PAIR.to_string(),
            side: OrderSide::Sell,
            order_type: OrderType::Market,
            price: None,
            quantity: pos.quantity,
            time_in_force: TimeInForce::Ioc,
            reduce_only: true,
        };
        
        info!(side = "SELL", qty = pos.quantity, reduce_only = true, "Closing position");
        let start = std::time::Instant::now();
        match adapter.place_order(close_order).await {
            Ok(resp) => {
                let elapsed = start.elapsed();
                info!(latency_ms = elapsed.as_millis() as u64, order_id = %resp.order_id,
                      status = ?resp.status, filled = resp.filled_quantity,
                      avg_price = resp.avg_price.unwrap_or(0.0), "Close order placed");
            }
            Err(e) => error!(error = %e, "Close FAILED"),
        }

        // Final position check
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        info!("Phase 7: Final position check...");
        match adapter.get_position(PARADEX_PAIR).await {
            Ok(Some(pos)) => {
                warn!(side = %pos.side.to_uppercase(), qty = pos.quantity, "Position still open");
            }
            Ok(None) => info!("Position closed successfully"),
            Err(e) => error!(error = %e, "Error checking position"),
        }
    }

    adapter.disconnect().await?;
    info!("=== TEST COMPLETE ===");
    Ok(())
}
