//! Live Market Monitor
//!
//! Connects to Vest and Paradex WebSockets and displays
//! real-time prices and spread calculations.
//!
//! Usage:
//! ```bash
//! cargo run --bin monitor
//! ```
//!
//! Requires environment variables:
//! - VEST_API_KEY, VEST_API_SECRET, VEST_SIGNER_KEY, VEST_SIGNER_ADDRESS
//! - PARADEX_PRIVATE_KEY, PARADEX_ACCOUNT_ADDRESS (optional for public channels)
//!
//! # Logging
//! - Uses LOG_FORMAT env var: `json` (default) or `pretty`
//! - For human-readable output, set LOG_FORMAT=pretty

use std::time::Duration;
use tokio::signal;
use tracing::{error, info, warn};

use hft_bot::adapters::{
    paradex::{ParadexAdapter, ParadexConfig},
    traits::ExchangeAdapter,
    vest::{VestAdapter, VestConfig},
};
use hft_bot::core::spread::SpreadCalculator;

/// Display refresh interval in milliseconds (100ms = 10 fps, distinct from HFT 25ms polling)
const DISPLAY_POLL_INTERVAL_MS: u64 = 100;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Shared init (dotenv + logging + config)
    let boot = hft_bot::bin_utils::boot();
    let vest_pair = boot.vest_pair;
    let paradex_pair = boot.paradex_pair;

    info!("═══════════════════════════════════════════════════════════");
    info!("🔍 HFT Arbitrage Bot - LIVE MONITOR MODE");
    info!("═══════════════════════════════════════════════════════════");
    info!(
        "📊 Vest pair: {} | Paradex pair: {}",
        vest_pair, paradex_pair
    );
    info!("⏱️  Poll interval: {}ms", DISPLAY_POLL_INTERVAL_MS);
    info!("Press Ctrl+C to exit");
    info!("═══════════════════════════════════════════════════════════\n");

    // Create spread calculator
    let calculator = SpreadCalculator::new("vest", "paradex");

    // Try to connect to Vest
    let mut vest: Option<VestAdapter> = None;
    match VestConfig::from_env() {
        Ok(config) => {
            info!("Connecting to Vest...");
            let mut adapter = VestAdapter::new(config);
            match adapter.connect().await {
                Ok(()) => match adapter.subscribe_orderbook(&vest_pair).await {
                    Ok(()) => {
                        info!("✅ Vest connected and subscribed to {}", vest_pair);
                        vest = Some(adapter);
                    }
                    Err(e) => warn!("⚠️  Vest subscription failed: {}", e),
                },
                Err(e) => warn!("⚠️  Vest connection failed: {}", e),
            }
        }
        Err(e) => warn!("⚠️  Vest config not found: {}", e),
    }

    // Try to connect to Paradex
    let mut paradex: Option<ParadexAdapter> = None;
    match ParadexConfig::from_env() {
        Ok(config) => {
            info!("Connecting to Paradex...");
            let mut adapter = ParadexAdapter::new(config);
            match adapter.connect().await {
                Ok(()) => match adapter.subscribe_orderbook(&paradex_pair).await {
                    Ok(()) => {
                        info!("✅ Paradex connected and subscribed to {}", paradex_pair);
                        paradex = Some(adapter);
                    }
                    Err(e) => warn!("⚠️  Paradex subscription failed: {}", e),
                },
                Err(e) => warn!("⚠️  Paradex connection failed: {}", e),
            }
        }
        Err(e) => warn!("⚠️  Paradex config not found: {}", e),
    }

    // Check we have at least one adapter
    if vest.is_none() && paradex.is_none() {
        error!("❌ No exchanges connected. Check your .env configuration.");
        return Ok(());
    }

    info!("\n🚀 Starting live monitoring...\n");
    info!(
        "{:<12} {:<12} {:>12} {:>12}",
        "EXCHANGE", "SYMBOL", "BID", "ASK"
    );
    info!("{}", "─".repeat(52));

    // Main loop
    loop {
        tokio::select! {
            _ = signal::ctrl_c() => {
                info!("\n\n🛑 Shutdown signal received. Disconnecting...");
                break;
            }
            _ = tokio::time::sleep(Duration::from_millis(DISPLAY_POLL_INTERVAL_MS)) => {
                // Get Vest orderbook (sync from shared storage first)
                let vest_ob = if let Some(ref mut adapter) = vest {
                    adapter.sync_orderbooks().await;
                    adapter.get_orderbook(&vest_pair).cloned()
                } else {
                    None
                };

                // Get Paradex orderbook (use async method that reads from shared storage)
                let paradex_ob = if let Some(ref adapter) = paradex {
                    adapter.get_orderbook_async(&paradex_pair).await
                } else {
                    None
                };

                // Display Vest prices
                if let Some(ref ob) = vest_ob {
                    let best_bid = ob.bids.first().map(|l| l.price).unwrap_or(0.0);
                    let best_ask = ob.asks.first().map(|l| l.price).unwrap_or(0.0);
                    info!(
                        "{:<12} {:<12} {:>12.2} {:>12.2}",
                        "[VEST]", vest_pair, best_bid, best_ask
                    );
                }

                // Display Paradex prices
                if let Some(ref ob) = paradex_ob {
                    let best_bid = ob.bids.first().map(|l| l.price).unwrap_or(0.0);
                    let best_ask = ob.asks.first().map(|l| l.price).unwrap_or(0.0);
                    info!(
                        "{:<12} {:<12} {:>12.2} {:>12.2}",
                        "[PARADEX]", paradex_pair, best_bid, best_ask
                    );
                }

                // Calculate and display spread if both orderbooks available
                if let (Some(ref vest_ob), Some(ref paradex_ob)) = (&vest_ob, &paradex_ob) {
                    if let Some((entry, exit)) = calculator.calculate_dual_spreads(vest_ob, paradex_ob) {
                        let entry_indicator = if entry >= 0.30 { "🎯" } else { "  " };
                        let exit_indicator = if exit >= 0.10 { "📤" } else { "  " };
                        info!(
                            "{:<12} entry={:>6.3}% {} exit={:>6.3}% {}",
                            "[SPREAD]", entry, entry_indicator, exit, exit_indicator
                        );
                    }
                }

                info!("{}", "─".repeat(52));
            }
        }
    }

    // Cleanup
    if let Some(mut v) = vest {
        let _ = v.disconnect().await;
        info!("Vest disconnected");
    }
    if let Some(mut p) = paradex {
        let _ = p.disconnect().await;
        info!("Paradex disconnected");
    }

    info!("👋 Monitor stopped.");
    Ok(())
}
