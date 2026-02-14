//! Pacifica Configuration
const MAINNET_WS_URL: &str = "wss://ws.pacifica.fi/ws";

#[derive(Debug, Clone)]
pub struct PacificaConfig { pub production: bool }
impl Default for PacificaConfig { fn default() -> Self { Self { production: true } } }
impl PacificaConfig {
    pub fn from_env() -> Self {
        let production = std::env::var("PACIFICA_PRODUCTION").unwrap_or_else(|_| "true".to_string()).parse::<bool>().unwrap_or(true);
        Self { production }
    }
    pub fn ws_url(&self) -> &str { MAINNET_WS_URL }
}
