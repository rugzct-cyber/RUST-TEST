//! Supabase configuration module
//!
//! Loads Supabase credentials from environment variables for analytics persistence.
//! Story 9.7: Supabase Backend Integration

use thiserror::Error;
use tracing::{debug, info, warn};

/// Errors for Supabase configuration
#[derive(Debug, Error)]
pub enum SupabaseConfigError {
    #[error("Missing required environment variable: {0}")]
    MissingEnvVar(String),
    
    #[error("Invalid Supabase URL format: {0}")]
    InvalidUrl(String),
}

/// Supabase configuration loaded from environment variables
#[derive(Debug, Clone)]
pub struct SupabaseConfig {
    /// Supabase project URL (e.g., <https://xxx.supabase.co>)
    pub url: String,
    /// Supabase anonymous key for API access
    pub anon_key: String,
    /// Whether Supabase integration is enabled
    pub enabled: bool,
}

impl SupabaseConfig {
    /// Load Supabase configuration from environment variables
    ///
    /// Required env vars (when enabled):
    /// - `SUPABASE_URL`: Supabase project URL
    /// - `SUPABASE_ANON_KEY`: API anonymous key
    ///
    /// Optional:
    /// - `SUPABASE_ENABLED`: "true" (default) or "false"
    ///
    /// # Returns
    /// - `Ok(Some(SupabaseConfig))` if enabled and configured
    /// - `Ok(None)` if explicitly disabled via SUPABASE_ENABLED=false
    /// - `Err` if enabled but missing required vars
    pub fn from_env() -> Result<Option<Self>, SupabaseConfigError> {
        // Check if Supabase is enabled (default: true if URL is set)
        let enabled = std::env::var("SUPABASE_ENABLED")
            .map(|v| v.to_lowercase() != "false")
            .unwrap_or(true);

        if !enabled {
            info!("Supabase integration disabled via SUPABASE_ENABLED=false");
            return Ok(None);
        }

        // Try to load URL - if not set, Supabase is disabled
        let url = match std::env::var("SUPABASE_URL") {
            Ok(u) if !u.is_empty() && !u.contains("your-project") => u,
            Ok(u) if u.contains("your-project") => {
                warn!("SUPABASE_URL contains placeholder value, Supabase disabled");
                return Ok(None);
            }
            _ => {
                debug!("SUPABASE_URL not set, Supabase integration disabled");
                return Ok(None);
            }
        };

        // Validate URL format
        if !url.starts_with("https://") || !url.contains("supabase") {
            return Err(SupabaseConfigError::InvalidUrl(url));
        }

        // Load anon key
        let anon_key = std::env::var("SUPABASE_ANON_KEY")
            .map_err(|_| SupabaseConfigError::MissingEnvVar("SUPABASE_ANON_KEY".to_string()))?;

        if anon_key.is_empty() || anon_key.contains("your-anon-key") {
            return Err(SupabaseConfigError::MissingEnvVar("SUPABASE_ANON_KEY (contains placeholder)".to_string()));
        }

        info!(url = %url, "Supabase configuration loaded");

        Ok(Some(Self {
            url,
            anon_key,
            enabled: true,
        }))
    }

    /// Create a SupabaseConfig for testing
    #[cfg(test)]
    pub fn new_for_test(url: &str, anon_key: &str) -> Self {
        Self {
            url: url.to_string(),
            anon_key: anon_key.to_string(),
            enabled: true,
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn clear_supabase_env() {
        env::remove_var("SUPABASE_URL");
        env::remove_var("SUPABASE_ANON_KEY");
        env::remove_var("SUPABASE_ENABLED");
    }

    #[test]
    fn test_disabled_when_env_not_set() {
        clear_supabase_env();
        
        let result = SupabaseConfig::from_env();
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_disabled_when_explicitly_false() {
        clear_supabase_env();
        env::set_var("SUPABASE_ENABLED", "false");
        env::set_var("SUPABASE_URL", "https://test.supabase.co");
        env::set_var("SUPABASE_ANON_KEY", "test-key");
        
        let result = SupabaseConfig::from_env();
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
        
        clear_supabase_env();
    }

    #[test]
    fn test_disabled_when_url_has_placeholder() {
        clear_supabase_env();
        env::set_var("SUPABASE_URL", "https://your-project.supabase.co");
        env::set_var("SUPABASE_ANON_KEY", "real-key");
        
        let result = SupabaseConfig::from_env();
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
        
        clear_supabase_env();
    }

    #[test]
    fn test_error_when_url_invalid_format() {
        clear_supabase_env();
        env::set_var("SUPABASE_URL", "http://not-supabase.com");
        env::set_var("SUPABASE_ANON_KEY", "real-key");
        
        let result = SupabaseConfig::from_env();
        assert!(result.is_err());
        
        clear_supabase_env();
    }

    #[test]
    fn test_error_when_key_missing() {
        clear_supabase_env();
        env::set_var("SUPABASE_URL", "https://test.supabase.co");
        
        let result = SupabaseConfig::from_env();
        assert!(result.is_err());
        
        clear_supabase_env();
    }

    #[test]
    fn test_success_with_valid_config() {
        clear_supabase_env();
        env::set_var("SUPABASE_URL", "https://test.supabase.co");
        env::set_var("SUPABASE_ANON_KEY", "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9");
        
        let result = SupabaseConfig::from_env();
        assert!(result.is_ok());
        let config = result.unwrap().unwrap();
        assert_eq!(config.url, "https://test.supabase.co");
        assert!(config.enabled);
        
        clear_supabase_env();
    }
}
