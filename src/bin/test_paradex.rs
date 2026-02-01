//! Paradex Connection and Authentication Test
//!
//! Simple test to verify:
//! 1. Environment variables are loaded correctly
//! 2. REST authentication works (JWT token obtained)
//! 3. WebSocket connection + authentication works
//! 4. Orderbook subscription works

use hft_bot::adapters::paradex::{ParadexAdapter, ParadexConfig};
use hft_bot::adapters::traits::ExchangeAdapter;
use tracing::{info, error, Level};
use tracing_subscriber::FmtSubscriber;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load .env file
    dotenvy::dotenv().ok();
    
    // Initialize logging
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::DEBUG)
        .with_target(true)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    info!("═══════════════════════════════════════════════════════════");
    info!("⚠️  Paradex Connection Test");
    info!("═══════════════════════════════════════════════════════════");

    // Load configuration from environment
    info!("\n📋 Loading configuration from .env...");
    let config = match ParadexConfig::from_env() {
        Ok(c) => {
            info!("   ✅ Configuration loaded successfully");
            info!("   Environment: {}", if c.production { "PRODUCTION" } else { "TESTNET" });
            c
        }
        Err(e) => {
            error!("   ❌ Failed to load configuration: {}", e);
            return Ok(());
        }
    };

    // Create adapter
    info!("\n🔧 Creating Paradex adapter...");
    let mut adapter = ParadexAdapter::new(config);

    // Connect (REST auth + WebSocket + WS auth)
    info!("\n📡 Connecting to Paradex (REST + WebSocket)...");
    let connect_start = std::time::Instant::now();
    
    match adapter.connect().await {
        Ok(()) => {
            let elapsed = connect_start.elapsed();
            info!("\n✅ Connected successfully!");
            info!("   ⏱️  Connection time: {:.2}ms", elapsed.as_secs_f64() * 1000.0);
            info!("   Is connected: {}", adapter.is_connected());
        }
        Err(e) => {
            error!("\n❌ Connection failed: {}", e);
            return Ok(());
        }
    }

    // Subscribe to orderbook
    let symbol = "BTC-USD-PERP";
    info!("\n📊 Subscribing to {} orderbook...", symbol);
    match adapter.subscribe_orderbook(symbol).await {
        Ok(()) => info!("   ✅ Subscribed to {}", symbol),
        Err(e) => error!("   ❌ Subscribe failed: {}", e),
    }

    // Wait for some orderbook data
    info!("\n⏳ Waiting 3 seconds for orderbook data...");
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // Sync and check orderbook
    adapter.sync_orderbooks();
    if let Some(ob) = adapter.get_orderbook(symbol) {
        info!("\n📈 Orderbook received:");
        info!("   Symbol: {}", symbol);
        info!("   Bids: {} levels", ob.bids.len());
        info!("   Asks: {} levels", ob.asks.len());
        if let (Some(best_bid), Some(best_ask)) = (ob.bids.first(), ob.asks.first()) {
            info!("   Best bid: ${:.2} x {}", best_bid.price, best_bid.quantity);
            info!("   Best ask: ${:.2} x {}", best_ask.price, best_ask.quantity);
            info!("   Spread: {:.2} bps", (best_ask.price - best_bid.price) / best_bid.price * 10000.0);
        }
    } else {
        info!("\n⚠️  No orderbook data yet (may need more time)");
    }

    // Check position (to verify REST API works)
    info!("\n📊 Checking position for {}...", symbol);
    match adapter.get_position(symbol).await {
        Ok(Some(pos)) => {
            info!("   Position found:");
            info!("   Size: {} {}", pos.quantity, pos.symbol);
            info!("   Entry price: ${:.2}", pos.entry_price);
        }
        Ok(None) => info!("   No open position (expected)"),
        Err(e) => error!("   ❌ Failed to get position: {}", e),
    }

    // Disconnect
    info!("\n👋 Disconnecting...");
    adapter.disconnect().await?;
    info!("   ✅ Disconnected");

    info!("\n═══════════════════════════════════════════════════════════");
    info!("🎉 Paradex connection test complete!");
    info!("═══════════════════════════════════════════════════════════");

    Ok(())
}
