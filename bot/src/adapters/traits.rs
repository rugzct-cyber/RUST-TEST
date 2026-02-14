//! Exchange adapter trait definition
//!
//! The ExchangeAdapter trait defines the common interface that all
//! exchange adapters must implement for read-only market data access.

use async_trait::async_trait;

use crate::adapters::errors::ExchangeResult;
use crate::adapters::types::Orderbook;
use crate::core::channels::{OrderbookNotify, SharedBestPrices, SharedOrderbooks};

/// Common trait for all exchange adapters (read-only market data)
///
/// This trait defines the interface for connecting to exchanges
/// and subscribing to real-time orderbook data.
#[async_trait]
pub trait ExchangeAdapter: Send + Sync {
    /// Establish WebSocket connection to the exchange (public data feed)
    async fn connect(&mut self) -> ExchangeResult<()>;

    /// Gracefully disconnect from the exchange
    async fn disconnect(&mut self) -> ExchangeResult<()>;

    /// Subscribe to orderbook updates for a trading symbol
    async fn subscribe_orderbook(&mut self, symbol: &str) -> ExchangeResult<()>;

    /// Unsubscribe from orderbook updates
    async fn unsubscribe_orderbook(&mut self, symbol: &str) -> ExchangeResult<()>;

    /// Get cached orderbook for a symbol (synchronous read)
    fn get_orderbook(&self, symbol: &str) -> Option<&Orderbook>;

    /// Check if adapter is currently connected to the exchange
    fn is_connected(&self) -> bool;

    /// Check if connection is stale (no data received in last 30 seconds)
    fn is_stale(&self) -> bool;

    /// Sync local orderbook cache from shared storage
    async fn sync_orderbooks(&mut self);

    /// Attempt to reconnect to the exchange
    async fn reconnect(&mut self) -> ExchangeResult<()>;

    /// Get the exchange name identifier
    fn exchange_name(&self) -> &'static str;

    // =========================================================================
    // Shared Data Access (for monitoring)
    // =========================================================================

    /// Get shared orderbooks for lock-free monitoring
    fn get_shared_orderbooks(&self) -> SharedOrderbooks;

    /// Get shared atomic best prices for lock-free hot-path monitoring
    fn get_shared_best_prices(&self) -> SharedBestPrices;

    /// Set the shared orderbook notification (event-driven monitoring)
    fn set_orderbook_notify(&mut self, notify: OrderbookNotify);
}


