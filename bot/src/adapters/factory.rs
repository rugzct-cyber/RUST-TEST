//! Adapter factory for dynamic exchange selection
//!
//! Creates `ExchangeAdapter` instances from config strings.
//! Uses an enum-based dispatch pattern (no `Box<dyn>`) to preserve monomorphization.

use async_trait::async_trait;

use crate::adapters::errors::{ExchangeError, ExchangeResult};
use crate::adapters::dydx::{DydxAdapter, DydxConfig};
use crate::adapters::ethereal::{EtherealAdapter, EtherealConfig};
use crate::adapters::extended::{ExtendedAdapter, ExtendedConfig};
use crate::adapters::grvt::{GrvtAdapter, GrvtConfig};
use crate::adapters::hotstuff::{HotstuffAdapter, HotstuffConfig};
use crate::adapters::hyperliquid::{HyperliquidAdapter, HyperliquidConfig};
use crate::adapters::lighter::{LighterAdapter, LighterConfig};
use crate::adapters::nado::{NadoAdapter, NadoConfig};
use crate::adapters::nord::{NordWsAdapter, NordConfig};
use crate::adapters::pacifica::{PacificaAdapter, PacificaConfig};
use crate::adapters::paradex::{ParadexAdapter, ParadexConfig};
use crate::adapters::reya::{ReyaAdapter, ReyaConfig};
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
    Hyperliquid(HyperliquidAdapter),
    Grvt(GrvtAdapter),
    Reya(ReyaAdapter),
    Hotstuff(HotstuffAdapter),
    Pacifica(PacificaAdapter),
    Extended(ExtendedAdapter),
    Nado(NadoAdapter),
    Nord(NordWsAdapter),
    Ethereal(EtherealAdapter),
    Dydx(DydxAdapter),
}

/// Macro to reduce boilerplate for delegating trait methods
macro_rules! delegate {
    ($self:expr, $method:ident ( $($arg:expr),* )) => {
        match $self {
            AnyAdapter::Vest(a) => a.$method($($arg),*),
            AnyAdapter::Paradex(a) => a.$method($($arg),*),
            AnyAdapter::Lighter(a) => a.$method($($arg),*),
            AnyAdapter::Hyperliquid(a) => a.$method($($arg),*),
            AnyAdapter::Grvt(a) => a.$method($($arg),*),
            AnyAdapter::Reya(a) => a.$method($($arg),*),
            AnyAdapter::Hotstuff(a) => a.$method($($arg),*),
            AnyAdapter::Pacifica(a) => a.$method($($arg),*),
            AnyAdapter::Extended(a) => a.$method($($arg),*),
            AnyAdapter::Nado(a) => a.$method($($arg),*),
            AnyAdapter::Nord(a) => a.$method($($arg),*),
            AnyAdapter::Ethereal(a) => a.$method($($arg),*),
            AnyAdapter::Dydx(a) => a.$method($($arg),*),
        }
    };
    (await $self:expr, $method:ident ( $($arg:expr),* )) => {
        match $self {
            AnyAdapter::Vest(a) => a.$method($($arg),*).await,
            AnyAdapter::Paradex(a) => a.$method($($arg),*).await,
            AnyAdapter::Lighter(a) => a.$method($($arg),*).await,
            AnyAdapter::Hyperliquid(a) => a.$method($($arg),*).await,
            AnyAdapter::Grvt(a) => a.$method($($arg),*).await,
            AnyAdapter::Reya(a) => a.$method($($arg),*).await,
            AnyAdapter::Hotstuff(a) => a.$method($($arg),*).await,
            AnyAdapter::Pacifica(a) => a.$method($($arg),*).await,
            AnyAdapter::Extended(a) => a.$method($($arg),*).await,
            AnyAdapter::Nado(a) => a.$method($($arg),*).await,
            AnyAdapter::Nord(a) => a.$method($($arg),*).await,
            AnyAdapter::Ethereal(a) => a.$method($($arg),*).await,
            AnyAdapter::Dydx(a) => a.$method($($arg),*).await,
        }
    };
    (mut $self:expr, $method:ident ( $($arg:expr),* )) => {
        match $self {
            AnyAdapter::Vest(a) => a.$method($($arg),*),
            AnyAdapter::Paradex(a) => a.$method($($arg),*),
            AnyAdapter::Lighter(a) => a.$method($($arg),*),
            AnyAdapter::Hyperliquid(a) => a.$method($($arg),*),
            AnyAdapter::Grvt(a) => a.$method($($arg),*),
            AnyAdapter::Reya(a) => a.$method($($arg),*),
            AnyAdapter::Hotstuff(a) => a.$method($($arg),*),
            AnyAdapter::Pacifica(a) => a.$method($($arg),*),
            AnyAdapter::Extended(a) => a.$method($($arg),*),
            AnyAdapter::Nado(a) => a.$method($($arg),*),
            AnyAdapter::Nord(a) => a.$method($($arg),*),
            AnyAdapter::Ethereal(a) => a.$method($($arg),*),
            AnyAdapter::Dydx(a) => a.$method($($arg),*),
        }
    };
    (mut await $self:expr, $method:ident ( $($arg:expr),* )) => {
        match $self {
            AnyAdapter::Vest(a) => a.$method($($arg),*).await,
            AnyAdapter::Paradex(a) => a.$method($($arg),*).await,
            AnyAdapter::Lighter(a) => a.$method($($arg),*).await,
            AnyAdapter::Hyperliquid(a) => a.$method($($arg),*).await,
            AnyAdapter::Grvt(a) => a.$method($($arg),*).await,
            AnyAdapter::Reya(a) => a.$method($($arg),*).await,
            AnyAdapter::Hotstuff(a) => a.$method($($arg),*).await,
            AnyAdapter::Pacifica(a) => a.$method($($arg),*).await,
            AnyAdapter::Extended(a) => a.$method($($arg),*).await,
            AnyAdapter::Nado(a) => a.$method($($arg),*).await,
            AnyAdapter::Nord(a) => a.$method($($arg),*).await,
            AnyAdapter::Ethereal(a) => a.$method($($arg),*).await,
            AnyAdapter::Dydx(a) => a.$method($($arg),*).await,
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

/// All supported exchange adapter names.
pub const SUPPORTED_EXCHANGES: &[&str] = &[
    "vest", "paradex", "lighter", "hyperliquid", "grvt", "reya",
    "hotstuff", "pacifica", "extended", "nado", "nord", "ethereal", "dydx",
];

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
        "hyperliquid" => {
            let config = HyperliquidConfig::from_env();
            Ok(AnyAdapter::Hyperliquid(HyperliquidAdapter::new(config)))
        }
        "grvt" => {
            let config = GrvtConfig::from_env();
            Ok(AnyAdapter::Grvt(GrvtAdapter::new(config)))
        }
        "reya" => {
            let config = ReyaConfig::from_env();
            Ok(AnyAdapter::Reya(ReyaAdapter::new(config)))
        }
        "hotstuff" => {
            let config = HotstuffConfig::from_env();
            Ok(AnyAdapter::Hotstuff(HotstuffAdapter::new(config)))
        }
        "pacifica" => {
            let config = PacificaConfig::from_env();
            Ok(AnyAdapter::Pacifica(PacificaAdapter::new(config)))
        }
        "extended" => {
            let config = ExtendedConfig::from_env();
            Ok(AnyAdapter::Extended(ExtendedAdapter::new(config)))
        }
        "nado" => {
            let config = NadoConfig::from_env();
            Ok(AnyAdapter::Nado(NadoAdapter::new(config)))
        }
        "nord" => {
            let config = NordConfig::from_env();
            Ok(AnyAdapter::Nord(NordWsAdapter::new(config)))
        }
        "ethereal" => {
            let config = EtherealConfig::from_env();
            Ok(AnyAdapter::Ethereal(EtherealAdapter::new(config)))
        }
        "dydx" => {
            let config = DydxConfig::from_env();
            Ok(AnyAdapter::Dydx(DydxAdapter::new(config)))
        }
        _ => Err(ExchangeError::ConnectionFailed(format!(
            "Unknown exchange adapter: '{}'. Supported: {}",
            name,
            SUPPORTED_EXCHANGES.join(", ")
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
        // New exchanges — most use BASE-USD format
        ("hyperliquid" | "grvt" | "reya" | "hotstuff" | "pacifica"
        | "nado" | "nord" | "ethereal" | "dydx", base) => format!("{}-USD", base),
        ("extended", base) => format!("{}-USD", base),
        // Fallback: use pair as-is
        _ => pair.to_string(),
    }
}
