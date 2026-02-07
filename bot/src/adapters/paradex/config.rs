//! Paradex Configuration
//!
//! Configuration structures for Paradex exchange connection.

use crate::adapters::errors::{ExchangeError, ExchangeResult};

// =============================================================================
// Test Constants (well-known Starknet test keys - PUBLIC, DO NOT USE IN PROD)
// =============================================================================

/// Test private key for Starknet signing (well-known public test key)
#[cfg(test)]
pub const TEST_PRIVATE_KEY: &str =
    "0x0000000000000000000000000000000000000000000000000000000000000001";

/// Test account address
#[cfg(test)]
pub const TEST_ACCOUNT_ADDRESS: &str =
    "0x0000000000000000000000000000000000000000000000000000000000000001";

// =============================================================================
// Configuration
// =============================================================================

/// Configuration for Paradex exchange connection
#[derive(Debug, Clone)]
pub struct ParadexConfig {
    /// Starknet private key (hex string with 0x prefix)
    pub private_key: String,
    /// Account address on Starknet (hex string with 0x prefix)
    pub account_address: String,
    /// Use production endpoints (true) or testnet (false)
    pub production: bool,
}

impl ParadexConfig {
    /// Create configuration from environment variables
    pub fn from_env() -> ExchangeResult<Self> {
        let private_key = std::env::var("PARADEX_PRIVATE_KEY").map_err(|_| {
            ExchangeError::AuthenticationFailed("PARADEX_PRIVATE_KEY not set".into())
        })?;
        // Account address is optional - it will be derived from private key if not provided
        let account_address =
            std::env::var("PARADEX_ACCOUNT_ADDRESS").unwrap_or_else(|_| "0x0".to_string()); // Placeholder, will be derived in authenticate()
        let production = std::env::var("PARADEX_PRODUCTION")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(true);

        Ok(Self {
            private_key,
            account_address,
            production,
        })
    }

    /// Get REST API base URL
    pub fn rest_base_url(&self) -> &'static str {
        if self.production {
            "https://api.prod.paradex.trade/v1"
        } else {
            "https://api.testnet.paradex.trade/v1"
        }
    }

    /// Get WebSocket base URL
    pub fn ws_base_url(&self) -> &'static str {
        if self.production {
            "wss://ws.api.prod.paradex.trade/v1"
        } else {
            "wss://ws.api.testnet.paradex.trade/v1"
        }
    }
}

impl Default for ParadexConfig {
    fn default() -> Self {
        Self {
            private_key: String::new(),
            account_address: String::new(),
            production: true,
        }
    }
}

// =============================================================================
// System Configuration (from /system/config API)
// =============================================================================

/// System configuration fetched from Paradex /system/config endpoint
/// Contains class hashes needed for Starknet account address derivation
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ParadexSystemConfig {
    /// Starknet chain ID (e.g., "PRIVATE_SN_PARACLEAR_MAINNET")
    pub starknet_chain_id: String,
    /// Account proxy class hash for address computation
    pub paraclear_account_proxy_hash: String,
    /// Account implementation class hash
    pub paraclear_account_hash: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_paradex_config_from_env() {
        // Set required env vars
        env::set_var("PARADEX_PRIVATE_KEY", "0x123abc");
        env::set_var("PARADEX_ACCOUNT_ADDRESS", "0x456def");
        env::set_var("PARADEX_PRODUCTION", "false");

        let config = ParadexConfig::from_env().expect("Should create config");
        assert_eq!(config.private_key, "0x123abc");
        assert_eq!(config.account_address, "0x456def");
        assert!(!config.production);

        // Cleanup
        env::remove_var("PARADEX_PRIVATE_KEY");
        env::remove_var("PARADEX_ACCOUNT_ADDRESS");
        env::remove_var("PARADEX_PRODUCTION");
    }

    #[test]
    fn test_paradex_config_default_production() {
        let config = ParadexConfig::default();
        assert!(config.production);
        assert!(config.private_key.is_empty());
    }

    #[test]
    fn test_paradex_config_urls() {
        let prod_config = ParadexConfig {
            production: true,
            ..Default::default()
        };
        assert!(prod_config.rest_base_url().contains("prod"));
        assert!(prod_config.ws_base_url().contains("prod"));

        let test_config = ParadexConfig {
            production: false,
            ..Default::default()
        };
        assert!(test_config.rest_base_url().contains("testnet"));
        assert!(test_config.ws_base_url().contains("testnet"));
    }
}
