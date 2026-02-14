//! GRVT Configuration
//!
//! Configuration for GRVT exchange WebSocket connection.

const MAINNET_WS_URL: &str = "wss://market-data.grvt.io/ws/full";

/// Configuration for GRVT exchange connection (public market data)
#[derive(Debug, Clone)]
pub struct GrvtConfig {
    pub production: bool,
}

impl Default for GrvtConfig {
    fn default() -> Self {
        Self { production: true }
    }
}

impl GrvtConfig {
    pub fn from_env() -> Self {
        let production = std::env::var("GRVT_PRODUCTION")
            .unwrap_or_else(|_| "true".to_string())
            .parse::<bool>()
            .unwrap_or(true);
        Self { production }
    }

    pub fn ws_url(&self) -> &str {
        MAINNET_WS_URL
    }
}
