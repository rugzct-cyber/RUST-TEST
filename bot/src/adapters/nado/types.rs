//! Nado Types — /v1/subscribe with permessage-deflate (via yawc)
//!
//! Product IDs (from docs.nado.xyz/developer-resources/api/symbols):
//!   2 = BTC-PERP, 4 = ETH-PERP, 8 = SOL-PERP
//! Prices are scaled by 1e18.

use serde::{Deserialize, Serialize};

// ── Outgoing: subscribe to a stream ────────────────────────────────
#[derive(Debug, Clone, Serialize)]
pub struct NadoStreamDef {
    #[serde(rename = "type")]
    pub stream_type: String,
    pub product_id: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct NadoSubscribeMsg {
    pub method: String,
    pub stream: NadoStreamDef,
    pub id: u32,
}

impl NadoSubscribeMsg {
    pub fn book_depth(product_id: u32, id: u32) -> Self {
        Self {
            method: "subscribe".to_string(),
            stream: NadoStreamDef { stream_type: "book_depth".to_string(), product_id },
            id,
        }
    }

    #[allow(dead_code)]
    pub fn best_bid_offer(product_id: u32, id: u32) -> Self {
        Self {
            method: "subscribe".to_string(),
            stream: NadoStreamDef { stream_type: "best_bid_offer".to_string(), product_id },
            id,
        }
    }
}

// ── Incoming: book_depth event (matches original TS nado-ws.ts) ────
// Format: {"type":"book_depth","product_id":2,"bids":[["86129000000000000000000","1219100000000000000"]],"asks":[...]}
#[derive(Debug, Clone, Deserialize)]
pub struct NadoBookDepthEvent {
    #[serde(rename = "type")]
    pub event_type: Option<String>,
    pub product_id: Option<u32>,
    pub bids: Option<Vec<Vec<String>>>,  // [[price_x18, size_x18], ...]
    pub asks: Option<Vec<Vec<String>>>,  // [[price_x18, size_x18], ...]
}

// ── Incoming: best_bid_offer event ─────────────────────────────────
#[derive(Debug, Clone, Deserialize)]
pub struct NadoBboEvent {
    #[serde(rename = "type")]
    pub event_type: Option<String>,
    pub product_id: Option<u32>,
    pub bid_price: Option<String>,
    pub ask_price: Option<String>,
    pub bid_qty: Option<String>,
    pub ask_qty: Option<String>,
    pub timestamp: Option<String>,
}

// ── Incoming: subscription confirmation ────────────────────────────
#[derive(Debug, Clone, Deserialize)]
pub struct NadoSubResponse {
    pub result: Option<serde_json::Value>,
    pub id: Option<u32>,
}

/// Convert Nado BigInt price string (1e18 scale) to f64
pub fn parse_nado_price(price_str: &str) -> Option<f64> {
    let val: f64 = price_str.parse().ok()?;
    let price = val / 1e18;
    if price > 0.0 && price < 1_000_000.0 { Some(price) } else { None }
}

pub fn get_nado_markets() -> Vec<(u32, &'static str)> {
    vec![
        (2, "BTC-USD"),   // BTC-PERP
        (4, "ETH-USD"),   // ETH-PERP
        (8, "SOL-USD"),   // SOL-PERP
    ]
}

pub fn product_id_to_symbol(product_id: u32) -> Option<&'static str> {
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
    fn test_parse_nado_price_btc() {
        let price = parse_nado_price("86129000000000000000000").unwrap();
        assert!((price - 86129.0).abs() < 0.01);
    }

    #[test]
    fn test_product_id_to_symbol() {
        assert_eq!(product_id_to_symbol(2), Some("BTC-USD"));
        assert_eq!(product_id_to_symbol(4), Some("ETH-USD"));
        assert_eq!(product_id_to_symbol(8), Some("SOL-USD"));
        assert_eq!(product_id_to_symbol(999), None);
    }

    #[test]
    fn test_subscribe_book_depth_serialization() {
        let msg = NadoSubscribeMsg::book_depth(2, 1);
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"method\":\"subscribe\""));
        assert!(json.contains("\"type\":\"book_depth\""));
        assert!(json.contains("\"product_id\":2"));
    }

    #[test]
    fn test_book_depth_event_parsing() {
        let json = r#"{"type":"book_depth","product_id":2,"bids":[["86129000000000000000000","1219100000000000000"]],"asks":[["86200000000000000000000","500000000000000000"]]}"#;
        let evt: NadoBookDepthEvent = serde_json::from_str(json).unwrap();
        assert_eq!(evt.event_type.as_deref(), Some("book_depth"));
        assert_eq!(evt.product_id, Some(2));
        let bids = evt.bids.as_ref().unwrap();
        assert_eq!(bids.len(), 1);
        let bid_price = parse_nado_price(&bids[0][0]).unwrap();
        assert!((bid_price - 86129.0).abs() < 0.01);
    }

    #[test]
    fn test_bbo_event_parsing() {
        let json = r#"{"type":"best_bid_offer","timestamp":"1676151190656903000","product_id":2,"bid_price":"69723000000000000000000","bid_qty":"5000000000000000000","ask_price":"69761000000000000000000","ask_qty":"3000000000000000000"}"#;
        let evt: NadoBboEvent = serde_json::from_str(json).unwrap();
        assert_eq!(evt.product_id, Some(2));
        let bid = parse_nado_price(evt.bid_price.as_ref().unwrap()).unwrap();
        assert!((bid - 69723.0).abs() < 0.01);
    }
}
