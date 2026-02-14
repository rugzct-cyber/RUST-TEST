//! Lighter-specific Types
//!
//! API response types, market info, and conversions to/from common adapter types.

use serde::Deserialize;
use crate::adapters::types::{Orderbook, OrderbookLevel, MAX_ORDERBOOK_DEPTH};

// =============================================================================
// Market / Exchange Info
// =============================================================================

/// Market information from Lighter's exchangeInfo endpoint
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct LighterMarketInfo {
    #[serde(rename = "market_id")]
    pub market_id: u8,
    pub symbol: String,
    /// Tick size (minimum price increment)
    pub tick_size: String,
    /// Step size (minimum quantity increment)
    pub step_size: String,
    /// Minimum order size
    pub min_order_size: String,
    /// Price precision (number of decimal places)
    #[serde(default)]
    pub price_precision: u8,
    /// Size precision (number of decimal places)
    #[serde(default)]
    pub size_precision: u8,
}

/// Lighter's symbol-to-market-id mapping
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct MarketMapping {
    pub symbol: String,
    pub market_id: u8,
    pub tick_size: f64,
    pub step_size: f64,
    pub price_precision: u8,
    pub size_precision: u8,
}

// =============================================================================
// Orderbook Parsing
// =============================================================================

/// Response from Lighter's orderbook REST endpoint
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct LighterOrderbookResponse {
    pub bids: Vec<Vec<String>>,
    pub asks: Vec<Vec<String>>,
}

impl LighterOrderbookResponse {
    /// Convert to our common Orderbook type
    #[allow(dead_code)]
    pub fn to_orderbook(&self) -> Orderbook {
        let parse_levels = |raw: &[Vec<String>]| -> Vec<OrderbookLevel> {
            raw.iter()
                .filter_map(|level| {
                    if level.len() >= 2 {
                        let price = level[0].parse::<f64>().ok()?;
                        let qty = level[1].parse::<f64>().ok()?;
                        Some(OrderbookLevel::new(price, qty))
                    } else {
                        None
                    }
                })
                .take(MAX_ORDERBOOK_DEPTH)
                .collect()
        };

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        Orderbook {
            bids: parse_levels(&self.bids),
            asks: parse_levels(&self.asks),
            timestamp: now_ms,
        }
    }
}


// =============================================================================
// Symbol Normalization Helpers
// =============================================================================

/// Normalize our internal symbol format to Lighter's convention
/// 
/// Internal: "BTC-PERP" or "BTC-USD-PERP"
/// Lighter: market_id integer (looked up from exchange info)
pub fn normalize_symbol_to_lighter(symbol: &str) -> String {
    // Strip "-PERP" or "-USD-PERP" suffix to get base asset
    symbol
        .replace("-USD-PERP", "")
        .replace("-PERP", "")
        .to_uppercase()
}

/// Convert Lighter's market symbol to our internal format
#[allow(dead_code)]
pub fn normalize_symbol_from_lighter(lighter_symbol: &str) -> String {
    // Lighter uses symbols like "BTC-PERP" typically
    lighter_symbol.to_uppercase()
}
