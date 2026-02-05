//! Vest Configuration
//!
//! Configuration for Vest exchange connection including environment loading.

use crate::adapters::errors::{ExchangeError, ExchangeResult};

// =============================================================================
// Test Constants (Hardhat/Foundry well-known keys - PUBLIC, DO NOT USE IN PROD)
// =============================================================================

/// Hardhat account #1 private key (well-known, public test key)
#[cfg(test)]
pub const TEST_PRIMARY_KEY: &str = "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";

/// Hardhat account #2 private key (well-known, public test key)  
#[cfg(test)]
pub const TEST_SIGNING_KEY: &str = "0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a";

/// Hardhat account #1 address
#[cfg(test)]
pub const TEST_PRIMARY_ADDR: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";

// =============================================================================
// Configuration
// =============================================================================

/// Configuration for Vest exchange connection
#[derive(Debug, Clone)]
pub struct VestConfig {
    /// Primary account address (holds balances)
    pub primary_addr: String,
    /// Primary private key (hex string with 0x prefix) - for signing registration
    pub primary_key: String,
    /// Signing private key (hex string with 0x prefix) - delegate signer
    pub signing_key: String,
    /// Account group for server routing (0-9)
    pub account_group: u8,
    /// Use production endpoints (true) or development (false)
    pub production: bool,
}

impl VestConfig {
    /// Create configuration from environment variables
    pub fn from_env() -> ExchangeResult<Self> {
        let primary_addr = std::env::var("VEST_PRIMARY_ADDR")
            .map_err(|_| ExchangeError::AuthenticationFailed("VEST_PRIMARY_ADDR not set".into()))?;
        if primary_addr.is_empty() {
            return Err(ExchangeError::AuthenticationFailed("VEST_PRIMARY_ADDR is empty".into()));
        }
        
        let primary_key = std::env::var("VEST_PRIMARY_KEY")
            .map_err(|_| ExchangeError::AuthenticationFailed("VEST_PRIMARY_KEY not set".into()))?;
        if primary_key.is_empty() {
            return Err(ExchangeError::AuthenticationFailed("VEST_PRIMARY_KEY is empty".into()));
        }
        
        let signing_key = std::env::var("VEST_SIGNING_KEY")
            .map_err(|_| ExchangeError::AuthenticationFailed("VEST_SIGNING_KEY not set".into()))?;
        if signing_key.is_empty() {
            return Err(ExchangeError::AuthenticationFailed("VEST_SIGNING_KEY is empty".into()));
        }
        
        let account_group: u8 = std::env::var("VEST_ACCOUNT_GROUP")
            .unwrap_or_else(|_| "0".to_string())
            .parse()
            .unwrap_or(0);
        let production = std::env::var("VEST_PRODUCTION")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(true);

        Ok(Self {
            primary_addr,
            primary_key,
            signing_key,
            account_group,
            production,
        })
    }

    /// Get REST API base URL
    pub fn rest_base_url(&self) -> &'static str {
        if self.production {
            "https://server-prod.hz.vestmarkets.com/v2"
        } else {
            "https://server-dev.hz.vestmarkets.com/v2"
        }
    }

    /// Get WebSocket base URL
    pub fn ws_base_url(&self) -> &'static str {
        if self.production {
            "wss://ws-prod.hz.vestmarkets.com/ws-api"
        } else {
            "wss://ws-dev.hz.vestmarkets.com/ws-api"
        }
    }

    /// Get verifying contract address for EIP-712
    pub fn verifying_contract(&self) -> &'static str {
        if self.production {
            "0x919386306C47b2Fe1036e3B4F7C40D22D2461a23"
        } else {
            "0x8E4D87AEf4AC4D5415C35A12319013e34223825B"
        }
    }
}

impl Default for VestConfig {
    fn default() -> Self {
        Self {
            primary_addr: String::new(),
            primary_key: String::new(),
            signing_key: String::new(),
            account_group: 0,
            production: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vest_config_default() {
        let config = VestConfig::default();
        assert!(config.primary_addr.is_empty());
        assert!(config.production);
        assert_eq!(config.account_group, 0);
    }

    #[test]
    fn test_vest_config_urls() {
        let config = VestConfig { production: true, ..Default::default() };
        assert!(config.rest_base_url().contains("prod"));
        assert!(config.ws_base_url().contains("prod"));

        let config = VestConfig { production: false, ..Default::default() };
        assert!(config.rest_base_url().contains("dev"));
        assert!(config.ws_base_url().contains("dev"));
    }

    #[test]
    fn test_vest_config_from_env_missing_vars() {
        // Clear env vars to test error handling
        std::env::remove_var("VEST_PRIMARY_ADDR");
        let result = VestConfig::from_env();
        assert!(result.is_err());
    }
}
