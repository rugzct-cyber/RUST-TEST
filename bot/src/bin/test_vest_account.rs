//! Test script to call Vest GET /account and display position info
//!
//! Usage: cargo run --bin test_vest_account
//!
//! This script connects to Vest and polls GET /account every 5 seconds,
//! displaying the raw entry price and all position details.
//!
//! # Logging
//! - Uses LOG_FORMAT env var: `json` (default) or `pretty`
//! - For human-readable output, set LOG_FORMAT=pretty

use std::time::Duration;
use tokio::time::sleep;

use hft_bot::adapters::vest::{VestAdapter, VestConfig};
use hft_bot::adapters::ExchangeAdapter;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Shared init (dotenv + logging only, no config.yaml needed)
    hft_bot::bin_utils::boot_minimal();

    info!(
        event_type = "TEST_START",
        "=== Vest Account Test Script ==="
    );

    // Create Vest adapter from .env
    let vest_config = VestConfig::from_env().expect("VEST credentials must be configured in .env");

    let mut vest_adapter = VestAdapter::new(vest_config);

    info!("Connecting to Vest...");
    vest_adapter.connect().await?;
    info!("Connected to Vest");

    info!("Polling GET /account every 5 seconds. Press Ctrl+C to stop.");

    loop {
        match vest_adapter.get_account_info().await {
            Ok(account) => {
                if account.positions.is_empty() {
                    info!(event_type = "ACCOUNT_POLL", "No positions found");
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

                        info!(
                            event_type = "POSITION",
                            symbol = %symbol,
                            side = %side,
                            entry_price = %entry_price,
                            mark_price = %mark_price,
                            size = %size,
                            unrealized_pnl = %unrealized_pnl,
                            liquidation_price = %liquidation_price,
                            "Position details"
                        );
                    }
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "Error fetching account");
            }
        }

        sleep(Duration::from_secs(5)).await;
    }
}
