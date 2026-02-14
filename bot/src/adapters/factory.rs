//! Adapter factory for dynamic exchange selection
//!
//! Creates `ExchangeAdapter` instances from config strings like "vest", "paradex", "lighter".
//! Uses an enum-based dispatch pattern (no `Box<dyn>`) to preserve monomorphization.

use async_trait::async_trait;

use crate::adapters::errors::{ExchangeError, ExchangeResult};
use crate::adapters::lighter::{LighterAdapter, LighterConfig};
use crate::adapters::paradex::{ParadexAdapter, ParadexConfig};
use crate::adapters::traits::ExchangeAdapter;
use crate::adapters::types::Orderbook;
use crate::adapters::vest::{VestAdapter, VestConfig};
use crate::core::channels::{OrderbookNotify, SharedBestPrices, SharedOrderbooks};

// =============================================================================
// AnyAdapter — enum-based dispatch for dynamic exchange selection
// =============================================================================

/// Enum wrapping all concrete adapter types for runtime dispatch.
pub enum AnyAdapter {
    Vest(VestAdapter),
    Paradex(ParadexAdapter),
    Lighter(LighterAdapter),
}

/// Macro to reduce boilerplate for delegating trait methods
macro_rules! delegate {
    ($self:expr, $method:ident ( $($arg:expr),* )) => {
        match $self {
            AnyAdapter::Vest(a) => a.$method($($arg),*),
            AnyAdapter::Paradex(a) => a.$method($($arg),*),
            AnyAdapter::Lighter(a) => a.$method($($arg),*),
        }
    };
    (await $self:expr, $method:ident ( $($arg:expr),* )) => {
        match $self {
            AnyAdapter::Vest(a) => a.$method($($arg),*).await,
            AnyAdapter::Paradex(a) => a.$method($($arg),*).await,
            AnyAdapter::Lighter(a) => a.$method($($arg),*).await,
        }
    };
    (mut $self:expr, $method:ident ( $($arg:expr),* )) => {
        match $self {
            AnyAdapter::Vest(a) => a.$method($($arg),*),
            AnyAdapter::Paradex(a) => a.$method($($arg),*),
            AnyAdapter::Lighter(a) => a.$method($($arg),*),
        }
    };
    (mut await $self:expr, $method:ident ( $($arg:expr),* )) => {
        match $self {
            AnyAdapter::Vest(a) => a.$method($($arg),*).await,
            AnyAdapter::Paradex(a) => a.$method($($arg),*).await,
            AnyAdapter::Lighter(a) => a.$method($($arg),*).await,
        }
    };
}

#[async_trait]
impl ExchangeAdapter for AnyAdapter {
    async fn connect(&mut self) -> ExchangeResult<()> {
        delegate!(mut await self, connect())
    }

    async fn disconnect(&mut self) -> ExchangeResult<()> {
        delegate!(mut await self, disconnect())
    }

    async fn subscribe_orderbook(&mut self, symbol: &str) -> ExchangeResult<()> {
        delegate!(mut await self, subscribe_orderbook(symbol))
    }

    async fn unsubscribe_orderbook(&mut self, symbol: &str) -> ExchangeResult<()> {
        delegate!(mut await self, unsubscribe_orderbook(symbol))
    }

    fn get_orderbook(&self, symbol: &str) -> Option<&Orderbook> {
        delegate!(self, get_orderbook(symbol))
    }

    fn is_connected(&self) -> bool {
        delegate!(self, is_connected())
    }

    fn is_stale(&self) -> bool {
        delegate!(self, is_stale())
    }

    async fn sync_orderbooks(&mut self) {
        delegate!(mut await self, sync_orderbooks())
    }

    async fn reconnect(&mut self) -> ExchangeResult<()> {
        delegate!(mut await self, reconnect())
    }

    fn exchange_name(&self) -> &'static str {
        delegate!(self, exchange_name())
    }

    fn get_shared_orderbooks(&self) -> SharedOrderbooks {
        delegate!(self, get_shared_orderbooks())
    }

    fn get_shared_best_prices(&self) -> SharedBestPrices {
        delegate!(self, get_shared_best_prices())
    }

    fn set_orderbook_notify(&mut self, notify: OrderbookNotify) {
        delegate!(mut self, set_orderbook_notify(notify))
    }
}

// =============================================================================
// Factory Functions
// =============================================================================

/// Create an adapter from a config name string.
///
/// The adapter is created but NOT connected — call `connect()` after.
pub fn create_adapter(name: &str) -> ExchangeResult<AnyAdapter> {
    match name {
        "vest" => {
            let config = VestConfig::from_env();
            Ok(AnyAdapter::Vest(VestAdapter::new(config)))
        }
        "paradex" => {
            let config = ParadexConfig::from_env();
            Ok(AnyAdapter::Paradex(ParadexAdapter::new(config)))
        }
        "lighter" => {
            let config = LighterConfig::from_env();
            Ok(AnyAdapter::Lighter(LighterAdapter::new(config)))
        }
        _ => Err(ExchangeError::ConnectionFailed(format!(
            "Unknown exchange adapter: '{}'. Supported: vest, paradex, lighter",
            name
        ))),
    }
}

/// Returns the default orderbook symbol for a given exchange + trading pair.
pub fn resolve_symbol(exchange: &str, pair: &str) -> String {
    match (exchange, pair) {
        // BTC perpetuals
        ("vest", "BTC") => "BTC-PERP".to_string(),
        ("paradex", "BTC") => "BTC-USD-PERP".to_string(),
        ("lighter", "BTC") => "BTC".to_string(),
        // ETH perpetuals
        ("vest", "ETH") => "ETH-PERP".to_string(),
        ("paradex", "ETH") => "ETH-USD-PERP".to_string(),
        ("lighter", "ETH") => "ETH".to_string(),
        // SOL perpetuals
        ("vest", "SOL") => "SOL-PERP".to_string(),
        ("paradex", "SOL") => "SOL-USD-PERP".to_string(),
        ("lighter", "SOL") => "SOL".to_string(),
        // Fallback: use pair as-is
        _ => pair.to_string(),
    }
}


