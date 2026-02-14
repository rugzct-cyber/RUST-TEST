//! dYdX Configuration
//!
//! Configuration for dYdX v4 Indexer WebSocket connection.

// =============================================================================
// Constants
// =============================================================================

/// dYdX v4 Indexer WebSocket URL
const WS_URL: &str = "wss://indexer.dydx.trade/v4/ws";

// =============================================================================
// Configuration
// =============================================================================

/// Configuration for dYdX exchange connection (public market data)
#[derive(Debug, Clone)]
pub struct DydxConfig {
    /// Use production endpoints
    pub production: bool,
}

impl Default for DydxConfig {
    fn default() -> Self {
        Self { production: true }
    }
}

impl DydxConfig {
    /// Create configuration from environment variables
    pub fn from_env() -> Self {
        let production = std::env::var("DYDX_PRODUCTION")
            .unwrap_or_else(|_| "true".to_string())
            .parse::<bool>()
            .unwrap_or(true);

        Self { production }
    }

    /// Get WebSocket URL
    pub fn ws_url(&self) -> &str {
        WS_URL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = DydxConfig::default();
        assert!(config.production);
        assert_eq!(config.ws_url(), "wss://indexer.dydx.trade/v4/ws");
    }
}
