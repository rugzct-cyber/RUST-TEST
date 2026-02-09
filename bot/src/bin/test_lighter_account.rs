//! Test script to verify Lighter API credentials and connection
//!
//! Usage: cargo run --bin test_lighter_account
//!
//! This script:
//! 1. Loads credentials from .env
//! 2. Connects to Lighter (WebSocket)
//! 3. Subscribes to BTC orderbook
//! 4. Displays live orderbook data for 10 seconds
//!
//! If you see orderbook data, your connection works!

use std::time::Duration;
use tokio::time::sleep;

use hft_bot::adapters::lighter::{LighterAdapter, LighterConfig};
use hft_bot::adapters::ExchangeAdapter;
use tracing::{info, error};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Shared init (dotenv + logging only, no config.yaml needed)
    hft_bot::bin_utils::boot_minimal();

    info!("=== Lighter Connection Test ===");

    // Step 1: Load config from .env
    info!("Step 1: Loading credentials from .env...");
    let config = match LighterConfig::from_env() {
        Ok(c) => {
            info!(
                account_index = c.account_index,
                api_key_index = c.api_key_index,
                production = c.production,
                rest_url = c.rest_url(),
                ws_url = c.ws_url(),
                "Credentials loaded OK"
            );
            c
        }
        Err(e) => {
            error!(error = %e, "Failed to load Lighter credentials from .env");
            return Err(anyhow::anyhow!("Credential loading failed: {}", e));
        }
    };

    // Step 2: Create adapter and connect
    info!("Step 2: Connecting to Lighter...");
    let mut adapter = LighterAdapter::new(config);
    match adapter.connect().await {
        Ok(()) => info!("Connected to Lighter WebSocket!"),
        Err(e) => {
            error!(error = %e, "Failed to connect to Lighter");
            return Err(anyhow::anyhow!("Connection failed: {}", e));
        }
    }

    // Step 3: Subscribe to BTC orderbook
    let symbol = "2"; // BTC market on Lighter
    info!(symbol = symbol, "Step 3: Subscribing to orderbook...");
    match adapter.subscribe_orderbook(symbol).await {
        Ok(()) => info!("Subscribed to orderbook"),
        Err(e) => {
            error!(error = %e, "Failed to subscribe to orderbook");
            return Err(anyhow::anyhow!("Subscription failed: {}", e));
        }
    }

    // Step 4: Poll orderbook for 10 seconds
    info!("Step 4: Reading orderbook data for 10 seconds...");
    for i in 1..=10 {
        sleep(Duration::from_secs(1)).await;

        // Sync any pending WS messages
        adapter.sync_orderbooks().await;

        match adapter.get_orderbook(symbol) {
            Some(ob) => {
                let best_bid = ob.best_bid().unwrap_or(0.0);
                let best_ask = ob.best_ask().unwrap_or(0.0);
                let bid_levels = ob.bids.len();
                let ask_levels = ob.asks.len();
                info!(
                    tick = i,
                    best_bid = %format!("${:.2}", best_bid),
                    best_ask = %format!("${:.2}", best_ask),
                    bid_levels = bid_levels,
                    ask_levels = ask_levels,
                    "Orderbook snapshot"
                );
            }
            None => {
                info!(tick = i, "No orderbook data yet (waiting for WS messages...)");
            }
        }
    }

    // Step 5: Disconnect
    info!("Step 5: Disconnecting...");
    adapter.disconnect().await?;
    info!("=== Lighter Connection Test PASSED ===");

    Ok(())
}
