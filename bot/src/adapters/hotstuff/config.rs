//! HotStuff Configuration
const MAINNET_WS_URL: &str = "wss://api.hotstuff.trade/ws/";

#[derive(Debug, Clone)]
pub struct HotstuffConfig { pub production: bool }
impl Default for HotstuffConfig { fn default() -> Self { Self { production: true } } }
impl HotstuffConfig {
    pub fn from_env() -> Self {
        let production = std::env::var("HOTSTUFF_PRODUCTION").unwrap_or_else(|_| "true".to_string()).parse::<bool>().unwrap_or(true);
        Self { production }
    }
    pub fn ws_url(&self) -> &str { MAINNET_WS_URL }
}
