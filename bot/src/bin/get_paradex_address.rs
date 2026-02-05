//! Simple script to display Paradex wallet address
//! Derives the Starknet account address from private key
//!
//! # Logging (Story 5.1)
//! - Uses LOG_FORMAT env var: `json` (default) or `pretty`

use hft_bot::adapters::paradex::{ParadexAdapter, ParadexConfig};
use hft_bot::adapters::traits::ExchangeAdapter;
use hft_bot::config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    
    // Initialize logging (Story 5.1: JSON/Pretty configurable via LOG_FORMAT)
    config::init_logging();
    
    println!("🔐 Paradex Wallet Address Derivation");
    println!("════════════════════════════════════════════");
    
    // Load config
    let config = ParadexConfig::from_env()?;
    println!("\n.env private key: {}...{}", 
        &config.private_key[..10], 
        &config.private_key[config.private_key.len()-4..]
    );
    
    // Create adapter and connect (this will derive the address and log signing details)
    let mut adapter = ParadexAdapter::new(config);
    
    println!("\n📡 Connecting to derive address from system config...\n");
    match adapter.connect().await {
        Ok(()) => {
            println!("\n✅ Connected!");
        }
        Err(e) => {
            println!("\n❌ Error: {}", e);
        }
    }
    
    let _ = adapter.disconnect().await;
    
    Ok(())
}
