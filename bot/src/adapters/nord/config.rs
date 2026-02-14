//! Nord Configuration
const MAINNET_WS_URL: &str = "wss://zo-mainnet.n1.xyz/ws/deltas@BTCUSD";

#[derive(Debug, Clone)]
pub struct NordConfig { pub production: bool }
impl Default for NordConfig { fn default() -> Self { Self { production: true } } }
impl NordConfig {
    pub fn from_env() -> Self {
        Self { production: std::env::var("NORD_PRODUCTION").unwrap_or_else(|_| "true".to_string()).parse::<bool>().unwrap_or(true) }
    }
    pub fn ws_url(&self) -> &str { MAINNET_WS_URL }
}
