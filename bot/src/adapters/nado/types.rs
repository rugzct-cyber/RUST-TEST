//! Nado Types — book_depth channel, BigInt prices (10^18 scale)
//!
//! Nado uses product IDs and prices scaled by 1e18.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct NadoBookLevel {
    pub price: String,
    pub size: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NadoBookDepthData {
    pub product_id: Option<String>,
    pub bids: Option<Vec<NadoBookLevel>>,
    pub asks: Option<Vec<NadoBookLevel>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NadoWsMessage {
    #[serde(rename = "type")]
    pub msg_type: Option<String>,
    pub channel: Option<String>,
    pub data: Option<NadoBookDepthData>,
}

/// Convert Nado BigInt price string (1e18 scale) to f64
pub fn parse_nado_price(price_str: &str) -> Option<f64> {
    // Nado prices are in 1e18 format
    let val: f64 = price_str.parse().ok()?;
    Some(val / 1e18)
}

pub fn get_nado_markets() -> Vec<(&'static str, &'static str)> {
    vec![
        ("1", "BTC-USD"),
        ("2", "ETH-USD"),
        ("3", "SOL-USD"),
    ]
}

pub fn product_id_to_symbol(product_id: &str) -> Option<&'static str> {
    get_nado_markets().iter().find(|(id, _)| *id == product_id).map(|(_, sym)| *sym)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_nado_price() {
        let price = parse_nado_price("96500000000000000000000").unwrap();
        assert!((price - 96500.0).abs() < 0.01);
    }

    #[test]
    fn test_product_id_to_symbol() {
        assert_eq!(product_id_to_symbol("1"), Some("BTC-USD"));
        assert_eq!(product_id_to_symbol("999"), None);
    }
}
