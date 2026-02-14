//! Configuration types for spread dashboard settings
//!
//! This module defines all configuration structs that are loaded from YAML
//! and shared across the application via `Arc<RwLock<AppConfig>>`.

use serde::{Deserialize, Serialize};

use crate::error::AppError;

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

impl TradingPair {
    /// Returns the base asset symbol (e.g. "BTC", "ETH", "SOL")
    /// for use with `resolve_symbol()`.
    pub fn base(&self) -> &'static str {
        match self {
            TradingPair::BtcPerp => "BTC",
            TradingPair::EthPerp => "ETH",
            TradingPair::SolPerp => "SOL",
        }
    }
}

/// Supported DEX exchanges
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Dex {
    Vest,
    Paradex,
    Lighter,
}

impl std::fmt::Display for Dex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Dex::Vest => write!(f, "vest"),
            Dex::Paradex => write!(f, "paradex"),
            Dex::Lighter => write!(f, "lighter"),
        }
    }
}

// ============================================================================
// Configuration Structs
// ============================================================================

/// Single dashboard monitor configuration (one pair × two exchanges)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardConfig {
    /// Unique identifier for this monitor (e.g., "btc_vest_paradex")
    pub id: String,
    /// Trading pair (e.g., BTC-PERP)
    pub pair: TradingPair,
    /// First DEX to monitor
    pub dex_a: Dex,
    /// Second DEX to monitor
    pub dex_b: Dex,
    /// Spread alert threshold (percentage, e.g., 0.30 = 0.30%)
    pub spread_entry: f64,
}

impl DashboardConfig {
    /// Validate dashboard configuration rules
    pub fn validate(&self) -> Result<(), AppError> {
        // Rule: ID cannot be empty
        if self.id.trim().is_empty() {
            return Err(AppError::Config("Monitor ID cannot be empty".to_string()));
        }

        // Rule: spread_entry must be in valid range (0% to 100%)
        if self.spread_entry <= 0.0 || self.spread_entry >= 100.0 {
            return Err(AppError::Config(format!(
                "Monitor '{}': spread_entry must be > 0 and < 100% (got {})",
                self.id, self.spread_entry
            )));
        }

        // Rule: dex_a ≠ dex_b
        if self.dex_a == self.dex_b {
            return Err(AppError::Config(format!(
                "Monitor '{}': dex_a and dex_b cannot be the same (both are {})",
                self.id, self.dex_a
            )));
        }

        // Rule: no NaN or Infinity in numeric fields
        if !self.spread_entry.is_finite() {
            return Err(AppError::Config(format!(
                "Monitor '{}': spread_entry must be a finite number (got {})",
                self.id, self.spread_entry
            )));
        }

        Ok(())
    }
}

/// Root application configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    /// List of dashboard monitor configurations
    #[serde(alias = "monitors")]
    pub bots: Vec<DashboardConfig>,
}

impl AppConfig {
    /// Validate all configuration rules
    pub fn validate(&self) -> Result<(), AppError> {
        // Rule: At least one monitor must be configured
        if self.bots.is_empty() {
            return Err(AppError::Config(
                "Configuration must contain at least one monitor".to_string(),
            ));
        }

        // Rule: No duplicate monitor IDs
        let mut seen_ids = std::collections::HashSet::new();
        for bot in &self.bots {
            if !seen_ids.insert(&bot.id) {
                return Err(AppError::Config(format!(
                    "Duplicate monitor ID: '{}'",
                    bot.id
                )));
            }
        }

        // Validate each monitor configuration
        for bot in &self.bots {
            bot.validate()?;
        }

        Ok(())
    }

}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn create_valid_config() -> DashboardConfig {
        DashboardConfig {
            id: "test_monitor".to_string(),
            pair: TradingPair::BtcPerp,
            dex_a: Dex::Vest,
            dex_b: Dex::Paradex,
            spread_entry: 0.30,
        }
    }

    #[test]
    fn test_valid_config() {
        let cfg = create_valid_config();
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_same_dex_fails() {
        let mut cfg = create_valid_config();
        cfg.dex_a = Dex::Vest;
        cfg.dex_b = Dex::Vest;
        let result = cfg.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("dex_a and dex_b cannot be the same"));
    }

    #[test]
    fn test_valid_config_deserialize() {
        let yaml = r#"
monitors:
  - id: test_monitor
    pair: BTC-PERP
    dex_a: vest
    dex_b: paradex
    spread_entry: 0.30
"#;
        let config: AppConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.validate().is_ok());
        assert_eq!(config.bots.len(), 1);
        assert_eq!(config.bots[0].id, "test_monitor");
    }

    #[test]
    fn test_bots_alias_deserialize() {
        // Backward compat: "bots" key still works
        let yaml = r#"
bots:
  - id: test_bot
    pair: BTC-PERP
    dex_a: vest
    dex_b: paradex
    spread_entry: 0.30
"#;
        let config: AppConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.validate().is_ok());
        assert_eq!(config.bots.len(), 1);
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
    fn test_empty_id_fails() {
        let mut cfg = create_valid_config();
        cfg.id = "".to_string();
        let result = cfg.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Monitor ID cannot be empty"));
    }

    #[test]
    fn test_whitespace_only_id_fails() {
        let mut cfg = create_valid_config();
        cfg.id = "   ".to_string();
        let result = cfg.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Monitor ID cannot be empty"));
    }

    #[test]
    fn test_negative_spread_entry_fails() {
        let mut cfg = create_valid_config();
        cfg.spread_entry = -0.30;
        let result = cfg.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("spread_entry must be > 0 and < 100%"));
    }

    #[test]
    fn test_empty_bots_array_fails() {
        let config = AppConfig { bots: vec![] };
        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("at least one monitor"));
    }

    #[test]
    fn test_spread_entry_zero_fails() {
        let mut cfg = create_valid_config();
        cfg.spread_entry = 0.0;
        let result = cfg.validate();
        assert!(result.is_err());
    }

    #[test]
    fn test_spread_entry_above_100_fails() {
        let mut cfg = create_valid_config();
        cfg.spread_entry = 100.5;
        let result = cfg.validate();
        assert!(result.is_err());
    }

    #[test]
    fn test_nan_spread_entry_fails() {
        let mut cfg = create_valid_config();
        cfg.spread_entry = f64::NAN;
        let result = cfg.validate();
        assert!(result.is_err(), "NaN spread_entry should fail validation");
    }
}
