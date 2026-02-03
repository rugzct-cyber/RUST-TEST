//! Core module - Spread calculation, VWAP, channels, logging
//!
//! # Module Architecture (Story 0.3, Story 7.3)
//!
//! This module uses **explicit re-exports** instead of glob exports (`pub use module::*`)
//! to provide better API visibility and prevent accidental public API changes.
//!
//! V1 HFT Mode: State and position_monitor modules removed for clean codebase.
//!
//! ## Usage
//! Prefer importing from `crate::core`:
//! ```ignore
//! use crate::core::{SpreadCalculator, VwapResult};
//! ```
//!
//! ## Adding New Public Types
//! When adding new public types to submodules, explicitly add them to the
//! re-exports below to make them part of the public API.

pub mod channels;
pub mod execution;
pub mod logging;
pub mod monitoring;
pub mod reconnect;
pub mod runtime;
pub mod spread;
pub mod vwap;

// Explicit re-exports for spread module
pub use spread::{SpreadCalculator, SpreadDirection, SpreadMonitor, SpreadMonitorError, SpreadResult, SpreadThresholds, SpreadTick};

// Explicit re-exports for vwap module
pub use vwap::{calculate_vwap, VwapResult};

// Explicit re-exports for channels module
pub use channels::{ChannelBundle, SpreadOpportunity, DEFAULT_CHANNEL_CAPACITY};

// Explicit re-exports for logging module
pub use logging::{
    init_logging, init_logging_with_config, sanitize, sanitize_signature, LoggingConfig,
    SanitizedValue, DEFAULT_LOG_LEVEL, SENSITIVE_FIELD_PATTERNS,
};

// Explicit re-exports for execution module (Story 2.3)
pub use execution::{DeltaNeutralExecutor, DeltaNeutralResult, LegStatus};

// Explicit re-exports for runtime module (Story 2.3, Story 7.3)
pub use runtime::execution_task;

// Explicit re-exports for reconnect module (Story 4.4)
pub use reconnect::{ReconnectConfig, reconnect_monitor_task};

// Explicit re-exports for monitoring module (Story 6.2)
pub use monitoring::{monitoring_task, MonitoringConfig, POLL_INTERVAL_MS};
