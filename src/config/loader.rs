//! Configuration loader for YAML files
//!
//! This module handles loading and validating configuration from YAML files.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use crate::error::AppError;

use super::types::AppConfig;

/// Load configuration from a YAML file
///
/// This function:
/// 1. Checks if the file exists
/// 2. Parses the YAML content
/// 3. Validates the configuration rules
///
/// # Arguments
/// * `path` - Path to the configuration YAML file
///
/// # Returns
/// * `Ok(AppConfig)` - Successfully loaded and validated configuration
/// * `Err(AppError)` - File not found, parse error, or validation failure
///
/// # Example
/// ```ignore
/// use std::path::Path;
/// use y_bot::config::load_config;
///
/// let config = load_config(Path::new("data/config.yaml"))?;
/// ```
pub fn load_config(path: &Path) -> Result<AppConfig, AppError> {
    // Check file exists
    if !path.exists() {
        return Err(AppError::Config(format!(
            "Configuration file not found: {}",
            path.display()
        )));
    }

    // Open file
    let file = File::open(path)?;
    let reader = BufReader::new(file);

    // Parse YAML
    let config: AppConfig = serde_yaml::from_reader(reader).map_err(|e| {
        AppError::Config(format!(
            "YAML parse error in '{}': {}",
            path.display(),
            e
        ))
    })?;

    // Validate configuration rules
    config.validate()?;

    Ok(config)
}

/// Load configuration from a YAML string (useful for testing)
///
/// # Arguments
/// * `yaml_content` - YAML content as a string
///
/// # Returns
/// * `Ok(AppConfig)` - Successfully parsed and validated configuration
/// * `Err(AppError)` - Parse error or validation failure
pub fn load_config_from_str(yaml_content: &str) -> Result<AppConfig, AppError> {
    let config: AppConfig = serde_yaml::from_str(yaml_content).map_err(|e| {
        AppError::Config(format!("YAML parse error: {}", e))
    })?;

    config.validate()?;

    Ok(config)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    const VALID_CONFIG_YAML: &str = r#"
bots:
  - id: btc_vest_paradex
    pair: BTC-PERP
    dex_a: vest
    dex_b: paradex
    spread_entry: 0.30
    spread_exit: 0.05
    leverage: 10
    capital: 100.0
risk:
  adl_warning: 10.0
  adl_critical: 5.0
  max_duration_hours: 24
api:
  port: 8080
  ws_heartbeat_sec: 30
"#;

    #[test]
    fn test_load_config_from_str_valid() {
        let config = load_config_from_str(VALID_CONFIG_YAML).unwrap();
        assert_eq!(config.bots.len(), 1);
        assert_eq!(config.bots[0].id, "btc_vest_paradex");
        assert_eq!(config.api.port, 8080);
    }

    #[test]
    fn test_load_config_from_str_invalid_yaml() {
        let invalid_yaml = "invalid: yaml: content: [";
        let result = load_config_from_str(invalid_yaml);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("YAML parse error"));
    }

    #[test]
    fn test_load_config_from_str_validation_failure() {
        let invalid_config = r#"
bots:
  - id: bad_bot
    pair: BTC-PERP
    dex_a: vest
    dex_b: vest
    spread_entry: 0.30
    spread_exit: 0.05
    leverage: 10
    capital: 100.0
risk:
  adl_warning: 10.0
  adl_critical: 5.0
  max_duration_hours: 24
api:
  port: 8080
  ws_heartbeat_sec: 30
"#;
        let result = load_config_from_str(invalid_config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("dex_a and dex_b cannot be the same"));
    }

    #[test]
    fn test_load_config_file_not_found() {
        let result = load_config(Path::new("/nonexistent/path/config.yaml"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Configuration file not found"));
    }

    #[test]
    fn test_load_config_from_file_valid() {
        // Create a temporary file with valid config
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(VALID_CONFIG_YAML.as_bytes()).unwrap();
        temp_file.flush().unwrap();

        let config = load_config(temp_file.path()).unwrap();
        assert_eq!(config.bots.len(), 1);
        assert_eq!(config.bots[0].id, "btc_vest_paradex");
    }

    #[test]
    fn test_load_config_from_file_invalid_yaml() {
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(b"invalid: [yaml: content").unwrap();
        temp_file.flush().unwrap();

        let result = load_config(temp_file.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("YAML parse error"));
    }

    #[test]
    fn test_multiple_bots_config() {
        let yaml = r#"
bots:
  - id: btc_bot
    pair: BTC-PERP
    dex_a: vest
    dex_b: paradex
    spread_entry: 0.30
    spread_exit: 0.05
    leverage: 10
    capital: 100.0
  - id: eth_bot
    pair: ETH-PERP
    dex_a: hyperliquid
    dex_b: lighter
    spread_entry: 0.40
    spread_exit: 0.10
    leverage: 20
    capital: 200.0
risk:
  adl_warning: 10.0
  adl_critical: 5.0
  max_duration_hours: 24
api:
  port: 8080
  ws_heartbeat_sec: 30
"#;
        let config = load_config_from_str(yaml).unwrap();
        assert_eq!(config.bots.len(), 2);
        assert_eq!(config.bots[0].id, "btc_bot");
        assert_eq!(config.bots[1].id, "eth_bot");
    }
}
