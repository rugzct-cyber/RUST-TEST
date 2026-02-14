//! Exchange manager — orchestrates multiple exchange adapters.
//!
//! Spawns one Tokio task per adapter, each emitting `PriceData` into
//! a shared `tokio::broadcast` channel by reading from the adapter's
//! `AtomicBestPrices` (lock-free hot path).

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::broadcast;
use tracing::{error, info, warn};

use crate::adapters::{ExchangeAdapter, create_adapter, resolve_symbol};
use crate::core::types::{current_time_ms, PriceData};

/// Manages multiple exchange adapters and emits price data.
pub struct ExchangeManager {
    /// Broadcast sender for price data
    price_tx: broadcast::Sender<PriceData>,
    /// Exchange names that are configured
    exchanges: Vec<String>,
    /// Normalized symbols to subscribe to (e.g. ["BTC", "ETH", "SOL"])
    symbols: Vec<String>,
    /// Poll interval in milliseconds
    poll_interval_ms: u64,
}

impl ExchangeManager {
    /// Create a new manager with a broadcast channel.
    ///
    /// `exchanges` - list of exchange names (e.g. ["vest", "paradex", "lighter"])
    /// `symbols` - normalized symbol list (e.g. ["BTC", "ETH", "SOL"])
    /// `price_tx` - broadcast sender that all adapters will emit into
    pub fn new(
        exchanges: Vec<String>,
        symbols: Vec<String>,
        price_tx: broadcast::Sender<PriceData>,
    ) -> Self {
        Self {
            price_tx,
            exchanges,
            symbols,
            poll_interval_ms: 100,
        }
    }

    /// Set the poll interval in milliseconds (default: 100ms).
    pub fn with_poll_interval(mut self, ms: u64) -> Self {
        self.poll_interval_ms = ms;
        self
    }

    /// Connect all adapters and start streaming prices.
    ///
    /// Returns a map of exchange → JoinHandle for monitoring.
    /// Each adapter runs in its own Tokio task.
    pub async fn connect_all(
        &self,
    ) -> HashMap<String, tokio::task::JoinHandle<()>> {
        let mut handles = HashMap::new();

        for exchange_name in &self.exchanges {
            let name = exchange_name.clone();
            let symbols = self.symbols.clone();
            let price_tx = self.price_tx.clone();
            let poll_ms = self.poll_interval_ms;

            let handle = tokio::spawn(async move {
                Self::run_adapter(name, symbols, price_tx, poll_ms).await;
            });

            handles.insert(exchange_name.clone(), handle);
        }

        info!(
            exchanges = ?self.exchanges,
            symbols = ?self.symbols,
            "All exchange adapters launched"
        );

        handles
    }

    /// Run a single adapter: connect → subscribe → poll prices → emit PriceData.
    async fn run_adapter(
        exchange: String,
        symbols: Vec<String>,
        price_tx: broadcast::Sender<PriceData>,
        poll_ms: u64,
    ) {
        info!(exchange = %exchange, "Starting adapter");

        // Create the adapter
        let mut adapter = match create_adapter(&exchange) {
            Ok(a) => a,
            Err(e) => {
                error!(exchange = %exchange, error = %e, "Failed to create adapter");
                return;
            }
        };

        // Connect
        if let Err(e) = adapter.connect().await {
            error!(exchange = %exchange, error = %e, "Failed to connect");
            return;
        }

        info!(exchange = %exchange, "Connected");

        // Subscribe to all symbols (using exchange-specific symbol names)
        for symbol in &symbols {
            let exchange_symbol = resolve_symbol(&exchange, symbol);
            if let Err(e) = adapter.subscribe_orderbook(&exchange_symbol).await {
                warn!(
                    exchange = %exchange,
                    symbol = %symbol,
                    exchange_symbol = %exchange_symbol,
                    error = %e,
                    "Failed to subscribe — skipping"
                );
            } else {
                info!(
                    exchange = %exchange,
                    symbol = %symbol,
                    exchange_symbol = %exchange_symbol,
                    "Subscribed"
                );
            }
        }

        // Read prices from the adapter's AtomicBestPrices (lock-free)
        let best_prices = adapter.get_shared_best_prices();
        let exchange_arc: Arc<str> = Arc::from(exchange.as_str());
        let poll_duration = tokio::time::Duration::from_millis(poll_ms);

        // Track previously seen bid/ask per symbol to avoid duplicate emissions.
        let mut last_seen: HashMap<String, (f64, f64)> = HashMap::new();

        // Track reconnection backoff
        let mut reconnect_backoff_ms: u64 = 1_000;
        const MAX_RECONNECT_BACKOFF_MS: u64 = 60_000;

        info!(exchange = %exchange, "Entering price poll loop");

        loop {
            tokio::time::sleep(poll_duration).await;

            // Check connection health — also detect stale (silent disconnect)
            if !adapter.is_connected() || adapter.is_stale() {
                let reason = if !adapter.is_connected() { "disconnected" } else { "stale (no data)" };
                warn!(exchange = %exchange, reason = %reason, "Adapter unhealthy, attempting reconnect...");

                match adapter.reconnect().await {
                    Ok(()) => {
                        info!(exchange = %exchange, "Reconnected successfully");
                        reconnect_backoff_ms = 1_000; // Reset backoff
                        continue;
                    }
                    Err(e) => {
                        error!(
                            exchange = %exchange,
                            error = %e,
                            retry_in_ms = reconnect_backoff_ms,
                            "Reconnect failed, retrying after backoff..."
                        );
                        tokio::time::sleep(tokio::time::Duration::from_millis(reconnect_backoff_ms)).await;
                        reconnect_backoff_ms = (reconnect_backoff_ms * 2).min(MAX_RECONNECT_BACKOFF_MS);
                        continue; // Keep trying, never break
                    }
                }
            }

            // Read atomic best prices
            let (bid, ask) = best_prices.load();
            if bid <= 0.0 || ask <= 0.0 {
                continue; // No valid prices yet
            }

            // For now, single-symbol adapters: emit for first symbol
            // In the future, adapters will provide per-symbol AtomicBestPrices
            // via the SharedOrderbooks multi-symbol map.
            //
            // We use the SharedOrderbooks to check which symbols have data.
            let shared_ob = adapter.get_shared_orderbooks();
            let books = shared_ob.read().await;

            for symbol in &symbols {
                let exchange_symbol = resolve_symbol(&exchange, symbol);

                if let Some(orderbook) = books.get(&exchange_symbol) {
                    let ob_bid = orderbook.best_bid().unwrap_or(0.0);
                    let ob_ask = orderbook.best_ask().unwrap_or(0.0);

                    if ob_bid <= 0.0 || ob_ask <= 0.0 {
                        continue;
                    }

                    // Only emit if price changed (avoid flooding)
                    let prev = last_seen.get(symbol.as_str());
                    if let Some(&(prev_bid, prev_ask)) = prev {
                        if (prev_bid - ob_bid).abs() < f64::EPSILON
                            && (prev_ask - ob_ask).abs() < f64::EPSILON
                        {
                            continue;
                        }
                    }

                    last_seen.insert(symbol.clone(), (ob_bid, ob_ask));

                    let price_data = PriceData {
                        exchange: exchange_arc.clone(),
                        symbol: Arc::from(symbol.as_str()),
                        bid: ob_bid,
                        ask: ob_ask,
                        timestamp_ms: current_time_ms(),
                    };

                    // Broadcast — if no receivers, just drop
                    let _ = price_tx.send(price_data);
                }
            }
            // Drop the read lock before next iteration
            drop(books);
        }
    }
}
