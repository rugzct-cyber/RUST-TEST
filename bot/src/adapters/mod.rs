//! Exchange adapters for Vest, Paradex, and Lighter
//!
//! This module provides the core abstractions for connecting to
//! various cryptocurrency exchanges via WebSocket for price monitoring.

pub mod errors;
pub mod factory;
pub mod lighter;
pub mod manager;
pub mod paradex;
pub mod shared;
pub mod traits;
pub mod types;
pub mod vest;

// Re-export commonly used types for convenience
pub use errors::{ExchangeError, ExchangeResult};
pub use factory::{AnyAdapter, create_adapter, resolve_symbol};
pub use lighter::{LighterAdapter, LighterConfig};
pub use manager::ExchangeManager;
pub use paradex::{ParadexAdapter, ParadexConfig};
pub use traits::ExchangeAdapter;
pub use types::{Orderbook, OrderbookLevel, OrderbookUpdate};
pub use vest::{SharedOrderbooks, VestAdapter, VestConfig};
