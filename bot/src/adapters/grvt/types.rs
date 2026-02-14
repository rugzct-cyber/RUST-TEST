//! GRVT Types
//!
//! API response types for GRVT WebSocket v1.mini.s mini ticker stream.
//! Uses JSON-RPC 2.0 protocol.
//!
//! Subscribe message: { id: N, method: "subscribe", params: { stream: "v1.mini.s", selectors: ["BTC_USDT_Perp@500"] } }
//! Response: { params: { data: { instrument: "BTC_USDT_Perp", best_bid_price: "96500", best_ask_price: "96501", ... } } }

use serde::Deserialize;

// =============================================================================
// WebSocket Message Types
// =============================================================================

/// Mini ticker data from v1.mini.s stream
#[derive(Debug, Clone, Deserialize)]
pub struct GrvtMiniTickerData {
    /// Instrument name (e.g. "BTC_USDT_Perp")
    pub instrument: String,
    /// Best bid price as string
    pub best_bid_price: String,
    /// Best ask price as string
    pub best_ask_price: String,
}

/// Params wrapper containing ticker data
#[derive(Debug, Clone, Deserialize)]
pub struct GrvtSubscribeParams {
    pub data: GrvtMiniTickerData,
}

/// Top-level JSON-RPC response for ticker updates
#[derive(Debug, Clone, Deserialize)]
pub struct GrvtTickerMessage {
    pub params: Option<GrvtSubscribeParams>,
    pub result: Option<serde_json::Value>,
    pub id: Option<u64>,
}

// =============================================================================
// Symbols — GRVT uses INSTRUMENT@RATE_MS format
// =============================================================================

/// GRVT market instrument names and their canonical symbols.
/// Each entry: (grvt_instrument, canonical_symbol)
pub fn get_grvt_markets() -> Vec<(&'static str, &'static str)> {
    vec![
        ("BTC_USDT_Perp", "BTC-USD"),
        ("ETH_USDT_Perp", "ETH-USD"),
        ("SOL_USDT_Perp", "SOL-USD"),
        ("ARB_USDT_Perp", "ARB-USD"),
        ("AVAX_USDT_Perp", "AVAX-USD"),
        ("DOGE_USDT_Perp", "DOGE-USD"),
        ("LINK_USDT_Perp", "LINK-USD"),
        ("SUI_USDT_Perp", "SUI-USD"),
        ("OP_USDT_Perp", "OP-USD"),
        ("APT_USDT_Perp", "APT-USD"),
        ("TIA_USDT_Perp", "TIA-USD"),
        ("INJ_USDT_Perp", "INJ-USD"),
        ("NEAR_USDT_Perp", "NEAR-USD"),
        ("WLD_USDT_Perp", "WLD-USD"),
        ("PEPE_USDT_Perp", "PEPE-USD"),
    ]
}

/// Convert GRVT instrument to canonical symbol (e.g. "BTC_USDT_Perp" → "BTC-USD")
pub fn instrument_to_symbol(instrument: &str) -> Option<String> {
    get_grvt_markets()
        .iter()
        .find(|(inst, _)| *inst == instrument)
        .map(|(_, sym)| sym.to_string())
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ticker_message_parsing() {
        let json = r#"{
            "id": 1,
            "params": {
                "data": {
                    "instrument": "BTC_USDT_Perp",
                    "best_bid_price": "96500.5",
                    "best_ask_price": "96501.0"
                }
            }
        }"#;

        let msg: GrvtTickerMessage = serde_json::from_str(json).unwrap();
        let params = msg.params.unwrap();
        assert_eq!(params.data.instrument, "BTC_USDT_Perp");
        assert_eq!(params.data.best_bid_price, "96500.5");
        assert_eq!(params.data.best_ask_price, "96501.0");
    }

    #[test]
    fn test_instrument_to_symbol() {
        assert_eq!(instrument_to_symbol("BTC_USDT_Perp"), Some("BTC-USD".to_string()));
        assert_eq!(instrument_to_symbol("ETH_USDT_Perp"), Some("ETH-USD".to_string()));
        assert_eq!(instrument_to_symbol("UNKNOWN"), None);
    }
}
