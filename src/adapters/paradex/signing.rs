//! Paradex Signing
//!
//! Starknet signature functions for Paradex authentication and order signing.
//! Implements SNIP-12 typed data signing (EIP-712 inspired) for Starknet.

use std::time::{SystemTime, UNIX_EPOCH};

use starknet_crypto::{pedersen_hash, FieldElement};
use starknet_core::crypto::compute_hash_on_elements as core_compute_hash_on_elements;
use starknet_core::utils::{cairo_short_string_to_felt, starknet_keccak};

use crate::adapters::errors::{ExchangeError, ExchangeResult};

// =============================================================================
// Helper Functions
// =============================================================================

/// Generate current timestamp in milliseconds
pub fn current_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Build WebSocket authentication message
pub fn build_ws_auth_message(jwt_token: &str) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "method": "auth",
        "params": {
            "bearer": jwt_token
        },
        "id": 0
    }).to_string()
}

/// Build WebSocket URL for Paradex
#[allow(dead_code)] // Used in tests via cfg(test) import
pub fn build_ws_url(production: bool) -> String {
    if production {
        "wss://ws.api.prod.paradex.trade/v1".to_string()
    } else {
        "wss://ws.api.testnet.paradex.trade/v1".to_string()
    }
}

// =============================================================================
// Starknet Signature Functions
// =============================================================================

/// Sign an authentication message for Paradex REST /auth endpoint
/// 
/// Implements StarkNet SNIP-12 typed data signing (EIP-712 inspired):
/// - Builds typed data message with method, path, body, timestamp, expiration
/// - Uses Paradex TypedData format: StarkNetDomain + Request message types
/// - Hash algorithm: compute_hash_on_elements = reduce(H, [*data, len], 0)
/// - Returns (signature_r, signature_s) as decimal strings
/// 
/// # Arguments
/// * `private_key` - Starknet private key as hex string (with 0x prefix)
/// * `account_address` - Starknet account address as hex string  
/// * `timestamp` - Timestamp in SECONDS (not milliseconds!)
/// * `expiration` - Signature expiration in SECONDS
/// * `chain_id` - Chain ID from Paradex /system/config (e.g., "PRIVATE_SN_POTC_SEPOLIA")
/// 
/// # Returns
/// Tuple of (signature_r, signature_s) as decimal strings (not hex!)
#[tracing::instrument(skip(private_key), fields(account = %account_address, chain = %chain_id))]
pub fn sign_auth_message(
    private_key: &str,
    account_address: &str,
    timestamp: u64,
    expiration: u64,
    chain_id: &str,
) -> ExchangeResult<(String, String)> {
    use starknet_signers::SigningKey;
    use starknet_core::types::Felt;
    
    // === Parse inputs using starknet_core::Felt (matching bot3) ===
    let pk_felt = Felt::from_hex(private_key)
        .map_err(|e| ExchangeError::AuthenticationFailed(format!("Invalid private key: {}", e)))?;
    let account_felt = Felt::from_hex(account_address)
        .map_err(|e| ExchangeError::AuthenticationFailed(format!("Invalid account: {}", e)))?;
    let chain_felt = cairo_short_string_to_felt(chain_id)
        .map_err(|e| ExchangeError::AuthenticationFailed(format!("Invalid chain_id: {}", e)))?;
    
    // === Build Domain hash (matching bot3/SDK) ===
    // IMPORTANT: bot3 uses NO QUOTES in type hash!
    let domain_type_hash = starknet_keccak("StarkNetDomain(name:felt,chainId:felt,version:felt)".as_bytes());
    let domain_name = cairo_short_string_to_felt("Paradex")
        .map_err(|e| ExchangeError::AuthenticationFailed(format!("Invalid domain name: {}", e)))?;
    
    let domain_hash = core_compute_hash_on_elements(&[
        domain_type_hash,
        domain_name,
        chain_felt,
        Felt::ONE,  // version = 1
    ]);
    
    // === Build Request struct hash ===
    let request_type_hash = starknet_keccak(
        "Request(method:felt,path:felt,body:felt,timestamp:felt,expiration:felt)".as_bytes()
    );
    let method = cairo_short_string_to_felt("POST")
        .map_err(|e| ExchangeError::AuthenticationFailed(format!("Invalid method: {}", e)))?;
    let path = cairo_short_string_to_felt("/v1/auth")
        .map_err(|e| ExchangeError::AuthenticationFailed(format!("Invalid path: {}", e)))?;
    let body = cairo_short_string_to_felt("")
        .map_err(|e| ExchangeError::AuthenticationFailed(format!("Invalid body: {}", e)))?;
    
    let request_hash = core_compute_hash_on_elements(&[
        request_type_hash,
        method,
        path,
        body,
        Felt::from(timestamp),
        Felt::from(expiration),
    ]);
    
    // === Final message hash using compute_hash_on_elements (WITH length prefix!) ===
    // CRITICAL: bot3 uses compute_hash_on_elements which adds length prefix at the end
    // This is different from raw pedersen_hash chaining!
    let starknet_message_prefix = Felt::from_raw([
        257012186512350467,
        18446744073709551605,
        10480951322775611302,
        16156019428408348868,
    ]);
    
    let final_hash = core_compute_hash_on_elements(&[
        starknet_message_prefix,
        domain_hash,
        account_felt,
        request_hash,
    ]);
    
    // Debug logging
    tracing::debug!("Paradex Auth (SDK-compatible):");
    tracing::debug!("  domain_hash: {:?}", domain_hash);
    tracing::debug!("  request_hash: {:?}", request_hash);
    tracing::debug!("  final_hash: {:?}", final_hash);
    
    // === Sign using SigningKey (matching bot3 exactly) ===
    let signing_key = SigningKey::from_secret_scalar(pk_felt);
    
    let signature = signing_key.sign(&final_hash)
        .map_err(|e| ExchangeError::AuthenticationFailed(format!("Signing failed: {}", e)))?;
    
    // Format as DECIMAL strings (not hex!)
    let r_bytes = signature.r.to_bytes_be();
    let s_bytes = signature.s.to_bytes_be();
    let r_decimal = num_bigint::BigUint::from_bytes_be(&r_bytes).to_string();
    let s_decimal = num_bigint::BigUint::from_bytes_be(&s_bytes).to_string();
    
    tracing::debug!("  signature.r (decimal): {}", r_decimal);
    tracing::debug!("  signature.s (decimal): {}", s_decimal);
    
    Ok((r_decimal, s_decimal))
}

/// Compute hash on elements using Starknet's algorithm:
/// h(h(h(h(h(0, data[0]), data[1]), ...), data[n-1]), n)
/// 
/// Starts at 0, chains all elements with pedersen_hash, then appends length
pub fn compute_hash_on_elements(data: &[FieldElement]) -> FieldElement {
    let mut result = FieldElement::ZERO;
    for elem in data {
        result = pedersen_hash(&result, elem);
    }
    // Append the length at the end
    let len = FieldElement::from(data.len() as u64);
    pedersen_hash(&result, &len)
}

// =============================================================================
// Order Signing (SDK-compatible - matches sign_auth_message approach)
// =============================================================================

/// Parameters for signing an order message
pub struct OrderSignParams<'a> {
    pub private_key: &'a str,
    pub account_address: &'a str,
    pub market: &'a str,
    pub side: &'a str,        // "BUY" or "SELL"
    pub order_type: &'a str,  // "LIMIT" or "MARKET"
    pub size: &'a str,        // Size as string (quantum with 8 decimals)
    pub price: &'a str,       // Price as string (quantum with 8 decimals or "0" for market)
    pub client_id: &'a str,
    pub timestamp_ms: u64,  // Timestamp in MILLISECONDS
    pub chain_id: &'a str,
}

/// Sign an order message for Paradex using TypedData (SNIP-12)
/// 
/// This function matches the Python SDK's order_sign_message + account.sign_message flow.
/// The Order type has fields: timestamp, market, side, orderType, size, price
pub fn sign_order_message(params: OrderSignParams) -> ExchangeResult<(String, String)> {
    use starknet_signers::SigningKey;
    use starknet_core::types::Felt;
    
    // === Parse inputs using starknet_core::Felt (matching sign_auth_message) ===
    let pk_felt = Felt::from_hex(params.private_key)
        .map_err(|e| ExchangeError::AuthenticationFailed(format!("Invalid private key: {}", e)))?;
    let account_felt = Felt::from_hex(params.account_address)
        .map_err(|e| ExchangeError::AuthenticationFailed(format!("Invalid account: {}", e)))?;
    let chain_felt = cairo_short_string_to_felt(params.chain_id)
        .map_err(|e| ExchangeError::AuthenticationFailed(format!("Invalid chain_id: {}", e)))?;
    
    // === Build Domain hash (same as auth) ===
    let domain_type_hash = starknet_keccak("StarkNetDomain(name:felt,chainId:felt,version:felt)".as_bytes());
    let domain_name = cairo_short_string_to_felt("Paradex")
        .map_err(|e| ExchangeError::AuthenticationFailed(format!("Invalid domain name: {}", e)))?;
    
    let domain_hash = core_compute_hash_on_elements(&[
        domain_type_hash,
        domain_name,
        chain_felt,
        Felt::ONE,  // version = 1
    ]);
    
    // === Build Order struct hash ===
    // Order type: timestamp, market, side, orderType, size, price
    let order_type_hash = starknet_keccak(
        "Order(timestamp:felt,market:felt,side:felt,orderType:felt,size:felt,price:felt)".as_bytes()
    );
    
    // Convert order fields to felts
    // Note: timestamp is in MILLISECONDS
    let timestamp_felt = Felt::from(params.timestamp_ms);
    let market_felt = cairo_short_string_to_felt(params.market)
        .map_err(|e| ExchangeError::AuthenticationFailed(format!("Invalid market: {}", e)))?;
    
    // Side: 1 for BUY, 2 for SELL (matches Python SDK)
    let side_value: u64 = match params.side.to_uppercase().as_str() {
        "BUY" => 1,
        "SELL" => 2,
        _ => return Err(ExchangeError::InvalidOrder(format!("Invalid side: {}", params.side))),
    };
    let side_felt = Felt::from(side_value);
    
    // Order type as short string 
    let order_type_felt = cairo_short_string_to_felt(params.order_type)
        .map_err(|e| ExchangeError::AuthenticationFailed(format!("Invalid order_type: {}", e)))?;
    
    // Size as quantum value - parse string to felt
    // Python uses chain_size() which converts Decimal to quantum (multiply by 10^8)
    let size_felt = parse_decimal_to_felt(params.size)?;
    
    // Price as quantum value (0 for market orders)
    let price_felt = parse_decimal_to_felt(params.price)?;
    
    let order_hash = core_compute_hash_on_elements(&[
        order_type_hash,
        timestamp_felt,
        market_felt,
        side_felt,
        order_type_felt,
        size_felt,
        price_felt,
    ]);
    
    // === Final message hash using compute_hash_on_elements (WITH length prefix!) ===
    let starknet_message_prefix = Felt::from_raw([
        257012186512350467,
        18446744073709551605,
        10480951322775611302,
        16156019428408348868,
    ]);
    
    let final_hash = core_compute_hash_on_elements(&[
        starknet_message_prefix,
        domain_hash,
        account_felt,
        order_hash,
    ]);
    
    // Debug logging
    tracing::debug!("Paradex Order Sign (SDK-compatible):");
    tracing::debug!("  domain_hash: {:?}", domain_hash);
    tracing::debug!("  order_hash: {:?}", order_hash);
    tracing::debug!("  final_hash: {:?}", final_hash);
    
    // === Sign using SigningKey (matching sign_auth_message) ===
    let signing_key = SigningKey::from_secret_scalar(pk_felt);
    
    let signature = signing_key.sign(&final_hash)
        .map_err(|e| ExchangeError::AuthenticationFailed(format!("Signing failed: {}", e)))?;
    
    // Format as DECIMAL strings (not hex!)
    let r_bytes = signature.r.to_bytes_be();
    let s_bytes = signature.s.to_bytes_be();
    let r_decimal = num_bigint::BigUint::from_bytes_be(&r_bytes).to_string();
    let s_decimal = num_bigint::BigUint::from_bytes_be(&s_bytes).to_string();
    
    tracing::debug!("  signature.r (decimal): {}", r_decimal);
    tracing::debug!("  signature.s (decimal): {}", s_decimal);
    
    Ok((r_decimal, s_decimal))
}

/// Parse a decimal string (like "0.001" or "105000") to Felt as quantum value (x 10^8)
pub fn parse_decimal_to_felt(s: &str) -> ExchangeResult<starknet_core::types::Felt> {
    use starknet_core::types::Felt;
    
    // Try parsing as float first
    if let Ok(val) = s.parse::<f64>() {
        // Convert to quantum (multiply by 10^8)
        let quantum = (val * 100_000_000.0).round() as u64;
        return Ok(Felt::from(quantum));
    }
    
    // Try parsing as integer (already quantum)
    if let Ok(val) = s.parse::<u64>() {
        return Ok(Felt::from(val));
    }
    
    Err(ExchangeError::InvalidOrder(format!("Invalid decimal value: {}", s)))
}

/// Compute Starknet selector (type hash) from a type string
/// This mimics starknet's get_selector_from_name which uses keccak256
/// Algorithm: keccak256(name) & ((1 << 250) - 1)
pub fn compute_starknet_selector(name: &str) -> FieldElement {
    use sha3::{Digest, Keccak256};
    
    // Starknet selector is keccak256(name) masked to 250 bits
    let mut hasher = Keccak256::new();
    hasher.update(name.as_bytes());
    let hash = hasher.finalize();
    
    // Convert full 32 bytes to FieldElement, then apply 250-bit mask
    // The hash is already 256 bits, we just zero the top 6 bits
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&hash);
    // Zero the top 6 bits of the first byte (256 - 250 = 6)
    bytes[0] &= 0x03; // Keep only bottom 2 bits of first byte (250 bits total)
    
    FieldElement::from_bytes_be(&bytes).unwrap_or(FieldElement::ZERO)
}

/// Convert a string to FieldElement (short string encoding)
/// Starknet short strings are encoded as felt252 (up to 31 chars)
pub fn string_to_felt(s: &str) -> FieldElement {
    let bytes = s.as_bytes();
    if bytes.len() > 31 {
        // Truncate to 31 bytes for short string
        let truncated = &bytes[..31];
        let mut arr = [0u8; 32];
        arr[32 - truncated.len()..].copy_from_slice(truncated);
        FieldElement::from_bytes_be(&arr).unwrap_or(FieldElement::ZERO)
    } else {
        let mut arr = [0u8; 32];
        arr[32 - bytes.len()..].copy_from_slice(bytes);
        FieldElement::from_bytes_be(&arr).unwrap_or(FieldElement::ZERO)
    }
}

// =============================================================================
// Starknet Address Derivation (CREATE2-like)
// =============================================================================

/// CONTRACT_ADDRESS_PREFIX constant for Starknet address computation
/// This is the felt252 encoding of the string "STARKNET_CONTRACT_ADDRESS"
fn contract_address_prefix() -> FieldElement {
    string_to_felt("STARKNET_CONTRACT_ADDRESS")
}

/// Compute Starknet contract address using CREATE2-like mechanism
/// 
/// Formula: pedersen(pedersen(pedersen(pedersen(pedersen(
///     CONTRACT_ADDRESS_PREFIX, 0), salt), class_hash), calldata_hash), deployer)
/// 
/// For Paradex accounts, deployer is 0 (self-deployed)
pub fn compute_starknet_address(
    class_hash: FieldElement,
    salt: FieldElement,
    constructor_calldata: &[FieldElement],
) -> FieldElement {
    let prefix = contract_address_prefix();
    let deployer = FieldElement::ZERO; // Self-deployed accounts have deployer = 0
    
    // Compute calldata hash using compute_hash_on_elements
    let calldata_hash = compute_hash_on_elements(constructor_calldata);
    
    // Chain the pedersen hashes: prefix -> deployer -> salt -> class_hash -> calldata_hash
    let h1 = pedersen_hash(&prefix, &deployer);
    let h2 = pedersen_hash(&h1, &salt);
    let h3 = pedersen_hash(&h2, &class_hash);
    // Apply 250-bit mask (Starknet addresses are 251 bits, but mod PRIME)
    // The result is already a valid felt, just return it
    pedersen_hash(&h3, &calldata_hash)
}

/// Derive the Paradex L2 account address from a private key and system config
/// 
/// This replicates the Python SDK's account address derivation:
/// 1. Derive public key from private key
/// 2. Build constructor calldata for account proxy
/// 3. Compute contract address using CREATE2
/// 
/// # Arguments
/// * `private_key` - Starknet private key as hex string
/// * `paraclear_account_hash` - Implementation class hash from /system/config
/// * `paraclear_account_proxy_hash` - Proxy class hash from /system/config
/// 
/// # Returns
/// The computed account address as FieldElement
#[tracing::instrument(skip(private_key))]
pub fn derive_account_address(
    private_key: &str,
    paraclear_account_hash: &str,
    paraclear_account_proxy_hash: &str,
) -> ExchangeResult<FieldElement> {
    // Parse private key
    let pk = FieldElement::from_hex_be(private_key)
        .map_err(|e| ExchangeError::AuthenticationFailed(format!("Invalid private key: {}", e)))?;
    
    // Derive public key from private key
    let public_key = starknet_crypto::get_public_key(&pk);
    
    // Parse class hashes
    let account_hash = FieldElement::from_hex_be(paraclear_account_hash)
        .map_err(|e| ExchangeError::AuthenticationFailed(format!("Invalid account hash: {}", e)))?;
    let proxy_hash = FieldElement::from_hex_be(paraclear_account_proxy_hash)
        .map_err(|e| ExchangeError::AuthenticationFailed(format!("Invalid proxy hash: {}", e)))?;
    
    // Get the "initialize" selector
    let initialize_selector = compute_starknet_selector("initialize");
    
    // Build constructor calldata for the account proxy
    // Format: [implementation_hash, initialize_selector, calldata_len, public_key, guardian]
    let calldata = vec![
        account_hash,           // Implementation class hash
        initialize_selector,    // Initialize function selector
        FieldElement::from(2u64), // Calldata length (public_key + guardian)
        public_key,             // L2 public key
        FieldElement::ZERO,     // Guardian (none)
    ];
    
    // Compute the contract address
    // Salt is the public key (as per Python SDK)
    let address = compute_starknet_address(proxy_hash, public_key, &calldata);
    
    tracing::debug!(
        private_key_prefix = %crate::core::logging::sanitize_signature(private_key),
        public_key = %format!("0x{:x}", public_key),
        derived_address = %format!("0x{:x}", address),
        "Address derivation completed"
    );
    
    Ok(address)
}

/// Verify that the provided account address matches the derived address
#[tracing::instrument(skip(private_key), fields(provided = %provided_address))]
pub fn verify_account_address(
    private_key: &str,
    provided_address: &str,
    paraclear_account_hash: &str,
    paraclear_account_proxy_hash: &str,
) -> ExchangeResult<bool> {
    let derived = derive_account_address(
        private_key,
        paraclear_account_hash,
        paraclear_account_proxy_hash,
    )?;
    
    let provided = FieldElement::from_hex_be(provided_address)
        .map_err(|e| ExchangeError::AuthenticationFailed(format!("Invalid address: {}", e)))?;
    
    Ok(derived == provided)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::config::{TEST_PRIVATE_KEY, TEST_ACCOUNT_ADDRESS};

    #[test]
    fn test_current_time_ms() {
        let time = current_time_ms();
        assert!(time > 0);
        assert!(time > 1700000000000); // After Oct 2023
    }

    #[test]
    fn test_build_ws_auth_message() {
        let msg = build_ws_auth_message("test-jwt-token");
        assert!(msg.contains("jsonrpc"));
        assert!(msg.contains("auth"));
        assert!(msg.contains("test-jwt-token"));
    }

    #[test]
    fn test_build_ws_url() {
        let prod = build_ws_url(true);
        assert!(prod.contains("prod"));
        
        let testnet = build_ws_url(false);
        assert!(testnet.contains("testnet"));
    }



    #[test]
    fn test_string_to_felt() {
        let felt = string_to_felt("test");
        assert_ne!(felt, FieldElement::ZERO);
    }

    #[test]
    fn test_compute_starknet_selector() {
        let selector = compute_starknet_selector("initialize");
        assert_ne!(selector, FieldElement::ZERO);
    }

    #[test]
    fn test_sign_auth_message_produces_valid_signature() {
        let timestamp = 1700000000u64;
        let expiration = timestamp + 300;
        let chain_id = "SN_SEPOLIA";
        
        let result = sign_auth_message(
            TEST_PRIVATE_KEY,
            TEST_ACCOUNT_ADDRESS,
            timestamp,
            expiration,
            chain_id,
        );
        
        assert!(result.is_ok());
        let (sig_r, sig_s) = result.unwrap();
        // Signatures should be non-empty decimal strings
        assert!(!sig_r.is_empty());
        assert!(!sig_s.is_empty());
        // Should parse as integers
        assert!(sig_r.parse::<num_bigint::BigUint>().is_ok());
        assert!(sig_s.parse::<num_bigint::BigUint>().is_ok());
    }

    #[test]
    fn test_sign_order_message_produces_valid_signature() {
        let params = OrderSignParams {
            private_key: TEST_PRIVATE_KEY,
            account_address: TEST_ACCOUNT_ADDRESS,
            market: "ETH-USD-PERP",
            side: "BUY",
            order_type: "LIMIT",
            size: "0.1",
            price: "2500.00",
            client_id: "test-order-123",
            timestamp_ms: 1700000000000,
            chain_id: "SN_SEPOLIA",
        };
        
        let result = sign_order_message(params);
        
        assert!(result.is_ok());
        let (sig_r, sig_s) = result.unwrap();
        assert!(!sig_r.is_empty());
        assert!(!sig_s.is_empty());
    }

    #[test]
    fn test_sign_order_message_market_order_with_zero_price() {
        let params = OrderSignParams {
            private_key: TEST_PRIVATE_KEY,
            account_address: TEST_ACCOUNT_ADDRESS,
            market: "BTC-USD-PERP",
            side: "SELL",
            order_type: "MARKET",
            size: "0.01",
            price: "0",  // Market orders have price 0
            client_id: "test-market-order",
            timestamp_ms: 1700000000000,
            chain_id: "SN_SEPOLIA",
        };
        
        let result = sign_order_message(params);
        assert!(result.is_ok());
    }
}
