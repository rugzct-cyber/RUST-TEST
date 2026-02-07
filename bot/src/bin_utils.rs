//! Shared bootstrap utilities for binary entry points
//!
//! Eliminates duplicated init boilerplate across `close_positions`, `monitor`,
//! `test_order`, and `test_vest_account` binaries.
//!
//! # Architecture Note (Why 4 connections?)
//!
//! Each exchange uses **2 connections** (4 total):
//! - **WebSocket** — persistent, for real-time orderbook streaming
//! - **REST/HTTP** — on-demand, for order placement and account queries
//!
//! The WebSocket is opened by `connect()` and kept alive for the bot's lifetime.
//! The REST client is created once and reused. Both are independent because:
//! - WebSocket is push-based (exchange → bot), REST is pull-based (bot → exchange)
//! - They have different auth mechanisms on some exchanges
//! - Losing the WebSocket shouldn't block order placement, and vice versa

use std::path::Path;
use tracing::info;

use crate::adapters::paradex::{ParadexAdapter, ParadexConfig};
use crate::adapters::traits::ExchangeAdapter;
use crate::adapters::vest::{VestAdapter, VestConfig};
use crate::config;

/// Shared boot result: config, first bot entry, vest pair, paradex pair
pub struct BootResult {
    pub cfg: config::AppConfig,
    pub vest_pair: String,
    pub paradex_pair: String,
}

/// Initialize dotenv, logging, and load config with trading pairs.
///
/// This covers the common init sequence shared by all binaries:
/// 1. Load `.env` file
/// 2. Initialize structured logging (JSON/Pretty/TUI via `LOG_FORMAT`)
/// 3. Load `config.yaml` and extract the first bot's trading pairs
///
/// # Panics
/// - If `config.yaml` cannot be loaded
/// - If `config.yaml` has no bot entries
pub fn boot() -> BootResult {
    dotenvy::dotenv().ok();
    config::init_logging();

    let cfg = config::load_config(Path::new("config.yaml")).expect("Failed to load config.yaml");
    let bot = cfg
        .bots
        .first()
        .expect("config.yaml must have at least one bot entry");
    let vest_pair = bot.pair.to_string();
    let paradex_pair = format!("{}-USD-PERP", vest_pair.split('-').next().unwrap_or("BTC"));

    BootResult {
        cfg,
        vest_pair,
        paradex_pair,
    }
}

/// Initialize dotenv and logging only (no config.yaml needed).
///
/// Use this for binaries that don't need config.yaml (e.g. `test_order`).
pub fn boot_minimal() {
    dotenvy::dotenv().ok();
    config::init_logging();
}

/// Create and connect both Vest and Paradex adapters in parallel.
///
/// # Architecture
/// This creates 4 connections total (2 per exchange: WS + REST).
/// See module-level docs for rationale.
///
/// # Errors
/// Returns the first connection error encountered.
pub async fn connect_adapters() -> Result<(VestAdapter, ParadexAdapter), Box<dyn std::error::Error>>
{
    let vest_config = VestConfig::from_env()?;
    let paradex_config = ParadexConfig::from_env()?;
    let mut vest_adapter = VestAdapter::new(vest_config);
    let mut paradex_adapter = ParadexAdapter::new(paradex_config);

    info!("Connecting to exchanges...");
    let (vest_conn, paradex_conn) = tokio::join!(vest_adapter.connect(), paradex_adapter.connect());
    vest_conn?;
    paradex_conn?;
    info!("Both adapters connected");

    Ok((vest_adapter, paradex_adapter))
}
