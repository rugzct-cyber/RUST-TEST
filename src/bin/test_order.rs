//! Order Placement Test Script
//!
//! ⚠️ WARNING: This script places REAL orders on MAINNET!
//!
//! Tests place_order functionality with a small $10 order.
//!
//! Usage:
//! ```bash
//! cargo run --bin test_order
//! ```
//!
//! Requires environment variables:
//! - VEST_PRIMARY_ADDR, VEST_PRIMARY_KEY, VEST_SIGNING_KEY, VEST_ACCOUNT_GROUP
//! - VEST_PRODUCTION=true (for mainnet)

use std::env;
use tracing::{info, error};
use uuid::Uuid;

use hft_bot::adapters::{
    vest::{VestAdapter, VestConfig},
    traits::ExchangeAdapter,
    types::{OrderRequest, OrderSide, OrderType, TimeInForce},
};

/// Trading pair for Vest
const VEST_PAIR: &str = "BTC-PERP";

/// Target order size in USD
const ORDER_USD_SIZE: f64 = 10.0;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("debug")
        .init();

    // Load environment variables
    dotenvy::dotenv().ok();

    info!("═══════════════════════════════════════════════════════════");
    info!("⚠️  HFT Bot - ORDER PLACEMENT TEST (MAINNET)");
    info!("═══════════════════════════════════════════════════════════");
    info!("💰 Target order size: ${}", ORDER_USD_SIZE);
    info!("📊 Trading pair: {}", VEST_PAIR);
    info!("═══════════════════════════════════════════════════════════\n");

    // Check production flag
    let is_production = env::var("VEST_PRODUCTION")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    if !is_production {
        error!("❌ VEST_PRODUCTION is not set to 'true'. Set it to confirm mainnet trading.");
        error!("   Add VEST_PRODUCTION=true to your .env file");
        return Ok(());
    }

    info!("🔴 MAINNET MODE CONFIRMED");

    // Create Vest adapter
    let config = match VestConfig::from_env() {
        Ok(c) => c,
        Err(e) => {
            error!("❌ Failed to load Vest config: {}", e);
            return Ok(());
        }
    };

    let mut adapter = VestAdapter::new(config);

    // Connect to Vest
    info!("\n📡 Connecting to Vest...");
    if let Err(e) = adapter.connect().await {
        error!("❌ Connection failed: {}", e);
        return Ok(());
    }
    info!("✅ Connected to Vest");

    // Subscribe to orderbook to get current price
    info!("📊 Subscribing to {} orderbook...", VEST_PAIR);
    if let Err(e) = adapter.subscribe_orderbook(VEST_PAIR).await {
        error!("❌ Subscription failed: {}", e);
        return Ok(());
    }

    // Wait a moment for orderbook data
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    adapter.sync_orderbooks().await;

    // Get current best ask price for limit order
    let orderbook = match adapter.get_orderbook(VEST_PAIR) {
        Some(ob) => ob.clone(),
        None => {
            error!("❌ No orderbook data received. Try again.");
            let _ = adapter.disconnect().await;
            return Ok(());
        }
    };

    let best_ask = orderbook.asks.first().map(|l| l.price).unwrap_or(0.0);
    let best_bid = orderbook.bids.first().map(|l| l.price).unwrap_or(0.0);

    if best_ask == 0.0 {
        error!("❌ Invalid orderbook data (ask=0)");
        let _ = adapter.disconnect().await;
        return Ok(());
    }

    info!("\n📈 Current Market:");
    info!("   Best Bid: ${:.2}", best_bid);
    info!("   Best Ask: ${:.2}", best_ask);

    // =========================================================================
    // NEW: Test Leverage APIs
    // =========================================================================
    info!("\n⚙️  Testing Leverage APIs...");
    
    // Get current leverage
    match adapter.get_leverage(VEST_PAIR).await {
        Ok(Some(lev)) => info!("   Current leverage: {}x", lev),
        Ok(None) => info!("   No leverage set for {}", VEST_PAIR),
        Err(e) => info!("   Get leverage error: {}", e),
    }

    // Set leverage to 5x
    const TARGET_LEVERAGE: u32 = 5;
    info!("   Setting leverage to {}x...", TARGET_LEVERAGE);
    match adapter.set_leverage(VEST_PAIR, TARGET_LEVERAGE).await {
        Ok(new_lev) => info!("   ✅ Leverage set to {}x", new_lev),
        Err(e) => info!("   ⚠️ Set leverage error: {} (continuing...)", e),
    }

    // =========================================================================
    // NEW: Show Current Positions
    // =========================================================================
    info!("\n📊 Current Positions:");
    match adapter.get_positions().await {
        Ok(positions) if positions.is_empty() => {
            info!("   No open positions");
        }
        Ok(positions) => {
            for pos in &positions {
                let symbol = pos.symbol.as_deref().unwrap_or("?");
                let size = pos.size.as_deref().unwrap_or("0");
                let entry = pos.entry_price.as_deref().unwrap_or("?");
                let pnl = pos.unrealized_pnl.as_deref().unwrap_or("0");
                info!("   {} | Size: {} | Entry: ${} | PnL: ${}", symbol, size, entry, pnl);
            }
        }
        Err(e) => info!("   ⚠️ Get positions error: {}", e),
    }

    // Calculate order size: $10 / price = BTC quantity
    let btc_quantity = ORDER_USD_SIZE / best_ask;
    // Round to 4 decimals (Vest minimum)
    let btc_quantity = (btc_quantity * 10000.0).floor() / 10000.0;

    // Use current market price for display (market orders don't need a price)
    let market_price = best_ask; // Taker buys at ask price

    info!("\n📝 Order Details:");
    info!("   Side: BUY (Long)");
    info!("   Type: MARKET (TAKER - EXECUTES IMMEDIATELY!)");
    info!("   Quantity: {} BTC", btc_quantity);
    info!("   Est. Price: ~${:.2} (market)", market_price);
    info!("   Est. Value: ~${:.2}", btc_quantity * market_price);

    // Generate unique client order ID
    let client_order_id = format!("test-{}", Uuid::new_v4().to_string()[..8].to_string());

    // Create MARKET order request (taker order - executes immediately)
    // For market BUY, use limit price ABOVE ask to allow slippage
    let buy_limit_price = best_ask * 1.005; // 0.5% slippage tolerance
    
    let order = OrderRequest {
        client_order_id: client_order_id.clone(),
        symbol: VEST_PAIR.to_string(),
        side: OrderSide::Buy,
        order_type: OrderType::Market, // MARKET order = taker
        price: Some(buy_limit_price), // Allow 5% slippage up
        quantity: btc_quantity,
        time_in_force: TimeInForce::Ioc, // Immediate or Cancel for market orders
        reduce_only: false, // Opening a new position
    };

    // Confirm before placing
    info!("\n⚠️  ABOUT TO PLACE REAL ORDER ON MAINNET!");
    info!("   Press Ctrl+C within 5 seconds to cancel...\n");
    
    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

    // Place the BUY order using PRE-SIGNED method (opening long)
    info!("🚀 Placing BUY order using PRE-SIGNING (opening long)...");
    
    // Step 1: Pre-sign (this can be done in advance while waiting for opportunity)
    let sign_start = std::time::Instant::now();
    let presigned = match adapter.pre_sign_order(&order).await {
        Ok(p) => {
            let sign_elapsed = sign_start.elapsed();
            info!("   ⏱️  Pre-sign time: {:.2}ms", sign_elapsed.as_secs_f64() * 1000.0);
            p
        }
        Err(e) => {
            error!("❌ Pre-signing failed: {}", e);
            let _ = adapter.disconnect().await;
            return Ok(());
        }
    };
    
    // Step 2: Send pre-signed order (the fast part)
    let send_start = std::time::Instant::now();
    match adapter.send_presigned_order(presigned).await {
        Ok(response) => {
            info!("\n✅ BUY ORDER FILLED!");
            info!("═══════════════════════════════════════════════════════════");
            info!("   Order ID: {}", response.order_id);
            info!("   Client ID: {}", response.client_order_id);
            info!("   Status: {:?}", response.status);
            info!("   Filled Qty: {}", response.filled_quantity);
            if let Some(avg) = response.avg_price {
                info!("   Avg Price: ${:.2}", avg);
            }
            let send_elapsed = send_start.elapsed();
            info!("   ⏱️  Send time (without signing): {:.2}ms", send_elapsed.as_secs_f64() * 1000.0);
            info!("═══════════════════════════════════════════════════════════");

            // Show positions after BUY (should show the new position)
            info!("\n📊 Positions after BUY:");
            match adapter.get_positions().await {
                Ok(positions) if positions.is_empty() => {
                    info!("   ⚠️ No positions (order may still be processing)");
                }
                Ok(positions) => {
                    for pos in &positions {
                        let symbol = pos.symbol.as_deref().unwrap_or("?");
                        let size = pos.size.as_deref().unwrap_or("0");
                        let entry = pos.entry_price.as_deref().unwrap_or("?");
                        let pnl = pos.unrealized_pnl.as_deref().unwrap_or("0");
                        info!("   ✅ {} | Size: {} | Entry: ${} | PnL: ${}", symbol, size, entry, pnl);
                    }
                }
                Err(e) => info!("   ⚠️ Get positions error: {}", e),
            }

            // Wait a moment before closing
            info!("\n⏳ Waiting 2 seconds before closing position...");
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

            // Now close the position with a SELL order (same quantity)
            info!("\n🔄 Placing SELL order (closing position)...");
            
            // For market SELL, use a limit price LOWER than best_bid to allow for slippage
            // Vest rejects if executed price < limitPrice for sells
            let sell_limit_price = best_bid * 0.995; // 0.5% slippage tolerance
            
            let close_order = OrderRequest {
                client_order_id: format!("close-{}", Uuid::new_v4().to_string()[..8].to_string()),
                symbol: VEST_PAIR.to_string(),
                side: OrderSide::Sell, // SELL to close long position
                order_type: OrderType::Market,
                price: Some(sell_limit_price), // Allow 1% slippage
                quantity: btc_quantity, // Same quantity to fully close
                time_in_force: TimeInForce::Ioc,
                reduce_only: true, // IMPORTANT: Only reduce position, don't open new short
            };

            let sell_start = std::time::Instant::now();
            match adapter.place_order(close_order).await {
                Ok(close_response) => {
                    let sell_elapsed = sell_start.elapsed();
                    info!("\n✅ POSITION CLOSED!");
                    info!("═══════════════════════════════════════════════════════════");
                    info!("   Order ID: {}", close_response.order_id);
                    info!("   Status: {:?}", close_response.status);
                    info!("   ⏱️  Execution time: {:.2}ms", sell_elapsed.as_secs_f64() * 1000.0);
                    info!("═══════════════════════════════════════════════════════════");
                    info!("\n🎉 Round-trip trade complete!");
                }
                Err(e) => {
                    error!("\n❌ CLOSE ORDER FAILED!");
                    error!("   Error: {}", e);
                    error!("   ⚠️ Position may still be open!");
                }
            }
        }
        Err(e) => {
            error!("\n❌ BUY ORDER FAILED!");
            error!("   Error: {}", e);
        }
    }

    // Disconnect
    let _ = adapter.disconnect().await;
    info!("\n👋 Test complete.");

    Ok(())
}
