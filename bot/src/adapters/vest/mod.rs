//! Vest Exchange Adapter
//!
//! Implements the ExchangeAdapter trait for Vest Markets.
//! WebSocket for public market data (orderbooks).
//!
//! This module is organized into submodules:
//! - `config` - Configuration and environment loading
//! - `types` - API response types and data structures
//! - `adapter` - Main VestAdapter implementation

mod adapter;
mod config;
mod types;

// Re-export public items
pub use adapter::{SharedOrderbooks, VestAdapter};
pub use config::VestConfig;
pub use types::{VestDepthData, VestDepthMessage};
