//! Lighter Signing
//!
//! Schnorr/Poseidon2 transaction signing for Lighter Protocol.
//! Uses vendored lighter-crypto crates for Goldilocks field operations.

use base64::Engine;
use lighter_signer::KeyManager;
use poseidon_hash::{Goldilocks, hash_to_quintic_extension};
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::adapters::errors::{ExchangeError, ExchangeResult};

// =============================================================================
// Transaction Type Constants
// =============================================================================

/// Create order transaction type
const TX_TYPE_CREATE_ORDER: u32 = 14;
/// Cancel order transaction type
const TX_TYPE_CANCEL_ORDER: u32 = 15;

/// Default transaction expiry: 10 minutes minus 1 second (in milliseconds)
const DEFAULT_EXPIRE_MS: i64 = 599_000;

// =============================================================================
// Signer
// =============================================================================

/// Lighter transaction signer
pub struct LighterSigner {
    key_manager: KeyManager,
    account_index: i64,
    api_key_index: u8,
    chain_id: u32,
    /// Optimistic nonce management (atomic for thread safety)
    nonce: AtomicI64,
}

impl LighterSigner {
    /// Create a new signer from private key hex string
    pub fn new(
        private_key_hex: &str,
        account_index: i64,
        api_key_index: u8,
        chain_id: u32,
    ) -> ExchangeResult<Self> {
        let key_manager = KeyManager::from_hex(private_key_hex)
            .map_err(|e| ExchangeError::AuthenticationFailed(format!("Invalid Lighter key: {}", e)))?;

        Ok(Self {
            key_manager,
            account_index,
            api_key_index,
            chain_id,
            nonce: AtomicI64::new(-1), // -1 = not initialized
        })
    }

    /// Set nonce from API response
    pub fn set_nonce(&self, nonce: i64) {
        self.nonce.store(nonce, Ordering::SeqCst);
    }

    /// Get and increment nonce
    pub fn next_nonce(&self) -> i64 {
        self.nonce.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Rollback nonce after failure
    pub fn rollback_nonce(&self) {
        let current = self.nonce.load(Ordering::SeqCst);
        if current > 0 {
            self.nonce.compare_exchange(current, current - 1, Ordering::SeqCst, Ordering::SeqCst).ok();
        }
    }

    /// Check if nonce is initialized
    #[allow(dead_code)]
    pub fn nonce_initialized(&self) -> bool {
        self.nonce.load(Ordering::SeqCst) >= 0
    }

    /// Get current timestamp in milliseconds
    fn now_ms() -> ExchangeResult<i64> {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .map_err(|e| ExchangeError::ConnectionFailed(format!("System time error: {}", e)))
    }

    /// Helper: convert signed i64 to Goldilocks field element
    fn to_goldi(val: i64) -> Goldilocks {
        Goldilocks::from_i64(val)
    }

    /// Sign a CreateOrder transaction (tx_type=14, 16 Goldilocks elements)
    ///
    /// Returns `(tx_info_json, nonce_used)` — the signed JSON ready for sendTx.
    pub fn sign_create_order(
        &self,
        market_index: u8,
        client_order_index: u64,
        base_amount: i64,
        price: u32,
        is_ask: bool,
        order_type: u8,
        time_in_force: u8,
        reduce_only: bool,
        trigger_price: u32,
        nonce: i64,
    ) -> ExchangeResult<String> {
        let now = Self::now_ms()?;
        let expired_at = now + DEFAULT_EXPIRE_MS;

        // OrderExpiry: 28 days for limit GoodTillTime or trigger orders, 0 for IOC/market
        let is_trigger = matches!(order_type, 2 | 3 | 4 | 5);
        let is_limit_gtt = time_in_force == 1 && order_type == 0;
        let order_expiry = if is_limit_gtt || is_trigger {
            now + (28 * 24 * 60 * 60 * 1000)
        } else {
            0
        };

        let is_ask_u8: u8 = if is_ask { 1 } else { 0 };
        let reduce_only_u8: u8 = if reduce_only { 1 } else { 0 };

        // Build 16 Goldilocks elements for CreateOrder hash
        let elements = vec![
            Goldilocks::from_canonical_u64(self.chain_id as u64),
            Goldilocks::from_canonical_u64(TX_TYPE_CREATE_ORDER as u64),
            Self::to_goldi(nonce),
            Self::to_goldi(expired_at),
            Self::to_goldi(self.account_index),
            Goldilocks::from_canonical_u64(self.api_key_index as u64),
            Goldilocks::from_canonical_u64(market_index as u64),
            Self::to_goldi(client_order_index as i64),
            Self::to_goldi(base_amount),
            Goldilocks::from_canonical_u64(price as u64),
            Goldilocks::from_canonical_u64(is_ask_u8 as u64),
            Goldilocks::from_canonical_u64(order_type as u64),
            Goldilocks::from_canonical_u64(time_in_force as u64),
            Goldilocks::from_canonical_u64(reduce_only_u8 as u64),
            Goldilocks::from_canonical_u64(trigger_price as u64),
            Self::to_goldi(order_expiry),
        ];

        // Poseidon2 hash → 40-byte digest → Schnorr sign
        let hash_result = hash_to_quintic_extension(&elements);
        let message_bytes = hash_result.to_bytes_le();
        let signature = self.key_manager.sign(&message_bytes)
            .map_err(|e| ExchangeError::InvalidResponse(format!("Signing failed: {}", e)))?;
        let sig_b64 = base64::engine::general_purpose::STANDARD.encode(&signature);

        // Build JSON tx_info (PascalCase, alphabetical order for serde_json::json! output)
        let tx_info = serde_json::json!({
            "AccountIndex": self.account_index,
            "ApiKeyIndex": self.api_key_index,
            "BaseAmount": base_amount,
            "ClientOrderIndex": client_order_index,
            "ExpiredAt": expired_at,
            "IsAsk": is_ask_u8,
            "MarketIndex": market_index,
            "Nonce": nonce,
            "OrderExpiry": order_expiry,
            "Price": price,
            "ReduceOnly": reduce_only_u8,
            "Sig": sig_b64,
            "TimeInForce": time_in_force,
            "TriggerPrice": trigger_price,
            "Type": order_type,
        });

        serde_json::to_string(&tx_info)
            .map_err(|e| ExchangeError::InvalidResponse(format!("JSON serialization failed: {}", e)))
    }

    /// Sign a CancelOrder transaction (tx_type=15, 8 Goldilocks elements)
    pub fn sign_cancel_order(
        &self,
        market_index: u8,
        order_index: i64,
        nonce: i64,
    ) -> ExchangeResult<String> {
        let now = Self::now_ms()?;
        let expired_at = now + DEFAULT_EXPIRE_MS;

        let elements = vec![
            Goldilocks::from_canonical_u64(self.chain_id as u64),
            Goldilocks::from_canonical_u64(TX_TYPE_CANCEL_ORDER as u64),
            Self::to_goldi(nonce),
            Self::to_goldi(expired_at),
            Self::to_goldi(self.account_index),
            Goldilocks::from_canonical_u64(self.api_key_index as u64),
            Goldilocks::from_canonical_u64(market_index as u64),
            Self::to_goldi(order_index),
        ];

        let hash_result = hash_to_quintic_extension(&elements);
        let message_bytes = hash_result.to_bytes_le();
        let signature = self.key_manager.sign(&message_bytes)
            .map_err(|e| ExchangeError::InvalidResponse(format!("Signing failed: {}", e)))?;
        let sig_b64 = base64::engine::general_purpose::STANDARD.encode(&signature);

        let tx_info = serde_json::json!({
            "AccountIndex": self.account_index,
            "ApiKeyIndex": self.api_key_index,
            "ExpiredAt": expired_at,
            "Index": order_index,
            "MarketIndex": market_index,
            "Nonce": nonce,
            "Sig": sig_b64,
        });

        serde_json::to_string(&tx_info)
            .map_err(|e| ExchangeError::InvalidResponse(format!("JSON serialization failed: {}", e)))
    }

    /// Create an authentication token for WebSocket connections
    pub fn create_auth_token(&self, expiry_seconds: i64) -> ExchangeResult<String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| ExchangeError::ConnectionFailed(format!("System time error: {}", e)))?
            .as_secs() as i64;
        let deadline = now + expiry_seconds;

        self.key_manager
            .create_auth_token(deadline, self.account_index, self.api_key_index)
            .map_err(|e| ExchangeError::AuthenticationFailed(format!("Auth token creation failed: {}", e)))
    }

    /// Get the account index
    pub fn account_index(&self) -> i64 {
        self.account_index
    }
}
