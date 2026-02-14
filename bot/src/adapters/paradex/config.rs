//! Paradex Configuration
//!
//! Configuration structures for Paradex exchange connection.

// =============================================================================
// Configuration
// =============================================================================

/// Configuration for Paradex exchange connection (public data only)
#[derive(Debug, Clone)]
pub struct ParadexConfig {
    /// Use production endpoints (true) or testnet (false)
    pub production: bool,
}

impl ParadexConfig {
    /// Create configuration from environment variables
    pub fn from_env() -> Self {
        let production = std::env::var("PARADEX_PRODUCTION")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(true);

        Self { production }
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
        Self { production: true }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_paradex_config_default_production() {
        let config = ParadexConfig::default();
        assert!(config.production);
    }

    #[test]
    fn test_paradex_config_urls() {
        let prod_config = ParadexConfig { production: true };
        assert!(prod_config.rest_base_url().contains("prod"));
        assert!(prod_config.ws_base_url().contains("prod"));

        let test_config = ParadexConfig { production: false };
        assert!(test_config.rest_base_url().contains("testnet"));
        assert!(test_config.ws_base_url().contains("testnet"));
    }
}
