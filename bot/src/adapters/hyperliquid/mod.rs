//! Hyperliquid exchange adapter module
//!
//! Provides WebSocket-based real-time orderbook data from Hyperliquid DEX.

pub mod adapter;
pub mod config;
pub mod types;

pub use adapter::HyperliquidAdapter;
pub use config::HyperliquidConfig;
