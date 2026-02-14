//! Paradex Exchange Adapter
//!
//! Implements the ExchangeAdapter trait for Paradex.
//! WebSocket for public market data (orderbooks).
//!
//! This module is organized into submodules:
//! - `config` - Configuration and environment loading
//! - `types` - API response types and data structures
//! - `adapter` - Main ParadexAdapter implementation

mod adapter;
mod config;
mod types;

// Re-export public items
pub use adapter::ParadexAdapter;
pub use config::ParadexConfig;
pub use types::{ParadexOrderbookData, ParadexOrderbookLevel, ParadexOrderbookMessage};
