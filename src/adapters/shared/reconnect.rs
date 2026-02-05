//! Shared reconnection logic with exponential backoff
//!
//! Provides a generic reconnection helper used by all exchange adapters.
//! Implements exponential backoff with jitter to prevent thundering herd issues.

use crate::adapters::errors::{ExchangeError, ExchangeResult};

/// Configuration for reconnection attempts
#[derive(Debug, Clone)]
pub struct ReconnectConfig {
    /// Maximum number of reconnection attempts
    pub max_attempts: u32,
    /// Initial delay in milliseconds (doubles each attempt)
    pub initial_delay_ms: u64,
    /// Maximum delay cap in milliseconds
    pub max_delay_ms: u64,
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_delay_ms: 500,
            max_delay_ms: 5000,
        }
    }
}

/// Reconnect with exponential backoff and jitter
///
/// This helper encapsulates the common reconnection pattern used across adapters.
/// It implements exponential backoff (500ms, 1000ms, 2000ms...) capped at max_delay_ms,
/// with random jitter (0-199ms) to prevent thundering herd when multiple connections
/// reconnect simultaneously.
///
/// # Type Parameters
/// * `F` - Async closure that attempts connection, returns `ExchangeResult<()>`
///
/// # Arguments
/// * `config` - Reconnection configuration (attempts, delays)
/// * `exchange_name` - Name for logging (e.g., "Paradex", "Vest")
/// * `connect_fn` - Async closure to attempt connection
///
/// # Returns
/// * `Ok(())` - Reconnection successful
/// * `Err(ExchangeError)` - All attempts failed
///
/// # Example
/// ```ignore
/// reconnect_with_backoff(
///     ReconnectConfig::default(),
///     "Paradex",
///     || async { self.connect().await }
/// ).await?;
/// ```
pub async fn reconnect_with_backoff<F, Fut>(
    config: ReconnectConfig,
    exchange_name: &str,
    mut connect_fn: F,
) -> ExchangeResult<()>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = ExchangeResult<()>>,
{
    let mut last_error: Option<ExchangeError> = None;

    for attempt in 0..config.max_attempts {
        // D5: Jitter anti-thundering herd (0-199ms random)
        let jitter = rand::random::<u64>() % 200;
        let backoff_ms = std::cmp::min(
            config.initial_delay_ms * (1u64 << attempt),
            config.max_delay_ms
        ) + jitter;

        tracing::info!(
            "{}: Reconnect attempt {} of {}, waiting {}ms...",
            exchange_name, attempt + 1, config.max_attempts, backoff_ms
        );

        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;

        match connect_fn().await {
            Ok(()) => return Ok(()),
            Err(e) => {
                tracing::warn!("{}: Reconnect attempt {} failed: {}", exchange_name, attempt + 1, e);
                last_error = Some(e);
            }
        }
    }

    Err(last_error.unwrap_or_else(||
        ExchangeError::ConnectionFailed("Reconnection failed after max attempts".into())
    ))
}
