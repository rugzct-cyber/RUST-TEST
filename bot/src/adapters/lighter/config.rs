//! Lighter Configuration
//!
//! Configuration for Lighter exchange connection including environment loading.

// =============================================================================
// Constants
// =============================================================================

/// Mainnet REST API base URL
const MAINNET_REST_URL: &str = "https://mainnet.zklighter.elliot.ai";
/// Mainnet WebSocket URL
const MAINNET_WS_URL: &str = "wss://mainnet.zklighter.elliot.ai/stream";

/// Testnet REST API base URL
#[allow(dead_code)]
const TESTNET_REST_URL: &str = "https://testnet.zklighter.elliot.ai";
/// Testnet WebSocket URL
#[allow(dead_code)]
const TESTNET_WS_URL: &str = "wss://testnet.zklighter.elliot.ai/stream";

// =============================================================================
// Configuration
// =============================================================================

/// Configuration for Lighter exchange connection (read-only market data)
#[derive(Debug, Clone)]
pub struct LighterConfig {
    /// Use production endpoints
    pub production: bool,
}

impl Default for LighterConfig {
    fn default() -> Self {
        Self { production: true }
    }
}

impl LighterConfig {
    /// Create configuration from environment variables
    pub fn from_env() -> Self {
        let production = std::env::var("LIGHTER_PRODUCTION")
            .unwrap_or_else(|_| "true".to_string())
            .parse::<bool>()
            .unwrap_or(true);

        Self { production }
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
}
