//! Vest Signing
//!
//! EIP-712 signing logic for Vest authentication and order signing.

use ethers::contract::EthAbiType;
use ethers::core::types::transaction::eip712::{EIP712Domain, Eip712};
use ethers::core::types::{Address, U256};
use ethers::signers::{LocalWallet, Signer};
use serde::Serialize;

use crate::adapters::errors::{ExchangeError, ExchangeResult};
use crate::adapters::types::OrderRequest;

use super::config::VestConfig;

// Re-export canonical timestamp function (defined in core::events)
pub use crate::core::events::current_timestamp_ms as current_time_ms;

/// Generate expiry timestamp (7 days from now)
pub fn expiry_7_days_ms() -> u64 {
    current_time_ms().saturating_add(7 * 24 * 3600 * 1000)
}

// =============================================================================
// EIP-712 Types for Vest Authentication
// =============================================================================

/// SignerProof type for EIP-712 signature
/// This struct is signed by the PRIMARY wallet to authorize the signing key as delegate.
#[derive(Debug, Clone, Serialize, EthAbiType)]
pub(crate) struct SignerProof {
    /// The address being authorized to sign on behalf of primary
    pub approved_signer: Address,
    /// Expiry timestamp in Unix milliseconds
    pub signer_expiry: U256,
}

impl Eip712 for SignerProof {
    type Error = std::convert::Infallible;

    fn domain(&self) -> Result<EIP712Domain, Self::Error> {
        // Domain struct for EIP-712 - verifying_contract is set dynamically in sign_registration_proof
        Ok(EIP712Domain {
            name: Some("VestRouterV2".into()),
            version: Some("0.0.1".into()),
            chain_id: None,
            verifying_contract: None,
            salt: None,
        })
    }

    fn type_hash() -> Result<[u8; 32], Self::Error> {
        // keccak256("SignerProof(address approvedSigner,uint256 signerExpiry)")
        use ethers::core::utils::keccak256;
        Ok(keccak256(
            "SignerProof(address approvedSigner,uint256 signerExpiry)",
        ))
    }

    fn struct_hash(&self) -> Result<[u8; 32], Self::Error> {
        use ethers::abi::{encode, Token};
        use ethers::core::utils::keccak256;

        let type_hash = Self::type_hash()?;
        let encoded = encode(&[
            Token::FixedBytes(type_hash.to_vec()),
            Token::Address(self.approved_signer),
            Token::Uint(self.signer_expiry),
        ]);
        Ok(keccak256(&encoded))
    }
}

// =============================================================================
// Signing Functions
// =============================================================================

/// Generate EIP-712 signature for registration
///
/// Signs the SignerProof struct with the PRIMARY key to authorize
/// the signing key as a delegate using proper EIP-712 typed data signing.
pub async fn sign_registration_proof(config: &VestConfig) -> ExchangeResult<(String, String, u64)> {
    // Parse the signing key to get its address
    let signing_wallet: LocalWallet = config
        .signing_key
        .parse()
        .map_err(|e| ExchangeError::AuthenticationFailed(format!("Invalid signing key: {}", e)))?;

    // Parse the primary key for signing
    let primary_wallet: LocalWallet = config
        .primary_key
        .parse()
        .map_err(|e| ExchangeError::AuthenticationFailed(format!("Invalid primary key: {}", e)))?;

    let expiry = expiry_7_days_ms();
    let signer_address = signing_wallet.address();

    // Create the SignerProof struct for EIP-712 signing
    let proof = SignerProof {
        approved_signer: signer_address,
        signer_expiry: U256::from(expiry),
    };

    // Build EIP-712 domain with verifying contract
    let verifying_contract: Address = config.verifying_contract().parse().map_err(|e| {
        ExchangeError::AuthenticationFailed(format!("Invalid verifying contract: {}", e))
    })?;

    // Compute the EIP-712 hash manually
    use ethers::abi::{encode, Token};
    use ethers::core::utils::keccak256;

    // Domain separator hash
    let domain_type_hash =
        keccak256("EIP712Domain(string name,string version,address verifyingContract)");
    let domain_encoded = encode(&[
        Token::FixedBytes(domain_type_hash.to_vec()),
        Token::FixedBytes(keccak256("VestRouterV2").to_vec()),
        Token::FixedBytes(keccak256("0.0.1").to_vec()),
        Token::Address(verifying_contract),
    ]);
    let domain_separator = keccak256(&domain_encoded);

    // Struct hash
    let struct_hash = proof
        .struct_hash()
        .map_err(|_| ExchangeError::AuthenticationFailed("Failed to compute struct hash".into()))?;

    // EIP-712 final hash: keccak256("\x19\x01" + domainSeparator + structHash)
    let mut data = Vec::with_capacity(66);
    data.push(0x19);
    data.push(0x01);
    data.extend_from_slice(&domain_separator);
    data.extend_from_slice(&struct_hash);
    let final_hash = keccak256(&data);

    // Sign the EIP-712 hash with primary wallet
    let signature = primary_wallet.sign_hash(final_hash.into()).map_err(|e| {
        ExchangeError::AuthenticationFailed(format!("EIP-712 signing failed: {}", e))
    })?;

    // Signature should be exactly 65 bytes (r: 32, s: 32, v: 1)
    let mut sig_bytes = signature.to_vec();
    if sig_bytes.len() != 65 {
        return Err(ExchangeError::AuthenticationFailed(format!(
            "Invalid signature length: {} (expected 65)",
            sig_bytes.len()
        )));
    }

    // Normalize v value: ethers-rs may return 0/1, but Vest expects 27/28
    let v = sig_bytes[64];
    if v == 0 || v == 1 {
        sig_bytes[64] = v + 27;
    } else if !matches!(v, 27 | 28) {
        return Err(ExchangeError::AuthenticationFailed(format!(
            "Invalid signature v value: {} (expected 0, 1, 27, or 28)",
            v
        )));
    }

    let sig_hex = format!("0x{}", hex::encode(sig_bytes));
    let signer_hex = format!("{:?}", signer_address).to_lowercase();

    Ok((sig_hex, signer_hex, expiry))
}

/// Sign an order using Vest's signature format
/// Returns (signature_hex, time, nonce)
///
/// Vest format: keccak256(encode([time, nonce, orderType, symbol, isBuy, size, limitPrice, reduceOnly]))
/// Then sign with personal_sign (encode_defunct)
pub async fn sign_order(
    config: &VestConfig,
    order: &OrderRequest,
) -> ExchangeResult<(String, u64, u64)> {
    use crate::adapters::types::{OrderSide, OrderType};
    use ethers::abi::{encode, Token};
    use ethers::core::utils::keccak256;

    // Parse the signing key (delegate signer for orders)
    let signing_wallet: LocalWallet = config
        .signing_key
        .parse()
        .map_err(|e| ExchangeError::AuthenticationFailed(format!("Invalid signing key: {}", e)))?;

    // time = current timestamp in ms, nonce = time (as recommended by Vest docs)
    let time = current_time_ms();
    let nonce: u64 = time; // Vest recommends using time as nonce

    // Order type string
    let order_type = match order.order_type {
        OrderType::Limit => "LIMIT",
        OrderType::Market => "MARKET",
    };

    // isBuy as bool
    let is_buy = matches!(order.side, OrderSide::Buy);

    // Size and price as strings - MUST match exactly with place_order payload format
    // Precision is market-specific, from Vest /exchangeInfo (sizeDecimals, priceDecimals)
    let size_str = super::adapter::format_vest_size(order.quantity, &order.symbol);
    let limit_price_str = order
        .price
        .map(|p| super::adapter::format_vest_price(p, &order.symbol))
        .unwrap_or_else(|| super::adapter::format_vest_price(0.0, &order.symbol));

    // reduceOnly - from order request (true for closing positions)
    let reduce_only = order.reduce_only;

    // Encode: ["uint256", "uint256", "string", "string", "bool", "string", "string", "bool"]
    // Values: [time, nonce, orderType, symbol, isBuy, size, limitPrice, reduceOnly]
    let encoded = encode(&[
        Token::Uint(U256::from(time)),
        Token::Uint(U256::from(nonce)),
        Token::String(order_type.to_string()),
        Token::String(order.symbol.clone()),
        Token::Bool(is_buy),
        Token::String(size_str),
        Token::String(limit_price_str),
        Token::Bool(reduce_only),
    ]);

    // Hash the encoded data
    let msg_hash = keccak256(&encoded);

    // Sign using personal_sign (encode_defunct equivalent)
    let signature = signing_wallet
        .sign_message(msg_hash)
        .await
        .map_err(|e| ExchangeError::AuthenticationFailed(format!("Order signing failed: {}", e)))?;

    let sig_bytes = signature.to_vec();
    if sig_bytes.len() != 65 {
        return Err(ExchangeError::AuthenticationFailed(format!(
            "Invalid signature length: {} (expected 65)",
            sig_bytes.len()
        )));
    }

    let sig_hex = format!("0x{}", hex::encode(sig_bytes));
    Ok((sig_hex, time, nonce))
}

/// Sign a cancel order request with EIP-712
pub async fn sign_cancel_order(
    config: &VestConfig,
    order_id: &str,
    nonce: u64,
) -> ExchangeResult<String> {
    use ethers::abi::{encode, Token};
    use ethers::core::utils::keccak256;

    let signing_wallet: LocalWallet = config
        .signing_key
        .parse()
        .map_err(|e| ExchangeError::AuthenticationFailed(format!("Invalid signing key: {}", e)))?;

    // Cancel type hash
    let cancel_type_hash = keccak256("CancelOrder(string orderId,uint256 nonce)");

    let struct_encoded = encode(&[
        Token::FixedBytes(cancel_type_hash.to_vec()),
        Token::FixedBytes(keccak256(order_id.as_bytes()).to_vec()),
        Token::Uint(U256::from(nonce)),
    ]);
    let struct_hash = keccak256(&struct_encoded);

    // Domain separator
    let verifying_contract: Address = config.verifying_contract().parse().map_err(|e| {
        ExchangeError::AuthenticationFailed(format!("Invalid verifying contract: {}", e))
    })?;

    let domain_type_hash =
        keccak256("EIP712Domain(string name,string version,address verifyingContract)");
    let domain_encoded = encode(&[
        Token::FixedBytes(domain_type_hash.to_vec()),
        Token::FixedBytes(keccak256("VestRouterV2").to_vec()),
        Token::FixedBytes(keccak256("0.0.1").to_vec()),
        Token::Address(verifying_contract),
    ]);
    let domain_separator = keccak256(&domain_encoded);

    // Final hash
    let mut data = Vec::with_capacity(66);
    data.push(0x19);
    data.push(0x01);
    data.extend_from_slice(&domain_separator);
    data.extend_from_slice(&struct_hash);
    let final_hash = keccak256(&data);

    let signature = signing_wallet.sign_hash(final_hash.into()).map_err(|e| {
        ExchangeError::AuthenticationFailed(format!("Cancel signing failed: {}", e))
    })?;

    // Validate signature length
    let sig_bytes = signature.to_vec();
    if sig_bytes.len() != 65 {
        return Err(ExchangeError::AuthenticationFailed(format!(
            "Invalid cancel signature length: {} (expected 65)",
            sig_bytes.len()
        )));
    }

    let sig_hex = format!("0x{}", hex::encode(sig_bytes));
    Ok(sig_hex)
}

/// Sign a leverage request using Vest's signature format
/// Returns (signature_hex, time, nonce)
pub async fn sign_leverage_request(
    config: &VestConfig,
    symbol: &str,
    leverage: u32,
) -> ExchangeResult<(String, u64, u64)> {
    use ethers::abi::{encode, Token};
    use ethers::core::utils::keccak256;

    let signing_wallet: LocalWallet = config
        .signing_key
        .parse()
        .map_err(|e| ExchangeError::AuthenticationFailed(format!("Invalid signing key: {}", e)))?;

    let time = current_time_ms();
    let nonce: u64 = time;

    // Encode: ["uint256", "uint256", "string", "uint256"]
    // Values: [time, nonce, symbol, leverage]
    let encoded = encode(&[
        Token::Uint(U256::from(time)),
        Token::Uint(U256::from(nonce)),
        Token::String(symbol.to_string()),
        Token::Uint(U256::from(leverage)),
    ]);

    let msg_hash = keccak256(&encoded);

    let signature = signing_wallet.sign_message(msg_hash).await.map_err(|e| {
        ExchangeError::AuthenticationFailed(format!("Leverage signing failed: {}", e))
    })?;

    let sig_bytes = signature.to_vec();
    if sig_bytes.len() != 65 {
        return Err(ExchangeError::AuthenticationFailed(format!(
            "Invalid signature length: {} (expected 65)",
            sig_bytes.len()
        )));
    }

    let sig_hex = format!("0x{}", hex::encode(sig_bytes));
    Ok((sig_hex, time, nonce))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::types::{OrderSide, OrderType};
    use crate::adapters::vest::config::{TEST_PRIMARY_ADDR, TEST_PRIMARY_KEY, TEST_SIGNING_KEY};

    #[test]
    fn test_current_time_ms() {
        let time = current_time_ms();
        assert!(time > 1700000000000); // Should be after 2023
    }

    #[test]
    fn test_expiry_7_days_ms() {
        let now = current_time_ms();
        let expiry = expiry_7_days_ms();
        assert!(expiry > now);
        assert!(expiry - now >= 7 * 24 * 3600 * 1000 - 1000); // Allow 1s tolerance
    }

    #[tokio::test]
    async fn test_sign_registration_proof() {
        let config = VestConfig {
            primary_addr: TEST_PRIMARY_ADDR.to_string(),
            primary_key: TEST_PRIMARY_KEY.to_string(),
            signing_key: TEST_SIGNING_KEY.to_string(),
            account_group: 0,
            production: false,
        };

        let result = sign_registration_proof(&config).await;
        assert!(result.is_ok());
        let (sig, signer, expiry) = result.unwrap();
        assert!(sig.starts_with("0x"));
        assert!(sig.len() == 132); // 0x + 130 hex chars (65 bytes)
        assert!(signer.starts_with("0x"));
        assert!(expiry > current_time_ms());
    }

    #[tokio::test]
    async fn test_sign_order() {
        let config = VestConfig {
            primary_addr: TEST_PRIMARY_ADDR.to_string(),
            primary_key: TEST_PRIMARY_KEY.to_string(),
            signing_key: TEST_SIGNING_KEY.to_string(),
            account_group: 0,
            production: false,
        };

        let order = OrderRequest {
            symbol: "BTC-PERP".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            quantity: 0.001,
            price: Some(50000.0),
            reduce_only: false,
            client_order_id: "test-123".to_string(),
            time_in_force: crate::adapters::types::TimeInForce::Gtc,
        };

        let result = sign_order(&config, &order).await;
        assert!(result.is_ok());
        let (sig, time, nonce) = result.unwrap();
        assert!(sig.starts_with("0x"));
        assert!(sig.len() == 132);
        assert_eq!(time, nonce); // Vest uses time as nonce
    }
}
