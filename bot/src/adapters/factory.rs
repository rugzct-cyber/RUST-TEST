//! Adapter factory for dynamic exchange selection
//!
//! Creates `ExchangeAdapter` instances from config strings like "vest", "paradex", "lighter".
//! Uses an enum-based dispatch pattern (no `Box<dyn>`) to preserve monomorphization.



use async_trait::async_trait;

use crate::adapters::errors::{ExchangeError, ExchangeResult};
use crate::adapters::traits::ExchangeAdapter;
use crate::adapters::types::{FillInfo, OrderRequest, OrderResponse, Orderbook, PositionInfo};
use crate::core::channels::{OrderbookNotify, SharedBestPrices, SharedOrderbooks};

use crate::adapters::vest::{VestAdapter, VestConfig};
use crate::adapters::paradex::{ParadexAdapter, ParadexConfig};
use crate::adapters::lighter::{LighterAdapter, LighterConfig};

// =============================================================================
// AnyAdapter — enum-based dispatch for dynamic exchange selection
// =============================================================================

/// Enum wrapping all concrete adapter types for runtime dispatch.
///
/// This avoids `Box<dyn ExchangeAdapter>` which would require object safety
/// and prevent the compiler from monomorphizing hot paths. The enum generates
/// a match-based vtable that's effectively zero-cost.
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

    async fn place_order(&self, order: OrderRequest) -> ExchangeResult<OrderResponse> {
        delegate!(await self, place_order(order))
    }

    async fn cancel_order(&self, order_id: &str) -> ExchangeResult<()> {
        delegate!(await self, cancel_order(order_id))
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

    async fn get_position(&self, symbol: &str) -> ExchangeResult<Option<PositionInfo>> {
        delegate!(await self, get_position(symbol))
    }

    async fn get_fill_info(&self, symbol: &str, order_id: &str) -> ExchangeResult<Option<FillInfo>> {
        delegate!(await self, get_fill_info(symbol, order_id))
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

    async fn subscribe_orders(&self, symbol: &str) -> ExchangeResult<()> {
        // Manual dispatch — Paradex has inherent subscribe_orders() -> ExchangeResult<u64>
        // that shadows the trait method, so we use ExchangeAdapter:: qualification.
        match self {
            AnyAdapter::Vest(a) => ExchangeAdapter::subscribe_orders(a, symbol).await,
            AnyAdapter::Paradex(a) => ExchangeAdapter::subscribe_orders(a, symbol).await,
            AnyAdapter::Lighter(a) => ExchangeAdapter::subscribe_orders(a, symbol).await,
        }
    }

    async fn set_leverage(&self, symbol: &str, leverage: u32) -> ExchangeResult<u32> {
        // Manual dispatch — Vest/Paradex have inherent set_leverage() that may differ
        match self {
            AnyAdapter::Vest(a) => ExchangeAdapter::set_leverage(a, symbol, leverage).await,
            AnyAdapter::Paradex(a) => ExchangeAdapter::set_leverage(a, symbol, leverage).await,
            AnyAdapter::Lighter(a) => ExchangeAdapter::set_leverage(a, symbol, leverage).await,
        }
    }
}

// =============================================================================
// Factory Functions
// =============================================================================

/// Create an adapter from a config name string.
///
/// The adapter is created but NOT connected — call `connect()` after.
///
/// # Supported names
/// - `"vest"` — creates VestAdapter from env vars
/// - `"paradex"` — creates ParadexAdapter from env vars
/// - `"lighter"` — creates LighterAdapter from env vars
pub fn create_adapter(name: &str) -> ExchangeResult<AnyAdapter> {
    match name {
        "vest" => {
            let config = VestConfig::from_env()?;
            Ok(AnyAdapter::Vest(VestAdapter::new(config)))
        }
        "paradex" => {
            let config = ParadexConfig::from_env()?;
            Ok(AnyAdapter::Paradex(ParadexAdapter::new(config)))
        }
        "lighter" => {
            let config = LighterConfig::from_env()?;
            Ok(AnyAdapter::Lighter(LighterAdapter::new(config)))
        }
        _ => Err(ExchangeError::ConnectionFailed(format!(
            "Unknown exchange adapter: '{}'. Supported: vest, paradex, lighter",
            name
        ))),
    }
}

/// Returns the default orderbook symbol for a given exchange + trading pair.
///
/// Different exchanges use different symbol formats for the same underlying pair.
/// For example, BTC perpetuals:
/// - Vest: "BTC-PERP"
/// - Paradex: "BTC-USD-PERP"
/// - Lighter: "BTC" (symbol from API)
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
        // Fallback: use pair as-is (allows overriding via config)
        _ => pair.to_string(),
    }
}

/// Downcast to ParadexAdapter for Paradex-specific setup (e.g., USDC rate cache).
///
/// Returns None if the adapter is not Paradex.
impl AnyAdapter {
    pub fn as_paradex_mut(&mut self) -> Option<&mut ParadexAdapter> {
        match self {
            AnyAdapter::Paradex(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_vest_mut(&mut self) -> Option<&mut VestAdapter> {
        match self {
            AnyAdapter::Vest(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_lighter_mut(&mut self) -> Option<&mut LighterAdapter> {
        match self {
            AnyAdapter::Lighter(a) => Some(a),
            _ => None,
        }
    }
}
