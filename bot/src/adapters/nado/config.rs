//! Nado Configuration
const MAINNET_WS_URL: &str = "wss://gateway.prod.nado.xyz/v1/subscribe";

#[derive(Debug, Clone)]
pub struct NadoConfig { pub production: bool }
impl Default for NadoConfig { fn default() -> Self { Self { production: true } } }
impl NadoConfig {
    pub fn from_env() -> Self {
        Self { production: std::env::var("NADO_PRODUCTION").unwrap_or_else(|_| "true".to_string()).parse::<bool>().unwrap_or(true) }
    }
    pub fn ws_url(&self) -> &str { MAINNET_WS_URL }
}
