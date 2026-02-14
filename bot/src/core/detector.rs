//! Arbitrage opportunity detector.
//!
//! Port of the TypeScript `ArbitrageDetector` from arbi-v5.
//! Implements freshness validation, sanity checks, confirmation logic,
//! and per-symbol cooldowns.

use std::collections::HashMap;
use std::sync::Arc;

use tracing::warn;

use crate::core::types::{AggregatedPrice, ArbitrageOpportunity, current_time_ms};

/// Configuration for the arbitrage detector.
#[derive(Debug, Clone)]
pub struct DetectorConfig {
    /// Minimum spread percentage to trigger (default 0.1%)
    pub min_spread_percent: f64,
    /// Maximum price age in ms (default 2000ms)
    pub max_price_age_ms: u64,
    /// Maximum realistic spread (default 5% — above this is likely an error)
    pub max_realistic_spread: f64,
    /// Number of consecutive confirmations required (default 2)
    pub min_confirmations: u32,
    /// Cooldown per symbol in ms (default 1000ms)
    pub cooldown_ms: u64,
}

impl Default for DetectorConfig {
    fn default() -> Self {
        Self {
            min_spread_percent: 0.1,
            max_price_age_ms: 2_000,
            max_realistic_spread: 5.0,
            min_confirmations: 2,
            cooldown_ms: 1_000,
        }
    }
}

/// Tracks pending confirmation state for a symbol.
struct PendingArb {
    buy_exchange: Arc<str>,
    sell_exchange: Arc<str>,
    count: u32,
}

/// Cross-exchange arbitrage detector with freshness and confirmation logic.
pub struct ArbitrageDetector {
    config: DetectorConfig,
    /// Pending confirmation tracking per symbol
    pending: HashMap<Arc<str>, PendingArb>,
    /// Cooldown tracking: symbol → last emission timestamp
    cooldowns: HashMap<Arc<str>, u64>,
}

impl ArbitrageDetector {
    /// Create with default configuration.
    pub fn new() -> Self {
        Self {
            config: DetectorConfig::default(),
            pending: HashMap::new(),
            cooldowns: HashMap::new(),
        }
    }

    /// Create with custom configuration.
    pub fn with_config(config: DetectorConfig) -> Self {
        Self {
            config,
            pending: HashMap::new(),
            cooldowns: HashMap::new(),
        }
    }

    /// Detect arbitrage opportunity from aggregated price data.
    ///
    /// Returns `Some(ArbitrageOpportunity)` if a confirmed opportunity is found.
    pub fn detect(&mut self, aggregated: &AggregatedPrice) -> Option<ArbitrageOpportunity> {
        let symbol = &aggregated.symbol;
        let now = current_time_ms();

        // Need at least 2 exchanges
        if aggregated.prices.len() < 2 {
            self.pending.remove(symbol.as_ref());
            return None;
        }

        let best_bid = aggregated.best_bid.as_ref()?;
        let best_ask = aggregated.best_ask.as_ref()?;

        // Best bid must exceed best ask (cross-exchange spread)
        if best_bid.price <= best_ask.price {
            self.pending.remove(symbol.as_ref());
            return None;
        }

        // Different exchanges required
        if best_bid.exchange == best_ask.exchange {
            self.pending.remove(symbol.as_ref());
            return None;
        }

        let spread_percent = ((best_bid.price - best_ask.price) / best_ask.price) * 100.0;

        // === FRESHNESS CHECK ===
        let bid_price = aggregated
            .prices
            .iter()
            .find(|p| p.exchange == best_bid.exchange)?;
        let ask_price = aggregated
            .prices
            .iter()
            .find(|p| p.exchange == best_ask.exchange)?;

        let bid_age = now.saturating_sub(bid_price.timestamp_ms);
        let ask_age = now.saturating_sub(ask_price.timestamp_ms);

        if bid_age > self.config.max_price_age_ms || ask_age > self.config.max_price_age_ms {
            self.pending.remove(symbol.as_ref());
            return None;
        }

        // === SANITY CHECK ===
        if spread_percent > self.config.max_realistic_spread {
            warn!(
                symbol = symbol.as_ref(),
                spread = format!("{:.2}%", spread_percent),
                max = self.config.max_realistic_spread,
                "Skipping unrealistic spread"
            );
            self.pending.remove(symbol.as_ref());
            return None;
        }

        // === MINIMUM THRESHOLD ===
        if spread_percent < self.config.min_spread_percent {
            self.pending.remove(symbol.as_ref());
            return None;
        }

        // === COOLDOWN CHECK ===
        if let Some(&last_emitted) = self.cooldowns.get(symbol.as_ref()) {
            if now.saturating_sub(last_emitted) < self.config.cooldown_ms {
                return None;
            }
        }

        // === CONFIRMATION LOGIC ===
        let confirmed = if let Some(pending) = self.pending.get_mut(symbol.as_ref()) {
            let same_pair = pending.buy_exchange == best_ask.exchange
                && pending.sell_exchange == best_bid.exchange;

            if same_pair {
                pending.count += 1;
                pending.count >= self.config.min_confirmations
            } else {
                // New pair — reset tracking
                *pending = PendingArb {
                    buy_exchange: best_ask.exchange.clone(),
                    sell_exchange: best_bid.exchange.clone(),
                    count: 1,
                };
                self.config.min_confirmations <= 1
            }
        } else {
            self.pending.insert(
                symbol.clone(),
                PendingArb {
                    buy_exchange: best_ask.exchange.clone(),
                    sell_exchange: best_bid.exchange.clone(),
                    count: 1,
                },
            );
            self.config.min_confirmations <= 1
        };

        if !confirmed {
            return None;
        }

        // Confirmed — emit opportunity and set cooldown
        self.cooldowns.insert(symbol.clone(), now);
        self.pending.remove(symbol.as_ref());

        Some(ArbitrageOpportunity {
            symbol: symbol.clone(),
            buy_exchange: best_ask.exchange.clone(),
            sell_exchange: best_bid.exchange.clone(),
            buy_price: best_ask.price,
            sell_price: best_bid.price,
            spread_percent,
            timestamp_ms: now,
        })
    }

    /// Clean up stale pending/cooldown entries.
    pub fn cleanup(&mut self) {
        let now = current_time_ms();
        self.cooldowns
            .retain(|_, ts| now.saturating_sub(*ts) < self.config.cooldown_ms * 10);
    }
}

impl Default for ArbitrageDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::{ExchangePrice, PriceData};

    fn make_aggregated(
        symbol: &str,
        prices: Vec<(&str, f64, f64)>,
    ) -> AggregatedPrice {
        let now = current_time_ms();
        let price_data: Vec<PriceData> = prices
            .iter()
            .map(|(ex, bid, ask)| PriceData {
                exchange: Arc::from(*ex),
                symbol: Arc::from(symbol),
                bid: *bid,
                ask: *ask,
                timestamp_ms: now,
            })
            .collect();

        // Compute best bid/ask
        let best_bid = price_data
            .iter()
            .max_by(|a, b| a.bid.partial_cmp(&b.bid).unwrap())
            .map(|p| ExchangePrice {
                exchange: p.exchange.clone(),
                price: p.bid,
            });
        let best_ask = price_data
            .iter()
            .min_by(|a, b| a.ask.partial_cmp(&b.ask).unwrap())
            .map(|p| ExchangePrice {
                exchange: p.exchange.clone(),
                price: p.ask,
            });

        AggregatedPrice {
            symbol: Arc::from(symbol),
            prices: price_data,
            best_bid,
            best_ask,
            timestamp_ms: now,
        }
    }

    #[test]
    fn test_no_opportunity_normal_spread() {
        let mut detector = ArbitrageDetector::new();
        // Normal market: bid < ask across all exchanges
        let agg = make_aggregated("BTC", vec![
            ("vest", 50000.0, 50010.0),
            ("paradex", 50005.0, 50015.0),
        ]);
        assert!(detector.detect(&agg).is_none());
    }

    #[test]
    fn test_opportunity_detected_after_confirmation() {
        let mut detector = ArbitrageDetector::with_config(DetectorConfig {
            min_spread_percent: 0.01,
            min_confirmations: 2,
            ..Default::default()
        });

        // Cross-spread: paradex bid (50020) > vest ask (50010)
        let agg = make_aggregated("BTC", vec![
            ("vest", 50000.0, 50010.0),
            ("paradex", 50020.0, 50030.0),
        ]);

        // First tick: pending
        assert!(detector.detect(&agg).is_none());
        // Second tick: confirmed
        let opp = detector.detect(&agg).expect("should detect opportunity");
        assert_eq!(opp.buy_exchange.as_ref(), "vest");
        assert_eq!(opp.sell_exchange.as_ref(), "paradex");
        assert!(opp.spread_percent > 0.0);
    }

    #[test]
    fn test_single_exchange_no_opportunity() {
        let mut detector = ArbitrageDetector::new();
        let agg = make_aggregated("BTC", vec![("vest", 50000.0, 50010.0)]);
        assert!(detector.detect(&agg).is_none());
    }

    #[test]
    fn test_unrealistic_spread_rejected() {
        let mut detector = ArbitrageDetector::with_config(DetectorConfig {
            min_confirmations: 1,
            ..Default::default()
        });
        // 10% spread — unrealistic
        let agg = make_aggregated("BTC", vec![
            ("vest", 50000.0, 50000.0),
            ("paradex", 55500.0, 56000.0),
        ]);
        assert!(detector.detect(&agg).is_none());
    }
}
