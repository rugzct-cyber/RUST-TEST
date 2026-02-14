//! Reya Configuration
const MAINNET_WS_URL: &str = "wss://ws.reya.xyz";

#[derive(Debug, Clone)]
pub struct ReyaConfig { pub production: bool }
impl Default for ReyaConfig { fn default() -> Self { Self { production: true } } }
impl ReyaConfig {
    pub fn from_env() -> Self {
        let production = std::env::var("REYA_PRODUCTION").unwrap_or_else(|_| "true".to_string()).parse::<bool>().unwrap_or(true);
        Self { production }
    }
    pub fn ws_url(&self) -> &str { MAINNET_WS_URL }
}
