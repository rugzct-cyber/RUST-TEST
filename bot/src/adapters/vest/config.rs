//! Vest Configuration
//!
//! Configuration for Vest exchange connection (public data only).

// =============================================================================
// Configuration
// =============================================================================

/// Configuration for Vest exchange connection (public data only)
#[derive(Debug, Clone)]
pub struct VestConfig {
    /// Account group for server routing (0-9)
    pub account_group: u8,
    /// Use production endpoints (true) or development (false)
    pub production: bool,
}

impl VestConfig {
    /// Create configuration from environment variables
    pub fn from_env() -> Self {
        let account_group: u8 = std::env::var("VEST_ACCOUNT_GROUP")
            .unwrap_or_else(|_| "0".to_string())
            .parse()
            .unwrap_or(0);
        let production = std::env::var("VEST_PRODUCTION")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(true);

        Self {
            account_group,
            production,
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
}

impl Default for VestConfig {
    fn default() -> Self {
        Self {
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
        assert!(config.production);
        assert_eq!(config.account_group, 0);
    }

    #[test]
    fn test_vest_config_urls() {
        let config = VestConfig { production: true, ..Default::default() };
        assert!(config.ws_base_url().contains("prod"));

        let config = VestConfig { production: false, ..Default::default() };
        assert!(config.ws_base_url().contains("dev"));
    }
}
