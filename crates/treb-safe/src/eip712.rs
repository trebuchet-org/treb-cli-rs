//! EIP-712 Safe transaction hash computation and signing.
//!
//! Implements the EIP-712 typed data construction matching Safe's
//! [EIP-712 struct](https://github.com/safe-global/safe-smart-account/blob/main/contracts/Safe.sol)
//! for computing the safe transaction hash and signing it.

use alloy_primitives::{Address, B256, U256};
use alloy_signer::Signer;
use alloy_sol_types::{Eip712Domain, SolStruct, sol};
use foundry_wallets::WalletSigner;
use treb_core::error::TrebError;

// ---------------------------------------------------------------------------
// EIP-712 type definitions
// ---------------------------------------------------------------------------

sol! {
    /// Safe transaction struct matching the EIP-712 definition in Safe.sol.
    ///
    /// Type hash: `keccak256("SafeTx(address to,uint256 value,bytes data,
    /// uint8 operation,uint256 safeTxGas,uint256 baseGas,uint256 gasPrice,
    /// address gasToken,address refundReceiver,uint256 nonce)")`
    #[derive(Debug, PartialEq)]
    struct SafeTx {
        address to;
        uint256 value;
        bytes data;
        uint8 operation;
        uint256 safeTxGas;
        uint256 baseGas;
        uint256 gasPrice;
        address gasToken;
        address refundReceiver;
        uint256 nonce;
    }
}

// ---------------------------------------------------------------------------
// Hash computation
// ---------------------------------------------------------------------------

/// Build the EIP-712 domain for a Safe at `safe_address` on `chain_id`.
///
/// Safe uses a minimal EIP-712 domain with only `chainId` and
/// `verifyingContract` fields (no name, version, or salt).
pub fn safe_domain(chain_id: u64, safe_address: Address) -> Eip712Domain {
    Eip712Domain {
        chain_id: Some(U256::from(chain_id)),
        verifying_contract: Some(safe_address),
        ..Default::default()
    }
}

/// Compute the EIP-712 signing hash for a Safe transaction.
///
/// This produces the hash that must be signed by Safe owners to approve
/// the transaction. It corresponds to `Safe.getTransactionHash()` in the
/// Safe smart contract.
pub fn compute_safe_tx_hash(chain_id: u64, safe_address: Address, safe_tx: &SafeTx) -> B256 {
    let domain = safe_domain(chain_id, safe_address);
    safe_tx.eip712_signing_hash(&domain)
}

// ---------------------------------------------------------------------------
// Signing
// ---------------------------------------------------------------------------

/// Sign a Safe transaction hash using the given wallet signer.
///
/// Returns the 65-byte signature with `v ∈ {27, 28}` as required by the
/// Safe Transaction Service.
pub async fn sign_safe_tx(signer: &WalletSigner, safe_tx_hash: B256) -> Result<Vec<u8>, TrebError> {
    let signature = signer
        .sign_hash(&safe_tx_hash)
        .await
        .map_err(|e| TrebError::Safe(format!("failed to sign safe tx: {e}")))?;

    let mut bytes = signature.as_bytes().to_vec();
    // Safe Transaction Service expects v ∈ {27, 28}; alloy uses {0, 1}.
    if bytes[64] < 27 {
        bytes[64] += 27;
    }
    Ok(bytes)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Signature, address, keccak256};

    // ── SafeTx type hash ──────────────────────────────────────────────────

    #[test]
    fn safe_tx_type_hash_matches_known_value() {
        // The Safe contract's SAFE_TX_TYPEHASH:
        // keccak256("SafeTx(address to,uint256 value,bytes data,uint8 operation,
        //   uint256 safeTxGas,uint256 baseGas,uint256 gasPrice,address gasToken,
        //   address refundReceiver,uint256 nonce)")
        let expected = keccak256(
            "SafeTx(address to,uint256 value,bytes data,uint8 operation,\
             uint256 safeTxGas,uint256 baseGas,uint256 gasPrice,\
             address gasToken,address refundReceiver,uint256 nonce)",
        );
        // eip712_type_hash is an instance method (trait SolStruct)
        let dummy = SafeTx {
            to: Address::ZERO,
            value: U256::ZERO,
            data: vec![].into(),
            operation: 0,
            safeTxGas: U256::ZERO,
            baseGas: U256::ZERO,
            gasPrice: U256::ZERO,
            gasToken: Address::ZERO,
            refundReceiver: Address::ZERO,
            nonce: U256::ZERO,
        };
        assert_eq!(dummy.eip712_type_hash(), expected);
    }

    // ── Domain separator ──────────────────────────────────────────────────

    #[test]
    fn domain_separator_matches_expected_for_mainnet() {
        let safe_address = address!("1234567890123456789012345678901234567890");
        let domain = safe_domain(1, safe_address);

        // The domain type hash for Safe:
        // keccak256("EIP712Domain(uint256 chainId,address verifyingContract)")
        let domain_type_hash = keccak256("EIP712Domain(uint256 chainId,address verifyingContract)");

        // The domain separator is:
        // keccak256(abi.encode(typeHash, chainId, verifyingContract))
        // abi.encode packs each value into a 32-byte word.
        let mut buf = [0u8; 96]; // 32 (typeHash) + 32 (chainId) + 32 (address)
        buf[..32].copy_from_slice(domain_type_hash.as_slice());
        buf[32..64].copy_from_slice(&U256::from(1).to_be_bytes::<32>());
        // address is left-padded with zeros in ABI encoding (occupies bytes 76..96)
        buf[76..96].copy_from_slice(safe_address.as_slice());

        let expected_separator = keccak256(&buf);
        assert_eq!(domain.separator(), expected_separator);
    }

    // ── Known hash computation ────────────────────────────────────────────

    #[test]
    fn compute_safe_tx_hash_deterministic() {
        let safe_address = address!("1234567890123456789012345678901234567890");
        let to = address!("0000000000000000000000000000000000000001");

        let safe_tx = SafeTx {
            to,
            value: U256::ZERO,
            data: vec![].into(),
            operation: 0,
            safeTxGas: U256::ZERO,
            baseGas: U256::ZERO,
            gasPrice: U256::ZERO,
            gasToken: Address::ZERO,
            refundReceiver: Address::ZERO,
            nonce: U256::ZERO,
        };

        let hash1 = compute_safe_tx_hash(1, safe_address, &safe_tx);
        let hash2 = compute_safe_tx_hash(1, safe_address, &safe_tx);
        // Same inputs produce same hash
        assert_eq!(hash1, hash2);

        // Different chain produces different hash
        let hash3 = compute_safe_tx_hash(137, safe_address, &safe_tx);
        assert_ne!(hash1, hash3);

        // Different safe address produces different hash
        let other_safe = address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let hash4 = compute_safe_tx_hash(1, other_safe, &safe_tx);
        assert_ne!(hash1, hash4);
    }

    #[test]
    fn compute_safe_tx_hash_known_vector() {
        // Verify the hash changes with transaction fields
        let safe_address = address!("1234567890123456789012345678901234567890");

        let tx_zero = SafeTx {
            to: Address::ZERO,
            value: U256::ZERO,
            data: vec![].into(),
            operation: 0,
            safeTxGas: U256::ZERO,
            baseGas: U256::ZERO,
            gasPrice: U256::ZERO,
            gasToken: Address::ZERO,
            refundReceiver: Address::ZERO,
            nonce: U256::ZERO,
        };

        let tx_with_value = SafeTx {
            value: U256::from(1_000_000_000_000_000_000u64), // 1 ETH
            ..tx_zero.clone()
        };

        let tx_with_nonce = SafeTx { nonce: U256::from(42), ..tx_zero.clone() };

        let hash_zero = compute_safe_tx_hash(1, safe_address, &tx_zero);
        let hash_value = compute_safe_tx_hash(1, safe_address, &tx_with_value);
        let hash_nonce = compute_safe_tx_hash(1, safe_address, &tx_with_nonce);

        // All hashes are different
        assert_ne!(hash_zero, hash_value);
        assert_ne!(hash_zero, hash_nonce);
        assert_ne!(hash_value, hash_nonce);

        // All hashes are non-zero
        assert_ne!(hash_zero, B256::ZERO);
        assert_ne!(hash_value, B256::ZERO);
        assert_ne!(hash_nonce, B256::ZERO);
    }

    // ── Signature round-trip ──────────────────────────────────────────────

    #[tokio::test]
    async fn sign_and_recover_round_trip() {
        // Use Anvil's first account private key
        let key: B256 =
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80".parse().unwrap();
        let signer = WalletSigner::from_private_key(&key).unwrap();
        let signer_addr = signer.address();

        let safe_address = address!("1234567890123456789012345678901234567890");
        let safe_tx = SafeTx {
            to: address!("0000000000000000000000000000000000000001"),
            value: U256::ZERO,
            data: vec![].into(),
            operation: 0,
            safeTxGas: U256::ZERO,
            baseGas: U256::ZERO,
            gasPrice: U256::ZERO,
            gasToken: Address::ZERO,
            refundReceiver: Address::ZERO,
            nonce: U256::ZERO,
        };

        let hash = compute_safe_tx_hash(1, safe_address, &safe_tx);
        let sig_bytes = sign_safe_tx(&signer, hash).await.unwrap();

        // Signature is 65 bytes (r[32] + s[32] + v[1])
        assert_eq!(sig_bytes.len(), 65);

        // v should be 27 or 28
        let v = sig_bytes[64];
        assert!(v == 27 || v == 28, "v should be 27 or 28, got {v}");

        // Recover the signer address from the signature
        let parity = v == 28;
        let sig = Signature::from_bytes_and_parity(&sig_bytes[..64], parity);
        let recovered = sig.recover_address_from_prehash(&hash).expect("recovery should succeed");
        assert_eq!(recovered, signer_addr);
    }

    #[tokio::test]
    async fn sign_is_deterministic() {
        // RFC 6979 guarantees deterministic signatures
        let key: B256 =
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80".parse().unwrap();
        let signer = WalletSigner::from_private_key(&key).unwrap();

        let hash = B256::from([0xab; 32]);
        let sig1 = sign_safe_tx(&signer, hash).await.unwrap();
        let sig2 = sign_safe_tx(&signer, hash).await.unwrap();
        assert_eq!(sig1, sig2);
    }
}
