//! ABI encoding of sender configurations for Solidity consumption.
//!
//! Converts resolved sender configurations into the `SenderInitConfig[]`
//! ABI-encoded hex string expected by the `SENDER_CONFIGS` environment
//! variable. The Solidity side decodes this via
//! `abi.decode(vm.envBytes("SENDER_CONFIGS"), (Senders.SenderInitConfig[]))`.
//!
//! Sender type magic constants are computed as `bytes8(keccak256(typeString))`
//! with composite types formed by bitwise OR, matching the constants in
//! `lib/treb-sol/src/internal/types.sol`.

use std::collections::HashMap;

use alloy_primitives::{Address, FixedBytes, keccak256};
use alloy_sol_types::{SolValue, sol};
use treb_config::{SenderConfig, SenderType};
use treb_core::error::TrebError;

use crate::sender::ResolvedSender;

// ---------------------------------------------------------------------------
// ABI struct definition matching Senders.SenderInitConfig in Solidity
// ---------------------------------------------------------------------------

sol! {
    /// Matches `Senders.SenderInitConfig` in `lib/treb-sol/src/internal/sender/Senders.sol`.
    struct SenderInitConfig {
        string name;
        address account;
        bytes8 senderType;
        bool canBroadcast;
        bytes config;
    }
}

// ---------------------------------------------------------------------------
// Sender type magic constants — bytes8(keccak256(typeString))
// ---------------------------------------------------------------------------

/// Compute the `bytes8` hash for a type string (first 8 bytes of keccak256).
fn calculate_bytes8(type_string: &str) -> FixedBytes<8> {
    let hash = keccak256(type_string);
    FixedBytes::from_slice(&hash[..8])
}

/// Perform bitwise OR on two `bytes8` values.
fn bitwise_or(a: FixedBytes<8>, b: FixedBytes<8>) -> FixedBytes<8> {
    let mut result = [0u8; 8];
    for i in 0..8 {
        result[i] = a[i] | b[i];
    }
    FixedBytes::from(result)
}

/// Sender type constants matching `SenderTypes` in `types.sol`.
struct SenderTypes;

impl SenderTypes {
    fn private_key() -> FixedBytes<8> {
        calculate_bytes8("private-key")
    }

    fn in_memory() -> FixedBytes<8> {
        bitwise_or(calculate_bytes8("in-memory"), Self::private_key())
    }

    fn multisig() -> FixedBytes<8> {
        calculate_bytes8("multisig")
    }

    fn gnosis_safe() -> FixedBytes<8> {
        bitwise_or(Self::multisig(), calculate_bytes8("gnosis-safe"))
    }

    fn hardware_wallet() -> FixedBytes<8> {
        bitwise_or(calculate_bytes8("hardware-wallet"), Self::private_key())
    }

    fn ledger() -> FixedBytes<8> {
        bitwise_or(calculate_bytes8("ledger"), Self::hardware_wallet())
    }

    fn trezor() -> FixedBytes<8> {
        bitwise_or(calculate_bytes8("trezor"), Self::hardware_wallet())
    }

    fn governance() -> FixedBytes<8> {
        calculate_bytes8("governance")
    }

    fn oz_governor() -> FixedBytes<8> {
        bitwise_or(Self::governance(), calculate_bytes8("oz-governor"))
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Encode sender names and addresses for Solidity consumption.
///
/// Produces a `0x`-prefixed hex string containing ABI-encoded
/// `(string[] names, address[] addrs)` for the `SENDER_CONFIGS`
/// environment variable. Type-specific configuration (private keys,
/// signer references) stays entirely in Rust.
pub fn encode_sender_configs(
    resolved_senders: &HashMap<String, ResolvedSender>,
) -> String {
    // Use broadcast_address() — for Governor+timelock, this returns the
    // timelock address (the on-chain executor), so the Solidity side calls
    // vm.broadcast(timelockAddress) instead of vm.broadcast(governorAddress).
    let mut pairs: Vec<(String, Address)> = resolved_senders
        .iter()
        .map(|(name, sender)| (name.clone(), sender.broadcast_address()))
        .collect();
    // Sort by name for deterministic output
    pairs.sort_by(|a, b| a.0.cmp(&b.0));

    let names: Vec<String> = pairs.iter().map(|(n, _)| n.clone()).collect();
    let addrs: Vec<Address> = pairs.iter().map(|(_, a)| *a).collect();

    let encoded = (names, addrs).abi_encode_params();
    format!("0x{}", alloy_primitives::hex::encode(&encoded))
}

/// Build a single `SenderInitConfig` from a resolved sender and its config.
fn build_sender_init_config(
    name: &str,
    resolved: &ResolvedSender,
    sender_config: &SenderConfig,
    all_configs: &HashMap<String, SenderConfig>,
) -> treb_core::Result<SenderInitConfig> {
    let sender_type = sender_config.type_.as_ref().ok_or_else(|| {
        TrebError::Config(format!("sender '{name}' is missing required 'type' field"))
    })?;

    match sender_type {
        SenderType::PrivateKey => build_private_key_config(name, resolved, sender_config),
        SenderType::Ledger => build_ledger_config(name, resolved, sender_config),
        SenderType::Trezor => build_trezor_config(name, resolved, sender_config),
        SenderType::Safe => build_safe_config(name, resolved, sender_config, all_configs),
        SenderType::Governance => {
            build_governance_config(name, resolved, sender_config, all_configs)
        }
    }
}

// ---------------------------------------------------------------------------
// Per-type config builders
// ---------------------------------------------------------------------------

/// Build config for a `private_key` sender.
///
/// Maps to `SenderTypes.InMemory` in Solidity (same as Go's behavior).
/// Config bytes contain the private key ABI-encoded as `uint256`.
fn build_private_key_config(
    name: &str,
    resolved: &ResolvedSender,
    sender_config: &SenderConfig,
) -> treb_core::Result<SenderInitConfig> {
    let key_hex = sender_config.private_key.as_deref().ok_or_else(|| {
        TrebError::Config(format!(
            "sender '{name}': private_key sender missing 'private_key' field"
        ))
    })?;

    // Parse the private key as a U256 for ABI encoding
    let key_hex_clean = key_hex.strip_prefix("0x").unwrap_or(key_hex);
    let key_bytes: [u8; 32] = alloy_primitives::hex::decode(key_hex_clean)
        .map_err(|e| TrebError::Config(format!("sender '{name}': invalid private key hex: {e}")))?
        .try_into()
        .map_err(|_| {
            TrebError::Config(format!("sender '{name}': private key must be exactly 32 bytes"))
        })?;
    let key_u256 = alloy_primitives::U256::from_be_bytes(key_bytes);

    // ABI-encode the private key as uint256
    let config_bytes = key_u256.abi_encode();

    Ok(SenderInitConfig {
        name: name.to_string(),
        account: resolved.sender_address(),
        senderType: SenderTypes::in_memory(),
        canBroadcast: true,
        config: config_bytes.into(),
    })
}

/// Build config for a `ledger` sender.
///
/// Config bytes contain the derivation path ABI-encoded as `string`.
fn build_ledger_config(
    name: &str,
    resolved: &ResolvedSender,
    sender_config: &SenderConfig,
) -> treb_core::Result<SenderInitConfig> {
    let derivation_path = sender_config.derivation_path.as_deref().unwrap_or("");

    // ABI-encode the derivation path as string
    let config_bytes = derivation_path.to_string().abi_encode();

    Ok(SenderInitConfig {
        name: name.to_string(),
        account: resolved.sender_address(),
        senderType: SenderTypes::ledger(),
        canBroadcast: true,
        config: config_bytes.into(),
    })
}

/// Build config for a `trezor` sender.
///
/// Config bytes contain the derivation path ABI-encoded as `string`.
fn build_trezor_config(
    name: &str,
    resolved: &ResolvedSender,
    sender_config: &SenderConfig,
) -> treb_core::Result<SenderInitConfig> {
    let derivation_path = sender_config.derivation_path.as_deref().unwrap_or("");

    // ABI-encode the derivation path as string
    let config_bytes = derivation_path.to_string().abi_encode();

    Ok(SenderInitConfig {
        name: name.to_string(),
        account: resolved.sender_address(),
        senderType: SenderTypes::trezor(),
        canBroadcast: true,
        config: config_bytes.into(),
    })
}

/// Build config for a `safe` sender.
///
/// Config bytes contain the signer (proposer) name ABI-encoded as `string`.
fn build_safe_config(
    name: &str,
    resolved: &ResolvedSender,
    sender_config: &SenderConfig,
    all_configs: &HashMap<String, SenderConfig>,
) -> treb_core::Result<SenderInitConfig> {
    let signer_name = sender_config.signer.as_deref().ok_or_else(|| {
        TrebError::Config(format!("sender '{name}': safe sender missing 'signer' field"))
    })?;

    // Validate signer exists
    if !all_configs.contains_key(signer_name) {
        return Err(TrebError::Config(format!(
            "sender '{name}': safe signer '{signer_name}' not found in sender configurations"
        )));
    }

    // ABI-encode the signer name as string
    let config_bytes = signer_name.to_string().abi_encode();

    Ok(SenderInitConfig {
        name: name.to_string(),
        account: resolved.sender_address(),
        senderType: SenderTypes::gnosis_safe(),
        canBroadcast: true,
        config: config_bytes.into(),
    })
}

/// Build config for a `governance` sender.
///
/// Config bytes contain `(address governor, address timelock, string proposerName)`
/// ABI-encoded as a tuple.
fn build_governance_config(
    name: &str,
    resolved: &ResolvedSender,
    sender_config: &SenderConfig,
    all_configs: &HashMap<String, SenderConfig>,
) -> treb_core::Result<SenderInitConfig> {
    let governor_addr: Address = sender_config
        .address
        .as_deref()
        .ok_or_else(|| {
            TrebError::Config(format!(
                "sender '{name}': governance sender missing 'address' field"
            ))
        })?
        .parse()
        .map_err(|e| {
            TrebError::Config(format!("sender '{name}': invalid governance address: {e}"))
        })?;

    let timelock_addr: Address = sender_config
        .timelock
        .as_deref()
        .map(|addr| {
            addr.parse().map_err(|e| {
                TrebError::Config(format!("sender '{name}': invalid timelock address: {e}"))
            })
        })
        .transpose()?
        .unwrap_or(Address::ZERO);

    let proposer_name = sender_config.proposer.as_deref().ok_or_else(|| {
        TrebError::Config(format!(
            "sender '{name}': governance sender missing 'proposer' field"
        ))
    })?;

    // Validate proposer exists
    if !all_configs.contains_key(proposer_name) {
        return Err(TrebError::Config(format!(
            "sender '{name}': governance proposer '{proposer_name}' not found in sender configurations"
        )));
    }

    // Account is the broadcast address (timelock if present, else governor)
    let account = resolved.broadcast_address();

    // ABI-encode (address governor, address timelock, string proposerName)
    let config_bytes = (governor_addr, timelock_addr, proposer_name.to_string()).abi_encode();

    Ok(SenderInitConfig {
        name: name.to_string(),
        account,
        senderType: SenderTypes::oz_governor(),
        canBroadcast: true,
        config: config_bytes.into(),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, FixedBytes, address, keccak256};

    // ── Sender type constant tests ────────────────────────────────────

    #[test]
    fn private_key_type_matches_solidity() {
        let expected = FixedBytes::from_slice(&keccak256("private-key")[..8]);
        assert_eq!(SenderTypes::private_key(), expected);
    }

    #[test]
    fn in_memory_type_is_or_of_in_memory_and_private_key() {
        let in_memory_hash = FixedBytes::<8>::from_slice(&keccak256("in-memory")[..8]);
        let pk_hash = FixedBytes::<8>::from_slice(&keccak256("private-key")[..8]);
        let expected = bitwise_or(in_memory_hash, pk_hash);
        assert_eq!(SenderTypes::in_memory(), expected);
    }

    #[test]
    fn gnosis_safe_type_is_or_of_multisig_and_gnosis_safe() {
        let multisig_hash = FixedBytes::<8>::from_slice(&keccak256("multisig")[..8]);
        let gnosis_hash = FixedBytes::<8>::from_slice(&keccak256("gnosis-safe")[..8]);
        let expected = bitwise_or(multisig_hash, gnosis_hash);
        assert_eq!(SenderTypes::gnosis_safe(), expected);
    }

    #[test]
    fn hardware_wallet_type_is_or_of_hardware_wallet_and_private_key() {
        let hw_hash = FixedBytes::<8>::from_slice(&keccak256("hardware-wallet")[..8]);
        let pk_hash = FixedBytes::<8>::from_slice(&keccak256("private-key")[..8]);
        let expected = bitwise_or(hw_hash, pk_hash);
        assert_eq!(SenderTypes::hardware_wallet(), expected);
    }

    #[test]
    fn ledger_type_is_or_of_ledger_and_hardware_wallet() {
        let ledger_hash = FixedBytes::<8>::from_slice(&keccak256("ledger")[..8]);
        let expected = bitwise_or(ledger_hash, SenderTypes::hardware_wallet());
        assert_eq!(SenderTypes::ledger(), expected);
    }

    #[test]
    fn trezor_type_is_or_of_trezor_and_hardware_wallet() {
        let trezor_hash = FixedBytes::<8>::from_slice(&keccak256("trezor")[..8]);
        let expected = bitwise_or(trezor_hash, SenderTypes::hardware_wallet());
        assert_eq!(SenderTypes::trezor(), expected);
    }

    #[test]
    fn oz_governor_type_is_or_of_governance_and_oz_governor() {
        let governance_hash = FixedBytes::<8>::from_slice(&keccak256("governance")[..8]);
        let oz_hash = FixedBytes::<8>::from_slice(&keccak256("oz-governor")[..8]);
        let expected = bitwise_or(governance_hash, oz_hash);
        assert_eq!(SenderTypes::oz_governor(), expected);
    }

    #[test]
    fn in_memory_type_contains_private_key_flag() {
        // isType check: senderType & _type == _type
        let in_memory = SenderTypes::in_memory();
        let pk = SenderTypes::private_key();
        let masked: [u8; 8] = core::array::from_fn(|i| in_memory[i] & pk[i]);
        assert_eq!(FixedBytes::from(masked), pk, "InMemory should contain PrivateKey flag");
    }

    #[test]
    fn ledger_type_contains_hardware_wallet_and_private_key_flags() {
        let ledger = SenderTypes::ledger();
        let hw = SenderTypes::hardware_wallet();
        let pk = SenderTypes::private_key();
        let masked_hw: [u8; 8] = core::array::from_fn(|i| ledger[i] & hw[i]);
        let masked_pk: [u8; 8] = core::array::from_fn(|i| ledger[i] & pk[i]);
        assert_eq!(FixedBytes::from(masked_hw), hw);
        assert_eq!(FixedBytes::from(masked_pk), pk);
    }

    // ── Encoding tests ────────────────────────────────────────────────

    /// Anvil account 0 private key (well-known test key).
    const ANVIL_KEY_0: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

    /// Anvil account 0 address.
    const ANVIL_ADDR_0: Address = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");

    fn pk_sender_config(key: &str) -> SenderConfig {
        SenderConfig {
            type_: Some(SenderType::PrivateKey),
            private_key: Some(key.to_string()),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn encode_produces_0x_prefixed_hex() {
        let sender_configs =
            HashMap::from([("deployer".to_string(), pk_sender_config(ANVIL_KEY_0))]);
        let resolved = crate::sender::resolve_all_senders(&sender_configs).await.unwrap();
        let encoded = encode_sender_configs(&resolved);

        assert!(encoded.starts_with("0x"), "should be 0x-prefixed");
        let hex_str = encoded.strip_prefix("0x").unwrap();
        let decoded = alloy_primitives::hex::decode(hex_str).unwrap();
        assert!(!decoded.is_empty(), "encoded data should be non-empty");
    }

    #[tokio::test]
    async fn encode_contains_correct_address() {
        let sender_configs =
            HashMap::from([("deployer".to_string(), pk_sender_config(ANVIL_KEY_0))]);
        let resolved = crate::sender::resolve_all_senders(&sender_configs).await.unwrap();
        let encoded = encode_sender_configs(&resolved);

        let addr_hex = format!("{:x}", ANVIL_ADDR_0).to_lowercase();
        assert!(
            encoded.to_lowercase().contains(&addr_hex),
            "encoded data should contain sender address: {addr_hex}"
        );
    }

    #[test]
    fn encode_empty_senders() {
        let resolved: HashMap<String, ResolvedSender> = HashMap::new();
        let encoded = encode_sender_configs(&resolved);
        assert!(encoded.starts_with("0x"));
    }

    #[tokio::test]
    async fn encode_is_deterministic() {
        let sender_configs = HashMap::from([
            ("deployer".to_string(), pk_sender_config(ANVIL_KEY_0)),
            (
                "other".to_string(),
                pk_sender_config(
                    "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d",
                ),
            ),
        ]);
        let resolved = crate::sender::resolve_all_senders(&sender_configs).await.unwrap();

        let encoded1 = encode_sender_configs(&resolved);
        let encoded2 = encode_sender_configs(&resolved);
        assert_eq!(encoded1, encoded2, "encoding should be deterministic");
    }
}
