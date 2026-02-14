//! Hyperliquid Types
//!
//! API response types for Hyperliquid WebSocket l2Book channel.
//!
//! Docs: https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/websocket/subscriptions
//!
//! L2 Book format:
//!   levels[0] = Bids (highest to lowest)
//!   levels[1] = Asks (lowest to highest)
//!   Each level: { px: "price", sz: "size", n: count }

use serde::Deserialize;

use crate::adapters::errors::{ExchangeError, ExchangeResult};
use crate::adapters::types::{Orderbook, OrderbookLevel, MAX_ORDERBOOK_DEPTH};

// =============================================================================
// WebSocket Message Types
// =============================================================================

/// A single level in the Hyperliquid L2 orderbook
#[derive(Debug, Clone, Deserialize)]
pub struct HyperliquidLevel {
    /// Price as string
    pub px: String,
    /// Size as string
    pub sz: String,
    /// Number of orders at this level
    #[allow(dead_code)]
    pub n: u64,
}

/// L2 Book data payload
#[derive(Debug, Clone, Deserialize)]
pub struct HyperliquidBookData {
    /// Coin symbol (e.g. "BTC", "ETH")
    pub coin: String,
    /// [bids, asks] — bids descending, asks ascending
    pub levels: (Vec<HyperliquidLevel>, Vec<HyperliquidLevel>),
    /// Timestamp in milliseconds
    #[allow(dead_code)]
    pub time: u64,
}

impl HyperliquidBookData {
    /// Convert to our canonical Orderbook type
    pub fn to_orderbook(&self) -> ExchangeResult<Orderbook> {
        let bids: Vec<OrderbookLevel> = self
            .levels
            .0
            .iter()
            .take(MAX_ORDERBOOK_DEPTH)
            .map(|level| {
                let price = level.px.parse::<f64>().map_err(|e| {
                    ExchangeError::InvalidResponse(format!("Invalid bid price: {}", e))
                })?;
                let qty = level.sz.parse::<f64>().map_err(|e| {
                    ExchangeError::InvalidResponse(format!("Invalid bid size: {}", e))
                })?;
                Ok(OrderbookLevel::new(price, qty))
            })
            .collect::<ExchangeResult<Vec<_>>>()?;

        let asks: Vec<OrderbookLevel> = self
            .levels
            .1
            .iter()
            .take(MAX_ORDERBOOK_DEPTH)
            .map(|level| {
                let price = level.px.parse::<f64>().map_err(|e| {
                    ExchangeError::InvalidResponse(format!("Invalid ask price: {}", e))
                })?;
                let qty = level.sz.parse::<f64>().map_err(|e| {
                    ExchangeError::InvalidResponse(format!("Invalid ask size: {}", e))
                })?;
                Ok(OrderbookLevel::new(price, qty))
            })
            .collect::<ExchangeResult<Vec<_>>>()?;

        let orderbook = Orderbook {
            bids,
            asks,
            timestamp: self.time,
        };

        tracing::debug!(
            exchange = "hyperliquid",
            coin = %self.coin,
            bids_count = orderbook.bids.len(),
            asks_count = orderbook.asks.len(),
            best_bid = ?orderbook.best_bid(),
            best_ask = ?orderbook.best_ask(),
            "Orderbook updated"
        );

        Ok(orderbook)
    }
}

/// Top-level WebSocket message (channel envelope)
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "channel", content = "data")]
pub(crate) enum HyperliquidWsMessage {
    /// L2 Book snapshot/update
    #[serde(rename = "l2Book")]
    L2Book(HyperliquidBookData),
    /// Pong response
    #[serde(rename = "pong")]
    Pong,
    /// Subscription confirmation
    #[serde(rename = "subscriptionResponse")]
    SubscriptionResponse(serde_json::Value),
}

// =============================================================================
// Symbols — Hyperliquid uses bare coin names (BTC, ETH, SOL, etc.)
// =============================================================================

/// Get the list of Hyperliquid symbols to subscribe to.
/// Hyperliquid uses plain coin symbols for l2Book subscriptions.
pub fn get_hyperliquid_symbols() -> Vec<&'static str> {
    vec![
        "BTC", "ETH", "SOL", "AVAX", "ARB", "DOGE", "LINK", "SUI",
        "OP", "MATIC", "APT", "TIA", "INJ", "SEI", "NEAR", "FTM",
        "ATOM", "WLD", "RUNE", "JUP", "WIF", "ONDO", "PENDLE", "STX",
        "PEPE", "FIL", "MKR", "AAVE", "LDO", "CRV", "RENDER", "PYTH",
        "JTO", "STRK", "ORDI", "DYM", "MANTA", "W", "ENA", "TON",
        "BONK", "ADA", "DOT", "UNI", "XRP", "LTC", "BCH", "HBAR",
    ]
}

/// Convert Hyperliquid coin to canonical symbol (e.g. "BTC" → "BTC-USD")
pub fn coin_to_symbol(coin: &str) -> String {
    format!("{}-USD", coin)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_l2book_message_parsing() {
        let json = r#"{
            "channel": "l2Book",
            "data": {
                "coin": "BTC",
                "levels": [
                    [{"px": "96500.0", "sz": "1.5", "n": 3}],
                    [{"px": "96501.0", "sz": "0.8", "n": 2}]
                ],
                "time": 1700000000000
            }
        }"#;

        let msg: HyperliquidWsMessage = serde_json::from_str(json).unwrap();
        match msg {
            HyperliquidWsMessage::L2Book(book) => {
                assert_eq!(book.coin, "BTC");
                let ob = book.to_orderbook().unwrap();
                assert_eq!(ob.bids.len(), 1);
                assert_eq!(ob.asks.len(), 1);
                assert_eq!(ob.best_bid(), Some(96500.0));
                assert_eq!(ob.best_ask(), Some(96501.0));
            }
            other => panic!("Expected L2Book, got {:?}", other),
        }
    }

    #[test]
    fn test_coin_to_symbol() {
        assert_eq!(coin_to_symbol("BTC"), "BTC-USD");
        assert_eq!(coin_to_symbol("ETH"), "ETH-USD");
    }

    #[test]
    fn test_pong_parsing() {
        let json = r#"{"channel": "pong"}"#;
        let msg: HyperliquidWsMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, HyperliquidWsMessage::Pong));
    }
}
