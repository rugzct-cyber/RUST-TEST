//! Application state management
//!
//! Includes both in-memory MVP state (BotState, AppState) and
//! Supabase persistence types (PositionState, StateManager) for Epic 3.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::config::{AppConfig, BotConfig};

/// Type alias for shared application state access across async tasks
pub type SharedAppState = Arc<RwLock<AppState>>;

/// Possible states for a bot instance
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BotStatus {
    #[default]
    Stopped,
    Starting,
    Trading,
    Recovering,
    Error,
}

/// State of a single bot instance
#[derive(Debug, Clone)]
pub struct BotState {
    pub config: BotConfig,
    pub status: BotStatus,
    pub last_update_ms: u64,
}

impl BotState {
    pub fn new(config: BotConfig) -> Self {
        Self {
            config,
            status: BotStatus::Stopped,
            last_update_ms: current_time_ms(),
        }
    }
}

/// Application-wide metrics
#[derive(Debug, Clone, Default)]
pub struct Metrics {
    pub total_trades: u64,
    pub total_pnl: f64,
    pub active_positions: usize,
    pub uptime_seconds: u64,
}

/// Root application state
#[derive(Debug, Clone)]
pub struct AppState {
    pub bots: HashMap<String, BotState>,
    pub metrics: Metrics,
    pub config: AppConfig,
    pub start_time_ms: u64,
}

impl AppState {
    pub fn new(config: AppConfig) -> Self {
        let mut bots = HashMap::new();
        
        for bot_config in &config.bots {
            let bot_state = BotState::new(bot_config.clone());
            bots.insert(bot_config.id.clone(), bot_state);
        }
        
        Self {
            bots,
            metrics: Metrics::default(),
            config,
            start_time_ms: current_time_ms(),
        }
    }
    
    pub fn into_shared(self) -> SharedAppState {
        Arc::new(RwLock::new(self))
    }
    
    pub fn get_bot(&self, id: &str) -> Option<&BotState> {
        self.bots.get(id)
    }
    
    pub fn update_bot_status(&mut self, id: &str, status: BotStatus) {
        if let Some(bot) = self.bots.get_mut(id) {
            bot.status = status;
            bot.last_update_ms = current_time_ms();
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            bots: HashMap::new(),
            metrics: Metrics::default(),
            config: AppConfig::default(),
            start_time_ms: current_time_ms(),
        }
    }
}

fn current_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ============================================================================
// EPIC 3: STATE PERSISTENCE TYPES (Story 3.1)
// ============================================================================

/// Status of a delta-neutral position
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PositionStatus {
    /// Position is fully open (both legs active)
    Open,
    /// Position is partially closed (one leg closed, other still open)
    PartialClose,
    /// Position is fully closed (both legs closed)
    Closed,
}

/// State of a delta-neutral position for persistence in Supabase
///
/// Uses Natural Key design: exchanges enforce 1 position per (asset, direction).
/// The UNIQUE(long_symbol, short_symbol) constraint provides natural reconciliation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionState {
    /// Unique identifier (UUID v4)
    pub id: Uuid,
    
    /// Trading pair (e.g., "BTC-USD")
    pub pair: String,
    
    /// Long leg symbol (e.g., "BTC-PERP" on Vest)
    pub long_symbol: String,
    
    /// Short leg symbol (e.g., "BTC-USD-PERP" on Paradex)
    pub short_symbol: String,
    
    /// Exchange for long leg (e.g., "vest")
    pub long_exchange: String,
    
    /// Exchange for short leg (e.g., "paradex")
    pub short_exchange: String,
    
    /// Initial size of long leg
    pub long_size: f64,
    
    /// Initial size of short leg
    pub short_size: f64,
    
    /// Remaining size (supports partial close from Story 2.5)
    pub remaining_size: f64,
    
    /// Entry spread percentage
    pub entry_spread: f64,
    
    /// Timestamp when position was opened
    pub entry_timestamp: DateTime<Utc>,
    
    /// Current status of the position
    pub status: PositionStatus,
}

impl PositionState {
    /// Create a new position state with auto-generated ID and current timestamp
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pair: String,
        long_symbol: String,
        short_symbol: String,
        long_exchange: String,
        short_exchange: String,
        long_size: f64,
        short_size: f64,
        entry_spread: f64,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            pair,
            long_symbol,
            short_symbol,
            long_exchange,
            short_exchange,
            long_size,
            short_size,
            remaining_size: long_size.min(short_size), // Initial remaining = min of both legs
            entry_spread,
            entry_timestamp: Utc::now(),
            status: PositionStatus::Open,
        }
    }
}

/// Errors for state management operations
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    /// Database operation failed
    #[error("Database error: {0}")]
    DatabaseError(String),
    
    /// Position not found
    #[error("Position not found")]
    NotFound,
    
    /// Invalid data provided
    #[error("Invalid data: {0}")]
    InvalidData(String),
    
    /// Network error during API call
    #[error("Network error: {0}")]
    NetworkError(#[from] reqwest::Error),
}

/// Update data for modifying an existing position
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionUpdate {
    /// Optional new remaining size
    pub remaining_size: Option<f64>,
    
    /// Optional new status
    pub status: Option<PositionStatus>,
}

/// Manager for persisting position state to Supabase
///
/// Story 3.1: Stub implementation - all methods return Ok(())
/// Story 3.2: Implementing actual Supabase save logic
pub struct StateManager {
    /// Supabase project URL (e.g., https://xxx.supabase.co)
    supabase_url: String,
    
    /// Optional Supabase client (None if disabled)
    supabase_client: Option<reqwest::Client>,
}

impl StateManager {
    /// Create a new state manager with Supabase configuration
    ///
    /// # Arguments
    /// * `config` - SupabaseConfig with URL, anon key, and enabled flag
    ///
    /// # Note
    /// Story 3.2: Initializes reqwest client with proper auth headers
    pub fn new(config: crate::config::SupabaseConfig) -> Self {
        let client = if config.enabled {
            // Build headers with apikey and Authorization
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(
                "apikey",
                reqwest::header::HeaderValue::from_str(&config.anon_key)
                    .expect("Invalid apikey header value"),
            );
            headers.insert(
                reqwest::header::AUTHORIZATION,
                reqwest::header::HeaderValue::from_str(&format!("Bearer {}", config.anon_key))
                    .expect("Invalid authorization header value"),
            );
            headers.insert(
                reqwest::header::CONTENT_TYPE,
                reqwest::header::HeaderValue::from_static("application/json"),
            );
            
            Some(
                reqwest::Client::builder()
                    .default_headers(headers)
                    .timeout(std::time::Duration::from_secs(10))  // NFR14: HTTP timeout for stability
                    .build()
                    .expect("Failed to build reqwest client"),
            )
        } else {
            None
        };
        
        Self {
            supabase_url: config.url,
            supabase_client: client,
        }
    }
    
    /// Save a position to Supabase
    ///
    /// # Story 3.2 - Full Implementation
    /// POST to /rest/v1/positions with position data
    ///
    /// # Errors
    /// - `StateError::DatabaseError` for 409 Conflict (duplicate natural key) or other DB errors
    /// - `StateError::NetworkError` for network/timeout failures
    pub async fn save_position(&self, pos: &PositionState) -> Result<(), StateError> {
        // Handle disabled case
        if self.supabase_client.is_none() {
            tracing::debug!("Supabase disabled, position not saved");
            return Ok(());
        }
        
        let client = self.supabase_client.as_ref().unwrap();
        let url = format!("{}/rest/v1/positions", self.supabase_url);
        
        // Send POST request
        let response = client
            .post(&url)
            .header("Prefer", "return=minimal")  // Optimize: don't return data
            .json(pos)
            .send()
            .await?;  // NetworkError auto-converted via #[from]
        
        // Handle HTTP status codes
        match response.status() {
            reqwest::StatusCode::CREATED => {
                tracing::info!(
                    pair = %pos.pair,
                    long_exchange = %pos.long_exchange,
                    short_exchange = %pos.short_exchange,
                    entry_spread = %pos.entry_spread,
                    "Position saved to Supabase"
                );
                Ok(())
            }
            reqwest::StatusCode::CONFLICT => {
                let err_msg = format!(
                    "Position already exists (natural key conflict): ({}, {})",
                    pos.long_symbol, pos.short_symbol
                );
                tracing::error!(
                    pair = %pos.pair,
                    long_symbol = %pos.long_symbol,
                    short_symbol = %pos.short_symbol,
                    "Failed to save position to Supabase: {}", err_msg
                );
                Err(StateError::DatabaseError(err_msg))
            }
            reqwest::StatusCode::UNAUTHORIZED => {
                let err_msg = "Invalid Supabase credentials".to_string();
                tracing::error!(
                    pair = %pos.pair,
                    "Failed to save position to Supabase: {}", err_msg
                );
                Err(StateError::DatabaseError(err_msg))
            }
            status => {
                let body = response.text().await.unwrap_or_else(|_| "<no body>".to_string());
                let err_msg = format!("Supabase error {}: {}", status, body);
                tracing::error!(
                    pair = %pos.pair,
                    status = %status,
                    response_body = %body,
                    "Failed to save position to Supabase"
                );
                Err(StateError::DatabaseError(err_msg))
            }
        }
    }
    
    /// Load all open positions from Supabase
    ///
    /// # Story 3.1 - Stub Implementation
    /// Always returns Ok(empty vector)
    ///
    /// # Story 3.3 - Full Implementation
    /// GET from /rest/v1/positions?status=eq.Open
    ///
    /// # Errors
    /// - `StateError::DatabaseError` for 401 Unauthorized or other DB errors
    /// - `StateError::NetworkError` for network/timeout failures
    pub async fn load_positions(&self) -> Result<Vec<PositionState>, StateError> {
        // Handle disabled case
        if self.supabase_client.is_none() {
            tracing::debug!("Supabase disabled, no positions to restore");
            return Ok(Vec::new());
        }
        
        let client = self.supabase_client.as_ref().unwrap();
        let url = format!("{}/rest/v1/positions?status=eq.Open", self.supabase_url);
        
        // Send GET request
        let response = client
            .get(&url)
            .send()
            .await?;  // NetworkError auto-converted via #[from]
        
        // Handle HTTP status codes
        match response.status() {
            reqwest::StatusCode::OK => {
                let positions: Vec<PositionState> = response.json().await?;
                
                if positions.is_empty() {
                    tracing::info!("No positions to restore");
                } else {
                    tracing::info!(
                        count = positions.len(),
                        "Restored positions from Supabase"
                    );
                }
                
                Ok(positions)
            }
            reqwest::StatusCode::UNAUTHORIZED => {
                let err_msg = "Invalid Supabase credentials".to_string();
                tracing::error!(
                    "Failed to load positions from Supabase: {}", err_msg
                );
                Err(StateError::DatabaseError(err_msg))
            }
            status => {
                let body = response.text().await.unwrap_or_else(|_| "<no body>".to_string());
                let err_msg = format!("Supabase error {}: {}", status, body);
                tracing::error!(
                    status = %status,
                    response_body = %body,
                    "Failed to load positions from Supabase"
                );
                Err(StateError::DatabaseError(err_msg))
            }
        }
    }
    
    /// Update an existing position in Supabase
    ///
    /// # Story 3.1 - Stub Implementation
    /// Always returns Ok(()) without actual update
    ///
    /// # Story 3.2 - Full Implementation
    /// Will PATCH /rest/v1/positions?id=eq.{id}
    pub async fn update_position(
        &self,
        _id: Uuid,
        _updates: PositionUpdate,
    ) -> Result<(), StateError> {
        // Stub for Story 3.1
        Ok(())
    }
    
    /// Remove a position from Supabase
    ///
    /// # Story 3.1 - Stub Implementation
    /// Always returns Ok(()) without actual deletion
    ///
    /// # Story 3.2 - Full Implementation
    /// Will DELETE /rest/v1/positions?id=eq.{id}
    pub async fn remove_position(&self, _id: Uuid) -> Result<(), StateError> {
        // Stub for Story 3.1
        Ok(())
    }
}

// ============================================================================
// MVP IN-MEMORY STATE (Pre-Epic 3)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_state_default() {
        let state = AppState::default();
        assert!(state.bots.is_empty());
        assert_eq!(state.metrics.total_trades, 0);
    }

    #[test]
    fn test_bot_status_default() {
        assert_eq!(BotStatus::default(), BotStatus::Stopped);
    }

    #[tokio::test]
    async fn test_shared_state() {
        let state = AppState::default();
        let shared = state.into_shared();
        
        {
            let guard = shared.read().await;
            assert!(guard.bots.is_empty());
        }
        
        {
            let mut guard = shared.write().await;
            guard.metrics.total_trades = 5;
        }
        
        {
            let guard = shared.read().await;
            assert_eq!(guard.metrics.total_trades, 5);
        }
    }

    // ========================================================================
    // EPIC 3: STATE PERSISTENCE TESTS (Story 3.1)
    // ========================================================================

    #[test]
    fn test_position_state_new() {
        // Story 3.1 Task 7.1: Verify UUID and timestamp generation
        let pos = PositionState::new(
            "BTC-USD".to_string(),
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
            "vest".to_string(),
            "paradex".to_string(),
            0.5,
            0.5,
            0.35,
        );

        // UUID should be auto-generated (non-zero)
        assert_ne!(pos.id, Uuid::nil());

        // Timestamp should be recent (within last second)
        let now = Utc::now();
        let diff = now.signed_duration_since(pos.entry_timestamp);
        assert!(diff.num_seconds() < 1, "Timestamp should be current");

        // Initial status should be Open
        assert_eq!(pos.status, PositionStatus::Open);

        // Remaining size should equal min(long_size, short_size)
        assert_eq!(pos.remaining_size, 0.5);

        // Verify Natural Key fields
        assert_eq!(pos.long_symbol, "BTC-PERP");
        assert_eq!(pos.short_symbol, "BTC-USD-PERP");
    }

    #[test]
    fn test_position_state_serialize() {
        // Story 3.1 Task 7.2: Verify serde JSON roundtrip
        let pos = PositionState::new(
            "ETH-USD".to_string(),
            "ETH-PERP".to_string(),
            "ETH-USD-PERP".to_string(),
            "vest".to_string(),
            "paradex".to_string(),
            1.0,
            1.0,
            0.25,
        );

        // Serialize to JSON
        let json = serde_json::to_string(&pos).expect("Should serialize to JSON");
        assert!(json.contains("ETH-USD"));
        assert!(json.contains("vest"));
        assert!(json.contains("paradex"));

        // Deserialize back
        let deserialized: PositionState =
            serde_json::from_str(&json).expect("Should deserialize from JSON");

        // Verify key fields match
        assert_eq!(deserialized.id, pos.id);
        assert_eq!(deserialized.pair, pos.pair);
        assert_eq!(deserialized.long_symbol, pos.long_symbol);
        assert_eq!(deserialized.short_symbol, pos.short_symbol);
        assert_eq!(deserialized.remaining_size, pos.remaining_size);
        assert_eq!(deserialized.status, pos.status);
    }

    #[tokio::test]
    async fn test_state_manager_stubs() {
        // Story 3.1 Task 7.3: Verify all stub methods return Ok(())
        // Updated Story 3.2: Use SupabaseConfig
        let config = crate::config::SupabaseConfig {
            url: "https://test.supabase.co".to_string(),
            anon_key: "test-anon-key".to_string(),
            enabled: false,  // Disabled to avoid HTTP requests
        };
        let manager = StateManager::new(config);

        let pos = PositionState::new(
            "BTC-USD".to_string(),
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
            "vest".to_string(),
            "paradex".to_string(),
            0.1,
            0.1,
            0.30,
        );

        // save_position stub should return Ok(())
        assert!(manager.save_position(&pos).await.is_ok());

        // load_positions stub should return Ok(empty vec)
        let loaded = manager.load_positions().await.expect("Should return Ok");
        assert_eq!(loaded.len(), 0);

        // update_position stub should return Ok(())
        let update = PositionUpdate {
            remaining_size: Some(0.05),
            status: Some(PositionStatus::PartialClose),
        };
        assert!(manager.update_position(pos.id, update).await.is_ok());

        // remove_position stub should return Ok(())
        assert!(manager.remove_position(pos.id).await.is_ok());
    }

    #[test]
    fn test_state_error_variants() {
        // Story 3.1 Task 7.4: Verify all error variants display correctly
        let db_error = StateError::DatabaseError("Connection failed".to_string());
        assert_eq!(format!("{}", db_error), "Database error: Connection failed");

        let not_found = StateError::NotFound;
        assert_eq!(format!("{}", not_found), "Position not found");

        let invalid_data = StateError::InvalidData("Bad UUID".to_string());
        assert_eq!(format!("{}", invalid_data), "Invalid data: Bad UUID");

        // Verify Debug trait (thiserror requirement)
        assert!(format!("{:?}", db_error).contains("DatabaseError"));
        assert!(format!("{:?}", not_found).contains("NotFound"));
    }
    // ========================================================================
    // STORY 3.2: SAVE POSITION TO SUPABASE TESTS
    // ========================================================================

    #[tokio::test]
    async fn test_save_position_disabled() {
        // Story 3.2 Task 6.3: Supabase disabled - should skip HTTP request
        let config = crate::config::SupabaseConfig {
            url: "https://test.supabase.co".to_string(),
            anon_key: "test-key".to_string(),
            enabled: false,
        };
        
        let manager = StateManager::new(config);
        let pos = PositionState::new(
            "BTC-USD".to_string(),
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
            "vest".to_string(),
            "paradex".to_string(),
            0.5,
            0.5,
            0.35,
        );

        // Should return Ok without network request
        assert!(manager.save_position(&pos).await.is_ok());
    }

    #[tokio::test]
    async fn test_save_position_success() {
        // Story 3.2 Task 6.1: Mock Supabase 201 Created
        let mut server = mockito::Server::new_async().await;
        
        let mock = server.mock("POST", "/rest/v1/positions")
            .with_status(201)
            .with_header("content-type", "application/json")
            .create_async()
            .await;

        let config = crate::config::SupabaseConfig {
            url: server.url(),
            anon_key: "test-anon-key".to_string(),
            enabled: true,
        };

        let manager = StateManager::new(config);
        let pos = PositionState::new(
            "BTC-USD".to_string(),
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
            "vest".to_string(),
            "paradex".to_string(),
            0.5,
            0.5,
            0.35,
        );

        // Should return Ok
        let result = manager.save_position(&pos).await;
        assert!(result.is_ok());
        
        // Verify mock was called
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_save_position_conflict() {
        // Story 3.2 Task 6.2: Mock 409 Conflict - natural key violation
        let mut server = mockito::Server::new_async().await;
        
        let mock = server.mock("POST", "/rest/v1/positions")
            .with_status(409)
            .with_header("content-type", "application/json")
            .with_body(r#"{"message":"duplicate key value violates unique constraint"}"#)
            .create_async()
            .await;

        let config = crate::config::SupabaseConfig {
            url: server.url(),
            anon_key: "test-anon-key".to_string(),
            enabled: true,
        };

        let manager = StateManager::new(config);
        let pos = PositionState::new(
            "ETH-USD".to_string(),
            "ETH-PERP".to_string(),
            "ETH-USD-PERP".to_string(),
            "vest".to_string(),
            "paradex".to_string(),
            1.0,
            1.0,
            0.25,
        );

        // Should return DatabaseError
        let result = manager.save_position(&pos).await;
        assert!(result.is_err());
        
        match result {
            Err(StateError::DatabaseError(msg)) => {
                assert!(msg.contains("already exists"));
                assert!(msg.contains("natural key conflict"));
            }
            _ => panic!("Expected DatabaseError for 409 Conflict"),
        }
        
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_save_position_unauthorized() {
        // Story 3.2 Task 4.2: Mock 401 Unauthorized
        let mut server = mockito::Server::new_async().await;
        
        let mock = server.mock("POST", "/rest/v1/positions")
            .with_status(401)
            .with_body("Unauthorized")
            .create_async()
            .await;

        let config = crate::config::SupabaseConfig {
            url: server.url(),
            anon_key: "invalid-key".to_string(),
            enabled: true,
        };

        let manager = StateManager::new(config);
        let pos = PositionState::new(
            "SOL-USD".to_string(),
            "SOL-PERP".to_string(),
            "SOL-USD-PERP".to_string(),
            "vest".to_string(),
            "paradex".to_string(),
            2.0,
            2.0,
            0.30,
        );

        // Should return DatabaseError
        let result = manager.save_position(&pos).await;
        assert!(result.is_err());
        
        match result {
            Err(StateError::DatabaseError(msg)) => {
                assert!(msg.contains("Invalid Supabase credentials"));
            }
            _ => panic!("Expected DatabaseError for 401 Unauthorized"),
        }
        
        mock.assert_async().await;
    }

    // ========================================================================
    // STORY 3.3: LOAD POSITIONS FROM SUPABASE TESTS
    // ========================================================================

    #[tokio::test]
    async fn test_load_positions_disabled() {
        // Story 3.3 Task 6.3: Supabase disabled - should skip HTTP request
        let config = crate::config::SupabaseConfig {
            url: "https://test.supabase.co".to_string(),
            anon_key: "test-key".to_string(),
            enabled: false,
        };
        
        let manager = StateManager::new(config);

        // Should return Ok with empty vec without network request
        let result = manager.load_positions().await;
        assert!(result.is_ok());
        let positions = result.unwrap();
        assert_eq!(positions.len(), 0);
    }

    #[tokio::test]
    async fn test_load_positions_success() {
        // Story 3.3 Task 6.1: Mock GET 200 OK with 2 positions
        let mut server = mockito::Server::new_async().await;
        
        let json_response = r#"[
            {
                "id": "550e8400-e29b-41d4-a716-446655440000",
                "pair": "BTC-USD",
                "long_symbol": "BTC-PERP",
                "short_symbol": "BTC-USD-PERP",
                "long_exchange": "vest",
                "short_exchange": "paradex",
                "long_size": 0.5,
                "short_size": 0.5,
                "remaining_size": 0.5,
                "entry_spread": 0.35,
                "entry_timestamp": "2026-02-02T01:30:00Z",
                "status": "Open"
            },
            {
                "id": "661e9511-f40c-52e5-b827-557766551111",
                "pair": "ETH-USD",
                "long_symbol": "ETH-PERP",
                "short_symbol": "ETH-USD-PERP",
                "long_exchange": "vest",
                "short_exchange": "paradex",
                "long_size": 1.0,
                "short_size": 1.0,
                "remaining_size": 1.0,
                "entry_spread": 0.25,
                "entry_timestamp": "2026-02-02T01:35:00Z",
                "status": "Open"
            }
        ]"#;
        
        let mock = server.mock("GET", "/rest/v1/positions?status=eq.Open")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(json_response)
            .create_async()
            .await;

        let config = crate::config::SupabaseConfig {
            url: server.url(),
            anon_key: "test-anon-key".to_string(),
            enabled: true,
        };

        let manager = StateManager::new(config);

        // Should return Ok with 2 positions
        let result = manager.load_positions().await;
        assert!(result.is_ok());
        
        let positions = result.unwrap();
        assert_eq!(positions.len(), 2);
        
        // Verify first position fields
        assert_eq!(positions[0].pair, "BTC-USD");
        assert_eq!(positions[0].long_symbol, "BTC-PERP");
        assert_eq!(positions[0].short_symbol, "BTC-USD-PERP");
        assert_eq!(positions[0].remaining_size, 0.5);
        
        // Verify second position
        assert_eq!(positions[1].pair, "ETH-USD");
        assert_eq!(positions[1].remaining_size, 1.0);
        
        // Verify mock was called
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_load_positions_empty() {
        // Story 3.3 Task 6.2: Mock GET 200 OK with empty array
        let mut server = mockito::Server::new_async().await;
        
        let mock = server.mock("GET", "/rest/v1/positions?status=eq.Open")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body("[]")
            .create_async()
            .await;

        let config = crate::config::SupabaseConfig {
            url: server.url(),
            anon_key: "test-anon-key".to_string(),
            enabled: true,
        };

        let manager = StateManager::new(config);

        // Should return Ok with empty vec
        let result = manager.load_positions().await;
        assert!(result.is_ok());
        
        let positions = result.unwrap();
        assert_eq!(positions.len(), 0);
        
        // Verify mock was called
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_load_positions_unauthorized() {
        // Story 3.3 Task 6.4: Mock 401 Unauthorized - invalid credentials
        let mut server = mockito::Server::new_async().await;
        
        let mock = server.mock("GET", "/rest/v1/positions?status=eq.Open")
            .with_status(401)
            .with_body("Unauthorized")
            .create_async()
            .await;

        let config = crate::config::SupabaseConfig {
            url: server.url(),
            anon_key: "invalid-key".to_string(),
            enabled: true,
        };

        let manager = StateManager::new(config);

        // Should return DatabaseError
        let result = manager.load_positions().await;
        assert!(result.is_err());
        
        match result {
            Err(StateError::DatabaseError(msg)) => {
                assert!(msg.contains("Invalid Supabase credentials"));
            }
            _ => panic!("Expected DatabaseError for 401 Unauthorized"),
        }
        
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_load_positions_network_error() {
        // Story 3.3 Task 6.5: Network error - connection refused
        let config = crate::config::SupabaseConfig {
            url: "http://localhost:1".to_string(),  // Port fermé - connection refused
            anon_key: "test-key".to_string(),
            enabled: true,
        };
        
        let manager = StateManager::new(config);

        // Should return NetworkError (auto-converted from reqwest::Error)
        let result = manager.load_positions().await;
        assert!(result.is_err());
        
        match result {
            Err(StateError::NetworkError(_)) => {
                // Success - network error properly converted
            }
            _ => panic!("Expected NetworkError for connection refused"),
        }
    }

    #[tokio::test]
    #[ignore]  // Run with: cargo test -- --ignored
    async fn test_load_positions_real_supabase_integration() {
        // Story 3.3 Task 6.6: Integration test with real Supabase
        // Requires: SUPABASE_URL and SUPABASE_ANON_KEY env vars
        
        use std::env;
        
        let url = match env::var("SUPABASE_URL") {
            Ok(u) if !u.is_empty() && !u.contains("your-project") => u,
            _ => {
                eprintln!("Skipping integration test - SUPABASE_URL not set");
                return;
            }
        };
        
        let anon_key = match env::var("SUPABASE_ANON_KEY") {
            Ok(k) if !k.is_empty() && !k.contains("your-anon-key") => k,
            _ => {
                eprintln!("Skipping integration test - SUPABASE_ANON_KEY not set");
                return;
            }
        };
        
        let config = crate::config::SupabaseConfig {
            url,
            anon_key,
            enabled: true,
        };
        
        let manager = StateManager::new(config);

        // Should successfully load positions from real Supabase
        let result = manager.load_positions().await;
        
        if let Err(e) = &result {
            eprintln!("Integration test failed: {:?}", e);
        }
        
        assert!(result.is_ok(), "Real Supabase load should succeed");
        
        let positions = result.unwrap();
        eprintln!("Loaded {} positions from Supabase", positions.len());
        
        // Verify deserialization worked if positions exist
        for pos in &positions {
            assert!(!pos.pair.is_empty(), "Position should have pair");
            assert!(!pos.long_symbol.is_empty(), "Position should have long_symbol");
            assert!(!pos.short_symbol.is_empty(), "Position should have short_symbol");
        }
    }

    #[tokio::test]
    async fn test_load_positions_malformed_json() {
        // Code Review Finding #2: Test JSON parsing error handling
        // Mock 200 OK with malformed JSON that cannot be parsed
        let mut server = mockito::Server::new_async().await;
        
        let mock = server.mock("GET", "/rest/v1/positions?status=eq.Open")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body("{broken json syntax [}")  // Invalid JSON
            .create_async()
            .await;

        let config = crate::config::SupabaseConfig {
            url: server.url(),
            anon_key: "test-key".to_string(),
            enabled: true,
        };

        let manager = StateManager::new(config);

        // Should return NetworkError (reqwest JSON parsing error auto-converted)
        let result = manager.load_positions().await;
        assert!(result.is_err());
        
        match result {
            Err(StateError::NetworkError(_)) => {
                // Success - JSON parsing error properly handled
            }
            _ => panic!("Expected NetworkError for malformed JSON"),
        }
        
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_load_positions_schema_mismatch() {
        // Code Review Finding #2: Test schema incompatibility
        // Mock 200 OK with valid JSON but incompatible schema
        let mut server = mockito::Server::new_async().await;
        
        let invalid_json = r#"[
            {
                "id": "550e8400-e29b-41d4-a716-446655440000",
                "pair": "BTC-USD",
                "long_symbol": "BTC-PERP",
                "short_symbol": "BTC-USD-PERP",
                "long_exchange": "vest",
                "short_exchange": "paradex",
                "long_size": 0.5,
                "short_size": 0.5,
                "remaining_size": 0.5,
                "entry_spread": 0.35,
                "entry_timestamp": "2026-02-02T01:30:00Z",
                "status": "InvalidStatus"
            }
        ]"#;
        
        let mock = server.mock("GET", "/rest/v1/positions?status=eq.Open")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(invalid_json)
            .create_async()
            .await;

        let config = crate::config::SupabaseConfig {
            url: server.url(),
            anon_key: "test-key".to_string(),
            enabled: true,
        };

        let manager = StateManager::new(config);

        // Should return NetworkError (serde deserialization failure)
        let result = manager.load_positions().await;
        assert!(result.is_err());
        
        match result {
            Err(StateError::NetworkError(_)) => {
                // Success - schema mismatch error properly handled
            }
            _ => panic!("Expected NetworkError for schema mismatch"),
        }
        
        mock.assert_async().await;
    }

}
