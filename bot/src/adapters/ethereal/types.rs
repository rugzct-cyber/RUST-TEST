//! Ethereal Types — BookDepth channel
//! Ethereal subscribes to BookDepth with product_id.
//! Response fields: bid_price, ask_price as strings.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct EtherealBookDepthData {
    pub product_id: Option<String>,
    pub bid_price: Option<String>,
    pub ask_price: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EtherealWsMessage {
    #[serde(rename = "type")]
    pub msg_type: Option<String>,
    pub channel: Option<String>,
    pub data: Option<EtherealBookDepthData>,
}

/// Map product_id → canonical symbol
pub fn get_ethereal_markets() -> Vec<(&'static str, &'static str)> {
    vec![
        ("1", "BTC-USD"),
        ("2", "ETH-USD"),
        ("3", "SOL-USD"),
    ]
}

pub fn product_id_to_symbol(product_id: &str) -> Option<&'static str> {
    get_ethereal_markets().iter().find(|(id, _)| *id == product_id).map(|(_, sym)| *sym)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_product_id_to_symbol() {
        assert_eq!(product_id_to_symbol("1"), Some("BTC-USD"));
        assert_eq!(product_id_to_symbol("999"), None);
    }
}
