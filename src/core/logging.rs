//! Centralized logging configuration for y_bot
//!
//! This module provides structured logging using the `tracing` crate with:
//! - JSON formatted output for production (parseable by log aggregation tools)
//! - Pretty-print format for development (controlled by `LOG_FORMAT=pretty`)
//! - Configurable log levels via `RUST_LOG` environment variable
//! - Optional file output via `LOG_FILE` environment variable
//! - Sensitive data sanitization utilities
//!
//! # Environment Variables
//!
//! | Variable | Default | Description |
//! |----------|---------|-------------|
//! | `RUST_LOG` | `y_bot=info` | Log level filter (standard tracing format) |
//! | `LOG_FORMAT` | `json` | Output format: `json` or `pretty` |
//!
//! # Example Usage
//!
//! ```rust,ignore
//! use y_bot::core::logging::{init_logging, SanitizedValue};
//!
//! // Initialize logging at application startup
//! init_logging();
//!
//! // Use SanitizedValue for sensitive data
//! let api_key = "sk-1234567890abcdef";
//! tracing::info!(api_key = %SanitizedValue::new(api_key), "Connecting to exchange");
//! // Output: api_key = "sk-1...REDACTED"
//! ```

use std::env;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

use tracing_subscriber::{
    fmt as ts_fmt,
    fmt::format::FmtSpan,
    prelude::*,
    EnvFilter,
};

/// Flag to track if logging has been initialized (prevents double-init)
static LOGGING_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Default log level when RUST_LOG is not set
pub const DEFAULT_LOG_LEVEL: &str = "y_bot=info";

/// Sensitive field patterns that should be redacted in logs.
///
/// This list serves as documentation for field naming conventions.
/// Use `SanitizedValue` or `sanitize_signature()` when logging any 
/// field matching these patterns. Use `#[instrument(skip(...))]` 
/// on functions that take these as parameters.
///
/// # Usage Guidelines
///
/// - Wrap values with `SanitizedValue::new()` before logging
/// - Add `skip(field_name)` to `#[instrument]` macros
/// - Use `sanitize_signature()` for cryptographic signatures
pub const SENSITIVE_FIELD_PATTERNS: &[&str] = &[
    "api_key",
    "private_key",
    "secret",
    "signature",
    "password",
    "token",
    "credential",
    "auth",
];

/// Wrapper for sensitive data that should be redacted in logs.
///
/// When displayed via `Display` or `Debug`, the value is redacted to show
/// only the first few characters followed by "...REDACTED" or just "REDACTED"
/// for short values.
///
/// # Example
///
/// ```rust,ignore
/// use y_bot::core::logging::SanitizedValue;
///
/// let api_key = "sk-1234567890abcdef";
/// let sanitized = SanitizedValue::new(api_key);
/// assert_eq!(format!("{}", sanitized), "sk-1...REDACTED");
///
/// let short_secret = "abc";
/// let sanitized_short = SanitizedValue::new(short_secret);
/// assert_eq!(format!("{}", sanitized_short), "REDACTED");
/// ```
#[derive(Clone)]
pub struct SanitizedValue<'a>(&'a str);

impl<'a> SanitizedValue<'a> {
    /// Create a new sanitized wrapper around a sensitive string value.
    pub fn new(value: &'a str) -> Self {
        Self(value)
    }

    /// Get the actual value (use with caution, only for actual processing).
    ///
    /// # Warning
    /// This exposes the sensitive value. Only use this when you actually
    /// need to use the value (e.g., for authentication), never for logging.
    pub fn expose(&self) -> &str {
        self.0
    }
}

impl<'a> fmt::Display for SanitizedValue<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            write!(f, "REDACTED")
        } else if self.0.len() > 8 {
            // Show first 4 chars for longer values
            write!(f, "{}...REDACTED", &self.0[..4])
        } else {
            // Fully redact short values
            write!(f, "REDACTED")
        }
    }
}

impl<'a> fmt::Debug for SanitizedValue<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SanitizedValue(***)")
    }
}

/// Convenience function to create a sanitized value.
///
/// This is a shorthand for `SanitizedValue::new(value)`.
pub fn sanitize(value: &str) -> SanitizedValue<'_> {
    SanitizedValue::new(value)
}

/// Sanitize a signature by showing only the first 8 characters.
///
/// Signatures are often very long hex strings, so we show a bit more
/// context while still protecting the full value.
pub fn sanitize_signature(sig: &str) -> String {
    if sig.len() > 12 {
        format!("{}...", &sig[..8])
    } else {
        "REDACTED".to_string()
    }
}

/// Configuration for the logging system
#[derive(Debug, Clone)]
pub struct LoggingConfig {
    /// Log level filter string (e.g., "y_bot=debug,y_bot::adapters=trace")
    pub level_filter: String,
    /// Use pretty format instead of JSON
    pub use_pretty_format: bool,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level_filter: DEFAULT_LOG_LEVEL.to_string(),
            use_pretty_format: false,
        }
    }
}

impl LoggingConfig {
    /// Create a LoggingConfig from environment variables.
    ///
    /// Reads the following environment variables:
    /// - `RUST_LOG`: Log level filter (defaults to `y_bot=info`)
    /// - `LOG_FORMAT`: `pretty` for human-readable, else JSON
    /// - `LOG_FILE`: Optional path for file output
    pub fn from_env() -> Self {
        let level_filter = env::var("RUST_LOG").unwrap_or_else(|_| DEFAULT_LOG_LEVEL.to_string());
        let use_pretty_format = env::var("LOG_FORMAT")
            .map(|v| v.to_lowercase() == "pretty")
            .unwrap_or(false);

        Self {
            level_filter,
            use_pretty_format,
        }
    }
}

/// Initialize the logging system with default configuration from environment.
///
/// This function reads configuration from environment variables:
/// - `RUST_LOG`: Log level filter (defaults to `y_bot=info`)
/// - `LOG_FORMAT`: `pretty` for human-readable format, else JSON
/// - `LOG_FILE`: Optional file path for log output (not yet implemented)
///
/// # Panics
///
/// This function will not panic if called multiple times; subsequent calls
/// are no-ops.
///
/// # Example
///
/// ```rust,ignore
/// use y_bot::core::logging::init_logging;
///
/// fn main() {
///     init_logging();
///     tracing::info!("Application started");
/// }
/// ```
pub fn init_logging() {
    init_logging_with_config(LoggingConfig::from_env());
}

/// Initialize the logging system with a specific configuration.
///
/// # Arguments
///
/// * `config` - The logging configuration to use
///
/// # Example
///
/// ```rust,ignore
/// use y_bot::core::logging::{init_logging_with_config, LoggingConfig};
///
/// let config = LoggingConfig {
///     level_filter: "y_bot=debug".to_string(),
///     use_pretty_format: true,
///     log_file: None,
/// };
/// init_logging_with_config(config);
/// ```
pub fn init_logging_with_config(config: LoggingConfig) {
    // Prevent double initialization
    if LOGGING_INITIALIZED.swap(true, Ordering::SeqCst) {
        return;
    }

    let env_filter = EnvFilter::try_new(&config.level_filter)
        .unwrap_or_else(|_| EnvFilter::new(DEFAULT_LOG_LEVEL));

    if config.use_pretty_format {
        // Human-readable format for development
        tracing_subscriber::registry()
            .with(
                ts_fmt::layer()
                    .pretty()
                    .with_span_events(FmtSpan::CLOSE)
                    .with_target(true)
                    .with_thread_ids(false)
                    .with_file(false)
                    .with_line_number(false),
            )
            .with(env_filter)
            .init();
    } else {
        // JSON format for production (default)
        tracing_subscriber::registry()
            .with(
                ts_fmt::layer()
                    .json()
                    .with_span_events(FmtSpan::CLOSE)
                    .with_target(true)
                    .with_current_span(true),
            )
            .with(env_filter)
            .init();
    }
}

/// Initialize logging for tests with a specific level.
///
/// This is useful for integration tests where you want to control
/// the verbosity of logs.
///
/// # Arguments
///
/// * `level` - The log level filter string (e.g., "debug", "y_bot=trace")
#[cfg(test)]
pub fn init_test_logging(level: &str) {
    // For tests, we create a new subscriber each time but ignore errors
    // from double-init since tests may run in parallel
    let filter = EnvFilter::try_new(level).unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_test_writer()
        .try_init();
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitized_value_long_string() {
        let api_key = "sk-1234567890abcdef";
        let sanitized = SanitizedValue::new(api_key);
        assert_eq!(format!("{}", sanitized), "sk-1...REDACTED");
    }

    #[test]
    fn test_sanitized_value_short_string() {
        let secret = "abc";
        let sanitized = SanitizedValue::new(secret);
        assert_eq!(format!("{}", sanitized), "REDACTED");
    }

    #[test]
    fn test_sanitized_value_empty_string() {
        let empty = "";
        let sanitized = SanitizedValue::new(empty);
        assert_eq!(format!("{}", sanitized), "REDACTED");
    }

    #[test]
    fn test_sanitized_value_exactly_8_chars() {
        let value = "12345678";
        let sanitized = SanitizedValue::new(value);
        // 8 chars is not > 8, so it should be fully redacted
        assert_eq!(format!("{}", sanitized), "REDACTED");
    }

    #[test]
    fn test_sanitized_value_9_chars() {
        let value = "123456789";
        let sanitized = SanitizedValue::new(value);
        // 9 chars is > 8, so show first 4
        assert_eq!(format!("{}", sanitized), "1234...REDACTED");
    }

    #[test]
    fn test_sanitized_value_debug() {
        let api_key = "sk-1234567890abcdef";
        let sanitized = SanitizedValue::new(api_key);
        assert_eq!(format!("{:?}", sanitized), "SanitizedValue(***)");
    }

    #[test]
    fn test_sanitize_function() {
        let value = "my-secret-token-12345";
        let sanitized = sanitize(value);
        assert_eq!(format!("{}", sanitized), "my-s...REDACTED");
    }

    #[test]
    fn test_sanitize_signature_long() {
        let sig = "0x1234567890abcdef1234567890abcdef";
        let result = sanitize_signature(sig);
        assert_eq!(result, "0x123456...");
    }

    #[test]
    fn test_sanitize_signature_short() {
        let sig = "short";
        let result = sanitize_signature(sig);
        assert_eq!(result, "REDACTED");
    }

    #[test]
    fn test_sanitize_signature_empty() {
        let sig = "";
        let result = sanitize_signature(sig);
        assert_eq!(result, "REDACTED");
    }

    #[test]
    fn test_expose_returns_original_value() {
        let secret = "my-super-secret";
        let sanitized = SanitizedValue::new(secret);
        assert_eq!(sanitized.expose(), "my-super-secret");
    }

    #[test]
    fn test_logging_config_default() {
        let config = LoggingConfig::default();
        assert_eq!(config.level_filter, DEFAULT_LOG_LEVEL);
        assert!(!config.use_pretty_format);
    }

    #[test]
    fn test_sensitive_field_patterns_contains_expected() {
        assert!(SENSITIVE_FIELD_PATTERNS.contains(&"api_key"));
        assert!(SENSITIVE_FIELD_PATTERNS.contains(&"private_key"));
        assert!(SENSITIVE_FIELD_PATTERNS.contains(&"password"));
        assert!(SENSITIVE_FIELD_PATTERNS.contains(&"token"));
    }

    #[test]
    fn test_default_log_level_is_info() {
        // Verify the default log level constant
        assert!(DEFAULT_LOG_LEVEL.contains("info"));
        assert!(DEFAULT_LOG_LEVEL.starts_with("y_bot"));
    }
}
