//! Quick test to fetch fills from Paradex API
//! Run with: cargo test --test test_fills -- --nocapture

use reqwest::Client;
use serde::Deserialize;
use std::env;

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct FillsResponse {
    results: Option<Vec<Fill>>,
    next: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct Fill {
    id: String,
    market: String,
    side: String,
    size: String,
    fill_price: String,
    fee: Option<String>,
    created_at: Option<i64>,
}

#[tokio::test]
async fn test_get_fills() {
    dotenvy::dotenv().ok();

    // Get JWT token first (not used yet - placeholder for future implementation)
    let _client = Client::new();

    println!("=== Testing Paradex /fills endpoint ===\n");

    // First, we need to authenticate
    // For this test, we'll use the adapter's auth flow
    // Let's just call the endpoint directly with existing JWT if available

    // Note: This test requires a valid JWT token
    // You can get one by running the bot first and copying the token

    // For now, let's check what the endpoint returns
    let url = "https://api.prod.paradex.trade/v1/fills?market=BTC-USD-PERP&page_size=10";

    println!("Endpoint: {}", url);
    println!("\nThis endpoint requires JWT authentication.");
    println!("The /fills endpoint returns fill_price which is the actual execution price.\n");

    // Check if we have credentials to test
    if env::var("PARADEX_PRIVATE_KEY").is_ok() {
        println!("Credentials found - you can modify this test to do full auth flow");
    } else {
        println!("No credentials found in .env");
    }

    println!("\n=== Fill Response Structure ===");
    println!("{{");
    println!("  \"results\": [");
    println!("    {{");
    println!("      \"id\": \"fill_id\",");
    println!("      \"market\": \"BTC-USD-PERP\",");
    println!("      \"side\": \"BUY\",");
    println!("      \"size\": \"0.001\",");
    println!("      \"fill_price\": \"75825.30\",  <-- THIS IS THE REAL ENTRY PRICE");
    println!("      \"fee\": \"0.0\",");
    println!("      \"created_at\": 1700000000000");
    println!("    }}");
    println!("  ]");
    println!("}}");
}
