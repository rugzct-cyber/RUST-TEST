//! Test script to place a single order on Lighter
//!
//! Usage: cargo run --bin test_lighter_order
//!
//! This places a BTC limit buy at slightly below market (~0.1% below).
//! Quantity: 0.001 BTC

use hft_bot::adapters::lighter::{LighterAdapter, LighterConfig};
use hft_bot::adapters::types::{OrderRequest, OrderSide, TimeInForce};
use hft_bot::adapters::ExchangeAdapter;
use tracing::{error, info};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    hft_bot::bin_utils::boot_minimal();

    info!("=== Lighter Order Test ===");

    // 1. Connect
    let config = LighterConfig::from_env()?;
    let mut adapter = LighterAdapter::new(config);
    adapter.connect().await?;
    info!("Connected to Lighter");

    // 2. Get BTC price from REST API (more reliable than WS for a quick test)
    let symbol = "BTC";
    let price = get_btc_price().await?;
    info!(
        symbol = symbol,
        last_trade_price = %format!("${:.2}", price),
        "Fetched BTC price from REST"
    );

    // 3. Place limit buy at last trade price (GTC = Good Til Cancel)
    let buy_price = price;
    let quantity = 0.001;

    info!(
        side = "BUY",
        price = %format!("${:.2}", buy_price),
        quantity = quantity,
        notional = %format!("${:.2}", buy_price * quantity),
        "Placing limit buy order..."
    );

    let order = OrderRequest {
        client_order_id: format!("test-lighter-{}", chrono::Utc::now().timestamp_millis()),
        symbol: symbol.to_string(),
        side: OrderSide::Buy,
        order_type: hft_bot::adapters::types::OrderType::Limit,
        price: Some(buy_price),
        quantity,
        time_in_force: TimeInForce::Ioc,
        reduce_only: false,
    };

    match adapter.place_order(order).await {
        Ok(resp) => {
            println!(">>> ORDER SUCCESS: id={} status={:?} filled={} avg_price={:?}",
                resp.order_id, resp.status, resp.filled_quantity, resp.avg_price);
            info!(
                order_id = %resp.order_id,
                status = ?resp.status,
                filled_qty = resp.filled_quantity,
                avg_price = resp.avg_price,
                "Order placed successfully!"
            );
        }
        Err(e) => {
            println!(">>> ORDER FAILED: {}", e);
            error!(error = %e, "Order failed");
        }
    }

    // Wait briefly to see async WS response
    println!(">>> Waiting 2s for WS response...");
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // 4. Disconnect
    adapter.disconnect().await?;
    info!("=== Test Complete ===");

    Ok(())
}

/// Fetch current BTC last_trade_price from REST API
async fn get_btc_price() -> anyhow::Result<f64> {
    let client = reqwest::Client::new();
    let resp = client
        .get("https://mainnet.zklighter.elliot.ai/api/v1/orderBookDetails")
        .send()
        .await?;
    let body: serde_json::Value = resp.json().await?;
    let markets = body["order_book_details"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("No order_book_details"))?;
    for m in markets {
        if m["symbol"].as_str() == Some("BTC") && m["market_type"].as_str() == Some("perp") {
            if let Some(price) = m["last_trade_price"].as_f64() {
                return Ok(price);
            }
        }
    }
    Err(anyhow::anyhow!("BTC perp not found"))
}
