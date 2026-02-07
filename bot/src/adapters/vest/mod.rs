//! Vest Exchange Adapter
//!
//! Implements the ExchangeAdapter trait for Vest Markets.
//! Uses EIP-712 signatures for authentication and WebSocket for market data.
//!
//! This module is organized into submodules:
//! - `config` - Configuration and environment loading
//! - `types` - API response types and data structures
//! - `signing` - EIP-712 signing logic
//! - `adapter` - Main VestAdapter implementation

mod adapter;
mod config;
mod signing;
mod types;

// Re-export public items
pub use adapter::{SharedOrderbooks, VestAdapter};
pub use config::VestConfig;
pub use types::{PreSignedOrder, VestDepthData, VestDepthMessage, VestPositionData};

// Test constants available in config module for test use
