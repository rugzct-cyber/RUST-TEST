//! Trading Event System
//!
//! This module provides structured event types for logging trading operations.
//! All trading events use a consistent schema to enable timeline reconstruction
//! and debugging.
//!
//! # Event Types
//!
//! - **SpreadDetected**: Spread crosses entry threshold
//! - **TradeEntry**: Delta-neutral position opened
//! - **TradeExit**: Position closed
//! - **OrderPlaced**: Order sent to exchange
//! - **OrderFilled**: Order confirmation received
//! - **PositionMonitoring**: Periodic exit condition check
//!
//! # Example
//!
//! ```ignore
//! use crate::core::events::{TradingEvent, log_event};
//!
//! log_event(TradingEvent::spread_detected(
//!     "BTC-PERP",
//!     0.35,
//!     0.10,
//!     "A_OVER_B",
//! ));
//! ```

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info};

/// Trading event types for structured logging
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradingEventType {
    // Spread Events
    SpreadDetected,    // Spread crosses threshold
    SpreadOpportunity, // Opportunity sent to executor

    // Trade Events
    TradeEntry, // Delta-neutral entry executed
    TradeExit,  // Position closed

    // Order Events
    OrderPlaced, // Order sent to exchange
    OrderFilled, // Order confirmation received
    OrderFailed, // Order rejected

    PositionClosed,     // Position fully closed
    PositionMonitoring, // Periodic monitoring tick (throttled)

    // System Events
    BotStarted,
    BotShutdown,

    // Analysis Events (Slippage Investigation)
    SlippageAnalysis, // Detailed slippage + timing breakdown after trade
}

impl fmt::Display for TradingEventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TradingEventType::SpreadDetected => write!(f, "SPREAD_DETECTED"),
            TradingEventType::SpreadOpportunity => write!(f, "SPREAD_OPPORTUNITY"),
            TradingEventType::TradeEntry => write!(f, "TRADE_ENTRY"),
            TradingEventType::TradeExit => write!(f, "TRADE_EXIT"),
            TradingEventType::OrderPlaced => write!(f, "ORDER_PLACED"),
            TradingEventType::OrderFilled => write!(f, "ORDER_FILLED"),
            TradingEventType::OrderFailed => write!(f, "ORDER_FAILED"),
            TradingEventType::PositionClosed => write!(f, "POSITION_CLOSED"),
            TradingEventType::PositionMonitoring => write!(f, "POSITION_MONITORING"),
            TradingEventType::BotStarted => write!(f, "BOT_STARTED"),
            TradingEventType::BotShutdown => write!(f, "BOT_SHUTDOWN"),
            TradingEventType::SlippageAnalysis => write!(f, "SLIPPAGE_ANALYSIS"),
        }
    }
}

/// Timing breakdown for slippage analysis
///
/// Captures timestamps at each phase of trade execution to identify bottlenecks.
#[derive(Debug, Clone)]
pub struct TimingBreakdown {
    /// Timestamp when spread was detected in monitoring task
    pub detection_timestamp_ms: u64,
    /// Timestamp when opportunity was received by execution task
    pub signal_timestamp_ms: u64,
    /// Timestamp when orders were sent to exchanges
    pub order_sent_timestamp_ms: u64,
    /// Timestamp when order confirmations received
    pub order_confirmed_timestamp_ms: u64,
    /// Time from detection to signal (channel transit)
    pub detection_to_signal_ms: u64,
    /// Time from signal to order send (order preparation)  
    pub signal_to_order_ms: u64,
    /// Time from order send to confirmation (network + exchange)
    pub order_to_confirm_ms: u64,
    /// Total end-to-end latency
    pub total_latency_ms: u64,
}

impl TimingBreakdown {
    /// Create a new TimingBreakdown from timestamps
    pub fn new(
        detection_timestamp_ms: u64,
        signal_timestamp_ms: u64,
        order_sent_timestamp_ms: u64,
        order_confirmed_timestamp_ms: u64,
    ) -> Self {
        Self {
            detection_timestamp_ms,
            signal_timestamp_ms,
            order_sent_timestamp_ms,
            order_confirmed_timestamp_ms,
            detection_to_signal_ms: signal_timestamp_ms.saturating_sub(detection_timestamp_ms),
            signal_to_order_ms: order_sent_timestamp_ms.saturating_sub(signal_timestamp_ms),
            order_to_confirm_ms: order_confirmed_timestamp_ms
                .saturating_sub(order_sent_timestamp_ms),
            total_latency_ms: order_confirmed_timestamp_ms.saturating_sub(detection_timestamp_ms),
        }
    }
}

/// Type-safe event payload — each variant carries only its relevant fields
#[derive(Debug, Clone)]
pub enum EventPayload {
    /// Pre-entry spread detection
    SpreadDetected {
        entry_spread: f64,
        spread_threshold: f64,
        direction: String,
    },
    /// Delta-neutral entry executed
    TradeEntry {
        entry_spread: f64,
        spread_threshold: f64,
        direction: String,
        latency_ms: u64,
        long_exchange: String,
        short_exchange: String,
        vest_fill_price: f64,
        paradex_fill_price: f64,
    },
    /// Position closed
    TradeExit {
        entry_spread: f64,
        exit_spread: f64,
        spread_threshold: f64,
        profit: f64,
        polls: u64,
    },
    /// Periodic monitoring tick (throttled)
    PositionMonitoring {
        entry_spread: f64,
        exit_spread: f64,
        spread_threshold: f64,
        polls: u64,
    },
    /// Order sent to exchange
    OrderPlaced { order_id: String, direction: String },
    /// Order confirmation received
    OrderFilled { order_id: String, latency_ms: u64 },
    /// Position fully closed
    PositionClosed {
        entry_spread: f64,
        exit_spread: f64,
        profit: f64,
    },
    /// Detailed slippage + timing breakdown after trade
    SlippageAnalysis {
        detection_spread: f64,
        execution_spread: f64,
        slippage_bps: f64,
        slippage: f64,
        timing: TimingBreakdown,
        long_exchange: String,
        short_exchange: String,
        direction: String,
    },
    /// Events with no extra fields (BotStarted, BotShutdown, OrderFailed)
    Simple,
}

/// Trading event with common header + typed payload for structured logging
///
/// # Design
/// Instead of 19 `Option<T>` fields, each event type gets only its relevant
/// fields via `EventPayload`. The common header (type, timestamp, pair, exchange)
/// is shared across all events.
#[derive(Debug, Clone)]
pub struct TradingEvent {
    pub event_type: TradingEventType,
    pub timestamp_ms: u64,
    pub pair: Option<String>,
    pub exchange: Option<String>,
    pub payload: EventPayload,
}

impl TradingEvent {
    /// Create a new event with the current timestamp and Simple payload
    pub fn new(event_type: TradingEventType) -> Self {
        Self {
            event_type,
            timestamp_ms: current_timestamp_ms(),
            pair: None,
            exchange: None,
            payload: EventPayload::Simple,
        }
    }

    /// Helper to create event with pair set (reduces boilerplate in factory methods)
    fn with_pair(event_type: TradingEventType, pair: &str) -> Self {
        let mut event = Self::new(event_type);
        event.pair = Some(pair.to_string());
        event
    }

    /// Create a SPREAD_DETECTED event
    pub fn spread_detected(
        pair: &str,
        entry_spread: f64,
        spread_threshold: f64,
        direction: &str,
    ) -> Self {
        let mut event = Self::with_pair(TradingEventType::SpreadDetected, pair);
        event.exchange = Some("both".to_string());
        event.payload = EventPayload::SpreadDetected {
            entry_spread,
            spread_threshold,
            direction: direction.to_string(),
        };
        event
    }

    /// Create a TRADE_ENTRY event
    pub fn trade_entry(
        pair: &str,
        entry_spread: f64,
        spread_threshold: f64,
        direction: &str,
        long_exchange: &str,
        short_exchange: &str,
        latency_ms: u64,
        vest_fill_price: f64,
        paradex_fill_price: f64,
    ) -> Self {
        let mut event = Self::with_pair(TradingEventType::TradeEntry, pair);
        event.exchange = Some(format!("long:{},short:{}", long_exchange, short_exchange));
        event.payload = EventPayload::TradeEntry {
            entry_spread,
            spread_threshold,
            direction: direction.to_string(),
            latency_ms,
            long_exchange: long_exchange.to_string(),
            short_exchange: short_exchange.to_string(),
            vest_fill_price,
            paradex_fill_price,
        };
        event
    }

    /// Create a TRADE_EXIT event  
    pub fn trade_exit(
        pair: &str,
        entry_spread: f64,
        exit_spread: f64,
        spread_threshold: f64,
        profit: f64,
        polls: u64,
    ) -> Self {
        let mut event = Self::with_pair(TradingEventType::TradeExit, pair);
        event.exchange = Some("both".to_string());
        event.payload = EventPayload::TradeExit {
            entry_spread,
            exit_spread,
            spread_threshold,
            profit,
            polls,
        };
        event
    }

    /// Create a POSITION_MONITORING event (throttled logging)
    pub fn position_monitoring(
        pair: &str,
        entry_spread: f64,
        exit_spread: f64,
        spread_threshold: f64,
        polls: u64,
    ) -> Self {
        let mut event = Self::with_pair(TradingEventType::PositionMonitoring, pair);
        event.payload = EventPayload::PositionMonitoring {
            entry_spread,
            exit_spread,
            spread_threshold,
            polls,
        };
        event
    }

    /// Create an ORDER_PLACED event
    pub fn order_placed(pair: &str, exchange: &str, order_id: &str, direction: &str) -> Self {
        let mut event = Self::with_pair(TradingEventType::OrderPlaced, pair);
        event.exchange = Some(exchange.to_string());
        event.payload = EventPayload::OrderPlaced {
            order_id: order_id.to_string(),
            direction: direction.to_string(),
        };
        event
    }

    /// Create an ORDER_FILLED event
    pub fn order_filled(pair: &str, exchange: &str, order_id: &str, latency_ms: u64) -> Self {
        let mut event = Self::with_pair(TradingEventType::OrderFilled, pair);
        event.exchange = Some(exchange.to_string());
        event.payload = EventPayload::OrderFilled {
            order_id: order_id.to_string(),
            latency_ms,
        };
        event
    }

    /// Create a POSITION_CLOSED event
    pub fn position_closed(pair: &str, entry_spread: f64, exit_spread: f64, profit: f64) -> Self {
        let mut event = Self::with_pair(TradingEventType::PositionClosed, pair);
        event.exchange = Some("both".to_string());
        event.payload = EventPayload::PositionClosed {
            entry_spread,
            exit_spread,
            profit,
        };
        event
    }

    /// Create a BOT_STARTED event
    pub fn bot_started() -> Self {
        Self::new(TradingEventType::BotStarted)
    }

    /// Create a SLIPPAGE_ANALYSIS event
    ///
    /// Captures detailed slippage and timing breakdown after trade execution.
    pub fn slippage_analysis(
        pair: &str,
        detection_spread: f64,
        execution_spread: f64,
        timing: TimingBreakdown,
        long_exchange: &str,
        short_exchange: &str,
        direction: &str,
    ) -> Self {
        let slippage_bps = (detection_spread - execution_spread) * 100.0;

        let mut event = Self::with_pair(TradingEventType::SlippageAnalysis, pair);
        event.exchange = Some("both".to_string());
        event.payload = EventPayload::SlippageAnalysis {
            detection_spread,
            execution_spread,
            slippage_bps,
            slippage: slippage_bps / 100.0,
            timing,
            long_exchange: long_exchange.to_string(),
            short_exchange: short_exchange.to_string(),
            direction: direction.to_string(),
        };
        event
    }

    /// Create a BOT_SHUTDOWN event
    pub fn bot_shutdown() -> Self {
        Self::new(TradingEventType::BotShutdown)
    }
}

/// Get current timestamp in milliseconds since Unix epoch
pub fn current_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Calculate latency between detection timestamp and now
pub fn calculate_latency_ms(detection_timestamp_ms: u64) -> u64 {
    let now = current_timestamp_ms();
    now.saturating_sub(detection_timestamp_ms)
}

/// Format a percentage value with 4 decimal places
///
/// # Examples
/// - `0.6` → "0.6000%"
/// - `-1.5` → "-1.5000%"
/// - NaN/Infinity will produce "NaN%" or "inf%"
#[inline]
pub fn format_pct(value: f64) -> String {
    format!("{:.4}%", value)
}

/// Format a percentage value with 2 decimal places (compact display)
///
/// # Examples
/// - `0.35` → "0.35%"
/// - `-0.08` → "-0.08%"
#[inline]
pub fn format_pct_compact(value: f64) -> String {
    format!("{:.2}%", value)
}

/// Format compact log message for terminal display (T2)
///
/// Produces a single-line format: `[TAG] key=value key=value`
///
/// # Examples
/// - `log_compact("SCAN", &[("LES", "0.35%".to_string())])` → `[SCAN] LES=0.35%`
#[inline]
pub fn log_compact(tag: &str, fields: &[(&str, String)]) -> String {
    let fields_str: String = fields
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join(" ");
    format!("[{}] {}", tag, fields_str)
}

/// Log a trading event using structured tracing fields
///
/// Trading events use compact format: [TAG] key=value key=value
/// - SCAN: Pre-entry monitoring (LiveEntrySpread = current detected spread)
/// - ENTRY: Fill executed (EntrySpread, LongPrice, ShortPrice, Latency)
/// - HOLD: Post-entry monitoring (LiveExitSpread = current exit spread)
/// - EXIT: Position closed (ExitSpread, Captured = total profit)
pub fn log_event(event: &TradingEvent) {
    let event_type = event.event_type.to_string();
    let timestamp = event.timestamp_ms;

    match &event.payload {
        // T3: [SCAN] LiveEntrySpread=X% - Pre-entry spread detection
        EventPayload::SpreadDetected {
            entry_spread,
            direction,
            ..
        } => {
            let msg = log_compact(
                "SCAN",
                &[("LiveEntrySpread", format_pct_compact(*entry_spread))],
            );
            info!(
                event_type = %event_type,
                event_ts_ms = timestamp,
                pair = ?event.pair,
                direction = %direction,
                "{}", msg
            );
        }
        // T4: [ENTRY] EntrySpread=X% VestPrice=Y ParadexPrice=Z Latency=Nms
        EventPayload::TradeEntry {
            entry_spread,
            latency_ms,
            vest_fill_price,
            paradex_fill_price,
            long_exchange,
            short_exchange,
            ..
        } => {
            let msg = log_compact(
                "ENTRY",
                &[
                    ("EntrySpread", format_pct_compact(*entry_spread)),
                    ("VestPrice", format!("{:.0}", vest_fill_price)),
                    ("ParadexPrice", format!("{:.0}", paradex_fill_price)),
                    ("Latency", format!("{}ms", latency_ms)),
                ],
            );
            info!(
                event_type = %event_type,
                event_ts_ms = timestamp,
                pair = ?event.pair,
                long_exchange = %long_exchange,
                short_exchange = %short_exchange,
                "{}", msg
            );
        }
        // T5: [HOLD] LiveExitSpread=X% - Position monitoring (DEBUG level, throttled)
        EventPayload::PositionMonitoring {
            exit_spread, polls, ..
        } => {
            let msg = log_compact(
                "HOLD",
                &[("LiveExitSpread", format_pct_compact(*exit_spread))],
            );
            debug!(
                event_type = %event_type,
                event_ts_ms = timestamp,
                pair = ?event.pair,
                polls = polls,
                "{}", msg
            );
        }
        // T6: [EXIT] ExitSpread=X% Captured=Y%
        EventPayload::TradeExit {
            entry_spread,
            exit_spread,
            profit,
            polls,
            ..
        } => {
            let captured = format_pct_compact(entry_spread + exit_spread);
            let msg = log_compact(
                "EXIT",
                &[
                    ("ExitSpread", format_pct_compact(*exit_spread)),
                    ("Captured", captured),
                ],
            );
            info!(
                event_type = %event_type,
                event_ts_ms = timestamp,
                pair = ?event.pair,
                polls = polls,
                profit = %format_pct(*profit),
                "{}", msg
            );
        }
        // SLIPPAGE_ANALYSIS with timing breakdown
        EventPayload::SlippageAnalysis {
            detection_spread,
            execution_spread,
            slippage_bps,
            timing,
            long_exchange,
            short_exchange,
            direction,
            ..
        } => {
            info!(
                event_type = %event_type,
                event_ts_ms = timestamp,
                pair = ?event.pair,
                detection_spread_pct = detection_spread,
                execution_spread_pct = execution_spread,
                slippage_bps = slippage_bps,
                detection_to_signal_ms = timing.detection_to_signal_ms,
                signal_to_order_ms = timing.signal_to_order_ms,
                order_to_confirm_ms = timing.order_to_confirm_ms,
                total_latency_ms = timing.total_latency_ms,
                long_exchange = %long_exchange,
                short_exchange = %short_exchange,
                direction = %direction,
                "[SLIPPAGE] Trade analysis"
            );
        }
        // ORDER_PLACED
        EventPayload::OrderPlaced {
            order_id,
            direction,
        } => {
            info!(
                event_type = %event_type,
                event_ts_ms = timestamp,
                pair = ?event.pair,
                exchange = ?event.exchange,
                order_id = %order_id,
                direction = %direction,
                ""
            );
        }
        // ORDER_FILLED
        EventPayload::OrderFilled {
            order_id,
            latency_ms,
        } => {
            info!(
                event_type = %event_type,
                event_ts_ms = timestamp,
                pair = ?event.pair,
                exchange = ?event.exchange,
                order_id = %order_id,
                latency_ms = latency_ms,
                ""
            );
        }
        // POSITION_CLOSED
        EventPayload::PositionClosed {
            entry_spread,
            exit_spread,
            profit,
        } => {
            info!(
                event_type = %event_type,
                event_ts_ms = timestamp,
                pair = ?event.pair,
                entry_spread = %format_pct(*entry_spread),
                exit_spread = %format_pct(*exit_spread),
                profit = %format_pct(*profit),
                ""
            );
        }
        // Simple events (BotStarted, BotShutdown, etc.)
        EventPayload::Simple => {
            info!(
                event_type = %event_type,
                event_ts_ms = timestamp,
                pair = ?event.pair,
                exchange = ?event.exchange,
                ""
            );
        }
    }
}

// =============================================================================
// SystemEvent (Log Centralization - Tech-Spec: log-centralization)
// =============================================================================

/// System event types for structured logging
///
/// These events are distinct from TradingEvents - they cover system lifecycle
/// and operational concerns rather than trading execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemEventType {
    /// Task started (DEBUG)
    TaskStarted,
    /// Task stopped cleanly (INFO)
    TaskStopped,
    /// Task shutting down with reason (INFO)
    TaskShutdown,
    /// Adapter reconnecting (DEBUG)
    AdapterReconnect,
    /// Entry positions verified (DEBUG)
    PositionVerified,
    /// Individual position detail (DEBUG)
    PositionDetail,
    /// Trade execution started (DEBUG)
    TradeStarted,
}

/// System event for centralized logging
///
/// All system-level logs should be created via factory methods and
/// logged via `log_system_event()` for consistent formatting and levels.
pub struct SystemEvent {
    pub event_type: SystemEventType,
    pub task_name: Option<String>,
    pub exchange: Option<String>,
    pub message: String,
    pub details: Option<String>,
}

impl SystemEvent {
    /// Task started event (DEBUG level)
    pub fn task_started(task_name: &str) -> Self {
        Self {
            event_type: SystemEventType::TaskStarted,
            task_name: Some(task_name.to_string()),
            exchange: None,
            message: format!("{} task started", task_name),
            details: None,
        }
    }

    /// Task stopped cleanly (INFO level)
    pub fn task_stopped(task_name: &str) -> Self {
        Self {
            event_type: SystemEventType::TaskStopped,
            task_name: Some(task_name.to_string()),
            exchange: None,
            message: format!("{} task stopped", task_name),
            details: None,
        }
    }

    /// Task shutting down with reason (INFO level)
    pub fn task_shutdown(task_name: &str, reason: &str) -> Self {
        Self {
            event_type: SystemEventType::TaskShutdown,
            task_name: Some(task_name.to_string()),
            exchange: None,
            message: format!("{} shutting down", task_name),
            details: Some(reason.to_string()),
        }
    }

    /// Adapter reconnect event (DEBUG level)
    pub fn adapter_reconnect(exchange: &str, status: &str) -> Self {
        Self {
            event_type: SystemEventType::AdapterReconnect,
            task_name: None,
            exchange: Some(exchange.to_string()),
            message: format!("Adapter {}", status),
            details: None,
        }
    }

    /// Position verified event (DEBUG level)
    pub fn position_verified(vest_price: f64, paradex_price: f64, captured_spread: f64) -> Self {
        Self {
            event_type: SystemEventType::PositionVerified,
            task_name: None,
            exchange: None,
            message: "Entry positions verified".to_string(),
            details: Some(format!(
                "vest={:.2} paradex={:.2} spread={:.4}%",
                vest_price, paradex_price, captured_spread
            )),
        }
    }

    /// Position detail event (DEBUG level)
    pub fn position_detail(exchange: &str, side: &str, quantity: f64, entry_price: f64) -> Self {
        Self {
            event_type: SystemEventType::PositionDetail,
            task_name: None,
            exchange: Some(exchange.to_string()),
            message: "Position details".to_string(),
            details: Some(format!(
                "side={} qty={:.4} price={:.2}",
                side, quantity, entry_price
            )),
        }
    }

    /// Trade started event (DEBUG level)
    pub fn trade_started() -> Self {
        Self {
            event_type: SystemEventType::TradeStarted,
            task_name: None,
            exchange: None,
            message: "Position lock acquired - executing delta-neutral trade".to_string(),
            details: None,
        }
    }
}

/// Log a system event with appropriate level (DEBUG or INFO)
///
/// Level mapping:
/// - TaskStopped, TaskShutdown → INFO
/// - All others → DEBUG
pub fn log_system_event(event: &SystemEvent) {
    let event_type_str = format!("{:?}", event.event_type).to_uppercase();

    match event.event_type {
        // INFO level events (lifecycle endpoints)
        SystemEventType::TaskStopped | SystemEventType::TaskShutdown => {
            if let Some(ref details) = event.details {
                info!(
                    event_type = %event_type_str,
                    task = ?event.task_name,
                    details = %details,
                    "{}", event.message
                );
            } else {
                info!(
                    event_type = %event_type_str,
                    task = ?event.task_name,
                    "{}", event.message
                );
            }
        }
        // DEBUG level events (operational noise)
        _ => {
            if let Some(ref exchange) = event.exchange {
                debug!(
                    event_type = %event_type_str,
                    exchange = %exchange,
                    details = ?event.details,
                    "{}", event.message
                );
            } else {
                debug!(
                    event_type = %event_type_str,
                    task = ?event.task_name,
                    details = ?event.details,
                    "{}", event.message
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_type_display() {
        assert_eq!(
            TradingEventType::SpreadDetected.to_string(),
            "SPREAD_DETECTED"
        );
        assert_eq!(TradingEventType::TradeEntry.to_string(), "TRADE_ENTRY");
        assert_eq!(TradingEventType::TradeExit.to_string(), "TRADE_EXIT");
        assert_eq!(
            TradingEventType::PositionMonitoring.to_string(),
            "POSITION_MONITORING"
        );
    }

    #[test]
    fn test_spread_detected_event() {
        let event = TradingEvent::spread_detected("BTC-PERP", 0.35, 0.10, "A_OVER_B");

        assert_eq!(event.event_type, TradingEventType::SpreadDetected);
        assert_eq!(event.pair, Some("BTC-PERP".to_string()));
        match &event.payload {
            EventPayload::SpreadDetected {
                entry_spread,
                spread_threshold,
                direction,
            } => {
                assert_eq!(*entry_spread, 0.35);
                assert_eq!(*spread_threshold, 0.10);
                assert_eq!(direction, "A_OVER_B");
            }
            _ => panic!("Expected SpreadDetected payload"),
        }
    }

    #[test]
    fn test_trade_entry_event() {
        let event = TradingEvent::trade_entry(
            "BTC-PERP", 0.35, 0.10, "A_OVER_B", "vest", "paradex", 150, 42000.0, 42100.0,
        );

        assert_eq!(event.event_type, TradingEventType::TradeEntry);
        assert!(event.exchange.unwrap().contains("vest"));
        match &event.payload {
            EventPayload::TradeEntry {
                entry_spread,
                latency_ms,
                vest_fill_price,
                paradex_fill_price,
                ..
            } => {
                assert_eq!(*entry_spread, 0.35);
                assert_eq!(*latency_ms, 150);
                assert_eq!(*vest_fill_price, 42000.0);
                assert_eq!(*paradex_fill_price, 42100.0);
            }
            _ => panic!("Expected TradeEntry payload"),
        }
    }

    #[test]
    fn test_trade_exit_event() {
        let event = TradingEvent::trade_exit(
            "BTC-PERP", 0.35,  // entry_spread (original)
            -0.08, // exit_spread (at close)
            -0.10, // spread_threshold
            0.27,  // profit
            1200,  // polls
        );

        assert_eq!(event.event_type, TradingEventType::TradeExit);
        match &event.payload {
            EventPayload::TradeExit {
                entry_spread,
                exit_spread,
                profit,
                polls,
                ..
            } => {
                assert_eq!(*entry_spread, 0.35);
                assert_eq!(*exit_spread, -0.08);
                assert_eq!(*profit, 0.27);
                assert_eq!(*polls, 1200);
            }
            _ => panic!("Expected TradeExit payload"),
        }
    }

    #[test]
    fn test_position_monitoring_event() {
        let event = TradingEvent::position_monitoring(
            "BTC-PERP", 0.35,  // entry_spread (original at entry)
            -0.05, // exit_spread (current)
            -0.10, // spread_threshold (exit target)
            40,
        );

        assert_eq!(event.event_type, TradingEventType::PositionMonitoring);
        match &event.payload {
            EventPayload::PositionMonitoring {
                entry_spread,
                exit_spread,
                polls,
                ..
            } => {
                assert_eq!(*entry_spread, 0.35);
                assert_eq!(*exit_spread, -0.05);
                assert_eq!(*polls, 40);
            }
            _ => panic!("Expected PositionMonitoring payload"),
        }
    }

    #[test]
    fn test_latency_calculation() {
        let past = current_timestamp_ms() - 100; // 100ms ago
        let latency = calculate_latency_ms(past);
        assert!(latency >= 100);
        assert!(latency < 200); // Should be close to 100ms
    }

    #[test]
    fn test_current_timestamp() {
        let ts = current_timestamp_ms();
        // Should be a reasonable Unix timestamp (after 2024)
        assert!(ts > 1704067200000); // 2024-01-01
    }

    // Slippage Analysis Tests

    #[test]
    fn test_timing_breakdown_new() {
        let t_detection = 1000u64;
        let t_signal = 1005u64;
        let t_order_sent = 1010u64;
        let t_order_confirmed = 1100u64;

        let timing = TimingBreakdown::new(t_detection, t_signal, t_order_sent, t_order_confirmed);

        assert_eq!(timing.detection_timestamp_ms, 1000);
        assert_eq!(timing.signal_timestamp_ms, 1005);
        assert_eq!(timing.order_sent_timestamp_ms, 1010);
        assert_eq!(timing.order_confirmed_timestamp_ms, 1100);
        assert_eq!(timing.detection_to_signal_ms, 5);
        assert_eq!(timing.signal_to_order_ms, 5);
        assert_eq!(timing.order_to_confirm_ms, 90);
        assert_eq!(timing.total_latency_ms, 100);
    }

    #[test]
    fn test_slippage_analysis_event() {
        let timing = TimingBreakdown::new(1000, 1005, 1010, 1100);

        let event = TradingEvent::slippage_analysis(
            "BTC-PERP", 0.35, 0.10, timing, "vest", "paradex", "AOverB",
        );

        assert_eq!(event.event_type, TradingEventType::SlippageAnalysis);
        assert_eq!(event.pair, Some("BTC-PERP".to_string()));
        match &event.payload {
            EventPayload::SlippageAnalysis {
                detection_spread,
                execution_spread,
                slippage_bps,
                timing: stored_timing,
                long_exchange,
                short_exchange,
                ..
            } => {
                assert_eq!(*detection_spread, 0.35);
                assert_eq!(*execution_spread, 0.10);
                // Slippage = (0.35 - 0.10) * 100 = 25 basis points
                assert!(
                    (*slippage_bps - 25.0).abs() < 0.001,
                    "Expected ~25.0 bps, got {}",
                    slippage_bps
                );
                assert_eq!(stored_timing.total_latency_ms, 100);
                assert_eq!(long_exchange, "vest");
                assert_eq!(short_exchange, "paradex");
            }
            _ => panic!("Expected SlippageAnalysis payload"),
        }
    }

    #[test]
    fn test_slippage_analysis_event_type_display() {
        assert_eq!(
            TradingEventType::SlippageAnalysis.to_string(),
            "SLIPPAGE_ANALYSIS"
        );
    }
}
