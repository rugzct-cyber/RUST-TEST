//! Lighter Exchange Adapter
//!
//! Implements the ExchangeAdapter trait for Lighter Protocol (zkLighter).
//! Read-only market data via WebSocket (public orderbooks).
//!
//! This module is organized into submodules:
//! - `config` - Configuration and environment loading
//! - `types` - API response types and data structures
//! - `adapter` - Main LighterAdapter implementation

mod adapter;
mod config;
mod types;

// Re-export public items
pub use adapter::LighterAdapter;
pub use config::LighterConfig;
