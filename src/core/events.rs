//! Trading Event System (Story 5.3)
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
use tracing::{info, debug};

/// Trading event types for structured logging
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradingEventType {
    // Spread Events
    SpreadDetected,       // Spread crosses threshold
    SpreadOpportunity,    // Opportunity sent to executor
    
    // Trade Events
    TradeEntry,           // Delta-neutral entry executed
    TradeExit,            // Position closed
    
    // Order Events
    OrderPlaced,          // Order sent to exchange
    OrderFilled,          // Order confirmation received
    OrderFailed,          // Order rejected
    
    // Position Events
    PositionOpened,       // New position tracked
    PositionClosed,       // Position fully closed
    PositionMonitoring,   // Periodic monitoring tick (throttled)
    
    // System Events
    BotStarted,
    BotShutdown,
    
    // Analysis Events (Story 8.1: Slippage Investigation)
    SlippageAnalysis,     // Detailed slippage + timing breakdown after trade
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
            TradingEventType::PositionOpened => write!(f, "POSITION_OPENED"),
            TradingEventType::PositionClosed => write!(f, "POSITION_CLOSED"),
            TradingEventType::PositionMonitoring => write!(f, "POSITION_MONITORING"),
            TradingEventType::BotStarted => write!(f, "BOT_STARTED"),
            TradingEventType::BotShutdown => write!(f, "BOT_SHUTDOWN"),
            TradingEventType::SlippageAnalysis => write!(f, "SLIPPAGE_ANALYSIS"),
        }
    }
}

/// Timing breakdown for slippage analysis (Story 8.1)
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
            order_to_confirm_ms: order_confirmed_timestamp_ms.saturating_sub(order_sent_timestamp_ms),
            total_latency_ms: order_confirmed_timestamp_ms.saturating_sub(detection_timestamp_ms),
        }
    }
}

/// Trading event with all context fields for structured logging
///
/// # Spread Fields (CRITICAL DISTINCTION)
/// - `entry_spread`: Spread at detection/entry (e.g., 0.35%)
/// - `exit_spread`: Spread at close/exit (e.g., -0.08%)
/// - `spread_threshold`: Configured threshold being checked
#[derive(Debug, Clone)]
pub struct TradingEvent {
    pub event_type: TradingEventType,
    pub timestamp_ms: u64,           // Unix epoch milliseconds
    pub pair: Option<String>,        // e.g., "BTC-PERP"
    pub exchange: Option<String>,    // e.g., "vest", "paradex", "both"
    
    // IMPORTANT: Two distinct spread types
    pub entry_spread: Option<f64>,   // Spread at detection/entry
    pub exit_spread: Option<f64>,    // Spread at close/exit
    pub spread_threshold: Option<f64>, // Configured threshold
    
    pub latency_ms: Option<u64>,     // Detection-to-event latency
    pub order_id: Option<String>,    // For order events
    pub direction: Option<String>,   // "A_OVER_B" or "B_OVER_A"
    pub profit: Option<f64>,         // For exit events
    pub slippage: Option<f64>,       // Difference between detected and executed spread
    pub polls: Option<u64>,          // For monitoring events
    
    // Story 8.1: Slippage Investigation fields
    pub detection_spread: Option<f64>,  // Spread at detection time
    pub execution_spread: Option<f64>,  // Spread at execution time (recalculated)
    pub slippage_bps: Option<f64>,      // Slippage in basis points
    pub timing: Option<TimingBreakdown>, // Timing breakdown for latency analysis
    pub long_exchange: Option<String>,  // Exchange that received long order
    pub short_exchange: Option<String>, // Exchange that received short order
}

impl TradingEvent {
    /// Create a new event with the current timestamp
    pub fn new(event_type: TradingEventType) -> Self {
        Self {
            event_type,
            timestamp_ms: current_timestamp_ms(),
            pair: None,
            exchange: None,
            entry_spread: None,
            exit_spread: None,
            spread_threshold: None,
            latency_ms: None,
            order_id: None,
            direction: None,
            profit: None,
            slippage: None,
            polls: None,
            // Story 8.1 fields
            detection_spread: None,
            execution_spread: None,
            slippage_bps: None,
            timing: None,
            long_exchange: None,
            short_exchange: None,
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
        event.entry_spread = Some(entry_spread);
        event.spread_threshold = Some(spread_threshold);
        event.direction = Some(direction.to_string());
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
    ) -> Self {
        let mut event = Self::with_pair(TradingEventType::TradeEntry, pair);
        event.exchange = Some(format!("long:{},short:{}", long_exchange, short_exchange));
        event.entry_spread = Some(entry_spread);
        event.spread_threshold = Some(spread_threshold);
        event.latency_ms = Some(latency_ms);
        event.direction = Some(direction.to_string());
        event.long_exchange = Some(long_exchange.to_string());
        event.short_exchange = Some(short_exchange.to_string());
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
        event.entry_spread = Some(entry_spread);
        event.exit_spread = Some(exit_spread);
        event.spread_threshold = Some(spread_threshold);
        event.profit = Some(profit);
        event.polls = Some(polls);
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
        event.entry_spread = Some(entry_spread);
        event.exit_spread = Some(exit_spread);
        event.spread_threshold = Some(spread_threshold);
        event.polls = Some(polls);
        event
    }
    
    /// Create an ORDER_PLACED event
    pub fn order_placed(
        pair: &str,
        exchange: &str,
        order_id: &str,
        direction: &str,
    ) -> Self {
        let mut event = Self::with_pair(TradingEventType::OrderPlaced, pair);
        event.exchange = Some(exchange.to_string());
        event.order_id = Some(order_id.to_string());
        event.direction = Some(direction.to_string());
        event
    }
    
    /// Create an ORDER_FILLED event
    pub fn order_filled(
        pair: &str,
        exchange: &str,
        order_id: &str,
        latency_ms: u64,
    ) -> Self {
        let mut event = Self::with_pair(TradingEventType::OrderFilled, pair);
        event.exchange = Some(exchange.to_string());
        event.order_id = Some(order_id.to_string());
        event.latency_ms = Some(latency_ms);
        event
    }
    
    /// Create a POSITION_CLOSED event
    pub fn position_closed(
        pair: &str,
        entry_spread: f64,
        exit_spread: f64,
        profit: f64,
    ) -> Self {
        let mut event = Self::with_pair(TradingEventType::PositionClosed, pair);
        event.exchange = Some("both".to_string());
        event.entry_spread = Some(entry_spread);
        event.exit_spread = Some(exit_spread);
        event.profit = Some(profit);
        event
    }
    
    /// Create a BOT_STARTED event
    pub fn bot_started() -> Self {
        Self::new(TradingEventType::BotStarted)
    }
    
    /// Create a SLIPPAGE_ANALYSIS event (Story 8.1)
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
        // Calculate slippage in basis points (detection - execution) * 100
        let slippage_bps = (detection_spread - execution_spread) * 100.0;
        
        let mut event = Self::with_pair(TradingEventType::SlippageAnalysis, pair);
        event.exchange = Some("both".to_string());
        event.entry_spread = Some(detection_spread);
        event.exit_spread = Some(execution_spread);
        event.latency_ms = Some(timing.total_latency_ms);
        event.direction = Some(direction.to_string());
        event.slippage = Some(slippage_bps / 100.0);
        event.detection_spread = Some(detection_spread);
        event.execution_spread = Some(execution_spread);
        event.slippage_bps = Some(slippage_bps);
        event.timing = Some(timing);
        event.long_exchange = Some(long_exchange.to_string());
        event.short_exchange = Some(short_exchange.to_string());
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

/// Format price with 2 decimals and $ prefix
/// 
/// # Examples
/// - `100.5` → "$100.50"
/// - `-50.0` → "$-50.00"
/// - NaN/Infinity will produce "$NaN" or "$inf"
#[inline]
pub fn fmt_price(value: f64) -> String {
    format!("${:.2}", value)
}

/// Log a trading event using structured tracing fields
///
/// Events are logged at INFO level (trading events) or DEBUG level (monitoring ticks).
pub fn log_event(event: &TradingEvent) {
    let event_type = event.event_type.to_string();
    let timestamp = event.timestamp_ms;
    
    // Format spreads using helper
    let entry_spread_str = event.entry_spread.map(format_pct);
    let exit_spread_str = event.exit_spread.map(format_pct);
    let threshold_str = event.spread_threshold.map(format_pct);
    let profit_str = event.profit.map(format_pct);
    
    match event.event_type {
        // DEBUG level for high-frequency monitoring
        TradingEventType::PositionMonitoring => {
            debug!(
                event_type = %event_type,
                timestamp = timestamp,
                pair = ?event.pair,
                entry_spread = ?entry_spread_str,
                exit_spread = ?exit_spread_str,
                spread_threshold = ?threshold_str,
                polls = ?event.polls,
                ""
            );
        }
        // Story 8.1: SLIPPAGE_ANALYSIS with timing breakdown for easy grep
        TradingEventType::SlippageAnalysis => {
            let timing = event.timing.as_ref();
            info!(
                event_type = %event_type,
                timestamp = timestamp,
                pair = ?event.pair,
                detection_spread_pct = ?event.detection_spread,
                execution_spread_pct = ?event.execution_spread,
                slippage_bps = ?event.slippage_bps,
                detection_to_signal_ms = timing.map(|t| t.detection_to_signal_ms),
                signal_to_order_ms = timing.map(|t| t.signal_to_order_ms),
                order_to_confirm_ms = timing.map(|t| t.order_to_confirm_ms),
                total_latency_ms = timing.map(|t| t.total_latency_ms),
                long_exchange = ?event.long_exchange,
                short_exchange = ?event.short_exchange,
                direction = ?event.direction,
                "[SLIPPAGE] Trade analysis"
            );
        }
        // INFO level for all business events
        _ => {
            info!(
                event_type = %event_type,
                timestamp = timestamp,
                pair = ?event.pair,
                exchange = ?event.exchange,
                entry_spread = ?entry_spread_str,
                exit_spread = ?exit_spread_str,
                spread_threshold = ?threshold_str,
                latency_ms = ?event.latency_ms,
                order_id = ?event.order_id,
                direction = ?event.direction,
                profit = ?profit_str,
                slippage = ?event.slippage,
                polls = ?event.polls,
                ""
            );
        }
    }
}

/// Log a trading event at INFO level (for important events)
pub fn log_trading_event(event: &TradingEvent) {
    log_event(event);
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_event_type_display() {
        assert_eq!(TradingEventType::SpreadDetected.to_string(), "SPREAD_DETECTED");
        assert_eq!(TradingEventType::TradeEntry.to_string(), "TRADE_ENTRY");
        assert_eq!(TradingEventType::TradeExit.to_string(), "TRADE_EXIT");
        assert_eq!(TradingEventType::PositionMonitoring.to_string(), "POSITION_MONITORING");
    }
    
    #[test]
    fn test_spread_detected_event() {
        let event = TradingEvent::spread_detected("BTC-PERP", 0.35, 0.10, "A_OVER_B");
        
        assert_eq!(event.event_type, TradingEventType::SpreadDetected);
        assert_eq!(event.pair, Some("BTC-PERP".to_string()));
        assert_eq!(event.entry_spread, Some(0.35));
        assert_eq!(event.exit_spread, None); // Not set for entry events
        assert_eq!(event.spread_threshold, Some(0.10));
        assert_eq!(event.direction, Some("A_OVER_B".to_string()));
    }
    
    #[test]
    fn test_trade_entry_event() {
        let event = TradingEvent::trade_entry(
            "BTC-PERP",
            0.35,
            0.10,
            "A_OVER_B",
            "vest",
            "paradex",
            150,
        );
        
        assert_eq!(event.event_type, TradingEventType::TradeEntry);
        assert_eq!(event.entry_spread, Some(0.35));
        assert_eq!(event.latency_ms, Some(150));
        assert!(event.exchange.unwrap().contains("vest"));
    }
    
    #[test]
    fn test_trade_exit_event() {
        let event = TradingEvent::trade_exit(
            "BTC-PERP",
            0.35,    // entry_spread (original)
            -0.08,   // exit_spread (at close)
            -0.10,   // spread_threshold
            0.27,    // profit
            1200,    // polls
        );
        
        assert_eq!(event.event_type, TradingEventType::TradeExit);
        assert_eq!(event.entry_spread, Some(0.35));
        assert_eq!(event.exit_spread, Some(-0.08));
        assert_eq!(event.profit, Some(0.27));
        assert_eq!(event.polls, Some(1200));
    }
    
    #[test]
    fn test_position_monitoring_event() {
        let event = TradingEvent::position_monitoring(
            "BTC-PERP",
            0.35,    // entry_spread (original at entry)
            -0.05,   // exit_spread (current)
            -0.10,   // spread_threshold (exit target)
            40,
        );
        
        assert_eq!(event.event_type, TradingEventType::PositionMonitoring);
        assert_eq!(event.entry_spread, Some(0.35));
        assert_eq!(event.exit_spread, Some(-0.05));
        assert_eq!(event.polls, Some(40));
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
    
    // Story 8.1 Tests
    
    #[test]
    fn test_timing_breakdown_new() {
        // Simulate timestamps with known values
        let t_detection = 1000u64;
        let t_signal = 1005u64;      // 5ms after detection
        let t_order_sent = 1010u64;   // 5ms after signal
        let t_order_confirmed = 1100u64; // 90ms after order sent
        
        let timing = TimingBreakdown::new(
            t_detection,
            t_signal,
            t_order_sent,
            t_order_confirmed,
        );
        
        assert_eq!(timing.detection_timestamp_ms, 1000);
        assert_eq!(timing.signal_timestamp_ms, 1005);
        assert_eq!(timing.order_sent_timestamp_ms, 1010);
        assert_eq!(timing.order_confirmed_timestamp_ms, 1100);
        assert_eq!(timing.detection_to_signal_ms, 5);  // 1005 - 1000
        assert_eq!(timing.signal_to_order_ms, 5);      // 1010 - 1005
        assert_eq!(timing.order_to_confirm_ms, 90);    // 1100 - 1010
        assert_eq!(timing.total_latency_ms, 100);      // 1100 - 1000
    }
    
    #[test]
    fn test_slippage_analysis_event() {
        let timing = TimingBreakdown::new(1000, 1005, 1010, 1100);
        
        let event = TradingEvent::slippage_analysis(
            "BTC-PERP",
            0.35,    // detection_spread (detected at 0.35%)
            0.10,    // execution_spread (executed at 0.10%)
            timing,
            "vest",
            "paradex",
            "AOverB",
        );
        
        assert_eq!(event.event_type, TradingEventType::SlippageAnalysis);
        assert_eq!(event.pair, Some("BTC-PERP".to_string()));
        assert_eq!(event.detection_spread, Some(0.35));
        assert_eq!(event.execution_spread, Some(0.10));
        
        // Slippage = (0.35 - 0.10) * 100 = 25 basis points
        // Use approximate comparison for floating point
        let slippage_bps = event.slippage_bps.unwrap();
        assert!((slippage_bps - 25.0).abs() < 0.001, "Expected ~25.0 bps, got {}", slippage_bps);
        
        // Check timing was stored
        assert!(event.timing.is_some());
        let stored_timing = event.timing.unwrap();
        assert_eq!(stored_timing.total_latency_ms, 100);
        
        // Check exchanges
        assert_eq!(event.long_exchange, Some("vest".to_string()));
        assert_eq!(event.short_exchange, Some("paradex".to_string()));
    }
    
    #[test]
    fn test_slippage_analysis_event_type_display() {
        assert_eq!(TradingEventType::SlippageAnalysis.to_string(), "SLIPPAGE_ANALYSIS");
    }
}

