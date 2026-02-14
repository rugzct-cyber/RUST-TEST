//! Configuration module for bot settings and YAML loading
//!
//! This module provides:
//! - Configuration types (`AppConfig`, `DashboardConfig`)
//! - YAML loading functionality (`load_config`)
//! - Logging configuration (`init_logging`)

mod loader;
pub mod logging;
mod types;

// Re-export types
pub use types::{AppConfig, DashboardConfig, Dex, TradingPair};

// Re-export loader functions
pub use loader::{load_config, load_config_from_str};

// Re-export logging functions
pub use logging::init_logging;
