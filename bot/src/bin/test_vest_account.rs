//! Test script to call Vest GET /account and display position info
//!
//! Usage: cargo run --bin test_vest_account
//!
//! This script connects to Vest and polls GET /account every 5 seconds,
//! displaying the raw entry price and all position details.

use std::time::Duration;
use tokio::time::sleep;

use hft_bot::adapters::vest::{VestAdapter, VestConfig};
use hft_bot::adapters::ExchangeAdapter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env file
    dotenvy::dotenv().ok();
    
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    println!("=== Vest Account Test Script ===\n");
    
    // Create Vest adapter from .env
    let vest_config = VestConfig::from_env()
        .expect("VEST credentials must be configured in .env");
    
    let mut vest_adapter = VestAdapter::new(vest_config);
    
    println!("Connecting to Vest...");
    vest_adapter.connect().await?;
    println!("✓ Connected to Vest!\n");
    
    println!("Polling GET /account every 5 seconds. Press Ctrl+C to stop.\n");
    println!("Take a position on Vest UI, then watch the entry_price here.\n");
    println!("{}", "=".repeat(80));
    
    loop {
        match vest_adapter.get_account_info().await {
            Ok(account) => {
                println!("\n📊 Account Info at {}", chrono::Local::now().format("%H:%M:%S"));
                println!("{}", "-".repeat(60));
                
                if account.positions.is_empty() {
                    println!("  No positions found");
                } else {
                    for pos in &account.positions {
                        let symbol = pos.symbol.as_deref().unwrap_or("?");
                        let entry_price = pos.entry_price.as_deref().unwrap_or("null");
                        let mark_price = pos.mark_price.as_deref().unwrap_or("null");
                        let size = pos.size.as_deref().unwrap_or("0");
                        let is_long = pos.is_long;
                        let unrealized_pnl = pos.unrealized_pnl.as_deref().unwrap_or("0");
                        let liquidation_price = pos.liquidation_price.as_deref().unwrap_or("null");
                        
                        let side = match is_long {
                            Some(true) => "LONG",
                            Some(false) => "SHORT", 
                            None => "?",
                        };
                        
                        println!("\n  📈 Position: {} ({})", symbol, side);
                        println!("  ┌─────────────────────────────────────────────────");
                        println!("  │ 🎯 Entry Price:       {}", entry_price);
                        println!("  │ 📍 Mark Price:        {}", mark_price);
                        println!("  │ 📦 Size:              {}", size);
                        println!("  │ 💰 Unrealized PnL:    {}", unrealized_pnl);
                        println!("  │ ⚠️  Liquidation:       {}", liquidation_price);
                        println!("  └─────────────────────────────────────────────────");
                    }
                }
                
                println!("{}", "-".repeat(60));
            }
            Err(e) => {
                println!("❌ Error fetching account: {}", e);
            }
        }
        
        sleep(Duration::from_secs(5)).await;
    }
}
