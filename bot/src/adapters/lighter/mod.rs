//! Lighter Exchange Adapter
//!
//! Implements the ExchangeAdapter trait for Lighter Protocol (zkLighter).
//! Uses Schnorr/Poseidon2/Goldilocks signatures for authentication.
//!
//! This module is organized into submodules:
//! - `config` - Configuration and environment loading
//! - `types` - API response types and data structures
//! - `signing` - Schnorr/Poseidon2 signing logic
//! - `adapter` - Main LighterAdapter implementation

mod adapter;
mod config;
mod signing;
mod types;

// Re-export public items
pub use adapter::LighterAdapter;
pub use config::LighterConfig;
