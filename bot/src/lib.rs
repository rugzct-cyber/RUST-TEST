//! Arbi v5 — Rust Backend
//!
//! Real-time multi-exchange price aggregation and arbitrage detection:
//! - Exchange adapters (Vest, Paradex, Lighter) via WebSocket
//! - Price aggregation and arbitrage detection pipeline
//! - WebSocket API server for frontend clients

pub mod adapters;
pub mod config;
pub mod core;
pub mod error;
pub mod server;

pub use error::AppError;
