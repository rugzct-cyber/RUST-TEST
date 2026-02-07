//! Exchange adapters for Vest and Paradex
//!
//! This module provides the core abstractions for connecting to
//! various cryptocurrency exchanges via WebSocket.

pub mod errors;
pub mod paradex;
pub mod shared;
pub mod traits;
pub mod types;
pub mod vest;

#[cfg(test)]
pub mod test_utils;

// Re-export commonly used types for convenience
pub use errors::{ExchangeError, ExchangeResult};
pub use paradex::{ParadexAdapter, ParadexConfig};
pub use traits::ExchangeAdapter;
pub use types::{
    OrderBuilder, OrderRequest, OrderResponse, OrderSide, OrderStatus, OrderType, Orderbook,
    OrderbookLevel, OrderbookUpdate, PositionInfo, TimeInForce,
};
pub use vest::{SharedOrderbooks, VestAdapter, VestConfig};
