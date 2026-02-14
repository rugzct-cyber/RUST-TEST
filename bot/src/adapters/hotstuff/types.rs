//! HotStuff Types — JSON-RPC 2.0 ticker channel
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct HotstuffTickerParams {
    pub channel: Option<String>,
    pub data: Option<HotstuffTickerData>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HotstuffTickerData {
    pub instrument_name: Option<String>,
    pub best_bid_price: Option<String>,
    pub best_ask_price: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HotstuffJsonRpcMessage {
    pub id: Option<u64>,
    pub method: Option<String>,
    pub params: Option<HotstuffTickerParams>,
    pub result: Option<serde_json::Value>,
}

pub fn get_hotstuff_markets() -> Vec<(&'static str, &'static str)> {
    vec![
        ("BTC-PERP", "BTC-USD"),
        ("ETH-PERP", "ETH-USD"),
        ("SOL-PERP", "SOL-USD"),
    ]
}

pub fn instrument_to_symbol(instrument: &str) -> Option<&'static str> {
    get_hotstuff_markets().iter().find(|(inst, _)| *inst == instrument).map(|(_, sym)| *sym)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ticker_parsing() {
        let json = r#"{"method":"subscription","params":{"channel":"ticker","data":{"instrument_name":"BTC-PERP","best_bid_price":"96500","best_ask_price":"96501"}}}"#;
        let msg: HotstuffJsonRpcMessage = serde_json::from_str(json).unwrap();
        let data = msg.params.unwrap().data.unwrap();
        assert_eq!(data.instrument_name.unwrap(), "BTC-PERP");
    }

    #[test]
    fn test_instrument_to_symbol() {
        assert_eq!(instrument_to_symbol("BTC-PERP"), Some("BTC-USD"));
        assert_eq!(instrument_to_symbol("UNKNOWN"), None);
    }
}
