//! Simple script to display Paradex wallet address
//! Derives the Starknet account address from private key
//!
//! # Logging
//! - Uses LOG_FORMAT env var: `json` (default) or `pretty`

use hft_bot::adapters::paradex::{ParadexAdapter, ParadexConfig};
use hft_bot::adapters::traits::ExchangeAdapter;
use hft_bot::config;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    // Initialize logging (JSON/Pretty configurable via LOG_FORMAT)
    config::init_logging();

    info!("🔐 Paradex Wallet Address Derivation");
    info!("════════════════════════════════════════════");

    // Load config
    let config = ParadexConfig::from_env()?;
    // Security: only show last 4 characters of private key
    let key_suffix = &config.private_key[config.private_key.len().saturating_sub(4)..];
    info!(key_hint = %format!("***...{}", key_suffix), "Private key loaded from .env");

    // Create adapter and connect (this will derive the address and log signing details)
    let mut adapter = ParadexAdapter::new(config);

    info!("📡 Connecting to derive address from system config...");
    match adapter.connect().await {
        Ok(()) => {
            info!("✅ Connected!");
        }
        Err(e) => {
            error!(error = %e, "❌ Connection failed");
        }
    }

    let _ = adapter.disconnect().await;

    Ok(())
}
