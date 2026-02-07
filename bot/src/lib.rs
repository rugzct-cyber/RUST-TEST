//! HFT Arbitrage Bot - MVP
//!
//! Minimal implementation focusing on:
//! - Exchange adapters (Vest, Paradex)
//! - Spread calculation engine
//! - Entry/Exit spread differentiation

pub mod adapters;
pub mod bin_utils;
pub mod config;
pub mod core;
pub mod error;
pub mod tui;

pub use error::AppError;
