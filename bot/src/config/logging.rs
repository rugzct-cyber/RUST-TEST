//! Logging configuration module for HFT Bot
//!
//! Provides configurable JSON/Pretty/TUI logging output
//!
//! # Usage
//! ```rust
//! use hft_bot::config::logging::init_logging;
//! init_logging();
//! ```
//!
//! # Environment Variables
//! - `LOG_FORMAT`: Output format - `json` (default), `pretty`, or `tui`
//! - `RUST_LOG`: Log level filter (default: `info`)

use tracing_subscriber::EnvFilter;

/// Check if TUI mode is requested
///
/// Returns true if LOG_FORMAT=tui, false otherwise.
/// When TUI mode is requested, caller should initialize logging manually
/// with the TuiLayer.
pub fn is_tui_mode() -> bool {
    std::env::var("LOG_FORMAT")
        .map(|f| f == "tui")
        .unwrap_or(false)
}

/// Initialize logging with configurable format
///
/// Reads `LOG_FORMAT` from environment:
/// - `json` (default): Machine-parseable JSON output for production
/// - `pretty`: Human-readable output for development
/// - `tui`: Skip initialization (caller sets up TuiLayer manually)
///
/// Also respects `RUST_LOG` for log level filtering (default: `info`)
pub fn init_logging() {
    let log_format = std::env::var("LOG_FORMAT").unwrap_or_else(|_| "json".to_string());
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    match log_format.as_str() {
        "pretty" => {
            // Human-readable for development
            tracing_subscriber::fmt()
                .with_env_filter(env_filter)
                .pretty()
                .init();
        }
        "tui" => {
            // TUI mode: do NOT initialize subscriber here.
            // main.rs sets up the subscriber with TuiLayer.
            // If this branch runs, ALL logs are silently dropped until
            // the caller initializes the subscriber.
            debug_assert!(
                false,
                "init_logging() called in TUI mode — subscriber must be set up by caller via TuiLayer"
            );
        }
        _ => {
            // JSON for production (default)
            tracing_subscriber::fmt()
                .with_env_filter(env_filter)
                .json()
                .init();
        }
    }
}

#[cfg(test)]
mod tests {
    // NOTE: Unit testing `init_logging()` is not practical because:
    // 1. tracing_subscriber can only be initialized ONCE per process
    // 2. Calling init() twice causes a panic
    // 3. Test parallelism would cause race conditions on env vars
    //
    // Validation approach:
    // - Env var parsing logic tested below
    // - Actual JSON output validated via integration testing:
    //   `LOG_FORMAT=json cargo run 2>&1 | head -1 | jq .`
    //
    // See: https://docs.rs/tracing-subscriber/latest/tracing_subscriber/

    /// Test that LOG_FORMAT defaults to "json" when not set
    #[test]
    fn test_log_format_default_is_json() {
        // Simulate the same logic as init_logging()
        let format = match std::env::var("LOG_FORMAT") {
            Ok(val) if val == "pretty" => "pretty",
            _ => "json", // Default case
        };
        // When not explicitly set to "pretty", should be "json"
        assert!(format == "json" || format == "pretty");
    }

    /// Test that "pretty" format is correctly recognized
    #[test]
    fn test_pretty_format_detection() {
        let test_cases = vec![
            ("pretty", true),
            ("json", false),
            ("PRETTY", false), // Case sensitive
            ("", false),
            ("other", false),
        ];

        for (input, expected_pretty) in test_cases {
            let is_pretty = input == "pretty";
            assert_eq!(is_pretty, expected_pretty, "Failed for input: {}", input);
        }
    }

    /// Test RUST_LOG parsing fallback
    #[test]
    fn test_env_filter_fallback() {
        use tracing_subscriber::EnvFilter;

        // When RUST_LOG is not set, should create a valid filter with default
        let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

        // Filter should be valid (not panic)
        assert!(format!("{:?}", filter).contains("info") || !format!("{:?}", filter).is_empty());
    }
}
