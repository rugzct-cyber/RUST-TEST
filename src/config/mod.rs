//! Configuration module for bot settings and YAML loading
//!
//! This module provides:
//! - Configuration types (`AppConfig`, `BotConfig`, `RiskConfig`, `ApiConfig`)
//! - YAML loading functionality (`load_config`)
//! - Shared state wrapper (`SharedConfig`)
//! - Supabase config loading (`SupabaseConfig`)
//! - Application constants with environment variable overrides

pub mod constants;
mod loader;
mod supabase;
mod types;

// Re-export types
pub use types::{ApiConfig, AppConfig, BotConfig, Dex, RiskConfig, SharedConfig, TradingPair};

// Re-export loader functions
pub use loader::{load_config, load_config_from_str};

// Re-export Supabase config
pub use supabase::{SupabaseConfig, SupabaseConfigError};

