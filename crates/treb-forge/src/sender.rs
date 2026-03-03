//! Sender/wallet resolution for treb.
//!
//! Bridges treb's `SenderConfig` definitions with foundry's `WalletSigner`
//! instances. Each sender type (PrivateKey, Ledger, Trezor, Safe, Governor)
//! is resolved into a `ResolvedSender` that can be wired into `ScriptArgs`
//! for in-process forge execution.

use std::collections::{HashMap, HashSet};

use alloy_primitives::{Address, B256, hex};
use alloy_signer::Signer;
use alloy_signer_ledger::HDPath as LedgerHDPath;
use alloy_signer_trezor::HDPath as TrezorHDPath;
use foundry_wallets::WalletSigner;
use treb_config::SenderConfig;
use treb_core::error::TrebError;

/// A fully resolved sender ready for use in script execution.
///
/// Each variant wraps the signing capability (or address-only stub) produced
/// by resolving a [`SenderConfig`] entry.
#[derive(Debug)]
pub enum ResolvedSender {
    /// A directly signable wallet from PrivateKey, Ledger, or Trezor config.
    Wallet(WalletSigner),

    /// An in-memory test signer derived from Anvil's default HD wallet.
    InMemory(WalletSigner),

    /// A Safe multisig — holds the safe address and the resolved sub-signer.
    /// Actual Safe transaction signing is deferred to a later phase.
    Safe {
        safe_address: Address,
        signer: Box<ResolvedSender>,
    },

    /// An OZ Governor — holds governor/timelock addresses and the resolved proposer.
    /// Actual governance proposal signing is deferred to a later phase.
    Governor {
        governor_address: Address,
        timelock_address: Option<Address>,
        proposer: Box<ResolvedSender>,
    },
}

/// Resolve a single sender by name from its configuration.
///
/// Recursively resolves sub-senders (e.g. a Safe's signer or a Governor's
/// proposer) using `all_senders`. The `visited` set detects circular
/// references to prevent infinite recursion.
pub async fn resolve_sender(
    name: &str,
    config: &SenderConfig,
    all_senders: &HashMap<String, SenderConfig>,
    visited: &mut HashSet<String>,
) -> treb_core::Result<ResolvedSender> {
    if !visited.insert(name.to_string()) {
        return Err(TrebError::Config(format!(
            "circular sender reference detected: '{name}' was already visited"
        )));
    }

    let sender_type = config.type_.as_ref().ok_or_else(|| {
        TrebError::Config(format!("sender '{name}' is missing required 'type' field"))
    })?;

    match sender_type {
        treb_config::SenderType::PrivateKey => resolve_private_key(name, config).await,
        treb_config::SenderType::Ledger => resolve_ledger(name, config).await,
        treb_config::SenderType::Trezor => resolve_trezor(name, config).await,
        treb_config::SenderType::Safe => resolve_safe(name, config, all_senders, visited).await,
        treb_config::SenderType::OZGovernor => {
            resolve_governor(name, config, all_senders, visited).await
        }
    }
}

/// Resolve all senders from a configuration map.
///
/// Returns a map of sender names to their resolved forms. Handles recursive
/// sub-sender references and detects circular dependencies.
pub async fn resolve_all_senders(
    senders: &HashMap<String, SenderConfig>,
) -> treb_core::Result<HashMap<String, ResolvedSender>> {
    let mut resolved = HashMap::new();

    for (name, config) in senders {
        if resolved.contains_key(name) {
            continue;
        }
        let mut visited = HashSet::new();
        let sender = resolve_sender(name, config, senders, &mut visited).await?;
        resolved.insert(name.clone(), sender);
    }

    Ok(resolved)
}

// ---------------------------------------------------------------------------
// Per-type resolution stubs (implemented in US-002 / US-003)
// ---------------------------------------------------------------------------

async fn resolve_private_key(
    name: &str,
    config: &SenderConfig,
) -> treb_core::Result<ResolvedSender> {
    let key_hex = config.private_key.as_deref().ok_or_else(|| {
        TrebError::Config(format!(
            "sender '{name}' of type PrivateKey is missing required 'private_key' field"
        ))
    })?;

    let key_bytes: B256 = hex::FromHex::from_hex(key_hex).map_err(|e| {
        TrebError::Config(format!("sender '{name}': invalid private key: {e}"))
    })?;

    let signer = WalletSigner::from_private_key(&key_bytes).map_err(|e| {
        TrebError::Config(format!("sender '{name}': invalid private key: {e}"))
    })?;

    // Validate derived address matches configured address when both are provided
    if let Some(ref addr_str) = config.address {
        let expected: Address = addr_str.parse().map_err(|e| {
            TrebError::Config(format!("sender '{name}': invalid address '{addr_str}': {e}"))
        })?;
        let derived = signer.address();
        if expected != derived {
            return Err(TrebError::Config(format!(
                "sender '{name}': address mismatch — configured {expected} but private key derives {derived}"
            )));
        }
    }

    Ok(ResolvedSender::Wallet(signer))
}

fn parse_ledger_path(derivation_path: Option<&str>) -> LedgerHDPath {
    match derivation_path {
        Some(path) => LedgerHDPath::Other(path.to_string()),
        None => LedgerHDPath::LedgerLive(0),
    }
}

async fn resolve_ledger(
    name: &str,
    config: &SenderConfig,
) -> treb_core::Result<ResolvedSender> {
    let path = parse_ledger_path(config.derivation_path.as_deref());

    let signer = WalletSigner::from_ledger_path(path).await.map_err(|e| {
        TrebError::Forge(format!(
            "sender '{name}': failed to connect to Ledger device: {e}"
        ))
    })?;

    Ok(ResolvedSender::Wallet(signer))
}

fn parse_trezor_path(derivation_path: Option<&str>) -> TrezorHDPath {
    match derivation_path {
        Some(path) => TrezorHDPath::Other(path.to_string()),
        None => TrezorHDPath::TrezorLive(0),
    }
}

async fn resolve_trezor(
    name: &str,
    config: &SenderConfig,
) -> treb_core::Result<ResolvedSender> {
    let path = parse_trezor_path(config.derivation_path.as_deref());

    let signer = WalletSigner::from_trezor_path(path).await.map_err(|e| {
        TrebError::Forge(format!(
            "sender '{name}': failed to connect to Trezor device: {e}"
        ))
    })?;

    Ok(ResolvedSender::Wallet(signer))
}

async fn resolve_safe(
    _name: &str,
    _config: &SenderConfig,
    _all_senders: &HashMap<String, SenderConfig>,
    _visited: &mut HashSet<String>,
) -> treb_core::Result<ResolvedSender> {
    todo!("US-003: Safe sender resolution")
}

async fn resolve_governor(
    _name: &str,
    _config: &SenderConfig,
    _all_senders: &HashMap<String, SenderConfig>,
    _visited: &mut HashSet<String>,
) -> treb_core::Result<ResolvedSender> {
    todo!("US-003: Governor sender resolution")
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::address;
    use treb_config::SenderType;

    /// Anvil account 0 private key (well-known test key).
    const ANVIL_KEY_0: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

    /// Anvil account 0 address.
    const ANVIL_ADDR_0: Address = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");

    fn pk_config(key: &str, address: Option<&str>) -> SenderConfig {
        SenderConfig {
            type_: Some(SenderType::PrivateKey),
            private_key: Some(key.to_string()),
            address: address.map(|a| a.to_string()),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn private_key_valid_with_0x_prefix() {
        let config = pk_config(ANVIL_KEY_0, None);
        let result = resolve_private_key("deployer", &config).await.unwrap();

        match result {
            ResolvedSender::Wallet(signer) => {
                assert_eq!(signer.address(), ANVIL_ADDR_0);
            }
            other => panic!("expected Wallet variant, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn private_key_valid_without_0x_prefix() {
        let key = ANVIL_KEY_0.strip_prefix("0x").unwrap();
        let config = pk_config(key, None);
        let result = resolve_private_key("deployer", &config).await.unwrap();

        match result {
            ResolvedSender::Wallet(signer) => {
                assert_eq!(signer.address(), ANVIL_ADDR_0);
            }
            other => panic!("expected Wallet variant, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn private_key_valid_with_matching_address() {
        let config = pk_config(
            ANVIL_KEY_0,
            Some("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
        );
        let result = resolve_private_key("deployer", &config).await.unwrap();

        match result {
            ResolvedSender::Wallet(signer) => {
                assert_eq!(signer.address(), ANVIL_ADDR_0);
            }
            other => panic!("expected Wallet variant, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn private_key_invalid_hex() {
        let config = pk_config("not-a-hex-key", None);
        let err = resolve_private_key("bad", &config).await.unwrap_err();

        match err {
            TrebError::Config(msg) => {
                assert!(msg.contains("bad"), "error should name the sender: {msg}");
                assert!(
                    msg.contains("invalid private key"),
                    "error should describe the issue: {msg}"
                );
            }
            other => panic!("expected Config error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn private_key_address_mismatch() {
        // Use a valid key but with a wrong address
        let wrong_address = "0x0000000000000000000000000000000000000001";
        let config = pk_config(ANVIL_KEY_0, Some(wrong_address));
        let err = resolve_private_key("deployer", &config).await.unwrap_err();

        match err {
            TrebError::Config(msg) => {
                assert!(msg.contains("address mismatch"), "should mention mismatch: {msg}");
                assert!(
                    msg.contains(wrong_address),
                    "should mention expected address: {msg}"
                );
                assert!(
                    msg.to_lowercase().contains(&ANVIL_ADDR_0.to_string().to_lowercase()),
                    "should mention derived address: {msg}"
                );
            }
            other => panic!("expected Config error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn private_key_missing_field() {
        let config = SenderConfig {
            type_: Some(SenderType::PrivateKey),
            ..Default::default()
        };
        let err = resolve_private_key("deployer", &config).await.unwrap_err();

        match err {
            TrebError::Config(msg) => {
                assert!(
                    msg.contains("missing required 'private_key' field"),
                    "should mention missing field: {msg}"
                );
            }
            other => panic!("expected Config error, got {other:?}"),
        }
    }
}
