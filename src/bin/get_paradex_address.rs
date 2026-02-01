//! Simple script to display Paradex wallet address
//! Derives the Starknet account address from private key

use hft_bot::adapters::paradex::{ParadexAdapter, ParadexConfig};
use hft_bot::adapters::traits::ExchangeAdapter;
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    
    // Initialize logging to see the address derivation
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .with_target(false)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;
    
    println!("🔐 Paradex Wallet Address Derivation");
    println!("════════════════════════════════════════════");
    
    // Load config
    let config = ParadexConfig::from_env()?;
    println!("\n.env private key: {}...{}", 
        &config.private_key[..10], 
        &config.private_key[config.private_key.len()-4..]
    );
    
    // Create adapter and connect (this will derive the address and log it)
    let mut adapter = ParadexAdapter::new(config);
    
    println!("\n📡 Connecting to derive address from system config...\n");
    match adapter.connect().await {
        Ok(()) => {
            println!("\n✅ Connected! Look for 'Derived:' address above ☝️");
        }
        Err(e) => {
            println!("\n❌ Error: {}", e);
        }
    }
    
    let _ = adapter.disconnect().await;
    
    Ok(())
}
