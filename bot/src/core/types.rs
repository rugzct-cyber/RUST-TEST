//! Core data types for the price aggregation and arbitrage detection pipeline.
//!
//! These types mirror the arbi-v5 TypeScript `@arbitrage/shared` package,
//! providing a common vocabulary for prices, aggregated views, and opportunities.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

// =============================================================================
// Price Data (from a single exchange)
// =============================================================================

/// Price data emitted by an exchange adapter.
///
/// Equivalent to the TypeScript `PriceData` interface.
/// Uses `Arc<str>` for zero-copy sharing across broadcast subscribers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceData {
    /// Exchange identifier (e.g. "vest", "paradex", "lighter")
    pub exchange: Arc<str>,
    /// Normalized symbol (e.g. "BTC", "ETH", "SOL")
    pub symbol: Arc<str>,
    /// Best bid price
    pub bid: f64,
    /// Best ask price
    pub ask: f64,
    /// Timestamp in milliseconds (epoch)
    pub timestamp_ms: u64,
}

impl PriceData {
    /// Compute bid-ask spread in basis points
    #[inline]
    pub fn spread_bps(&self) -> f64 {
        if self.bid == 0.0 {
            return 0.0;
        }
        ((self.ask - self.bid) / self.bid) * 10_000.0
    }
}

// =============================================================================
// Exchange Price (best bid or best ask from one exchange)
// =============================================================================

/// Reference to the best price from a specific exchange.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExchangePrice {
    pub exchange: Arc<str>,
    pub price: f64,
}

// =============================================================================
// Aggregated Price (across all exchanges for one symbol)
// =============================================================================

/// Aggregated price across all exchanges for a single symbol.
///
/// Equivalent to the TypeScript `AggregatedPrice` interface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatedPrice {
    /// Normalized symbol
    pub symbol: Arc<str>,
    /// All valid (non-stale) prices from each exchange
    pub prices: Vec<PriceData>,
    /// Highest bid across exchanges
    pub best_bid: Option<ExchangePrice>,
    /// Lowest ask across exchanges
    pub best_ask: Option<ExchangePrice>,
    /// Timestamp of aggregation
    pub timestamp_ms: u64,
}

// =============================================================================
// Arbitrage Opportunity
// =============================================================================

/// Detected cross-exchange arbitrage opportunity.
///
/// Equivalent to the TypeScript `ArbitrageOpportunity` interface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbitrageOpportunity {
    /// Normalized symbol
    pub symbol: Arc<str>,
    /// Exchange with the best ask (buy here)
    pub buy_exchange: Arc<str>,
    /// Exchange with the best bid (sell here)
    pub sell_exchange: Arc<str>,
    /// Best ask price (buy price)
    pub buy_price: f64,
    /// Best bid price (sell price)
    pub sell_price: f64,
    /// Spread as a percentage: (sell - buy) / buy * 100
    pub spread_percent: f64,
    /// Detection timestamp
    pub timestamp_ms: u64,
}

// =============================================================================
// Broadcast Event (union type for WebSocket clients)
// =============================================================================

/// Events broadcast to WebSocket clients.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum BroadcastEvent {
    /// New price from an exchange
    #[serde(rename = "price")]
    Price(PriceData),
    /// Arbitrage opportunity detected
    #[serde(rename = "opportunity")]
    Opportunity(ArbitrageOpportunity),
    /// Exchange status change
    #[serde(rename = "exchange_status")]
    ExchangeStatus {
        exchange: String,
        connected: bool,
    },
}

// =============================================================================
// Utility
// =============================================================================

/// Get current time in milliseconds since epoch.
#[inline]
pub fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before UNIX epoch")
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_price_data_spread_bps() {
        let price = PriceData {
            exchange: Arc::from("vest"),
            symbol: Arc::from("BTC"),
            bid: 100_000.0,
            ask: 100_010.0,
            timestamp_ms: 0,
        };
        // (10 / 100_000) * 10_000 = 1.0 bps
        assert!((price.spread_bps() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_price_data_zero_bid() {
        let price = PriceData {
            exchange: Arc::from("test"),
            symbol: Arc::from("BTC"),
            bid: 0.0,
            ask: 100.0,
            timestamp_ms: 0,
        };
        assert_eq!(price.spread_bps(), 0.0);
    }

    #[test]
    fn test_broadcast_event_serialization() {
        let event = BroadcastEvent::Price(PriceData {
            exchange: Arc::from("vest"),
            symbol: Arc::from("BTC"),
            bid: 50000.0,
            ask: 50010.0,
            timestamp_ms: 1700000000000,
        });
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"price\""));
        assert!(json.contains("\"exchange\":\"vest\""));
    }

    #[test]
    fn test_current_time_ms() {
        let now = current_time_ms();
        // Should be after 2024-01-01
        assert!(now > 1_704_067_200_000);
    }
}
