//! Configuration module for bot settings and YAML loading
//!
//! This module provides:
//! - Configuration types (`AppConfig`, `BotConfig`)
//! - YAML loading functionality (`load_config`)
//! - Shared state wrapper (`SharedConfig`)
//! - Logging configuration (`init_logging`)

mod loader;
pub mod logging;
mod types;

// Re-export types
pub use types::{AppConfig, BotConfig, Dex, SharedConfig, TradingPair};

// Re-export loader functions
pub use loader::{load_config, load_config_from_str};

// Re-export logging functions
pub use logging::{init_logging, is_tui_mode};
