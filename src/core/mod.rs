//! Core module - Spread calculation, VWAP, state management, channels, logging

pub mod channels;
pub mod logging;
pub mod spread;
pub mod state;
pub mod vwap;

// Explicit re-exports for spread module
pub use spread::{SpreadCalculator, SpreadDirection, SpreadResult, SpreadTick};

// Explicit re-exports for vwap module
pub use vwap::{calculate_vwap, VwapResult};

// Explicit re-exports for state module
pub use state::{AppState, BotState, BotStatus, Metrics, SharedAppState};

// Explicit re-exports for channels module
pub use channels::{ChannelBundle, SpreadOpportunity, DEFAULT_CHANNEL_CAPACITY};

// Explicit re-exports for logging module
pub use logging::{
    init_logging, init_logging_with_config, sanitize, sanitize_signature, LoggingConfig,
    SanitizedValue, DEFAULT_LOG_LEVEL, SENSITIVE_FIELD_PATTERNS,
};

