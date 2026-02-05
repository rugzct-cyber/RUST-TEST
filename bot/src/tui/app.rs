//! TUI Application State
//!
//! Shared state container for real-time display data.
//! Wrapped in Arc<Mutex<>> for safe sharing between tasks.

use std::collections::VecDeque;
use std::time::Instant;
use crate::core::spread::SpreadDirection;

/// Maximum number of log entries to keep in memory
pub const MAX_LOG_ENTRIES: usize = 100;

/// Single log entry for display
#[derive(Clone, Debug)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String,
    pub message: String,
}

/// Central application state shared between TUI and bot tasks
#[derive(Debug)]
pub struct AppState {
    // Orderbooks live
    pub vest_best_bid: f64,
    pub vest_best_ask: f64,
    pub paradex_best_bid: f64,
    pub paradex_best_ask: f64,
    
    // Spread actuel
    pub current_spread_pct: f64,
    pub spread_direction: Option<SpreadDirection>,
    
    // Position
    pub position_open: bool,
    pub entry_spread: Option<f64>,
    pub entry_direction: Option<SpreadDirection>,
    pub position_polls: u64,
    
    // Config
    pub pair: String,
    pub spread_entry_threshold: f64,
    pub spread_exit_threshold: f64,
    pub position_size: f64,
    pub leverage: u32,
    
    // Stats
    pub trades_count: u32,
    pub total_profit_pct: f64,
    pub last_latency_ms: Option<u64>,
    pub uptime_start: Instant,
    
    // Logs (ring buffer)
    pub recent_logs: VecDeque<LogEntry>,
    
    // Control
    pub should_quit: bool,
    pub log_scroll_offset: usize,
    pub show_debug_logs: bool,
}

impl AppState {
    /// Create new AppState with config values
    pub fn new(
        pair: String,
        spread_entry: f64,
        spread_exit: f64,
        position_size: f64,
        leverage: u32,
    ) -> Self {
        Self {
            vest_best_bid: 0.0,
            vest_best_ask: 0.0,
            paradex_best_bid: 0.0,
            paradex_best_ask: 0.0,
            current_spread_pct: 0.0,
            spread_direction: None,
            position_open: false,
            entry_spread: None,
            entry_direction: None,
            position_polls: 0,
            pair,
            spread_entry_threshold: spread_entry,
            spread_exit_threshold: spread_exit,
            position_size,
            leverage,
            trades_count: 0,
            total_profit_pct: 0.0,
            last_latency_ms: None,
            uptime_start: Instant::now(),
            recent_logs: VecDeque::with_capacity(MAX_LOG_ENTRIES),
            should_quit: false,
            log_scroll_offset: 0,
            show_debug_logs: false,
        }
    }
    
    /// Add a log entry with automatic rotation
    pub fn push_log(&mut self, entry: LogEntry) {
        if self.recent_logs.len() >= MAX_LOG_ENTRIES {
            self.recent_logs.pop_front();
        }
        self.recent_logs.push_back(entry);
    }
    
    /// Get formatted uptime string
    pub fn uptime_str(&self) -> String {
        let elapsed = self.uptime_start.elapsed();
        let hours = elapsed.as_secs() / 3600;
        let minutes = (elapsed.as_secs() % 3600) / 60;
        format!("{}h{:02}m", hours, minutes)
    }
    
    /// Update orderbook prices
    pub fn update_prices(
        &mut self,
        vest_bid: f64,
        vest_ask: f64,
        paradex_bid: f64,
        paradex_ask: f64,
    ) {
        self.vest_best_bid = vest_bid;
        self.vest_best_ask = vest_ask;
        self.paradex_best_bid = paradex_bid;
        self.paradex_best_ask = paradex_ask;
    }
    
    /// Update spread info
    pub fn update_spread(&mut self, spread_pct: f64, direction: Option<SpreadDirection>) {
        self.current_spread_pct = spread_pct;
        self.spread_direction = direction;
    }
    
    /// Record trade entry
    pub fn record_entry(&mut self, spread: f64, direction: SpreadDirection) {
        self.position_open = true;
        self.entry_spread = Some(spread);
        self.entry_direction = Some(direction);
        self.position_polls = 0;
    }
    
    /// Record trade exit
    pub fn record_exit(&mut self, profit_pct: f64, latency_ms: u64) {
        self.position_open = false;
        self.entry_spread = None;
        self.entry_direction = None;
        self.trades_count += 1;
        self.total_profit_pct += profit_pct;
        self.last_latency_ms = Some(latency_ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_app_state_creation() {
        let state = AppState::new(
            "BTC-PERP".to_string(),
            0.15,
            0.05,
            0.001,
            10,
        );
        assert_eq!(state.pair, "BTC-PERP");
        assert_eq!(state.spread_entry_threshold, 0.15);
        assert!(!state.position_open);
        assert!(state.recent_logs.is_empty());
    }
    
    #[test]
    fn test_log_rotation() {
        let mut state = AppState::new("BTC".into(), 0.1, 0.05, 0.001, 10);
        
        // Fill beyond capacity
        for i in 0..150 {
            state.push_log(LogEntry {
                timestamp: format!("12:00:{:02}", i % 60),
                level: "INFO".to_string(),
                message: format!("Log {}", i),
            });
        }
        
        // Should be capped at MAX_LOG_ENTRIES
        assert_eq!(state.recent_logs.len(), MAX_LOG_ENTRIES);
        
        // First entry should be from i=50 (after rotation)
        assert!(state.recent_logs.front().unwrap().message.contains("50"));
    }
    
    #[test]
    fn test_trade_recording() {
        let mut state = AppState::new("BTC".into(), 0.1, 0.05, 0.001, 10);
        
        state.record_entry(0.12, SpreadDirection::AOverB);
        assert!(state.position_open);
        assert_eq!(state.entry_spread, Some(0.12));
        
        state.record_exit(0.08, 45);
        assert!(!state.position_open);
        assert_eq!(state.trades_count, 1);
        assert_eq!(state.total_profit_pct, 0.08);
        assert_eq!(state.last_latency_ms, Some(45));
    }
    
    #[test]
    fn test_uptime_str_format() {
        let state = AppState::new("BTC".into(), 0.1, 0.05, 0.001, 10);
        let uptime = state.uptime_str();
        // Format should be "Xh00m" or similar
        assert!(uptime.contains('h'));
        assert!(uptime.contains('m'));
    }
    
    #[test]
    fn test_update_prices() {
        let mut state = AppState::new("BTC".into(), 0.1, 0.05, 0.001, 10);
        state.update_prices(97000.0, 97010.0, 97100.0, 97110.0);
        
        assert_eq!(state.vest_best_bid, 97000.0);
        assert_eq!(state.vest_best_ask, 97010.0);
        assert_eq!(state.paradex_best_bid, 97100.0);
        assert_eq!(state.paradex_best_ask, 97110.0);
    }
}
