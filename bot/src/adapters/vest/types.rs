//! Vest Types
//!
//! API response types and data structures for Vest exchange.

use serde::Deserialize;

use crate::adapters::errors::{ExchangeError, ExchangeResult};
use crate::adapters::types::{Orderbook, OrderbookLevel};


// =============================================================================
// WebSocket Message Types for Orderbook Streaming
// =============================================================================

/// Vest depth channel message (orderbook update)
#[derive(Debug, Clone, Deserialize)]
pub struct VestDepthMessage {
    pub channel: String,
    pub data: VestDepthData,
}

/// Depth data containing bids and asks
#[derive(Debug, Clone, Deserialize)]
pub struct VestDepthData {
    pub bids: Vec<[String; 2]>,
    pub asks: Vec<[String; 2]>,
}

impl VestDepthData {
    /// Convert to Orderbook type, taking only top 10 levels
    pub fn to_orderbook(&self) -> ExchangeResult<Orderbook> {
        let mut bids: Vec<OrderbookLevel> = self
            .bids
            .iter()
            .map(|[price, qty]| {
                let p = price.parse::<f64>().map_err(|e| {
                    ExchangeError::InvalidResponse(format!("Invalid bid price: {}", e))
                })?;
                let q = qty.parse::<f64>().map_err(|e| {
                    ExchangeError::InvalidResponse(format!("Invalid bid quantity: {}", e))
                })?;
                Ok(OrderbookLevel::new(p, q))
            })
            .collect::<ExchangeResult<Vec<_>>>()?;

        let mut asks: Vec<OrderbookLevel> = self
            .asks
            .iter()
            .map(|[price, qty]| {
                let p = price.parse::<f64>().map_err(|e| {
                    ExchangeError::InvalidResponse(format!("Invalid ask price: {}", e))
                })?;
                let q = qty.parse::<f64>().map_err(|e| {
                    ExchangeError::InvalidResponse(format!("Invalid ask quantity: {}", e))
                })?;
                Ok(OrderbookLevel::new(p, q))
            })
            .collect::<ExchangeResult<Vec<_>>>()?;

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

        let depth = crate::adapters::types::MAX_ORDERBOOK_DEPTH;
        if bids.len() > depth || asks.len() > depth {
            tracing::debug!(
                exchange = "vest",
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
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        };

        tracing::debug!(
            exchange = "vest",
            bids_count = orderbook.bids.len(),
            asks_count = orderbook.asks.len(),
            best_bid = ?orderbook.best_bid(),
            best_ask = ?orderbook.best_ask(),
            "Orderbook updated"
        );

        Ok(orderbook)
    }
}

/// Subscription confirmation response
#[derive(Debug, Deserialize)]
pub(crate) struct VestSubscriptionResponse {
    #[allow(dead_code)]
    pub result: Option<serde_json::Value>,
    pub id: u64,
}

/// Generic WebSocket message
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum VestWsMessage {
    Depth(VestDepthMessage),
    Pong {
        #[serde(rename = "data")]
        _data: String,
    },
    Subscription(VestSubscriptionResponse),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vest_depth_data_parsing() {
        let data = VestDepthData {
            bids: vec![
                ["50000.00".to_string(), "1.5".to_string()],
                ["49999.00".to_string(), "2.0".to_string()],
            ],
            asks: vec![
                ["50001.00".to_string(), "1.0".to_string()],
                ["50002.00".to_string(), "0.5".to_string()],
            ],
        };

        let orderbook = data.to_orderbook().unwrap();
        assert_eq!(orderbook.bids.len(), 2);
        assert_eq!(orderbook.asks.len(), 2);
        assert_eq!(orderbook.bids[0].price, 50000.0);
        assert_eq!(orderbook.asks[0].price, 50001.0);
    }

    #[test]
    fn test_vest_pong_parsing() {
        let pong_json = r#"{"data": "PONG"}"#;
        let result: Result<VestWsMessage, _> = serde_json::from_str(pong_json);

        match result {
            Ok(VestWsMessage::Pong { .. }) => {}
            Ok(other) => panic!("Expected Pong variant, got: {:?}", other),
            Err(e) => panic!("Failed to parse PONG message: {}", e),
        }
    }
}
