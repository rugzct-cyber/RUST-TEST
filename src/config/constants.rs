//! Application-wide constants and configuration defaults
//!
//! This module centralizes all hardcoded values to make them configurable
//! and maintainable. Values can be overridden via environment variables.

use std::time::Duration;

// =============================================================================
// WebSocket Configuration
// =============================================================================

/// WebSocket heartbeat ping/pong interval (default: 30 seconds)
///
/// Environment variable: `WS_HEARTBEAT_INTERVAL_SECS`
pub fn ws_heartbeat_interval() -> Duration {
    let secs = std::env::var("WS_HEARTBEAT_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    Duration::from_secs(secs)
}

/// WebSocket broadcast channel capacity (default: 256 messages)
///
/// Environment variable: `WS_BROADCAST_CAPACITY`
pub fn ws_broadcast_capacity() -> usize {
    std::env::var("WS_BROADCAST_CAPACITY")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(256)
}

/// Maximum logs to keep in memory (default: 50)
///
/// Environment variable: `MAX_LOGS_IN_MEMORY`
pub fn max_logs_in_memory() -> usize {
    std::env::var("MAX_LOGS_IN_MEMORY")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50)
}

// =============================================================================
// Execution & Retry Configuration
// =============================================================================

/// Maximum retry attempts for failed orders (default: 3)
///
/// Environment variable: `MAX_RETRY_ATTEMPTS`
pub fn max_retry_attempts() -> u32 {
    std::env::var("MAX_RETRY_ATTEMPTS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3)
}

/// Order timeout in milliseconds (default: 2000ms)
///
/// Environment variable: `ORDER_TIMEOUT_MS`
pub fn order_timeout_ms() -> u64 {
    std::env::var("ORDER_TIMEOUT_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2000)
}

// =============================================================================
// Detection & Monitoring Intervals
// =============================================================================

/// Directional position detection interval (default: 500ms)
///
/// Environment variable: `DIRECTIONAL_DETECTION_INTERVAL_MS`
pub fn directional_detection_interval() -> Duration {
    let ms = std::env::var("DIRECTIONAL_DETECTION_INTERVAL_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(500);
    Duration::from_millis(ms)
}

/// Exchange failover timeout (default: 30 seconds)
///
/// Environment variable: `EXCHANGE_FAILOVER_TIMEOUT_SECS`
pub fn exchange_failover_timeout() -> Duration {
    let secs = std::env::var("EXCHANGE_FAILOVER_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    Duration::from_secs(secs)
}

/// Position reconciliation interval (default: 60 seconds)
///
/// Environment variable: `POSITION_RECONCILIATION_INTERVAL_SECS`
pub fn position_reconciliation_interval() -> Duration {
    let secs = std::env::var("POSITION_RECONCILIATION_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);
    Duration::from_secs(secs)
}

// =============================================================================
// Memory Management
// =============================================================================

/// Maximum number of positions to keep in memory (default: 1000)
///
/// Environment variable: `MAX_POSITIONS_IN_MEMORY`
pub fn max_positions_in_memory() -> usize {
    std::env::var("MAX_POSITIONS_IN_MEMORY")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000)
}

/// Position cleanup age threshold in hours (default: 24h)
///
/// Positions older than this and in terminal state will be cleaned up.
///
/// Environment variable: `POSITION_CLEANUP_AGE_HOURS`
pub fn position_cleanup_age() -> Duration {
    let hours = std::env::var("POSITION_CLEANUP_AGE_HOURS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(24);
    Duration::from_secs(hours * 3600)
}

/// Memory cleanup interval (default: 1 hour)
///
/// Environment variable: `MEMORY_CLEANUP_INTERVAL_SECS`
pub fn memory_cleanup_interval() -> Duration {
    let secs = std::env::var("MEMORY_CLEANUP_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3600);
    Duration::from_secs(secs)
}

// =============================================================================
// Logging & Monitoring
// =============================================================================

/// Discord webhook rate limit (requests per minute, default: 5)
///
/// Environment variable: `DISCORD_RATE_LIMIT_PER_MIN`
pub fn discord_rate_limit() -> usize {
    std::env::var("DISCORD_RATE_LIMIT_PER_MIN")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5)
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Print all configuration values (for debugging/startup logs)
pub fn log_configuration() {
    tracing::info!("=== Application Configuration ===");
    tracing::info!("WebSocket:");
    tracing::info!("  - Heartbeat interval: {:?}", ws_heartbeat_interval());
    tracing::info!("  - Broadcast capacity: {}", ws_broadcast_capacity());
    tracing::info!("  - Max logs in memory: {}", max_logs_in_memory());

    tracing::info!("Execution:");
    tracing::info!("  - Max retry attempts: {}", max_retry_attempts());
    tracing::info!("  - Order timeout: {}ms", order_timeout_ms());

    tracing::info!("Detection:");
    tracing::info!("  - Directional detection interval: {:?}", directional_detection_interval());
    tracing::info!("  - Exchange failover timeout: {:?}", exchange_failover_timeout());
    tracing::info!("  - Position reconciliation interval: {:?}", position_reconciliation_interval());

    tracing::info!("Memory Management:");
    tracing::info!("  - Max positions in memory: {}", max_positions_in_memory());
    tracing::info!("  - Position cleanup age: {:?}", position_cleanup_age());
    tracing::info!("  - Memory cleanup interval: {:?}", memory_cleanup_interval());

    tracing::info!("Monitoring:");
    tracing::info!("  - Discord rate limit: {} req/min", discord_rate_limit());
    tracing::info!("==================================");
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    fn test_default_values() {
        // Test that defaults are sensible
        assert_eq!(ws_heartbeat_interval(), Duration::from_secs(30));
        assert_eq!(ws_broadcast_capacity(), 256);
        assert_eq!(max_logs_in_memory(), 50);
        assert_eq!(max_retry_attempts(), 3);
        assert_eq!(order_timeout_ms(), 2000);
    }

    #[test]
    #[serial(env)]  // Shares serial group with supabase.rs env tests
    fn test_env_override() {
        // Set environment variable
        std::env::set_var("MAX_RETRY_ATTEMPTS", "5");

        // Should use env value
        assert_eq!(max_retry_attempts(), 5);

        // Cleanup
        std::env::remove_var("MAX_RETRY_ATTEMPTS");
    }
}
