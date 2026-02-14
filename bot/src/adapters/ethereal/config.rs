//! Ethereal Configuration
//! Ethereal uses Socket.IO but we approximate with raw WS.
const MAINNET_WS_URL: &str = "wss://ws.ethereal.trade/v1/stream";

#[derive(Debug, Clone)]
pub struct EtherealConfig { pub production: bool }
impl Default for EtherealConfig { fn default() -> Self { Self { production: true } } }
impl EtherealConfig {
    pub fn from_env() -> Self {
        Self { production: std::env::var("ETHEREAL_PRODUCTION").unwrap_or_else(|_| "true".to_string()).parse::<bool>().unwrap_or(true) }
    }
    pub fn ws_url(&self) -> &str { MAINNET_WS_URL }
}
