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

mod config;
mod types;
mod signing;
mod adapter;

// Re-export public items
pub use config::VestConfig;
pub use types::{VestDepthMessage, VestDepthData, VestPositionData, PreSignedOrder};
pub use adapter::{VestAdapter, SharedOrderbooks};

// Test constants available in config module for test use
