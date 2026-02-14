//! Reya Types
//!
//! Reya WS uses /v2/prices channel.
//! Actual format: { type: "channel_data", channel: "/v2/prices", data: [{ symbol: "BTCRUSDPERP", poolPrice: "...", oraclePrice: "..." }] }
//! Also: { type: "subscribed", channel: "/v2/prices", contents: [...] }

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct ReyaPriceItem {
    pub symbol: Option<String>,
    #[serde(rename = "poolPrice")]
    pub pool_price: String,
    #[serde(rename = "oraclePrice")]
    pub oracle_price: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum ReyaWsMessage {
    #[serde(rename = "subscribed")]
    Subscribed {
        #[serde(default)]
        contents: Vec<ReyaPriceItem>,
    },
    #[serde(rename = "channel_data")]
    ChannelData {
        #[serde(default)]
        data: Vec<ReyaPriceItem>,
    },
    #[serde(rename = "pong")]
    Pong,
}

/// Map Reya's symbol strings to canonical symbols.
/// Reya uses format like "BTCRUSDPERP", "ETHRUSDPERP", "SOLRUSDPERP"
pub fn reya_symbol_to_canonical(symbol: &str) -> Option<&'static str> {
    match symbol {
        "BTCRUSDPERP" => Some("BTC-USD"),
        "ETHRUSDPERP" => Some("ETH-USD"),
        "SOLRUSDPERP" => Some("SOL-USD"),
        "ADARUSDPERP" => Some("ADA-USD"),
        "AVAXRUSDPERP" => Some("AVAX-USD"),
        "DOGERUSDPERP" => Some("DOGE-USD"),
        "LINKRUSDPERP" => Some("LINK-USD"),
        "OPMRUSDPERP" | "OPRUSDPERP" => Some("OP-USD"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_data_parsing() {
        let json = r#"{"type":"channel_data","timestamp":1771083587735,"channel":"/v2/prices","data":[{"symbol":"BTCRUSDPERP","oraclePrice":"96500.5","poolPrice":"96501.0"}]}"#;
        let msg: ReyaWsMessage = serde_json::from_str(json).unwrap();
        match msg {
            ReyaWsMessage::ChannelData { data } => {
                assert_eq!(data.len(), 1);
                assert_eq!(data[0].symbol.as_deref(), Some("BTCRUSDPERP"));
            }
            _ => panic!("Expected ChannelData"),
        }
    }

    #[test]
    fn test_subscribed_parsing() {
        let json = r#"{"type":"subscribed","channel":"/v2/prices","contents":[{"symbol":"BTCRUSDPERP","oraclePrice":"96500.5","poolPrice":"96501.0"}]}"#;
        let msg: ReyaWsMessage = serde_json::from_str(json).unwrap();
        match msg {
            ReyaWsMessage::Subscribed { contents } => {
                assert_eq!(contents.len(), 1);
            }
            _ => panic!("Expected Subscribed"),
        }
    }

    #[test]
    fn test_symbol_mapping() {
        assert_eq!(reya_symbol_to_canonical("BTCRUSDPERP"), Some("BTC-USD"));
        assert_eq!(reya_symbol_to_canonical("ETHRUSDPERP"), Some("ETH-USD"));
        assert_eq!(reya_symbol_to_canonical("UNKNOWN"), None);
    }
}
