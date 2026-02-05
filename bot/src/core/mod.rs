//! Core module - Spread calculation, VWAP, channels, execution, events
//!
//! # Module Architecture (Story 0.3, Story 7.3, Story 7.4, Story 5.3)
//!
//! This module uses **explicit re-exports** instead of glob exports (`pub use module::*`)
//! to provide better API visibility and prevent accidental public API changes.
//!
//! V1 HFT Mode: State, position_monitor, logging, and reconnect modules removed for clean codebase.
//!
//! ## Usage
//! Prefer importing from `crate::core`:
//! ```ignore
//! use crate::core::{SpreadCalculator, VwapResult, TradingEvent};
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
pub mod spread;

// Explicit re-exports for spread module (Story 7.4: SpreadMonitor, SpreadTick, SpreadThresholds removed - unused)
pub use spread::{SpreadCalculator, SpreadDirection, SpreadResult};

// Explicit re-exports for channels module
pub use channels::{ChannelBundle, SpreadOpportunity, DEFAULT_CHANNEL_CAPACITY};

// Explicit re-exports for execution module (Story 2.3)
pub use execution::{DeltaNeutralExecutor, DeltaNeutralResult, LegStatus};

// Explicit re-exports for runtime module (Story 2.3, Story 7.3)
pub use runtime::execution_task;

// Explicit re-exports for monitoring module (Story 6.2)
pub use monitoring::{monitoring_task, MonitoringConfig, POLL_INTERVAL_MS};

// Explicit re-exports for events module (Story 5.3)
pub use events::{TradingEvent, TradingEventType, log_event, log_trading_event, current_timestamp_ms, calculate_latency_ms, format_pct, fmt_price};

// Explicit re-exports for pyth module (USD/USDC conversion)
pub use pyth::{UsdcRateCache, spawn_rate_refresh_task};

