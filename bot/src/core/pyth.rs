//! Pyth Network Integration for USD/USDC Rate
//!
//! This module fetches the USD/USDC exchange rate from Pyth Hermes API
//! and provides thread-safe access for converting Paradex USD prices to USDC.
//!
//! # Architecture
//! - `UsdcRateCache`: Thread-safe cache using AtomicU64 (rate × 1_000_000)
//! - `fetch_usdc_rate`: Single fetch with 5s timeout
//! - `fetch_with_retry`: 3 attempts with exponential backoff
//! - `spawn_rate_refresh_task`: Background task refreshing every 15 minutes
//!
//! # Red Team Hardening
//! - Bounds validation: rates outside [0.90, 1.10] are rejected
//! - Retry at startup: 3 attempts with 1s/2s/4s backoff
//! - Monitoring: WARN log if rate changes >0.5% between refreshes

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;

/// Pyth Hermes API endpoint for USDC/USD price
const PYTH_USDC_USD_URL: &str = 
    "https://hermes.pyth.network/v2/updates/price/latest?ids[]=eaa020c61cc479712813461ce153894a96a6c00b21ed0cfc2798d1f9a9e9c94a";

/// Rate refresh interval: 15 minutes
const REFRESH_INTERVAL_SECS: u64 = 900;

/// HTTP timeout for Pyth API calls
const FETCH_TIMEOUT_SECS: u64 = 5;

/// Minimum valid rate (protection against aberrant values)
const RATE_MIN: f64 = 0.90;

/// Maximum valid rate (protection against aberrant values)
const RATE_MAX: f64 = 1.10;

/// Rate change threshold for WARN logging (0.5%)
const RATE_CHANGE_WARN_THRESHOLD: f64 = 0.005;

/// Multiplier for storing rate as u64 (6 decimal precision)
const RATE_MULTIPLIER: f64 = 1_000_000.0;

/// Default rate (1.0 = parity) - used if Pyth unavailable
const DEFAULT_RATE_MICROS: u64 = 1_000_000;

// =============================================================================
// Pyth API Response Types
// =============================================================================

#[derive(Debug, Deserialize)]
struct PythResponse {
    parsed: Vec<PythParsedPrice>,
}

#[derive(Debug, Deserialize)]
struct PythParsedPrice {
    price: PythPriceData,
}

#[derive(Debug, Deserialize)]
struct PythPriceData {
    /// Price as string (e.g., "99970000" for 0.9997)
    price: String,
    /// Exponent (e.g., -8 means price is scaled by 10^-8)
    expo: i32,
}

// =============================================================================
// UsdcRateCache
// =============================================================================

/// Thread-safe cache for USD/USDC exchange rate
///
/// Stores rate as AtomicU64 (rate × 1_000_000) for lock-free access.
/// Default rate is 1.0 (parity) if Pyth is unavailable.
#[derive(Debug)]
pub struct UsdcRateCache {
    /// Rate stored as micros (rate × 1_000_000)
    rate_micros: AtomicU64,
}

impl Default for UsdcRateCache {
    fn default() -> Self {
        Self::new()
    }
}

impl UsdcRateCache {
    /// Create a new cache with default rate of 1.0
    pub fn new() -> Self {
        Self {
            rate_micros: AtomicU64::new(DEFAULT_RATE_MICROS),
        }
    }

    /// Get the current USD/USDC rate
    pub fn get_rate(&self) -> f64 {
        let micros = self.rate_micros.load(Ordering::Relaxed);
        micros as f64 / RATE_MULTIPLIER
    }

    /// Update the rate with bounds validation
    ///
    /// Returns `true` if update was accepted, `false` if rejected (out of bounds)
    pub fn update(&self, new_rate: f64) -> bool {
        // Bounds validation: reject rates outside [0.90, 1.10]
        if new_rate < RATE_MIN || new_rate > RATE_MAX {
            tracing::warn!(
                new_rate = %new_rate,
                bounds = %format!("[{}, {}]", RATE_MIN, RATE_MAX),
                "USDC rate rejected: out of bounds"
            );
            return false;
        }

        let old_rate = self.get_rate();
        let new_micros = (new_rate * RATE_MULTIPLIER) as u64;
        self.rate_micros.store(new_micros, Ordering::Relaxed);

        // Check for significant change (>0.5%)
        if old_rate > 0.0 {
            let change_pct = ((new_rate - old_rate) / old_rate).abs();
            if change_pct > RATE_CHANGE_WARN_THRESHOLD {
                tracing::warn!(
                    old_rate = %format!("{:.6}", old_rate),
                    new_rate = %format!("{:.6}", new_rate),
                    change_pct = %format!("{:.4}%", change_pct * 100.0),
                    "USDC rate significant change detected"
                );
            }
        }

        true
    }
}

// =============================================================================
// API Functions
// =============================================================================

/// Fetch the current USD/USDC rate from Pyth Hermes API
///
/// Returns the rate (e.g., 0.9997 means 1 USDC = 0.9997 USD)
pub async fn fetch_usdc_rate(client: &reqwest::Client) -> Result<f64, String> {
    let response = client
        .get(PYTH_USDC_USD_URL)
        .timeout(Duration::from_secs(FETCH_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| format!("Pyth request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Pyth API error: {}", response.status()));
    }

    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read Pyth response: {}", e))?;

    let pyth_response: PythResponse = serde_json::from_str(&body)
        .map_err(|e| format!("Failed to parse Pyth JSON: {} - body: {}", e, body))?;

    let parsed = pyth_response
        .parsed
        .first()
        .ok_or_else(|| "No parsed price in Pyth response".to_string())?;

    // Convert price string + exponent to f64
    // Example: price="99970000", expo=-8 => 0.9997
    let price_int: i64 = parsed
        .price
        .price
        .parse()
        .map_err(|e| format!("Invalid price value: {}", e))?;
    
    let expo = parsed.price.expo;
    let rate = price_int as f64 * 10f64.powi(expo);

    Ok(rate)
}

/// Fetch with retry: 3 attempts with exponential backoff (1s, 2s, 4s)
pub async fn fetch_with_retry(client: &reqwest::Client) -> Result<f64, String> {
    let backoff_delays = [1, 2, 4]; // seconds
    let mut last_error = String::new();

    for (attempt, delay_secs) in backoff_delays.iter().enumerate() {
        match fetch_usdc_rate(client).await {
            Ok(rate) => {
                if attempt > 0 {
                    tracing::info!(
                        attempt = attempt + 1,
                        rate = %format!("{:.6}", rate),
                        "Pyth fetch succeeded after retry"
                    );
                }
                return Ok(rate);
            }
            Err(e) => {
                last_error = e;
                tracing::warn!(
                    attempt = attempt + 1,
                    error = %last_error,
                    retry_in_secs = delay_secs,
                    "Pyth fetch failed, retrying"
                );
                tokio::time::sleep(Duration::from_secs(*delay_secs)).await;
            }
        }
    }

    Err(format!("Pyth fetch failed after 3 attempts: {}", last_error))
}

/// Spawn background task to refresh the rate every 15 minutes
///
/// The task will:
/// 1. Fetch initial rate with retry
/// 2. Update cache every 15 minutes
/// 3. Log WARN if rate changes >0.5%
/// 4. Continue using last known rate on failure
pub fn spawn_rate_refresh_task(cache: Arc<UsdcRateCache>, client: reqwest::Client) {
    tokio::spawn(async move {
        // Initial fetch with retry
        match fetch_with_retry(&client).await {
            Ok(rate) => {
                if cache.update(rate) {
                    tracing::info!(
                        rate = %format!("{:.6}", rate),
                        "Pyth USDC rate initialized"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    fallback_rate = "1.0",
                    "Pyth initial fetch failed, using fallback"
                );
            }
        }

        // Periodic refresh every 15 minutes
        let mut interval = tokio::time::interval(Duration::from_secs(REFRESH_INTERVAL_SECS));
        // Skip the first immediate tick (we just fetched)
        interval.tick().await;

        loop {
            interval.tick().await;

            match fetch_usdc_rate(&client).await {
                Ok(rate) => {
                    let old_rate = cache.get_rate();
                    if cache.update(rate) {
                        tracing::debug!(
                            old_rate = %format!("{:.6}", old_rate),
                            new_rate = %format!("{:.6}", rate),
                            "Pyth USDC rate refreshed"
                        );
                    }
                }
                Err(e) => {
                    let current = cache.get_rate();
                    tracing::warn!(
                        error = %e,
                        keeping_rate = %format!("{:.6}", current),
                        "Pyth refresh failed, keeping current rate"
                    );
                }
            }
        }
    });

    tracing::info!(
        interval_mins = REFRESH_INTERVAL_SECS / 60,
        "Pyth rate refresh task spawned"
    );
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_cache_default_is_one() {
        let cache = UsdcRateCache::new();
        let rate = cache.get_rate();
        assert!((rate - 1.0).abs() < 0.000001, "Default rate should be 1.0, got {}", rate);
    }

    #[test]
    fn test_rate_cache_update_and_get() {
        let cache = UsdcRateCache::new();
        
        // Update with valid rate
        assert!(cache.update(0.9997));
        let rate = cache.get_rate();
        assert!((rate - 0.9997).abs() < 0.000001, "Rate should be 0.9997, got {}", rate);
        
        // Update again
        assert!(cache.update(1.0003));
        let rate = cache.get_rate();
        assert!((rate - 1.0003).abs() < 0.000001, "Rate should be 1.0003, got {}", rate);
    }

    #[test]
    fn test_rate_cache_rejects_out_of_bounds() {
        let cache = UsdcRateCache::new();
        
        // Set a known value first
        assert!(cache.update(0.9997));
        
        // Try to update with rate < 0.90 (should reject)
        assert!(!cache.update(0.50));
        assert!((cache.get_rate() - 0.9997).abs() < 0.000001, "Rate should remain 0.9997 after rejected update");
        
        // Try to update with rate > 1.10 (should reject)
        assert!(!cache.update(1.50));
        assert!((cache.get_rate() - 0.9997).abs() < 0.000001, "Rate should remain 0.9997 after rejected update");
        
        // Edge cases: exactly at bounds should be accepted
        assert!(cache.update(0.90));
        assert!((cache.get_rate() - 0.90).abs() < 0.000001);
        
        assert!(cache.update(1.10));
        assert!((cache.get_rate() - 1.10).abs() < 0.000001);
    }

    #[test]
    fn test_usd_to_usdc_conversion() {
        // Given: USDC rate is 0.9997 (1 USDC = 0.9997 USD)
        // When: We have a price of 42000 USD
        // Then: Converting to USDC = 42000 / 0.9997 = 42012.60 USDC
        
        let usd_price = 42000.0;
        let usdc_rate = 0.9997;
        let usdc_price = usd_price / usdc_rate;
        
        // Expected: 42012.60378... (allowing some floating point tolerance)
        assert!((usdc_price - 42012.6037811_f64).abs() < 0.001, 
            "USDC price should be ~42012.60, got {}", usdc_price);
    }
}
