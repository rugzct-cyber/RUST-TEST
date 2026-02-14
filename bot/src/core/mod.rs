//! Core module - Price pipeline, spread calculation, and shared channels
//!
//! # Module Architecture
//!
//! This module uses **explicit re-exports** instead of glob exports (`pub use module::*`)
//! to provide better API visibility and prevent accidental public API changes.
//!
//! ## Usage
//! Prefer importing from `crate::core`:
//! ```ignore
//! use crate::core::{PriceAggregator, ArbitrageDetector, PriceData};
//! ```

pub mod aggregator;
pub mod channels;
pub mod detector;
pub mod pyth;
pub mod spread;
pub mod types;

// Explicit re-exports for new pipeline types
pub use types::{
    AggregatedPrice, ArbitrageOpportunity, BroadcastEvent, ExchangePrice, PriceData,
    current_time_ms,
};
pub use aggregator::PriceAggregator;
pub use detector::{ArbitrageDetector, DetectorConfig};

// Explicit re-exports for spread module
pub use spread::{SpreadCalculator, SpreadDirection, SpreadResult};

// Explicit re-exports for channels module
pub use channels::{AtomicBestPrices, OrderbookNotify, SharedBestPrices, SharedOrderbooks};

// Explicit re-exports for pyth module (USD/USDC conversion)
pub use pyth::{spawn_rate_refresh_task, UsdcRateCache};
