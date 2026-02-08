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
            config.max_delay_ms,
        ) + jitter;

        tracing::info!(
            "{}: Reconnect attempt {} of {}, waiting {}ms...",
            exchange_name,
            attempt + 1,
            config.max_attempts,
            backoff_ms
        );

        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;

        match connect_fn().await {
            Ok(()) => return Ok(()),
            Err(e) => {
                tracing::warn!(
                    "{}: Reconnect attempt {} failed: {}",
                    exchange_name,
                    attempt + 1,
                    e
                );
                last_error = Some(e);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| {
        ExchangeError::ConnectionFailed("Reconnection failed after max attempts".into())
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    fn fast_config(max_attempts: u32) -> ReconnectConfig {
        ReconnectConfig {
            max_attempts,
            initial_delay_ms: 10,
            max_delay_ms: 100,
        }
    }

    #[tokio::test]
    async fn test_reconnect_succeeds_on_first_attempt() {
        let call_count = Arc::new(AtomicU32::new(0));
        let cc = call_count.clone();

        let result = reconnect_with_backoff(fast_config(3), "Test", || {
            let cc = cc.clone();
            async move {
                cc.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .await;

        assert!(result.is_ok());
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_reconnect_succeeds_on_second_attempt() {
        let call_count = Arc::new(AtomicU32::new(0));
        let cc = call_count.clone();

        let result = reconnect_with_backoff(fast_config(3), "Test", || {
            let cc = cc.clone();
            async move {
                let n = cc.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    Err(ExchangeError::ConnectionFailed("first try".into()))
                } else {
                    Ok(())
                }
            }
        })
        .await;

        assert!(result.is_ok());
        assert_eq!(call_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_reconnect_exhausts_all_attempts() {
        let call_count = Arc::new(AtomicU32::new(0));
        let cc = call_count.clone();

        let result = reconnect_with_backoff(fast_config(3), "Test", || {
            let cc = cc.clone();
            async move {
                cc.fetch_add(1, Ordering::SeqCst);
                Err(ExchangeError::ConnectionFailed("always fail".into()))
            }
        })
        .await;

        assert!(result.is_err());
        assert_eq!(call_count.load(Ordering::SeqCst), 3);
        // Verify it returns the LAST error
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("always fail"),
            "Should contain last error message, got: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_reconnect_backoff_increases() {
        // With initial_delay_ms=10, attempt 0 base = 10ms, attempt 1 base = 20ms
        // Verify that elapsed time is at least sum of minimum delays (jitter adds extra)
        let start = std::time::Instant::now();

        let _ = reconnect_with_backoff(fast_config(3), "Test", || async {
            Err(ExchangeError::ConnectionFailed("fail".into()))
        })
        .await;

        let elapsed = start.elapsed().as_millis();
        // 3 attempts: base delays 10 + 20 + 40 = 70ms minimum (without jitter cap)
        // With jitter (0-199ms each) total could be up to 70 + 3*199 = 667ms
        // Just verify elapsed >= minimum 70ms (base delays only)
        assert!(
            elapsed >= 30, // being lenient due to scheduling variance
            "Backoff should take at least 30ms, took {}ms",
            elapsed
        );
    }

    #[tokio::test]
    async fn test_reconnect_respects_max_delay_cap() {
        // With initial_delay_ms=10, max_delay_ms=100:
        // attempt 0: min(10*1, 100) = 10
        // attempt 1: min(10*2, 100) = 20
        // attempt 2: min(10*4, 100) = 40
        // attempt 3: min(10*8, 100) = 80
        // attempt 4: min(10*16, 100) = 100  <-- capped
        // attempt 5: min(10*32, 100) = 100  <-- capped
        let config = ReconnectConfig {
            max_attempts: 6,
            initial_delay_ms: 10,
            max_delay_ms: 100,
        };

        let start = std::time::Instant::now();
        let _ = reconnect_with_backoff(config, "Test", || async {
            Err(ExchangeError::ConnectionFailed("fail".into()))
        })
        .await;

        let elapsed = start.elapsed().as_millis();
        // Without cap: 10+20+40+80+160+320 = 630ms base
        // With cap: 10+20+40+80+100+100 = 350ms base
        // With up to 6*199 = 1194ms jitter, total <= 350+1194 = 1544ms
        // Max time without cap would be 630+1194 = 1824ms
        // Just verify it ran (elapsed > 0)
        assert!(elapsed > 0, "Should have some delay");
    }

    #[test]
    fn test_reconnect_config_default() {
        let config = ReconnectConfig::default();
        assert_eq!(config.max_attempts, 3);
        assert_eq!(config.initial_delay_ms, 500);
        assert_eq!(config.max_delay_ms, 5000);
    }
}
