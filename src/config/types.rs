//! Configuration types for bot settings
//!
//! This module defines all configuration structs that are loaded from YAML
//! and shared across the application via `Arc<RwLock<AppConfig>>`.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::error::AppError;

// ============================================================================
// Type Aliases
// ============================================================================

/// Type alias for shared configuration access across async tasks
pub type SharedConfig = Arc<RwLock<AppConfig>>;

// ============================================================================
// Enums
// ============================================================================

/// Supported trading pairs
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum TradingPair {
    #[serde(rename = "BTC-PERP")]
    BtcPerp,
    #[serde(rename = "ETH-PERP")]
    EthPerp,
    #[serde(rename = "SOL-PERP")]
    SolPerp,
}

impl std::fmt::Display for TradingPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TradingPair::BtcPerp => write!(f, "BTC-PERP"),
            TradingPair::EthPerp => write!(f, "ETH-PERP"),
            TradingPair::SolPerp => write!(f, "SOL-PERP"),
        }
    }
}

/// Supported DEX exchanges
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Dex {
    Vest,
    Paradex,
    Hyperliquid,
    Lighter,
}

impl std::fmt::Display for Dex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Dex::Vest => write!(f, "vest"),
            Dex::Paradex => write!(f, "paradex"),
            Dex::Hyperliquid => write!(f, "hyperliquid"),
            Dex::Lighter => write!(f, "lighter"),
        }
    }
}

// ============================================================================
// Configuration Structs
// ============================================================================

/// Single bot configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotConfig {
    /// Unique identifier for the bot (e.g., "btc_vest_paradex")
    pub id: String,
    /// Trading pair (e.g., BTC-PERP)
    pub pair: TradingPair,
    /// First DEX for arbitrage
    pub dex_a: Dex,
    /// Second DEX for arbitrage
    pub dex_b: Dex,
    /// Spread threshold to enter position (percentage, e.g., 0.30 = 0.30%)
    pub spread_entry: f64,
    /// Spread threshold to exit position (percentage, e.g., 0.05 = 0.05%)
    pub spread_exit: f64,
    /// Leverage multiplier (1-100)
    pub leverage: u8,
    /// Position size in base asset (e.g., 0.001 BTC)
    pub position_size: f64,
}

impl BotConfig {
    /// Validate bot configuration rules
    pub fn validate(&self) -> Result<(), AppError> {
        // Rule: bot ID cannot be empty
        if self.id.trim().is_empty() {
            return Err(AppError::Config(
                "Bot ID cannot be empty".to_string()
            ));
        }

        // Rule: spread values must be in valid range (0% to 100%)
        if self.spread_entry <= 0.0 || self.spread_entry >= 100.0 {
            return Err(AppError::Config(format!(
                "Bot '{}': spread_entry must be > 0 and < 100% (got {})",
                self.id, self.spread_entry
            )));
        }

        // Rule: spread_exit can be negative (profit when spread inverts) but must be < 100%
        // Negative exit spread = close when spread reverses (profit-taking)
        // Example: entry=0.09%, exit=-0.05% = close when spread inverts by -0.05%
        if self.spread_exit >= 100.0 || self.spread_exit <= -100.0 {
            return Err(AppError::Config(format!(
                "Bot '{}': spread_exit must be between -100% and 100% (got {})",
                self.id, self.spread_exit
            )));
        }

        // Rule: spread_entry > spread_exit (entry threshold must be higher)
        // This ensures we enter at a higher spread than we exit
        if self.spread_entry <= self.spread_exit {
            return Err(AppError::Config(format!(
                "Bot '{}': spread_entry ({}) must be > spread_exit ({})",
                self.id, self.spread_entry, self.spread_exit
            )));
        }

        // Rule: dex_a ≠ dex_b
        if self.dex_a == self.dex_b {
            return Err(AppError::Config(format!(
                "Bot '{}': dex_a and dex_b cannot be the same (both are {})",
                self.id, self.dex_a
            )));
        }

        // Rule: leverage in range 1-100
        if self.leverage < 1 || self.leverage > 100 {
            return Err(AppError::Config(format!(
                "Bot '{}': leverage must be 1-100, got {}",
                self.id, self.leverage
            )));
        }

        // Rule: position_size > 0
        if self.position_size <= 0.0 {
            return Err(AppError::Config(format!(
                "Bot '{}': position_size must be > 0, got {}",
                self.id, self.position_size
            )));
        }

        Ok(())
    }
}

/// Risk management parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskConfig {
    /// ADL warning threshold (percentage distance from liquidation)
    pub adl_warning: f64,
    /// ADL critical threshold - triggers position reduction
    pub adl_critical: f64,
    /// Maximum position duration in hours before force exit
    pub max_duration_hours: u32,
}

/// API server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    /// HTTP/WebSocket server port
    pub port: u16,
    /// WebSocket heartbeat interval in seconds
    pub ws_heartbeat_sec: u32,
}

/// Root application configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    /// List of bot configurations
    pub bots: Vec<BotConfig>,
    /// Risk management settings
    pub risk: RiskConfig,
    /// API server settings
    pub api: ApiConfig,
}

impl AppConfig {
    /// Validate all configuration rules
    pub fn validate(&self) -> Result<(), AppError> {
        // Rule: At least one bot must be configured
        if self.bots.is_empty() {
            return Err(AppError::Config(
                "Configuration must contain at least one bot".to_string()
            ));
        }

        // Validate each bot configuration
        for bot in &self.bots {
            bot.validate()?;
        }

        // Validate risk config
        if self.risk.adl_warning <= self.risk.adl_critical {
            return Err(AppError::Config(format!(
                "Risk config: adl_warning ({}) must be > adl_critical ({})",
                self.risk.adl_warning, self.risk.adl_critical
            )));
        }

        Ok(())
    }

    /// Convert to shared state wrapper for async access
    pub fn into_shared(self) -> SharedConfig {
        Arc::new(RwLock::new(self))
    }
}

impl Default for RiskConfig {
    fn default() -> Self {
        Self {
            adl_warning: 10.0,
            adl_critical: 5.0,
            max_duration_hours: 24,
        }
    }
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            port: 8080,
            ws_heartbeat_sec: 30,
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn create_valid_bot_config() -> BotConfig {
        BotConfig {
            id: "test_bot".to_string(),
            pair: TradingPair::BtcPerp,
            dex_a: Dex::Vest,
            dex_b: Dex::Paradex,
            spread_entry: 0.30,
            spread_exit: 0.05,
            leverage: 10,
            position_size: 0.001,
        }
    }

    #[test]
    fn test_valid_bot_config() {
        let bot = create_valid_bot_config();
        assert!(bot.validate().is_ok());
    }

    #[test]
    fn test_spread_entry_must_be_greater_than_exit() {
        let mut bot = create_valid_bot_config();
        bot.spread_entry = 0.05;
        bot.spread_exit = 0.30;
        
        let result = bot.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("spread_entry"));
    }

    #[test]
    fn test_spread_entry_equal_exit_fails() {
        let mut bot = create_valid_bot_config();
        bot.spread_entry = 0.10;
        bot.spread_exit = 0.10;
        
        let result = bot.validate();
        assert!(result.is_err());
    }

    #[test]
    fn test_same_dex_fails() {
        let mut bot = create_valid_bot_config();
        bot.dex_a = Dex::Vest;
        bot.dex_b = Dex::Vest;
        
        let result = bot.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("dex_a and dex_b cannot be the same"));
    }

    #[test]
    fn test_leverage_too_low_fails() {
        let mut bot = create_valid_bot_config();
        bot.leverage = 0;
        
        let result = bot.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("leverage must be 1-100"));
    }

    #[test]
    fn test_leverage_too_high_fails() {
        let mut bot = create_valid_bot_config();
        bot.leverage = 101;
        
        let result = bot.validate();
        assert!(result.is_err());
    }

    #[test]
    fn test_position_size_zero_fails() {
        let mut bot = create_valid_bot_config();
        bot.position_size = 0.0;
        
        let result = bot.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("position_size must be > 0"));
    }

    #[test]
    fn test_position_size_negative_fails() {
        let mut bot = create_valid_bot_config();
        bot.position_size = -0.001;
        
        let result = bot.validate();
        assert!(result.is_err());
    }

    #[test]
    fn test_valid_config_deserialize() {
        let yaml = r#"
bots:
  - id: test_bot
    pair: BTC-PERP
    dex_a: vest
    dex_b: paradex
    spread_entry: 0.30
    spread_exit: 0.05
    leverage: 10
    position_size: 0.001
risk:
  adl_warning: 10.0
  adl_critical: 5.0
  max_duration_hours: 24
api:
  port: 8080
  ws_heartbeat_sec: 30
"#;
        let config: AppConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.validate().is_ok());
        assert_eq!(config.bots.len(), 1);
        assert_eq!(config.bots[0].id, "test_bot");
        assert_eq!(config.api.port, 8080);
    }

    #[test]
    fn test_trading_pair_serde() {
        let yaml = "\"BTC-PERP\"";
        let pair: TradingPair = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(pair, TradingPair::BtcPerp);
    }

    #[test]
    fn test_dex_serde() {
        let yaml = "\"vest\"";
        let dex: Dex = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(dex, Dex::Vest);
    }

    #[test]
    fn test_empty_bot_id_fails() {
        let mut bot = create_valid_bot_config();
        bot.id = "".to_string();
        
        let result = bot.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Bot ID cannot be empty"));
    }

    #[test]
    fn test_whitespace_only_bot_id_fails() {
        let mut bot = create_valid_bot_config();
        bot.id = "   ".to_string();
        
        let result = bot.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Bot ID cannot be empty"));
    }

    #[test]
    fn test_negative_spread_entry_fails() {
        let mut bot = create_valid_bot_config();
        bot.spread_entry = -0.30;
        bot.spread_exit = -0.50;  // Still less than entry, but both negative
        
        let result = bot.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("spread_entry must be > 0 and < 100%"));
    }

    #[test]
    fn test_negative_spread_exit_is_valid() {
        // Negative spread_exit is now VALID - allows profit-taking when spread inverts
        let mut bot = create_valid_bot_config();
        bot.spread_entry = 0.30;
        bot.spread_exit = -0.05;  // Exit when spread >= -0.05% (profit-taking)
        
        let result = bot.validate();
        assert!(result.is_ok(), "Negative spread_exit should be valid for profit-taking");
    }

    #[test]
    fn test_empty_bots_array_fails() {
        let config = AppConfig {
            bots: vec![],
            risk: RiskConfig {
                adl_warning: 10.0,
                adl_critical: 5.0,
                max_duration_hours: 24,
            },
            api: ApiConfig {
                port: 8080,
                ws_heartbeat_sec: 30,
            },
        };
        
        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("at least one bot"));
    }

    #[test]
    fn test_into_shared() {
        let config = AppConfig {
            bots: vec![],
            risk: RiskConfig {
                adl_warning: 10.0,
                adl_critical: 5.0,
                max_duration_hours: 24,
            },
            api: ApiConfig {
                port: 8080,
                ws_heartbeat_sec: 30,
            },
        };
        
        let shared = config.into_shared();
        // Verify it compiles and creates Arc<RwLock<AppConfig>>
        assert!(Arc::strong_count(&shared) == 1);
    }

    #[test]
    fn test_spread_entry_zero_fails() {
        let mut bot = create_valid_bot_config();
        bot.spread_entry = 0.0;
        
        let result = bot.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("spread_entry must be > 0 and < 100%"));
    }

    #[test]
    fn test_spread_entry_above_100_fails() {
        let mut bot = create_valid_bot_config();
        bot.spread_entry = 100.5;
        bot.spread_exit = 0.05;  // Valid exit
        
        let result = bot.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("spread_entry must be > 0 and < 100%"));
    }

    #[test]
    fn test_spread_exit_above_100_fails() {
        let mut bot = create_valid_bot_config();
        bot.spread_entry = 0.30;  // Valid entry
        bot.spread_exit = 150.0;
        
        let result = bot.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("spread_exit must be between -100% and 100%"));
    }

    #[test]
    fn test_spread_thresholds_at_boundaries() {
        let mut bot = create_valid_bot_config();
        bot.spread_entry = 99.99;  // Just below 100%
        bot.spread_exit = 0.01;    // Just above 0%
        
        let result = bot.validate();
        assert!(result.is_ok(), "Valid boundary values should pass validation");
    }
}
