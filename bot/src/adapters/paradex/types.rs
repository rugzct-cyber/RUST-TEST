//! Paradex Types
//!
//! API response types and data structures for Paradex exchange.
//! Public orderbook data only — no auth/order/position/margin types.

use serde::Deserialize;

use crate::adapters::errors::{ExchangeError, ExchangeResult};
use crate::adapters::types::{Orderbook, OrderbookLevel};

// =============================================================================
// WebSocket Message Types
// =============================================================================

/// JSON-RPC 2.0 response wrapper for Paradex WebSocket
#[derive(Debug, Deserialize)]
pub(crate) struct JsonRpcResponse {
    #[allow(dead_code)] // Required by JSON-RPC spec, validated by serde
    pub jsonrpc: String,
    #[allow(dead_code)] // Deserialized from JSON-RPC spec but not directly accessed
    pub result: Option<serde_json::Value>,
    pub error: Option<JsonRpcError>,
    pub id: u64,
}

/// JSON-RPC error
#[derive(Debug, Deserialize)]
pub(crate) struct JsonRpcError {
    pub code: i32,
    pub message: String,
}

/// JSON-RPC 2.0 subscription notification (for orderbook updates)
/// Different from JsonRpcResponse - no id field, has method="subscription"
#[derive(Debug, Deserialize)]
pub(crate) struct JsonRpcSubscriptionNotification {
    #[allow(dead_code)] // Required by JSON-RPC spec
    pub jsonrpc: String,
    /// Method is always "subscription" for notifications
    #[allow(dead_code)] // Required by JSON-RPC spec, validated by serde
    pub method: String,
    /// Subscription params containing channel and data
    pub params: SubscriptionParams,
}

/// Subscription notification params
#[derive(Debug, Deserialize)]
pub(crate) struct SubscriptionParams {
    /// Channel name (e.g., "order_book.ETH-USD-PERP.snapshot@15@100ms")
    pub channel: String,
    /// Orderbook data
    pub data: ParadexOrderbookData,
}

/// Paradex orderbook message from subscription
#[derive(Debug, Clone, Deserialize)]
pub struct ParadexOrderbookMessage {
    /// Channel name (e.g., "order_book.BTC-PERP.snapshot@15@100ms")
    pub channel: String,
    /// Orderbook data
    pub data: ParadexOrderbookData,
}

/// Orderbook data with bids and asks (Paradex format)
#[derive(Debug, Clone, Deserialize)]
pub struct ParadexOrderbookData {
    /// Market symbol
    pub market: String,
    /// Bid levels as inserts
    #[serde(default)]
    pub inserts: Vec<ParadexOrderbookLevel>,
    /// Timestamp in milliseconds
    pub last_updated_at: u64,
    /// Sequence number
    pub seq_no: u64,
}

/// Single orderbook level from Paradex
#[derive(Debug, Clone, Deserialize)]
pub struct ParadexOrderbookLevel {
    /// Price as string
    pub price: String,
    /// Quantity as string
    pub size: String,
    /// Side: "BID" or "ASK"
    pub side: String,
}

impl ParadexOrderbookData {
    /// Convert to Orderbook type, taking only top 10 levels per side
    ///
    /// If `usdc_rate` is provided, prices are converted from USD to USDC:
    /// `usdc_price = usd_price / usdc_rate`
    ///
    /// Example: If USDC rate is 0.9997, then 42000 USD = 42012.60 USDC
    pub fn to_orderbook(&self, usdc_rate: Option<f64>) -> ExchangeResult<Orderbook> {
        let mut bids: Vec<OrderbookLevel> = Vec::new();
        let mut asks: Vec<OrderbookLevel> = Vec::new();

        for level in &self.inserts {
            let mut price = level
                .price
                .parse::<f64>()
                .map_err(|e| ExchangeError::InvalidResponse(format!("Invalid price: {}", e)))?;

            // Convert USD → USDC if rate provided and within sane bounds
            if let Some(rate) = usdc_rate {
                if rate <= 0.0 || rate > 2.0 {
                    tracing::warn!(
                        "Paradex: suspicious USD/USDC rate {:.6}, skipping conversion",
                        rate
                    );
                } else {
                    price = price / rate;
                }
            }

            let quantity = level
                .size
                .parse::<f64>()
                .map_err(|e| ExchangeError::InvalidResponse(format!("Invalid quantity: {}", e)))?;

            let book_level = OrderbookLevel::new(price, quantity);

            match level.side.to_uppercase().as_str() {
                "BID" | "BUY" => bids.push(book_level),
                "ASK" | "SELL" => asks.push(book_level),
                other => {
                    tracing::warn!(side = %other, "Unknown Paradex orderbook side");
                }
            }
        }

        // Sort: bids descending (best bid = highest price first),
        //        asks ascending  (best ask = lowest price first)
        bids.sort_by(|a, b| {
            b.price
                .partial_cmp(&a.price)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        asks.sort_by(|a, b| {
            a.price
                .partial_cmp(&b.price)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Take only top N levels after sorting
        let depth = crate::adapters::types::MAX_ORDERBOOK_DEPTH;
        if bids.len() > depth || asks.len() > depth {
            tracing::debug!(
                exchange = "paradex",
                raw_bids = bids.len(),
                raw_asks = asks.len(),
                max_depth = depth,
                "Orderbook truncated to max depth"
            );
        }
        bids.truncate(depth);
        asks.truncate(depth);

        let orderbook = Orderbook {
            bids,
            asks,
            timestamp: self.last_updated_at,
        };

        // DEBUG log when orderbook is parsed
        tracing::debug!(
            exchange = "paradex",
            pair = %self.market,
            bids_count = orderbook.bids.len(),
            asks_count = orderbook.asks.len(),
            best_bid = ?orderbook.best_bid(),
            best_ask = ?orderbook.best_ask(),
            usdc_conversion = usdc_rate.is_some(),
            "Orderbook updated"
        );

        Ok(orderbook)
    }
}

/// Subscription confirmation response
#[derive(Debug, Deserialize)]
#[allow(dead_code)] // Used by serde to parse subscription confirmations
pub(crate) struct ParadexSubscriptionResponse {
    pub result: Option<serde_json::Value>,
    pub id: u64,
}

/// Generic WebSocket message that could be orderbook, subscription confirmation, etc.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum ParadexWsMessage {
    /// JSON-RPC subscription notification (orderbook updates)
    /// Must be listed before JsonRpc as it has more specific structure
    SubscriptionNotification(JsonRpcSubscriptionNotification),
    /// Orderbook update message with channel field (legacy/direct format)
    Orderbook(ParadexOrderbookMessage),
    /// JSON-RPC response (subscription confirmations)
    JsonRpc(JsonRpcResponse),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_paradex_orderbook_parsing() {
        let data = ParadexOrderbookData {
            market: "ETH-USD-PERP".to_string(),
            inserts: vec![
                ParadexOrderbookLevel {
                    price: "2500.50".to_string(),
                    size: "1.5".to_string(),
                    side: "BID".to_string(),
                },
                ParadexOrderbookLevel {
                    price: "2501.00".to_string(),
                    size: "2.0".to_string(),
                    side: "ASK".to_string(),
                },
            ],
            last_updated_at: 1700000000000,
            seq_no: 12345,
        };

        // Pass None for usdc_rate (no conversion)
        let orderbook = data.to_orderbook(None).unwrap();
        assert_eq!(orderbook.bids.len(), 1);
        assert_eq!(orderbook.asks.len(), 1);
        assert_eq!(orderbook.bids[0].price, 2500.50);
        assert_eq!(orderbook.asks[0].price, 2501.00);
    }

    #[test]
    fn test_paradex_orderbook_sorting() {
        let data = ParadexOrderbookData {
            market: "BTC-USD-PERP".to_string(),
            inserts: vec![
                ParadexOrderbookLevel {
                    price: "40000.00".to_string(),
                    size: "1.0".to_string(),
                    side: "BID".to_string(),
                },
                ParadexOrderbookLevel {
                    price: "40100.00".to_string(),
                    size: "1.0".to_string(),
                    side: "BID".to_string(),
                },
                ParadexOrderbookLevel {
                    price: "40200.00".to_string(),
                    size: "1.0".to_string(),
                    side: "ASK".to_string(),
                },
                ParadexOrderbookLevel {
                    price: "40150.00".to_string(),
                    size: "1.0".to_string(),
                    side: "ASK".to_string(),
                },
            ],
            last_updated_at: 1700000000000,
            seq_no: 12346,
        };

        // Pass None for usdc_rate (no conversion)
        let orderbook = data.to_orderbook(None).unwrap();
        // Bids should be sorted descending (best bid first)
        assert_eq!(orderbook.bids[0].price, 40100.00);
        assert_eq!(orderbook.bids[1].price, 40000.00);
        // Asks should be sorted ascending (best ask first)
        assert_eq!(orderbook.asks[0].price, 40150.00);
        assert_eq!(orderbook.asks[1].price, 40200.00);
    }

    #[test]
    fn test_to_orderbook_with_usdc_conversion() {
        // Given: Orderbook with USD price of 42000
        let data = ParadexOrderbookData {
            market: "BTC-USD-PERP".to_string(),
            inserts: vec![
                ParadexOrderbookLevel {
                    price: "42000.00".to_string(),
                    size: "1.0".to_string(),
                    side: "BID".to_string(),
                },
                ParadexOrderbookLevel {
                    price: "42010.00".to_string(),
                    size: "1.0".to_string(),
                    side: "ASK".to_string(),
                },
            ],
            last_updated_at: 1700000000000,
            seq_no: 12347,
        };

        // When: Convert with USDC rate of 0.9997
        // Expected: 42000 / 0.9997 = 42012.6037811...
        let orderbook = data.to_orderbook(Some(0.9997)).unwrap();

        // Then: Prices should be converted
        let expected_bid = 42000.0 / 0.9997;
        let expected_ask = 42010.0 / 0.9997;

        assert!(
            (orderbook.bids[0].price - expected_bid).abs() < 0.01,
            "Bid should be ~{:.2}, got {:.2}",
            expected_bid,
            orderbook.bids[0].price
        );
        assert!(
            (orderbook.asks[0].price - expected_ask).abs() < 0.01,
            "Ask should be ~{:.2}, got {:.2}",
            expected_ask,
            orderbook.asks[0].price
        );
    }
}
