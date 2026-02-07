//! Vest Types
//!
//! API response types and data structures for Vest exchange.

use std::time::Instant;
use serde::Deserialize;

use crate::adapters::errors::{ExchangeError, ExchangeResult};
use crate::adapters::types::{Orderbook, OrderbookLevel, OrderRequest};

// =============================================================================
// REST API Response Types
// =============================================================================

/// Response from POST /register
#[derive(Debug, Deserialize)]
pub(crate) struct RegisterResponse {
    #[serde(rename = "apiKey")]
    pub api_key: Option<String>,
    pub code: Option<i32>,
    pub msg: Option<String>,
}

/// Response from POST /listenKey
#[derive(Debug, Deserialize)]
pub(crate) struct ListenKeyResponse {
    #[serde(rename = "listenKey")]
    pub listen_key: Option<String>,
    pub code: Option<i32>,
    pub msg: Option<String>,
}

/// Response from POST /orders (Vest API)
#[derive(Debug, Deserialize)]
#[allow(dead_code)] // Some fields used only by serde for complete API response parsing
pub(crate) struct VestOrderResponse {
    /// Exchange-assigned order ID (Vest returns "id", not "orderId")
    pub id: Option<String>,
    /// Nonce used for the order
    pub nonce: Option<u64>,
    /// Order status: NEW, PARTIALLY_FILLED, FILLED, CANCELLED, REJECTED
    pub status: Option<String>,
    /// Size of the order
    pub size: Option<String>,
    /// Order type
    #[serde(rename = "orderType")]
    pub order_type: Option<String>,
    /// Post time
    #[serde(rename = "postTime")]
    pub post_time: Option<u64>,
    /// Average fill price (only present when status is FILLED)
    #[serde(rename = "avgFilledPrice")]
    pub avg_filled_price: Option<String>,
    /// Last filled price (only present when status is FILLED)
    #[serde(rename = "lastFilledPrice")]
    pub last_filled_price: Option<String>,
    /// Error code if any
    pub code: Option<i32>,
    /// Error message
    pub msg: Option<String>,
}

/// Response from GET /account (Vest API)
#[derive(Debug, Deserialize)]
#[allow(dead_code)] // Some fields used only by serde
pub struct VestAccountResponse {
    /// List of active positions
    #[serde(default)]
    pub positions: Vec<VestPositionData>,
    /// Account balances
    #[serde(default)]
    pub balances: Vec<VestBalanceData>,
    /// Leverage settings per symbol
    #[serde(default)]
    pub leverages: Vec<VestLeverageData>,
}

/// Position data from Vest API
#[derive(Debug, Clone, Deserialize)]
pub struct VestPositionData {
    /// Trading symbol (e.g., "BTC-PERP")
    pub symbol: Option<String>,
    /// Position side: true = long, false = short
    #[serde(rename = "isLong")]
    pub is_long: Option<bool>,
    /// Position size (always positive, use isLong for direction)
    pub size: Option<String>,
    /// Entry price
    #[serde(rename = "entryPrice")]
    pub entry_price: Option<String>,
    /// Mark price
    #[serde(rename = "markPrice")]
    pub mark_price: Option<String>,
    /// Unrealized PnL
    #[serde(rename = "unrealizedPnl")]
    pub unrealized_pnl: Option<String>,
    /// Realized PnL
    #[serde(rename = "realizedPnl")]
    pub realized_pnl: Option<String>,
    /// Liquidation price
    #[serde(rename = "liquidationPrice")]
    pub liquidation_price: Option<String>,
}

/// Balance data from Vest API
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]  // Fields used by serde for parsing
pub struct VestBalanceData {
    /// Asset symbol (e.g., "USDC")
    pub asset: Option<String>,
    /// Available balance
    pub available: Option<String>,
    /// Total balance
    pub total: Option<String>,
}

/// Leverage setting per symbol from Vest API
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)] // Fields used by serde
pub struct VestLeverageData {
    /// Trading symbol
    pub symbol: Option<String>,
    /// Leverage value (e.g., 10)
    pub value: Option<u32>,
}

/// Response from POST /account/leverage (Vest API)
#[derive(Debug, Deserialize)]
#[allow(dead_code)] // Fields used by serde
pub(crate) struct VestLeverageResponse {
    /// Symbol the leverage was set for
    pub symbol: Option<String>,
    /// New leverage value
    pub value: Option<u32>,
}

/// Pre-signed order for optimized latency
/// Sign the order in advance, then send it quickly when needed
#[derive(Debug, Clone)]
pub struct PreSignedOrder {
    /// The original order request
    pub order: OrderRequest,
    /// Pre-computed signature
    pub signature: String,
    /// Timestamp used in signature
    pub time: u64,
    /// Nonce used in signature
    pub nonce: u64,
    /// When this was created (for expiration check)
    pub created_at: Instant,
    /// Formatted size string (pre-computed)
    pub size_str: String,
    /// Formatted price string (pre-computed)
    pub price_str: String,
}

/// Pre-signed orders expire after this many seconds (leaving ~5s buffer for recvWindow of 60s)
const SIGNATURE_EXPIRY_SECS: u64 = 55;

impl PreSignedOrder {
    /// Check if this pre-signed order is still valid (not expired)
    pub fn is_valid(&self) -> bool {
        self.created_at.elapsed().as_secs() < SIGNATURE_EXPIRY_SECS
    }
}

// =============================================================================
// WebSocket Message Types for Orderbook Streaming
// =============================================================================

/// Vest depth channel message (orderbook update)
#[derive(Debug, Clone, Deserialize)]
pub struct VestDepthMessage {
    /// Channel name (e.g., "BTC-PERP@depth")
    pub channel: String,
    /// Depth data with bids and asks
    pub data: VestDepthData,
}

/// Depth data containing bids and asks
#[derive(Debug, Clone, Deserialize)]
pub struct VestDepthData {
    /// Bid levels as ["price", "quantity"]
    pub bids: Vec<[String; 2]>,
    /// Ask levels as ["price", "quantity"]
    pub asks: Vec<[String; 2]>,
}

impl VestDepthData {
    /// Convert to Orderbook type, taking only top 10 levels
    /// 
    /// Parses all levels, sorts by price, then truncates to top 10.
    /// Bids: sorted descending (highest first = best bid)
    /// Asks: sorted ascending (lowest first = best ask)
    pub fn to_orderbook(&self) -> ExchangeResult<Orderbook> {
        // Parse ALL bids first, then sort and truncate
        let mut bids: Vec<OrderbookLevel> = self.bids.iter()
            .map(|[price, qty]| {
                let p = price.parse::<f64>().map_err(|e| 
                    ExchangeError::InvalidResponse(format!("Invalid bid price: {}", e)))?;
                let q = qty.parse::<f64>().map_err(|e| 
                    ExchangeError::InvalidResponse(format!("Invalid bid quantity: {}", e)))?;
                Ok(OrderbookLevel::new(p, q))
            })
            .collect::<ExchangeResult<Vec<_>>>()?;
        
        // Parse ALL asks first, then sort and truncate
        let mut asks: Vec<OrderbookLevel> = self.asks.iter()
            .map(|[price, qty]| {
                let p = price.parse::<f64>().map_err(|e| 
                    ExchangeError::InvalidResponse(format!("Invalid ask price: {}", e)))?;
                let q = qty.parse::<f64>().map_err(|e| 
                    ExchangeError::InvalidResponse(format!("Invalid ask quantity: {}", e)))?;
                Ok(OrderbookLevel::new(p, q))
            })
            .collect::<ExchangeResult<Vec<_>>>()?;
        
        // Sort bids descending (highest price first = best bid)
        bids.sort_by(|a, b| b.price.partial_cmp(&a.price).unwrap_or(std::cmp::Ordering::Equal));
        // Sort asks ascending (lowest price first = best ask)
        asks.sort_by(|a, b| a.price.partial_cmp(&b.price).unwrap_or(std::cmp::Ordering::Equal));
        
        // Take only top 10 levels after sorting
        bids.truncate(crate::adapters::types::MAX_ORDERBOOK_DEPTH);
        asks.truncate(crate::adapters::types::MAX_ORDERBOOK_DEPTH);
        
        let orderbook = Orderbook {
            bids,
            asks,
            timestamp: super::signing::current_time_ms(),
        };

        // Story 1.3: DEBUG log when orderbook is parsed
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
    #[allow(dead_code)] // Used by serde for parsing subscription confirmations
    pub result: Option<serde_json::Value>,
    pub id: u64,
}

/// Generic WebSocket message that could be depth, subscription confirmation, etc.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum VestWsMessage {
    /// Depth update message with channel field
    Depth(VestDepthMessage),
    /// PONG response - must be before Subscription to match first
    /// Format: {"data": "PONG"}
    Pong {
        #[serde(rename = "data")]
        _data: String,
    },
    /// Subscription/unsubscription confirmation
    Subscription(VestSubscriptionResponse),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_presigned_order_validity() {
        use crate::adapters::types::{OrderType, OrderSide};
        
        let order = OrderRequest {
            symbol: "BTC-PERP".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            quantity: 0.001,
            price: Some(50000.0),
            reduce_only: false,
            client_order_id: "test-123".to_string(),
            time_in_force: crate::adapters::types::TimeInForce::Gtc,
        };
        
        let presigned = PreSignedOrder {
            order,
            signature: "0x123".to_string(),
            time: 0,
            nonce: 0,
            created_at: Instant::now(),
            size_str: "0.001".to_string(),
            price_str: "50000.00".to_string(),
        };
        
        assert!(presigned.is_valid());
    }

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
            Ok(VestWsMessage::Pong { .. }) => {
                // Success - PONG was parsed correctly
            }
            Ok(other) => panic!("Expected Pong variant, got: {:?}", other),
            Err(e) => panic!("Failed to parse PONG message: {}", e),
        }
    }
}
