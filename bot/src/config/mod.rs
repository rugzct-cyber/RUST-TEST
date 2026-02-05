//! Configuration module for bot settings and YAML loading
//!
//! This module provides:
//! - Configuration types (`AppConfig`, `BotConfig`, `RiskConfig`, `ApiConfig`)
//! - YAML loading functionality (`load_config`)
//! - Shared state wrapper (`SharedConfig`)
//! - Application constants with environment variable overrides
//! - Logging configuration (`init_logging`) - Story 5.1

pub mod constants;
pub mod logging;
mod loader;
mod types;

// Re-export types
pub use types::{ApiConfig, AppConfig, BotConfig, Dex, RiskConfig, SharedConfig, TradingPair};

// Re-export loader functions
pub use loader::{load_config, load_config_from_str};

// Re-export logging functions  
pub use logging::{init_logging, is_tui_mode};

