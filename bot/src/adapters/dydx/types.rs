//! dYdX Types
//!
//! API response types for dYdX v4 Indexer WebSocket v4_orderbook channel.
//!
//! Docs: https://docs.dydx.xyz/indexer-client/websockets
//!
//! Orderbook format:
//!   bids: [{price: "96500.0", size: "1.5"}, ...]
//!   asks: [{price: "96501.0", size: "0.8"}, ...]

use serde::Deserialize;

use crate::adapters::errors::ExchangeResult;
use crate::adapters::types::{Orderbook, OrderbookLevel, MAX_ORDERBOOK_DEPTH};

// =============================================================================
// WebSocket Message Types
// =============================================================================

/// A single price level in the dYdX orderbook
#[derive(Debug, Clone, Deserialize)]
pub struct DydxPriceLevel {
    /// Price as string
    pub price: String,
    /// Size as string
    pub size: String,
}

/// Orderbook contents (used for both initial snapshot and updates)
#[derive(Debug, Clone, Deserialize)]
pub struct DydxOrderbookContents {
    /// Bid levels
    #[serde(default)]
    pub bids: Vec<DydxPriceLevel>,
    /// Ask levels
    #[serde(default)]
    pub asks: Vec<DydxPriceLevel>,
}

impl DydxOrderbookContents {
    /// Convert to our canonical Orderbook type
    pub fn to_orderbook(&self) -> ExchangeResult<Orderbook> {
        let bids: Vec<OrderbookLevel> = self
            .bids
            .iter()
            .take(MAX_ORDERBOOK_DEPTH)
            .filter_map(|level| {
                let price = level.price.parse::<f64>().ok()?;
                let qty = level.size.parse::<f64>().ok()?;
                // Filter out zero-size levels (used for deletions in updates)
                if qty > 0.0 {
                    Some(OrderbookLevel::new(price, qty))
                } else {
                    None
                }
            })
            .collect();

        let asks: Vec<OrderbookLevel> = self
            .asks
            .iter()
            .take(MAX_ORDERBOOK_DEPTH)
            .filter_map(|level| {
                let price = level.price.parse::<f64>().ok()?;
                let qty = level.size.parse::<f64>().ok()?;
                if qty > 0.0 {
                    Some(OrderbookLevel::new(price, qty))
                } else {
                    None
                }
            })
            .collect();

        let orderbook = Orderbook {
            bids,
            asks,
            timestamp: crate::adapters::dydx::adapter::current_time_ms(),
        };

        tracing::debug!(
            exchange = "dydx",
            bids_count = orderbook.bids.len(),
            asks_count = orderbook.asks.len(),
            best_bid = ?orderbook.best_bid(),
            best_ask = ?orderbook.best_ask(),
            "Orderbook updated"
        );

        Ok(orderbook)
    }
}

/// Top-level WebSocket message from dYdX v4 Indexer
///
/// dYdX uses a `type` field to discriminate messages:
/// - `subscribed`: initial subscription response with snapshot
/// - `channel_data`: incremental orderbook updates
/// - `unsubscribed`: unsubscription confirmation
/// - `connected`: initial connection message
/// - `error`: error response
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum DydxWsMessage {
    /// Initial subscription response with full orderbook snapshot
    #[serde(rename = "subscribed")]
    Subscribed {
        #[allow(dead_code)]
        channel: Option<String>,
        id: Option<String>,
        contents: DydxOrderbookContents,
    },
    /// Incremental orderbook update
    #[serde(rename = "channel_data")]
    ChannelData {
        #[allow(dead_code)]
        channel: Option<String>,
        id: Option<String>,
        contents: DydxOrderbookContents,
    },
    /// Unsubscription confirmation
    #[serde(rename = "unsubscribed")]
    Unsubscribed {
        #[serde(default)]
        #[allow(dead_code)]
        channel: Option<String>,
        #[serde(default)]
        #[allow(dead_code)]
        id: Option<String>,
    },
    /// Connection established
    #[serde(rename = "connected")]
    Connected {
        #[serde(default)]
        #[allow(dead_code)]
        connection_id: Option<String>,
    },
    /// Error message
    #[serde(rename = "error")]
    Error {
        #[serde(default)]
        message: Option<String>,
    },
}

// =============================================================================
// Symbols — dYdX uses "COIN-USD" format for perpetual markets
// =============================================================================

/// Get the list of dYdX symbols to subscribe to (bare coin names).
/// We subscribe using "COIN-USD" as the market id.
pub fn get_dydx_symbols() -> Vec<&'static str> {
    vec![
        "BTC", "ETH", "SOL", "AVAX", "ARB", "DOGE", "LINK", "SUI",
        "OP", "MATIC", "APT", "TIA", "INJ", "SEI", "NEAR", "FTM",
        "ATOM", "WLD", "RUNE", "JUP", "WIF", "ONDO", "PENDLE", "STX",
        "PEPE", "FIL", "MKR", "AAVE", "LDO", "CRV", "RENDER", "PYTH",
        "JTO", "STRK", "ORDI", "DYM", "MANTA", "ENA", "TON",
        "ADA", "DOT", "UNI", "XRP", "LTC", "BCH", "HBAR",
    ]
}

/// Convert coin symbol to dYdX market id (e.g. "BTC" → "BTC-USD")
pub fn coin_to_market(coin: &str) -> String {
    format!("{}-USD", coin)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subscribed_message_parsing() {
        let json = r#"{
            "type": "subscribed",
            "connection_id": "abc123",
            "message_id": 1,
            "channel": "v4_orderbook",
            "id": "BTC-USD",
            "contents": {
                "bids": [
                    {"price": "96500.0", "size": "1.5"},
                    {"price": "96499.0", "size": "2.0"}
                ],
                "asks": [
                    {"price": "96501.0", "size": "0.8"},
                    {"price": "96502.0", "size": "1.2"}
                ]
            }
        }"#;

        let msg: DydxWsMessage = serde_json::from_str(json).unwrap();
        match msg {
            DydxWsMessage::Subscribed { id, contents, .. } => {
                assert_eq!(id, Some("BTC-USD".to_string()));
                let ob = contents.to_orderbook().unwrap();
                assert_eq!(ob.bids.len(), 2);
                assert_eq!(ob.asks.len(), 2);
                assert_eq!(ob.best_bid(), Some(96500.0));
                assert_eq!(ob.best_ask(), Some(96501.0));
            }
            other => panic!("Expected Subscribed, got {:?}", other),
        }
    }

    #[test]
    fn test_channel_data_parsing() {
        let json = r#"{
            "type": "channel_data",
            "connection_id": "abc123",
            "message_id": 2,
            "channel": "v4_orderbook",
            "id": "ETH-USD",
            "contents": {
                "bids": [{"price": "3200.0", "size": "5.0"}],
                "asks": [{"price": "3201.0", "size": "3.0"}]
            }
        }"#;

        let msg: DydxWsMessage = serde_json::from_str(json).unwrap();
        match msg {
            DydxWsMessage::ChannelData { id, contents, .. } => {
                assert_eq!(id, Some("ETH-USD".to_string()));
                let ob = contents.to_orderbook().unwrap();
                assert_eq!(ob.best_bid(), Some(3200.0));
                assert_eq!(ob.best_ask(), Some(3201.0));
            }
            other => panic!("Expected ChannelData, got {:?}", other),
        }
    }

    #[test]
    fn test_connected_message_parsing() {
        let json = r#"{"type": "connected", "connection_id": "abc123"}"#;
        let msg: DydxWsMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, DydxWsMessage::Connected { .. }));
    }

    #[test]
    fn test_error_message_parsing() {
        let json = r#"{"type": "error", "message": "Invalid channel"}"#;
        let msg: DydxWsMessage = serde_json::from_str(json).unwrap();
        match msg {
            DydxWsMessage::Error { message } => {
                assert_eq!(message, Some("Invalid channel".to_string()));
            }
            other => panic!("Expected Error, got {:?}", other),
        }
    }

    #[test]
    fn test_zero_size_levels_filtered() {
        let json = r#"{
            "type": "channel_data",
            "channel": "v4_orderbook",
            "id": "BTC-USD",
            "contents": {
                "bids": [
                    {"price": "96500.0", "size": "1.5"},
                    {"price": "96499.0", "size": "0"}
                ],
                "asks": [
                    {"price": "96501.0", "size": "0"},
                    {"price": "96502.0", "size": "0.8"}
                ]
            }
        }"#;

        let msg: DydxWsMessage = serde_json::from_str(json).unwrap();
        match msg {
            DydxWsMessage::ChannelData { contents, .. } => {
                let ob = contents.to_orderbook().unwrap();
                // Zero-size levels should be filtered out
                assert_eq!(ob.bids.len(), 1);
                assert_eq!(ob.asks.len(), 1);
                assert_eq!(ob.best_bid(), Some(96500.0));
                assert_eq!(ob.best_ask(), Some(96502.0));
            }
            other => panic!("Expected ChannelData, got {:?}", other),
        }
    }

    #[test]
    fn test_coin_to_market() {
        assert_eq!(coin_to_market("BTC"), "BTC-USD");
        assert_eq!(coin_to_market("ETH"), "ETH-USD");
    }
}
