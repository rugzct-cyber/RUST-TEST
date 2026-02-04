//! Exchange adapters for Vest, Paradex, Hyperliquid, Lighter
//!
//! This module provides the core abstractions for connecting to
//! various cryptocurrency exchanges via WebSocket.

pub mod errors;
pub mod types;
pub mod traits;
pub mod vest;
pub mod paradex;

// Re-export commonly used types for convenience
pub use errors::{ExchangeError, ExchangeResult};
pub use types::{
    Orderbook, OrderbookLevel, OrderbookUpdate,
    OrderRequest, OrderResponse, OrderSide, OrderStatus, OrderType, TimeInForce,
    PositionInfo, OrderBuilder,
};
pub use traits::{ExchangeAdapter, OrderbookStream};
pub use vest::{VestAdapter, VestConfig, SharedOrderbooks};
pub use paradex::{ParadexAdapter, ParadexConfig};

