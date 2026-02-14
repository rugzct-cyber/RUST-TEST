//! Pacifica Types — prices channel
//!
//! Pacifica uses {"method": "subscribe", "params": {"source": "prices"}} for subscription.
//! Response format needs to be determined from actual data (will log RAW messages).

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct PacificaBboData {
    /// Symbol (e.g. "BTC-USD")
    pub s: Option<String>,
    /// Best bid price
    pub b: Option<String>,
    /// Best ask price
    pub a: Option<String>,
}

/// Flexible message type — we parse the raw JSON first and handle different formats
#[derive(Debug, Clone, Deserialize)]
pub struct PacificaPriceData {
    /// Market / symbol
    pub market: Option<String>,
    #[serde(alias = "s")]
    pub symbol: Option<String>,
    /// Best bid
    #[serde(alias = "b")]
    pub bid: Option<serde_json::Value>,
    /// Best ask
    #[serde(alias = "a")]
    pub ask: Option<serde_json::Value>,
    /// Mark price
    #[serde(rename = "markPrice")]
    pub mark_price: Option<String>,
    /// Index price
    #[serde(rename = "indexPrice")]
    pub index_price: Option<String>,
}

/// Root-level message from Pacifica WS
#[derive(Debug, Clone, Deserialize)]
pub struct PacificaWsResponse {
    /// Method or type field
    pub method: Option<String>,
    #[serde(rename = "type")]
    pub msg_type: Option<String>,
    /// Source field
    pub source: Option<String>,
    /// Data payload (could be object or array)
    pub data: Option<serde_json::Value>,
    /// Error info
    pub err: Option<String>,
    pub code: Option<i32>,
}

pub fn get_pacifica_symbols() -> Vec<&'static str> {
    vec!["BTC-USD", "ETH-USD", "SOL-USD", "AVAX-USD", "ARB-USD", "DOGE-USD", "LINK-USD", "SUI-USD"]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_response_parsing() {
        let json = r#"{"code":400,"err":"Error parsing request","t":1771083588130}"#;
        let msg: PacificaWsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(msg.code, Some(400));
        assert!(msg.err.is_some());
    }
}
