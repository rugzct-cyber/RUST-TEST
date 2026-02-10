//! Benchmark: Lighter WebSocket order placement latency
//!
//! Measures:
//!   1. Sign latency (Poseidon2 hash + Schnorr sign)
//!   2. WS send latency (message serialization + TCP write)
//!   3. Total latency (sign + send)
//!
//! Places 10 IOC buy orders at $1 below market (will not fill).
//! Usage: cargo run --release --bin bench_lighter_ws

use hft_bot::adapters::lighter::{LighterAdapter, LighterConfig};
use hft_bot::adapters::types::{OrderRequest, OrderSide, OrderType, TimeInForce};
use hft_bot::adapters::ExchangeAdapter;
use std::time::Instant;

const NUM_ORDERS: usize = 10;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    hft_bot::bin_utils::boot_minimal();

    println!("=== Lighter WS Order Latency Benchmark ===");
    println!("Orders: {}", NUM_ORDERS);
    println!();

    // 1. Connect
    let config = LighterConfig::from_env()?;
    let mut adapter = LighterAdapter::new(config);
    adapter.connect().await?;
    println!("✓ Connected to Lighter");

    // 2. Get BTC price
    let price = get_btc_price().await?;
    // Place far below market so IOC won't fill (just tests send speed)
    let buy_price = (price - 1000.0).max(1.0);
    println!("✓ BTC price: ${:.2}, order price: ${:.2} (won't fill)", price, buy_price);
    println!();

    // 3. Warm up (1 order to prime the connection)
    let warmup = OrderRequest {
        client_order_id: "warmup-0".to_string(),
        symbol: "BTC".to_string(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        price: Some(buy_price),
        quantity: 0.001,
        time_in_force: TimeInForce::Ioc,
        reduce_only: false,
    };
    let _ = adapter.place_order(warmup).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    println!("✓ Warmup done");
    println!();

    // 4. Benchmark
    let mut latencies = Vec::with_capacity(NUM_ORDERS);

    for i in 0..NUM_ORDERS {
        let order = OrderRequest {
            client_order_id: format!("bench-{}", i),
            symbol: "BTC".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            price: Some(buy_price),
            quantity: 0.001,
            time_in_force: TimeInForce::Ioc,
            reduce_only: false,
        };

        let start = Instant::now();
        let result = adapter.place_order(order).await;
        let elapsed = start.elapsed();

        match result {
            Ok(_) => {
                let us = elapsed.as_micros();
                latencies.push(us);
                println!("  #{:2}: {:>6} µs  ✓", i + 1, us);
            }
            Err(e) => {
                println!("  #{:2}: FAILED - {}", i + 1, e);
            }
        }

        // Small gap between orders to avoid nonce issues
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    // 5. Stats
    if !latencies.is_empty() {
        let mut sorted = latencies.clone();
        sorted.sort();

        let sum: u128 = sorted.iter().sum();
        let avg = sum / sorted.len() as u128;
        let min = sorted[0];
        let max = sorted[sorted.len() - 1];
        let median = sorted[sorted.len() / 2];
        let p95_idx = (sorted.len() as f64 * 0.95).ceil() as usize - 1;
        let p95 = sorted[p95_idx.min(sorted.len() - 1)];

        println!();
        println!("=== Results ({} orders) ===", sorted.len());
        println!("  Min:    {:>6} µs  ({:.2} ms)", min, min as f64 / 1000.0);
        println!("  Median: {:>6} µs  ({:.2} ms)", median, median as f64 / 1000.0);
        println!("  Avg:    {:>6} µs  ({:.2} ms)", avg, avg as f64 / 1000.0);
        println!("  P95:    {:>6} µs  ({:.2} ms)", p95, p95 as f64 / 1000.0);
        println!("  Max:    {:>6} µs  ({:.2} ms)", max, max as f64 / 1000.0);
    }

    // 6. Cleanup
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    adapter.disconnect().await?;
    println!("\n✓ Done");

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
