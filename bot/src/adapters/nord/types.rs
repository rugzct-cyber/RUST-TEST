//! Nord Types — delta stream
//!
//! Actual API format:
//! {"delta":{"last_update_id":...,"update_id":...,"market_symbol":"BTCUSD","asks":[[price,qty],...],"bids":[[price,qty],...]}}

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct NordDeltaData {
    pub market_symbol: Option<String>,
    /// Bids as [[price, qty], ...]
    #[serde(default)]
    pub bids: Vec<(f64, f64)>,
    /// Asks as [[price, qty], ...]
    #[serde(default)]
    pub asks: Vec<(f64, f64)>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NordWsMessage {
    pub delta: Option<NordDeltaData>,
}

pub fn get_nord_markets() -> Vec<(&'static str, &'static str)> {
    vec![
        ("BTCUSD", "BTC-USD"),
        ("ETHUSD", "ETH-USD"),
        ("SOLUSD", "SOL-USD"),
    ]
}

pub fn nord_symbol_to_canonical(symbol: &str) -> Option<&'static str> {
    get_nord_markets().iter().find(|(s, _)| *s == symbol).map(|(_, c)| *c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delta_parsing() {
        let json = r#"{"delta":{"last_update_id":123,"update_id":124,"market_symbol":"BTCUSD","asks":[[69830.0,0.5],[69831.0,0.3]],"bids":[[69764.1,0.00186]]}}"#;
        let msg: NordWsMessage = serde_json::from_str(json).unwrap();
        let delta = msg.delta.unwrap();
        assert_eq!(delta.market_symbol.as_deref(), Some("BTCUSD"));
        assert_eq!(delta.bids.len(), 1);
        assert_eq!(delta.asks.len(), 2);
        assert!((delta.bids[0].0 - 69764.1).abs() < 0.01);
    }

    #[test]
    fn test_symbol_mapping() {
        assert_eq!(nord_symbol_to_canonical("BTCUSD"), Some("BTC-USD"));
        assert_eq!(nord_symbol_to_canonical("UNKNOWN"), None);
    }
}
