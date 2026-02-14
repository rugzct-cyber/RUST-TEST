//! Hyperliquid Configuration
//!
//! Configuration for Hyperliquid exchange WebSocket connection.

// =============================================================================
// Constants
// =============================================================================

/// Mainnet WebSocket URL
const MAINNET_WS_URL: &str = "wss://api.hyperliquid.xyz/ws";

// =============================================================================
// Configuration
// =============================================================================

/// Configuration for Hyperliquid exchange connection (public market data)
#[derive(Debug, Clone)]
pub struct HyperliquidConfig {
    /// Use production endpoints
    pub production: bool,
}

impl Default for HyperliquidConfig {
    fn default() -> Self {
        Self { production: true }
    }
}

impl HyperliquidConfig {
    /// Create configuration from environment variables
    pub fn from_env() -> Self {
        let production = std::env::var("HYPERLIQUID_PRODUCTION")
            .unwrap_or_else(|_| "true".to_string())
            .parse::<bool>()
            .unwrap_or(true);

        Self { production }
    }

    /// Get WebSocket URL
    pub fn ws_url(&self) -> &str {
        MAINNET_WS_URL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = HyperliquidConfig::default();
        assert!(config.production);
        assert_eq!(config.ws_url(), "wss://api.hyperliquid.xyz/ws");
    }
}
