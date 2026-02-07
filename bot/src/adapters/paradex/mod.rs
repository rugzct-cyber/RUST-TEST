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

mod adapter;
mod config;
mod signing;
mod types;

// Re-export public items
pub use adapter::ParadexAdapter;
pub use config::{ParadexConfig, ParadexSystemConfig};
pub use signing::{
    compute_starknet_address, derive_account_address, sign_auth_message, sign_order_message,
    verify_account_address, OrderSignParams,
};
pub use types::{ParadexOrderbookData, ParadexOrderbookLevel, ParadexOrderbookMessage};
