//! Price aggregator — stores latest prices from all exchanges per symbol.
//!
//! Port of the TypeScript `PriceAggregator` from arbi-v5.
//! Stores `HashMap<symbol, HashMap<exchange, PriceData>>` and computes
//! best bid/ask across exchanges.

use std::collections::HashMap;
use std::sync::Arc;

use crate::core::types::{AggregatedPrice, ExchangePrice, PriceData, current_time_ms};

/// Default maximum age for prices (30 seconds).
const DEFAULT_MAX_AGE_MS: u64 = 30_000;

/// Multi-symbol, multi-exchange price aggregator.
///
/// Thread-safe when wrapped in `Arc<RwLock<>>` or used from a single task.
pub struct PriceAggregator {
    /// symbol → exchange → latest price
    prices: HashMap<Arc<str>, HashMap<Arc<str>, PriceData>>,
    /// Maximum age for a price to be considered valid
    max_age_ms: u64,
}

impl PriceAggregator {
    /// Create a new aggregator with default max age.
    pub fn new() -> Self {
        Self {
            prices: HashMap::new(),
            max_age_ms: DEFAULT_MAX_AGE_MS,
        }
    }

    /// Create with custom max price age.
    pub fn with_max_age(max_age_ms: u64) -> Self {
        Self {
            prices: HashMap::new(),
            max_age_ms,
        }
    }

    /// Update a price and return the aggregated view for that symbol.
    pub fn update(&mut self, price: PriceData) -> AggregatedPrice {
        let symbol = price.symbol.clone();

        let symbol_prices = self.prices
            .entry(symbol.clone())
            .or_default();

        symbol_prices.insert(price.exchange.clone(), price);

        self.aggregate(&symbol)
    }

    /// Get aggregated price for a specific symbol.
    pub fn aggregate(&self, symbol: &str) -> AggregatedPrice {
        let now = current_time_ms();

        let symbol_prices = match self.prices.get(symbol) {
            Some(p) => p,
            None => {
                return AggregatedPrice {
                    symbol: Arc::from(symbol),
                    prices: vec![],
                    best_bid: None,
                    best_ask: None,
                    timestamp_ms: now,
                };
            }
        };

        // Collect valid (non-stale) prices
        let valid_prices: Vec<PriceData> = symbol_prices
            .values()
            .filter(|p| now.saturating_sub(p.timestamp_ms) <= self.max_age_ms)
            .cloned()
            .collect();

        // Find best bid (highest) and best ask (lowest)
        let mut best_bid: Option<ExchangePrice> = None;
        let mut best_ask: Option<ExchangePrice> = None;

        for price in &valid_prices {
            match &best_bid {
                None => {
                    best_bid = Some(ExchangePrice {
                        exchange: price.exchange.clone(),
                        price: price.bid,
                    });
                }
                Some(current) if price.bid > current.price => {
                    best_bid = Some(ExchangePrice {
                        exchange: price.exchange.clone(),
                        price: price.bid,
                    });
                }
                _ => {}
            }

            match &best_ask {
                None => {
                    best_ask = Some(ExchangePrice {
                        exchange: price.exchange.clone(),
                        price: price.ask,
                    });
                }
                Some(current) if price.ask < current.price => {
                    best_ask = Some(ExchangePrice {
                        exchange: price.exchange.clone(),
                        price: price.ask,
                    });
                }
                _ => {}
            }
        }

        AggregatedPrice {
            symbol: Arc::from(symbol),
            prices: valid_prices,
            best_bid,
            best_ask,
            timestamp_ms: now,
        }
    }

    /// Get aggregated prices for all symbols.
    pub fn get_all(&self) -> Vec<AggregatedPrice> {
        self.prices
            .keys()
            .map(|symbol| self.aggregate(symbol))
            .collect()
    }

    /// Get raw price for a specific exchange + symbol.
    pub fn get_price(&self, exchange: &str, symbol: &str) -> Option<&PriceData> {
        self.prices.get(symbol)?.get(exchange)
    }

    /// Remove stale prices from all symbols.
    pub fn cleanup(&mut self) {
        let now = current_time_ms();
        self.prices.retain(|_, exchange_prices| {
            exchange_prices.retain(|_, price| {
                now.saturating_sub(price.timestamp_ms) <= self.max_age_ms
            });
            !exchange_prices.is_empty()
        });
    }

    /// Number of symbols currently tracked.
    pub fn symbol_count(&self) -> usize {
        self.prices.len()
    }

    /// Total number of exchange prices currently stored.
    pub fn price_count(&self) -> usize {
        self.prices.values().map(|m| m.len()).sum()
    }
}

impl Default for PriceAggregator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_price(exchange: &str, symbol: &str, bid: f64, ask: f64) -> PriceData {
        PriceData {
            exchange: Arc::from(exchange),
            symbol: Arc::from(symbol),
            bid,
            ask,
            timestamp_ms: current_time_ms(),
        }
    }

    #[test]
    fn test_single_price_update() {
        let mut agg = PriceAggregator::new();
        let result = agg.update(make_price("vest", "BTC", 50000.0, 50010.0));
        assert_eq!(result.prices.len(), 1);
        assert_eq!(result.best_bid.unwrap().price, 50000.0);
        assert_eq!(result.best_ask.unwrap().price, 50010.0);
    }

    #[test]
    fn test_multi_exchange_aggregation() {
        let mut agg = PriceAggregator::new();
        agg.update(make_price("vest", "BTC", 50000.0, 50010.0));
        let result = agg.update(make_price("paradex", "BTC", 50020.0, 50030.0));

        assert_eq!(result.prices.len(), 2);
        // Best bid should be paradex (50020)
        assert_eq!(result.best_bid.as_ref().unwrap().exchange.as_ref(), "paradex");
        assert_eq!(result.best_bid.as_ref().unwrap().price, 50020.0);
        // Best ask should be vest (50010)
        assert_eq!(result.best_ask.as_ref().unwrap().exchange.as_ref(), "vest");
        assert_eq!(result.best_ask.as_ref().unwrap().price, 50010.0);
    }

    #[test]
    fn test_stale_price_eviction() {
        let mut agg = PriceAggregator::with_max_age(100); // 100ms
        let mut price = make_price("vest", "BTC", 50000.0, 50010.0);
        price.timestamp_ms = current_time_ms().saturating_sub(200); // 200ms old
        agg.update(price);

        let result = agg.aggregate("BTC");
        assert_eq!(result.prices.len(), 0); // Stale, filtered out
        assert!(result.best_bid.is_none());
    }

    #[test]
    fn test_get_all() {
        let mut agg = PriceAggregator::new();
        agg.update(make_price("vest", "BTC", 50000.0, 50010.0));
        agg.update(make_price("vest", "ETH", 3000.0, 3001.0));

        let all = agg.get_all();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_cleanup() {
        let mut agg = PriceAggregator::with_max_age(100);
        let mut price = make_price("vest", "BTC", 50000.0, 50010.0);
        price.timestamp_ms = current_time_ms().saturating_sub(200);
        agg.update(price);

        assert_eq!(agg.symbol_count(), 1);
        agg.cleanup();
        assert_eq!(agg.symbol_count(), 0);
    }
}
