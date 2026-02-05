//! Application-wide error types using thiserror
//!
//! All errors in the application should be wrapped in AppError
//! to provide consistent error handling across the codebase.

use thiserror::Error;
use crate::adapters::errors::ExchangeError;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Configuration error: {0}")]
    Config(String),
    
    #[error("Exchange error: {0}")]
    Exchange(#[from] ExchangeError),
    
    #[error("Execution error: {0}")]
    Execution(String),
    
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
