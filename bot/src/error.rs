//! Application-wide error types using thiserror
//!
//! All errors in the application should be wrapped in AppError
//! to provide consistent error handling across the codebase.

use crate::adapters::errors::ExchangeError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Exchange error: {0}")]
    Exchange(#[from] ExchangeError),



    #[error("WebSocket error: {0}")]
    WebSocket(Box<tokio_tungstenite::tungstenite::Error>),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("API error: {0}")]
    Api(String),
}

/// Result type alias using AppError
pub type Result<T> = std::result::Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exchange_error_converts_to_app_error() {
        let exchange_err = ExchangeError::ConnectionFailed("timeout".into());
        let app_err: AppError = exchange_err.into();
        let msg = app_err.to_string();
        assert!(msg.contains("Exchange error"), "Got: {}", msg);
        assert!(msg.contains("timeout"), "Got: {}", msg);
    }

    #[test]
    fn test_serde_error_converts_to_app_error() {
        let serde_err = serde_json::from_str::<serde_json::Value>("invalid json").unwrap_err();
        let app_err: AppError = serde_err.into();
        let msg = app_err.to_string();
        assert!(msg.contains("Serialization error"), "Got: {}", msg);
    }

    #[test]
    fn test_io_error_converts_to_app_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let app_err: AppError = io_err.into();
        let msg = app_err.to_string();
        assert!(msg.contains("IO error"), "Got: {}", msg);
        assert!(msg.contains("file missing"), "Got: {}", msg);
    }

    #[test]
    fn test_config_error_display() {
        let err = AppError::Config("missing API key".into());
        assert_eq!(err.to_string(), "Configuration error: missing API key");
    }

    #[test]
    fn test_api_error_display() {
        let err = AppError::Api("rate limited".into());
        assert_eq!(err.to_string(), "API error: rate limited");
    }

    #[test]
    fn test_app_error_is_debug_printable() {
        let err = AppError::Config("test".into());
        let debug_str = format!("{:?}", err);
        assert!(!debug_str.is_empty(), "Debug output should not be empty");
        assert!(debug_str.contains("Config"), "Got: {}", debug_str);
    }
}
