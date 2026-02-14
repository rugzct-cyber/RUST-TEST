//! Arbi v5 — Rust Backend Entry Point
//!
//! Orchestrates:
//! 1. Config + logging initialization
//! 2. Broadcast channels (price bus + event bus)
//! 3. ExchangeManager → all adapters
//! 4. PriceAggregator + ArbitrageDetector pipeline
//! 5. axum WebSocket API server
//! 6. Ctrl+C graceful shutdown

use std::sync::Arc;

use tokio::sync::{broadcast, RwLock};
use tracing::{error, info, warn};

use hft_bot::adapters::ExchangeManager;
use hft_bot::config::{init_logging, load_config};
use hft_bot::core::{
    ArbitrageDetector, BroadcastEvent, DetectorConfig, PriceAggregator, PriceData,
};
use hft_bot::server::{self, AppState};

/// Broadcast channel capacity for price data
const PRICE_CHANNEL_CAPACITY: usize = 4096;
/// Broadcast channel capacity for client events
const EVENT_CHANNEL_CAPACITY: usize = 1024;
/// Default server port (can be overridden with PORT env var)
const DEFAULT_PORT: u16 = 8080;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // =========================================================================
    // 1. Config + logging
    // =========================================================================
    dotenvy::dotenv().ok();
    init_logging();

    info!("=== Arbi v5 Rust Backend ===");

    // Load config for exchange/symbol list
    let config_path = std::path::Path::new("config.yaml");
    let (exchanges, symbols) = match load_config(config_path) {
        Ok(config) => {
            info!("Config loaded from config.yaml");
            // Extract unique exchanges and symbols from monitor configs
            let mut exch_set = std::collections::HashSet::new();
            let mut sym_set = std::collections::HashSet::new();
            for bot in &config.bots {
                exch_set.insert(bot.dex_a.to_string());
                exch_set.insert(bot.dex_b.to_string());
                sym_set.insert(bot.pair.base().to_string());
            }
            (
                exch_set.into_iter().collect::<Vec<_>>(),
                sym_set.into_iter().collect::<Vec<_>>(),
            )
        }
        Err(e) => {
            warn!(error = %e, "Could not load config.yaml, using all exchanges");
            // Fallback: connect to ALL supported exchanges × 3 symbols
            (
                vec![
                    "vest".into(), "paradex".into(), "lighter".into(),
                    "hyperliquid".into(), "grvt".into(), "reya".into(),
                    "hotstuff".into(), "pacifica".into(), "extended".into(),
                    "nado".into(), "nord".into(), "ethereal".into(),
                ],
                vec!["BTC".into(), "ETH".into(), "SOL".into()],
            )
        }
    };

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_PORT);

    info!(
        exchanges = ?exchanges,
        symbols = ?symbols,
        port = port,
        "Starting with configuration"
    );

    // =========================================================================
    // 2. Broadcast channels
    // =========================================================================
    let (price_tx, _) = broadcast::channel::<PriceData>(PRICE_CHANNEL_CAPACITY);
    let (event_tx, _) = broadcast::channel::<BroadcastEvent>(EVENT_CHANNEL_CAPACITY);

    // =========================================================================
    // 3. ExchangeManager → launch all adapters
    // =========================================================================
    let manager = ExchangeManager::new(exchanges, symbols, price_tx.clone());

    let adapter_handles = manager.connect_all().await;
    info!(
        count = adapter_handles.len(),
        "Exchange adapters launched"
    );

    // =========================================================================
    // 4. Price pipeline: Aggregator + Detector
    // =========================================================================
    let aggregator = Arc::new(RwLock::new(PriceAggregator::new()));
    let pipeline_aggregator = aggregator.clone();
    let pipeline_event_tx = event_tx.clone();

    let pipeline_handle = tokio::spawn(async move {
        let mut price_rx = price_tx.subscribe();
        let mut detector = ArbitrageDetector::with_config(DetectorConfig {
            min_spread_percent: 0.05,
            max_price_age_ms: 5_000,
            min_confirmations: 2,
            ..Default::default()
        });

        let mut update_count: u64 = 0;

        loop {
            match price_rx.recv().await {
                Ok(price_data) => {
                    update_count += 1;

                    // Forward raw price as event
                    let _ = pipeline_event_tx.send(BroadcastEvent::Price(price_data.clone()));

                    // Aggregate
                    let aggregated = {
                        let mut agg = pipeline_aggregator.write().await;
                        agg.update(price_data)
                    };

                    // Detect arbitrage
                    if let Some(opportunity) = detector.detect(&aggregated) {
                        info!(
                            symbol = opportunity.symbol.as_ref(),
                            buy = opportunity.buy_exchange.as_ref(),
                            sell = opportunity.sell_exchange.as_ref(),
                            spread = format!("{:.4}%", opportunity.spread_percent),
                            "🔥 Arbitrage opportunity detected"
                        );
                        let _ = pipeline_event_tx
                            .send(BroadcastEvent::Opportunity(opportunity));
                    }

                    // Periodic cleanup
                    if update_count % 1000 == 0 {
                        let mut agg = pipeline_aggregator.write().await;
                        agg.cleanup();
                        detector.cleanup();
                    }

                    // Periodic stats
                    if update_count % 500 == 0 {
                        let agg = pipeline_aggregator.read().await;
                        info!(
                            updates = update_count,
                            symbols = agg.symbol_count(),
                            prices = agg.price_count(),
                            "Pipeline stats"
                        );
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!(skipped = n, "Pipeline lagged, skipped price updates");
                }
                Err(broadcast::error::RecvError::Closed) => {
                    info!("Price channel closed, pipeline shutting down");
                    break;
                }
            }
        }
    });

    // =========================================================================
    // 5. axum WebSocket API server
    // =========================================================================
    let state = AppState {
        event_tx: event_tx.clone(),
        aggregator: aggregator.clone(),
    };

    let server_handle = tokio::spawn(async move {
        if let Err(e) = server::start_server(state, port).await {
            error!(error = %e, "WebSocket server failed");
        }
    });

    // =========================================================================
    // 6. Wait for Ctrl+C → graceful shutdown
    // =========================================================================
    info!("Server running on http://0.0.0.0:{}", port);
    info!("WebSocket endpoint: ws://0.0.0.0:{}/ws", port);
    info!("Press Ctrl+C to shutdown");

    tokio::signal::ctrl_c().await?;
    info!("Shutdown signal received");

    // Abort all tasks
    pipeline_handle.abort();
    server_handle.abort();
    for (name, handle) in adapter_handles {
        info!(exchange = %name, "Stopping adapter");
        handle.abort();
    }

    info!("=== Shutdown complete ===");
    Ok(())
}
