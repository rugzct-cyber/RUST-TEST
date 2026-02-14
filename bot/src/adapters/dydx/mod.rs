//! dYdX v4 exchange adapter module
//!
//! Provides WebSocket-based real-time orderbook data from dYdX v4 Indexer.

pub mod adapter;
pub mod config;
pub mod types;

pub use adapter::DydxAdapter;
pub use config::DydxConfig;
