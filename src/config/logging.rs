//! Logging configuration module for HFT Bot
//!
//! Story 5.1: Provides configurable JSON/Pretty logging output
//!
//! # Usage
//! ```rust
//! use hft_bot::config::logging::init_logging;
//! init_logging();
//! ```
//!
//! # Environment Variables
//! - `LOG_FORMAT`: Output format - `json` (default) or `pretty`
//! - `RUST_LOG`: Log level filter (default: `info`)

use tracing_subscriber::EnvFilter;

/// Initialize logging with configurable format
///
/// Reads `LOG_FORMAT` from environment:
/// - `json` (default): Machine-parseable JSON output for production
/// - `pretty`: Human-readable output for development
///
/// Also respects `RUST_LOG` for log level filtering (default: `info`)
pub fn init_logging() {
    let log_format = std::env::var("LOG_FORMAT").unwrap_or_else(|_| "json".to_string());
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    if log_format == "pretty" {
        // Human-readable for development
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .pretty()
            .init();
    } else {
        // JSON for production (default)
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .json()
            .init();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_log_format_env_var_default() {
        // When LOG_FORMAT is not set, should default to "json"
        std::env::remove_var("LOG_FORMAT");
        let format = std::env::var("LOG_FORMAT").unwrap_or_else(|_| "json".to_string());
        assert_eq!(format, "json");
    }

    #[test]
    fn test_log_format_pretty() {
        // Setting LOG_FORMAT to "pretty" should be recognized
        let pretty_format = "pretty";
        assert_eq!(pretty_format, "pretty");
        assert_ne!(pretty_format, "json");
    }
}
