//! HFT Arbitrage Bot - MVP
//! 
//! Minimal implementation focusing on:
//! - Exchange adapters (Vest, Paradex)
//! - Spread calculation with VWAP
//! - Entry/Exit spread differentiation

pub mod adapters;
pub mod config;
pub mod core;
pub mod error;

pub use error::AppError;
