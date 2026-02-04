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
        }
    }
    
    /// Create a SPREAD_DETECTED event
    pub fn spread_detected(
        pair: &str,
        entry_spread: f64,
        spread_threshold: f64,
        direction: &str,
    ) -> Self {
        Self {
            event_type: TradingEventType::SpreadDetected,
            timestamp_ms: current_timestamp_ms(),
            pair: Some(pair.to_string()),
            exchange: Some("both".to_string()),
            entry_spread: Some(entry_spread),
            exit_spread: None,
            spread_threshold: Some(spread_threshold),
            latency_ms: None,
            order_id: None,
            direction: Some(direction.to_string()),
            profit: None,
            slippage: None,
            polls: None,
        }
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
        Self {
            event_type: TradingEventType::TradeEntry,
            timestamp_ms: current_timestamp_ms(),
            pair: Some(pair.to_string()),
            exchange: Some(format!("long:{},short:{}", long_exchange, short_exchange)),
            entry_spread: Some(entry_spread),
            exit_spread: None,
            spread_threshold: Some(spread_threshold),
            latency_ms: Some(latency_ms),
            order_id: None,
            direction: Some(direction.to_string()),
            profit: None,
            slippage: None,
            polls: None,
        }
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
        Self {
            event_type: TradingEventType::TradeExit,
            timestamp_ms: current_timestamp_ms(),
            pair: Some(pair.to_string()),
            exchange: Some("both".to_string()),
            entry_spread: Some(entry_spread),
            exit_spread: Some(exit_spread),
            spread_threshold: Some(spread_threshold),
            latency_ms: None,
            order_id: None,
            direction: None,
            profit: Some(profit),
            slippage: None,
            polls: Some(polls),
        }
    }
    
    /// Create a POSITION_MONITORING event (throttled logging)
    pub fn position_monitoring(
        pair: &str,
        entry_spread: f64,
        exit_spread: f64,
        spread_threshold: f64,
        polls: u64,
    ) -> Self {
        Self {
            event_type: TradingEventType::PositionMonitoring,
            timestamp_ms: current_timestamp_ms(),
            pair: Some(pair.to_string()),
            exchange: None,
            entry_spread: Some(entry_spread),
            exit_spread: Some(exit_spread),
            spread_threshold: Some(spread_threshold),
            latency_ms: None,
            order_id: None,
            direction: None,
            profit: None,
            slippage: None,
            polls: Some(polls),
        }
    }
    
    /// Create an ORDER_PLACED event
    pub fn order_placed(
        pair: &str,
        exchange: &str,
        order_id: &str,
        direction: &str,
    ) -> Self {
        Self {
            event_type: TradingEventType::OrderPlaced,
            timestamp_ms: current_timestamp_ms(),
            pair: Some(pair.to_string()),
            exchange: Some(exchange.to_string()),
            entry_spread: None,
            exit_spread: None,
            spread_threshold: None,
            latency_ms: None,
            order_id: Some(order_id.to_string()),
            direction: Some(direction.to_string()),
            profit: None,
            slippage: None,
            polls: None,
        }
    }
    
    /// Create an ORDER_FILLED event
    pub fn order_filled(
        pair: &str,
        exchange: &str,
        order_id: &str,
        latency_ms: u64,
    ) -> Self {
        Self {
            event_type: TradingEventType::OrderFilled,
            timestamp_ms: current_timestamp_ms(),
            pair: Some(pair.to_string()),
            exchange: Some(exchange.to_string()),
            entry_spread: None,
            exit_spread: None,
            spread_threshold: None,
            latency_ms: Some(latency_ms),
            order_id: Some(order_id.to_string()),
            direction: None,
            profit: None,
            slippage: None,
            polls: None,
        }
    }
    
    /// Create a POSITION_CLOSED event
    pub fn position_closed(
        pair: &str,
        entry_spread: f64,
        exit_spread: f64,
        profit: f64,
    ) -> Self {
        Self {
            event_type: TradingEventType::PositionClosed,
            timestamp_ms: current_timestamp_ms(),
            pair: Some(pair.to_string()),
            exchange: Some("both".to_string()),
            entry_spread: Some(entry_spread),
            exit_spread: Some(exit_spread),
            spread_threshold: None,
            latency_ms: None,
            order_id: None,
            direction: None,
            profit: Some(profit),
            slippage: None,
            polls: None,
        }
    }
    
    /// Create a BOT_STARTED event
    pub fn bot_started() -> Self {
        Self::new(TradingEventType::BotStarted)
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

/// Log a trading event using structured tracing fields
///
/// Events are logged at INFO level (trading events) or DEBUG level (monitoring ticks).
pub fn log_event(event: &TradingEvent) {
    let event_type = event.event_type.to_string();
    let timestamp = event.timestamp_ms;
    
    // Format spreads with 4 decimal places
    let entry_spread_str = event.entry_spread.map(|s| format!("{:.4}%", s));
    let exit_spread_str = event.exit_spread.map(|s| format!("{:.4}%", s));
    let threshold_str = event.spread_threshold.map(|s| format!("{:.4}%", s));
    let profit_str = event.profit.map(|p| format!("{:.4}%", p));
    
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
}
