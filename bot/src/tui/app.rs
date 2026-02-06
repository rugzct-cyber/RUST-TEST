//! TUI Application State
//!
//! Shared state container for real-time display data.
//! Wrapped in Arc<Mutex<>> for safe sharing between tasks.

use std::collections::VecDeque;
use std::time::Instant;
use crate::core::spread::SpreadDirection;

/// Maximum number of log entries to keep in memory
pub const MAX_LOG_ENTRIES: usize = 100;

/// Maximum number of trade records to keep in history
pub const MAX_TRADE_HISTORY: usize = 10;

/// Single log entry for display
#[derive(Clone, Debug)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String,
    pub message: String,
}

/// Trade history record
#[derive(Clone, Debug)]
pub struct TradeRecord {
    pub direction: SpreadDirection,
    pub entry_spread: f64,
    pub exit_spread: f64,
    pub pnl_usd: f64,
    pub timestamp: String,
}

/// Central application state shared between TUI and bot tasks
#[derive(Debug)]
pub struct AppState {
    // Orderbooks live
    pub vest_best_bid: f64,
    pub vest_best_ask: f64,
    pub paradex_best_bid: f64,
    pub paradex_best_ask: f64,
    
    // Spread actuel (live from orderbooks)
    pub current_spread_pct: f64,
    pub spread_direction: Option<SpreadDirection>,
    pub live_entry_spread: f64,  // (BID_B - ASK_A) / ASK_A * 100
    pub live_exit_spread: f64,   // (BID_A - ASK_B) / ASK_B * 100
    
    // Position
    pub position_open: bool,
    pub entry_spread: Option<f64>,
    pub entry_direction: Option<SpreadDirection>,
    pub entry_vest_price: Option<f64>,
    pub entry_paradex_price: Option<f64>,
    pub position_polls: u64,
    
    // Config
    pub pair: String,
    pub spread_entry_threshold: f64,
    pub spread_exit_threshold: f64,
    pub position_size: f64,
    pub leverage: u32,
    
    // Stats
    pub trades_count: u32,
    pub total_profit_usd: f64,
    pub last_latency_ms: Option<u64>,
    pub uptime_start: Instant,
    
    // Logs (ring buffer)
    pub recent_logs: VecDeque<LogEntry>,
    
    // Trade history (ring buffer)
    pub trade_history: VecDeque<TradeRecord>,
    
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
            live_entry_spread: 0.0,
            live_exit_spread: 0.0,
            position_open: false,
            entry_spread: None,
            entry_direction: None,
            entry_vest_price: None,
            entry_paradex_price: None,
            position_polls: 0,
            pair,
            spread_entry_threshold: spread_entry,
            spread_exit_threshold: spread_exit,
            position_size,
            leverage,
            trades_count: 0,
            total_profit_usd: 0.0,
            last_latency_ms: None,
            uptime_start: Instant::now(),
            recent_logs: VecDeque::with_capacity(MAX_LOG_ENTRIES),
            trade_history: VecDeque::with_capacity(MAX_TRADE_HISTORY),
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
    
    /// Update live entry/exit spreads from current orderbook prices
    /// 
    /// Entry spread: Shows the best opportunity spread (max of both directions)
    /// Exit spread: When in position, shows spread for closing that specific position direction
    pub fn update_live_spreads(&mut self) {
        // Calculate spreads for both directions
        // AOverB: buy on Vest (at ask), sell on Paradex (at bid) => (paradex_bid - vest_ask) / vest_ask
        // BOverA: buy on Paradex (at ask), sell on Vest (at bid) => (vest_bid - paradex_ask) / paradex_ask
        
        let spread_a_over_b = if self.vest_best_ask > 0.0 {
            ((self.paradex_best_bid - self.vest_best_ask) / self.vest_best_ask) * 100.0
        } else {
            f64::NEG_INFINITY
        };
        
        let spread_b_over_a = if self.paradex_best_ask > 0.0 {
            ((self.vest_best_bid - self.paradex_best_ask) / self.paradex_best_ask) * 100.0
        } else {
            f64::NEG_INFINITY
        };
        
        // Entry spread: Show the best available opportunity (highest spread)
        self.live_entry_spread = spread_a_over_b.max(spread_b_over_a);
        
        // Exit spread: Based on position direction (inverse of entry)
        match self.entry_direction {
            Some(SpreadDirection::AOverB) => {
                // Entry was Long Vest / Short Paradex
                // Exit: Sell Vest (at bid), Buy Paradex (at ask)
                if self.paradex_best_ask > 0.0 {
                    self.live_exit_spread = ((self.vest_best_bid - self.paradex_best_ask) / self.paradex_best_ask) * 100.0;
                }
            }
            Some(SpreadDirection::BOverA) => {
                // Entry was Long Paradex / Short Vest
                // Exit: Sell Paradex (at bid), Buy Vest (at ask)
                if self.vest_best_ask > 0.0 {
                    self.live_exit_spread = ((self.paradex_best_bid - self.vest_best_ask) / self.vest_best_ask) * 100.0;
                }
            }
            None => {
                // No position, show the exit spread for best direction
                // (opposite of entry spread direction)
                if spread_a_over_b >= spread_b_over_a && self.paradex_best_ask > 0.0 {
                    self.live_exit_spread = ((self.vest_best_bid - self.paradex_best_ask) / self.paradex_best_ask) * 100.0;
                } else if self.vest_best_ask > 0.0 {
                    self.live_exit_spread = ((self.paradex_best_bid - self.vest_best_ask) / self.vest_best_ask) * 100.0;
                }
            }
        }
    }
    
    /// Record trade entry with prices
    pub fn record_entry(&mut self, spread: f64, direction: SpreadDirection, vest_price: f64, paradex_price: f64) {
        self.position_open = true;
        self.entry_spread = Some(spread);
        self.entry_direction = Some(direction);
        self.entry_vest_price = Some(vest_price);
        self.entry_paradex_price = Some(paradex_price);
        self.position_polls = 0;
    }
    
    /// Record trade exit
    pub fn record_exit(&mut self, exit_spread: f64, pnl_usd: f64, latency_ms: u64) {
        // Save trade to history BEFORE resetting position fields
        if let (Some(direction), Some(entry_spread)) = (self.entry_direction.clone(), self.entry_spread) {
            let record = TradeRecord {
                direction,
                entry_spread,
                exit_spread,
                pnl_usd,
                timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
            };
            
            // Ring buffer: remove oldest if at capacity
            if self.trade_history.len() >= MAX_TRADE_HISTORY {
                self.trade_history.pop_front();
            }
            self.trade_history.push_back(record);
        }
        
        self.position_open = false;
        self.entry_spread = None;
        self.entry_direction = None;
        self.entry_vest_price = None;
        self.entry_paradex_price = None;
        self.trades_count += 1;
        self.total_profit_usd += pnl_usd;
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
        
        state.record_entry(0.12, SpreadDirection::AOverB, 97000.0, 97100.0);
        assert!(state.position_open);
        assert_eq!(state.entry_spread, Some(0.12));
        assert_eq!(state.entry_vest_price, Some(97000.0));
        assert_eq!(state.entry_paradex_price, Some(97100.0));
        
        state.record_exit(-0.04, 0.08, 45);  // exit_spread, profit_pct, latency_ms
        assert!(!state.position_open);
        assert_eq!(state.trades_count, 1);
        assert_eq!(state.total_profit_usd, 0.08);
        assert_eq!(state.last_latency_ms, Some(45));
        assert_eq!(state.entry_vest_price, None);
        assert_eq!(state.entry_paradex_price, None);
        
        // Verify trade was added to history
        assert_eq!(state.trade_history.len(), 1);
        let trade = state.trade_history.front().unwrap();
        assert_eq!(trade.entry_spread, 0.12);
        assert_eq!(trade.exit_spread, -0.04);
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
    
    #[test]
    fn test_trade_history_ring_buffer() {
        let mut state = AppState::new("BTC".into(), 0.1, 0.05, 0.001, 10);
        
        // Simulate 11 trades to test ring buffer behavior (AC2)
        for i in 0..11 {
            let spread = 0.10 + (i as f64 * 0.01);  // Varying entry spreads
            state.record_entry(spread, SpreadDirection::AOverB, 97000.0, 97100.0);
            state.record_exit(-0.02, 0.05, 30);  // exit_spread, pnl_usd, latency
        }
        
        // Should be capped at MAX_TRADE_HISTORY (10)
        assert_eq!(state.trade_history.len(), MAX_TRADE_HISTORY);
        
        // Oldest trade (i=0, spread 0.10) should have been rotated out
        // First trade should now be i=1, spread 0.11
        let first_trade = state.trade_history.front().unwrap();
        assert!((first_trade.entry_spread - 0.11).abs() < 0.001);
        
        // Most recent trade should be i=10, spread 0.20
        let last_trade = state.trade_history.back().unwrap();
        assert!((last_trade.entry_spread - 0.20).abs() < 0.001);
    }
}
