//! Paradex Exchange Adapter
//!
//! Implements the ExchangeAdapter trait for Paradex.
//! Uses Starknet signatures for authentication and WebSocket for market data.
//!
//! This module is organized into submodules:
//! - `config` - Configuration and environment loading
//! - `types` - API response types and data structures
//! - `signing` - Starknet signing logic (SNIP-12/EIP-712 inspired)
//! - `adapter` - Main ParadexAdapter implementation

mod config;
mod types;
mod signing;
mod adapter;

// Re-export public items
pub use config::{ParadexConfig, ParadexSystemConfig};
pub use types::{ParadexOrderbookMessage, ParadexOrderbookData, ParadexOrderbookLevel};
pub use signing::{
    sign_auth_message, sign_order_message, OrderSignParams,
    compute_starknet_address, derive_account_address, verify_account_address,
};
pub use adapter::ParadexAdapter;
