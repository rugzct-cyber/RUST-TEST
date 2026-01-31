//! Simplified application state for MVP
//!
//! Minimal state management without complex dependencies.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::config::{AppConfig, BotConfig};

/// Type alias for shared application state access across async tasks
pub type SharedAppState = Arc<RwLock<AppState>>;

/// Possible states for a bot instance
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BotStatus {
    #[default]
    Stopped,
    Starting,
    Trading,
    Recovering,
    Error,
}

/// State of a single bot instance
#[derive(Debug, Clone)]
pub struct BotState {
    pub config: BotConfig,
    pub status: BotStatus,
    pub last_update_ms: u64,
}

impl BotState {
    pub fn new(config: BotConfig) -> Self {
        Self {
            config,
            status: BotStatus::Stopped,
            last_update_ms: current_time_ms(),
        }
    }
}

/// Application-wide metrics
#[derive(Debug, Clone, Default)]
pub struct Metrics {
    pub total_trades: u64,
    pub total_pnl: f64,
    pub active_positions: usize,
    pub uptime_seconds: u64,
}

/// Root application state
#[derive(Debug, Clone)]
pub struct AppState {
    pub bots: HashMap<String, BotState>,
    pub metrics: Metrics,
    pub config: AppConfig,
    pub start_time_ms: u64,
}

impl AppState {
    pub fn new(config: AppConfig) -> Self {
        let mut bots = HashMap::new();
        
        for bot_config in &config.bots {
            let bot_state = BotState::new(bot_config.clone());
            bots.insert(bot_config.id.clone(), bot_state);
        }
        
        Self {
            bots,
            metrics: Metrics::default(),
            config,
            start_time_ms: current_time_ms(),
        }
    }
    
    pub fn into_shared(self) -> SharedAppState {
        Arc::new(RwLock::new(self))
    }
    
    pub fn get_bot(&self, id: &str) -> Option<&BotState> {
        self.bots.get(id)
    }
    
    pub fn update_bot_status(&mut self, id: &str, status: BotStatus) {
        if let Some(bot) = self.bots.get_mut(id) {
            bot.status = status;
            bot.last_update_ms = current_time_ms();
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            bots: HashMap::new(),
            metrics: Metrics::default(),
            config: AppConfig::default(),
            start_time_ms: current_time_ms(),
        }
    }
}

fn current_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_state_default() {
        let state = AppState::default();
        assert!(state.bots.is_empty());
        assert_eq!(state.metrics.total_trades, 0);
    }

    #[test]
    fn test_bot_status_default() {
        assert_eq!(BotStatus::default(), BotStatus::Stopped);
    }

    #[tokio::test]
    async fn test_shared_state() {
        let state = AppState::default();
        let shared = state.into_shared();
        
        {
            let guard = shared.read().await;
            assert!(guard.bots.is_empty());
        }
        
        {
            let mut guard = shared.write().await;
            guard.metrics.total_trades = 5;
        }
        
        {
            let guard = shared.read().await;
            assert_eq!(guard.metrics.total_trades, 5);
        }
    }
}
