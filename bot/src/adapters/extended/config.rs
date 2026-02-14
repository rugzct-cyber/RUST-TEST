//! Extended Configuration
const MAINNET_WS_URL: &str = "wss://api.starknet.extended.exchange/stream.extended.exchange/v1/orderbooks?depth=1";

#[derive(Debug, Clone)]
pub struct ExtendedConfig {
    pub production: bool,
    pub api_key: Option<String>,
}
impl Default for ExtendedConfig { fn default() -> Self { Self { production: true, api_key: None } } }
impl ExtendedConfig {
    pub fn from_env() -> Self {
        Self {
            production: std::env::var("EXTENDED_PRODUCTION").unwrap_or_else(|_| "true".to_string()).parse::<bool>().unwrap_or(true),
            api_key: std::env::var("EXTENDED_API_KEY").ok(),
        }
    }
    pub fn ws_url(&self) -> &str { MAINNET_WS_URL }
}
