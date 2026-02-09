//! Lighter Configuration
//!
//! Configuration for Lighter exchange connection including environment loading.

use crate::adapters::errors::{ExchangeError, ExchangeResult};

// =============================================================================
// Constants
// =============================================================================

/// Mainnet REST API base URL
const MAINNET_REST_URL: &str = "https://mainnet.zklighter.elliot.ai";
/// Mainnet WebSocket URL
const MAINNET_WS_URL: &str = "wss://mainnet.zklighter.elliot.ai/stream";
/// Mainnet chain ID for transaction signing
const MAINNET_CHAIN_ID: u32 = 304;

/// Testnet REST API base URL
#[allow(dead_code)]
const TESTNET_REST_URL: &str = "https://testnet.zklighter.elliot.ai";
/// Testnet WebSocket URL
#[allow(dead_code)]
const TESTNET_WS_URL: &str = "wss://testnet.zklighter.elliot.ai/stream";
/// Testnet chain ID for transaction signing
#[allow(dead_code)]
const TESTNET_CHAIN_ID: u32 = 300;

// =============================================================================
// Configuration
// =============================================================================

/// Configuration for Lighter exchange connection
#[derive(Debug, Clone)]
pub struct LighterConfig {
    /// Private key (80 hex chars = 40-byte Goldilocks key)
    pub private_key: String,
    /// Account index on Lighter
    pub account_index: i64,
    /// API key index (0-255)
    pub api_key_index: u8,
    /// Use production endpoints
    pub production: bool,
}

impl LighterConfig {
    /// Create configuration from environment variables
    pub fn from_env() -> ExchangeResult<Self> {
        let private_key = std::env::var("LIGHTER_PRIVATE_KEY")
            .map_err(|_| ExchangeError::AuthenticationFailed("LIGHTER_PRIVATE_KEY not set".into()))?;
        if private_key.is_empty() {
            return Err(ExchangeError::AuthenticationFailed(
                "LIGHTER_PRIVATE_KEY is empty".into(),
            ));
        }

        let account_index: i64 = std::env::var("LIGHTER_ACCOUNT_INDEX")
            .map_err(|_| ExchangeError::AuthenticationFailed("LIGHTER_ACCOUNT_INDEX not set".into()))?
            .parse()
            .map_err(|_| ExchangeError::AuthenticationFailed("LIGHTER_ACCOUNT_INDEX must be a number".into()))?;

        let api_key_index: u8 = std::env::var("LIGHTER_API_KEY_INDEX")
            .unwrap_or_else(|_| "0".to_string())
            .parse()
            .map_err(|_| ExchangeError::AuthenticationFailed("LIGHTER_API_KEY_INDEX must be 0-255".into()))?;

        let production = std::env::var("LIGHTER_PRODUCTION")
            .unwrap_or_else(|_| "true".to_string())
            .parse::<bool>()
            .unwrap_or(true);

        Ok(Self {
            private_key,
            account_index,
            api_key_index,
            production,
        })
    }

    /// Get REST API base URL
    pub fn rest_url(&self) -> &str {
        if self.production {
            MAINNET_REST_URL
        } else {
            TESTNET_REST_URL
        }
    }

    /// Get WebSocket URL
    pub fn ws_url(&self) -> &str {
        if self.production {
            MAINNET_WS_URL
        } else {
            TESTNET_WS_URL
        }
    }

    /// Get chain ID for transaction signing
    pub fn chain_id(&self) -> u32 {
        if self.production {
            MAINNET_CHAIN_ID
        } else {
            TESTNET_CHAIN_ID
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    #[serial]
    fn test_config_from_env() {
        std::env::set_var("LIGHTER_PRIVATE_KEY", "a".repeat(80));
        std::env::set_var("LIGHTER_ACCOUNT_INDEX", "42");
        std::env::set_var("LIGHTER_API_KEY_INDEX", "1");
        std::env::set_var("LIGHTER_PRODUCTION", "true");

        let config = LighterConfig::from_env().unwrap();
        assert_eq!(config.account_index, 42);
        assert_eq!(config.api_key_index, 1);
        assert!(config.production);
        assert_eq!(config.rest_url(), MAINNET_REST_URL);
        assert_eq!(config.ws_url(), MAINNET_WS_URL);
        assert_eq!(config.chain_id(), 304);

        std::env::remove_var("LIGHTER_PRIVATE_KEY");
        std::env::remove_var("LIGHTER_ACCOUNT_INDEX");
        std::env::remove_var("LIGHTER_API_KEY_INDEX");
        std::env::remove_var("LIGHTER_PRODUCTION");
    }

    #[test]
    #[serial]
    fn test_config_missing_key() {
        std::env::remove_var("LIGHTER_PRIVATE_KEY");
        let result = LighterConfig::from_env();
        assert!(result.is_err());
    }
}
