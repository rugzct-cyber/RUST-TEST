//! Core module - Spread calculation, channels, execution, events
//!
//! # Module Architecture
//!
//! This module uses **explicit re-exports** instead of glob exports (`pub use module::*`)
//! to provide better API visibility and prevent accidental public API changes.
//!
//! V1 HFT Mode: State, position_monitor, logging, and reconnect modules removed for clean codebase.
//!
//! ## Usage
//! Prefer importing from `crate::core`:
//! ```ignore
//! use crate::core::{SpreadCalculator, TradingEvent};
//! ```
//!
//! ## Adding New Public Types
//! When adding new public types to submodules, explicitly add them to the
//! re-exports below to make them part of the public API.

pub mod channels;
pub mod events;
pub mod execution;
pub mod monitoring;
pub mod pyth;
pub mod runtime;
pub mod scaling;
pub mod spread;

// Explicit re-exports for spread module (SpreadMonitor, SpreadTick, SpreadThresholds removed - unused)
pub use spread::{SpreadCalculator, SpreadDirection, SpreadResult};

// Explicit re-exports for channels module
pub use channels::{AtomicBestPrices, OrderbookNotify, SharedBestPrices, SharedOrderbooks, SpreadOpportunity};

// Explicit re-exports for execution module
pub use execution::{DeltaNeutralExecutor, DeltaNeutralResult, LegStatus};

// Explicit re-exports for runtime module
pub use runtime::execution_task;

// Explicit re-exports for monitoring module
pub use monitoring::{monitoring_task, MonitoringConfig};

// Explicit re-exports for events module
pub use events::{
    current_timestamp_ms, format_pct, log_event, EventPayload, TradingEvent, TradingEventType,
};

// Explicit re-exports for pyth module (USD/USDC conversion)
pub use pyth::{spawn_rate_refresh_task, UsdcRateCache};
