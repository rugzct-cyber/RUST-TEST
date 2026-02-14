//! Exchange adapters for multiple DEX platforms
//!
//! This module provides the core abstractions for connecting to
//! various cryptocurrency exchanges via WebSocket for price monitoring.

pub mod errors;
pub mod ethereal;
pub mod extended;
pub mod factory;
pub mod grvt;
pub mod hotstuff;
pub mod hyperliquid;
pub mod lighter;
pub mod manager;
pub mod nado;
pub mod nord;
pub mod pacifica;
pub mod paradex;
pub mod reya;
pub mod shared;
pub mod traits;
pub mod types;
pub mod vest;

// Re-export commonly used types for convenience
pub use errors::{ExchangeError, ExchangeResult};
pub use factory::{AnyAdapter, create_adapter, resolve_symbol};
pub use ethereal::{EtherealAdapter, EtherealConfig};
pub use extended::{ExtendedAdapter, ExtendedConfig};
pub use grvt::{GrvtAdapter, GrvtConfig};
pub use hotstuff::{HotstuffAdapter, HotstuffConfig};
pub use hyperliquid::{HyperliquidAdapter, HyperliquidConfig};
pub use lighter::{LighterAdapter, LighterConfig};
pub use manager::ExchangeManager;
pub use nado::{NadoAdapter, NadoConfig};
pub use nord::{NordWsAdapter, NordConfig};
pub use pacifica::{PacificaAdapter, PacificaConfig};
pub use paradex::{ParadexAdapter, ParadexConfig};
pub use reya::{ReyaAdapter, ReyaConfig};
pub use traits::ExchangeAdapter;
pub use types::{Orderbook, OrderbookLevel, OrderbookUpdate};
pub use vest::{SharedOrderbooks, VestAdapter, VestConfig};
