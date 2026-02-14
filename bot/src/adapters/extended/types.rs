//! Extended Types — orderbook depth=1 SNAPSHOT/DELTA messages
//!
//! Actual API format:
//! { type: "SNAPSHOT", data: { t: "SNAPSHOT", m: "ETH-USD", b: [{q, p}], a: [{q, p}], d: "1" }, ts: ..., seq: ... }

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct ExtendedOrderbookLevel {
    /// Price
    pub p: String,
    /// Quantity
    pub q: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExtendedOrderbookData {
    /// Type: "SNAPSHOT" or "DELTA"
    pub t: Option<String>,
    /// Market symbol (e.g. "ETH-USD", "BTC-USD")
    pub m: Option<String>,
    /// Bids
    pub b: Option<Vec<ExtendedOrderbookLevel>>,
    /// Asks
    pub a: Option<Vec<ExtendedOrderbookLevel>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExtendedOrderbookMsg {
    #[serde(rename = "type")]
    pub msg_type: Option<String>,
    pub data: Option<ExtendedOrderbookData>,
}

/// Extended uses market symbols like "ETH-USD", "BTC-USD" directly — they match canonical symbols.
pub fn get_extended_symbols() -> Vec<(&'static str, &'static str)> {
    vec![
        ("BTC-USD", "BTC-USD"),
        ("ETH-USD", "ETH-USD"),
        ("SOL-USD", "SOL-USD"),
    ]
}

pub fn extended_symbol_to_canonical(symbol: &str) -> Option<&'static str> {
    get_extended_symbols().iter().find(|(ext, _)| *ext == symbol).map(|(_, canon)| *canon)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_orderbook_parsing() {
        let json = r#"{"type":"SNAPSHOT","data":{"t":"SNAPSHOT","m":"ETH-USD","b":[{"q":"221.068","p":"2084.6"}],"a":[{"q":"45.800","p":"2084.7"}],"d":"1"},"ts":1771083590529,"seq":3}"#;
        let msg: ExtendedOrderbookMsg = serde_json::from_str(json).unwrap();
        assert_eq!(msg.msg_type.unwrap(), "SNAPSHOT");
        let data = msg.data.unwrap();
        assert_eq!(data.m.unwrap(), "ETH-USD");
        let bid = &data.b.unwrap()[0];
        assert_eq!(bid.p, "2084.6");
    }

    #[test]
    fn test_symbol_mapping() {
        assert_eq!(extended_symbol_to_canonical("BTC-USD"), Some("BTC-USD"));
        assert_eq!(extended_symbol_to_canonical("UNKNOWN"), None);
    }
}
