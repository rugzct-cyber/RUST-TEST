//! Exchange adapter error types
//!
//! All exchange-related errors are wrapped in ExchangeError enum
//! which implements thiserror for consistent error handling.

use thiserror::Error;

/// Exchange-specific error types for adapter operations
#[derive(Error, Debug)]
pub enum ExchangeError {
    /// Connection to exchange failed
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    /// Subscription to market data failed
    #[error("Subscription failed for {symbol}: {reason}")]
    SubscriptionFailed { symbol: String, reason: String },

    /// Network operation timed out
    #[error("Network timeout after {0}ms")]
    NetworkTimeout(u64),

    /// Invalid or unexpected response from exchange
    #[error("Invalid response: {0}")]
    InvalidResponse(String),

    /// WebSocket protocol error (boxed to reduce enum size)
    #[error("WebSocket error: {0}")]
    WebSocket(Box<tokio_tungstenite::tungstenite::Error>),
}

/// Result type alias for exchange operations
pub type ExchangeResult<T> = std::result::Result<T, ExchangeError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_failed_display() {
        let err = ExchangeError::ConnectionFailed("timeout".to_string());
        assert_eq!(err.to_string(), "Connection failed: timeout");
    }

    #[test]
    fn test_subscription_failed_display() {
        let err = ExchangeError::SubscriptionFailed {
            symbol: "BTC-PERP".to_string(),
            reason: "symbol not found".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "Subscription failed for BTC-PERP: symbol not found"
        );
    }

    #[test]
    fn test_network_timeout_display() {
        let err = ExchangeError::NetworkTimeout(5000);
        assert_eq!(err.to_string(), "Network timeout after 5000ms");
    }

    #[test]
    fn test_invalid_response_display() {
        let err = ExchangeError::InvalidResponse("malformed JSON".to_string());
        assert_eq!(err.to_string(), "Invalid response: malformed JSON");
    }
}
