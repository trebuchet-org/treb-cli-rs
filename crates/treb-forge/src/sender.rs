//! Sender/wallet resolution for treb.
//!
//! Bridges treb's `SenderConfig` definitions with foundry's `WalletSigner`
//! instances. Each sender type (PrivateKey, Ledger, Trezor, Safe, Governor)
//! is resolved into a `ResolvedSender` that can be wired into `ScriptArgs`
//! for in-process forge execution.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;

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

impl ResolvedSender {
    /// Returns the on-chain address for this resolved sender.
    ///
    /// For Wallet/InMemory senders, this is the signer's derived address.
    /// For Safe senders, this is the Safe multisig contract address.
    /// For Governor senders, this is the Governor contract address.
    pub fn sender_address(&self) -> Address {
        match self {
            Self::Wallet(ws) | Self::InMemory(ws) => ws.address(),
            Self::Safe { safe_address, .. } => *safe_address,
            Self::Governor {
                governor_address, ..
            } => *governor_address,
        }
    }
}

/// Resolve a single sender by name from its configuration.
///
/// Recursively resolves sub-senders (e.g. a Safe's signer or a Governor's
/// proposer) using `all_senders`. The `visited` set detects circular
/// references to prevent infinite recursion.
pub fn resolve_sender<'a>(
    name: &'a str,
    config: &'a SenderConfig,
    all_senders: &'a HashMap<String, SenderConfig>,
    visited: &'a mut HashSet<String>,
) -> Pin<Box<dyn Future<Output = treb_core::Result<ResolvedSender>> + Send + 'a>> {
    Box::pin(async move {
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
            treb_config::SenderType::Safe => {
                resolve_safe(name, config, all_senders, visited).await
            }
            treb_config::SenderType::OZGovernor => {
                resolve_governor(name, config, all_senders, visited).await
            }
        }
    })
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
    name: &str,
    config: &SenderConfig,
    all_senders: &HashMap<String, SenderConfig>,
    visited: &mut HashSet<String>,
) -> treb_core::Result<ResolvedSender> {
    let safe_address: Address = config
        .safe
        .as_deref()
        .or(config.address.as_deref())
        .ok_or_else(|| {
            TrebError::Config(format!(
                "sender '{name}' of type Safe is missing required 'safe' or 'address' field"
            ))
        })?
        .parse()
        .map_err(|e| {
            TrebError::Config(format!("sender '{name}': invalid safe address: {e}"))
        })?;

    let signer_name = config.signer.as_deref().ok_or_else(|| {
        TrebError::Config(format!(
            "sender '{name}' of type Safe is missing required 'signer' field"
        ))
    })?;

    let signer_config = all_senders.get(signer_name).ok_or_else(|| {
        TrebError::Config(format!(
            "sender '{name}': signer '{signer_name}' not found in sender configuration"
        ))
    })?;

    let signer = resolve_sender(signer_name, signer_config, all_senders, visited).await?;

    Ok(ResolvedSender::Safe {
        safe_address,
        signer: Box::new(signer),
    })
}

async fn resolve_governor(
    name: &str,
    config: &SenderConfig,
    all_senders: &HashMap<String, SenderConfig>,
    visited: &mut HashSet<String>,
) -> treb_core::Result<ResolvedSender> {
    let governor_address: Address = config
        .governor
        .as_deref()
        .or(config.address.as_deref())
        .ok_or_else(|| {
            TrebError::Config(format!(
                "sender '{name}' of type OZGovernor is missing required 'governor' or 'address' field"
            ))
        })?
        .parse()
        .map_err(|e| {
            TrebError::Config(format!("sender '{name}': invalid governor address: {e}"))
        })?;

    let timelock_address: Option<Address> = config
        .timelock
        .as_deref()
        .map(|addr| {
            addr.parse().map_err(|e| {
                TrebError::Config(format!("sender '{name}': invalid timelock address: {e}"))
            })
        })
        .transpose()?;

    let proposer_name = config.proposer.as_deref().ok_or_else(|| {
        TrebError::Config(format!(
            "sender '{name}' of type OZGovernor is missing required 'proposer' field"
        ))
    })?;

    let proposer_config = all_senders.get(proposer_name).ok_or_else(|| {
        TrebError::Config(format!(
            "sender '{name}': proposer '{proposer_name}' not found in sender configuration"
        ))
    })?;

    let proposer = resolve_sender(proposer_name, proposer_config, all_senders, visited).await?;

    Ok(ResolvedSender::Governor {
        governor_address,
        timelock_address,
        proposer: Box::new(proposer),
    })
}

/// Anvil's default HD wallet mnemonic.
const ANVIL_MNEMONIC: &str =
    "test test test test test test test test test test test junk";

/// Create an in-memory test signer derived from Anvil's default HD wallet.
///
/// The returned signer produces the same addresses as Anvil's default accounts
/// using derivation path `m/44'/60'/0'/0/{index}`.
pub fn in_memory_signer(index: u32) -> treb_core::Result<WalletSigner> {
    WalletSigner::from_mnemonic(ANVIL_MNEMONIC, None, None, index).map_err(|e| {
        TrebError::Forge(format!("failed to create in-memory signer at index {index}: {e}"))
    })
}

/// Create multiple in-memory test signers starting from index 0.
///
/// Returns `count` signers derived from Anvil's default HD wallet mnemonic,
/// matching the accounts available in a default Anvil instance.
pub fn default_test_signers(count: u32) -> treb_core::Result<Vec<WalletSigner>> {
    (0..count).map(in_memory_signer).collect()
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

    // ---- InMemory signer tests ----

    #[test]
    fn in_memory_signer_index_0_produces_anvil_addr_0() {
        let signer = in_memory_signer(0).unwrap();
        assert_eq!(
            signer.address(),
            ANVIL_ADDR_0,
            "index 0 should produce Anvil's default account 0"
        );
    }

    #[test]
    fn in_memory_signer_different_indices_produce_different_addresses() {
        let s0 = in_memory_signer(0).unwrap();
        let s1 = in_memory_signer(1).unwrap();
        assert_ne!(
            s0.address(),
            s1.address(),
            "different indices should produce different addresses"
        );
    }

    #[test]
    fn default_test_signers_returns_requested_count() {
        let signers = default_test_signers(5).unwrap();
        assert_eq!(signers.len(), 5);
        // First signer should be Anvil account 0
        assert_eq!(signers[0].address(), ANVIL_ADDR_0);
    }

    #[test]
    fn default_test_signers_zero_returns_empty() {
        let signers = default_test_signers(0).unwrap();
        assert!(signers.is_empty());
    }

    // ---- Safe sender resolution tests ----

    fn safe_config(safe_addr: &str, signer_name: &str) -> SenderConfig {
        SenderConfig {
            type_: Some(SenderType::Safe),
            safe: Some(safe_addr.to_string()),
            signer: Some(signer_name.to_string()),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn safe_resolves_with_pk_signer() {
        let safe_addr = "0x0000000000000000000000000000000000000042";
        let mut senders = HashMap::new();
        senders.insert("my-safe".to_string(), safe_config(safe_addr, "deployer"));
        senders.insert("deployer".to_string(), pk_config(ANVIL_KEY_0, None));

        let mut visited = HashSet::new();
        let result = resolve_sender(
            "my-safe",
            senders.get("my-safe").unwrap(),
            &senders,
            &mut visited,
        )
        .await
        .unwrap();

        match result {
            ResolvedSender::Safe {
                safe_address,
                signer,
            } => {
                assert_eq!(safe_address, safe_addr.parse::<Address>().unwrap());
                match *signer {
                    ResolvedSender::Wallet(ref ws) => {
                        assert_eq!(ws.address(), ANVIL_ADDR_0);
                    }
                    ref other => panic!("expected Wallet sub-signer, got {other:?}"),
                }
            }
            other => panic!("expected Safe variant, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn safe_missing_signer_field() {
        let config = SenderConfig {
            type_: Some(SenderType::Safe),
            safe: Some("0x0000000000000000000000000000000000000042".to_string()),
            ..Default::default()
        };
        let senders = HashMap::from([("my-safe".to_string(), config.clone())]);
        let mut visited = HashSet::new();
        let err = resolve_sender("my-safe", &config, &senders, &mut visited)
            .await
            .unwrap_err();

        match err {
            TrebError::Config(msg) => {
                assert!(
                    msg.contains("missing required 'signer' field"),
                    "should mention missing signer: {msg}"
                );
            }
            other => panic!("expected Config error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn safe_signer_not_found() {
        let config = safe_config("0x0000000000000000000000000000000000000042", "nonexistent");
        let senders = HashMap::from([("my-safe".to_string(), config.clone())]);
        let mut visited = HashSet::new();
        let err = resolve_sender("my-safe", &config, &senders, &mut visited)
            .await
            .unwrap_err();

        match err {
            TrebError::Config(msg) => {
                assert!(
                    msg.contains("nonexistent"),
                    "should mention missing sender name: {msg}"
                );
                assert!(
                    msg.contains("not found"),
                    "should say not found: {msg}"
                );
            }
            other => panic!("expected Config error, got {other:?}"),
        }
    }

    // ---- Governor sender resolution tests ----

    fn governor_config(
        governor_addr: &str,
        timelock_addr: Option<&str>,
        proposer_name: &str,
    ) -> SenderConfig {
        SenderConfig {
            type_: Some(SenderType::OZGovernor),
            governor: Some(governor_addr.to_string()),
            timelock: timelock_addr.map(|a| a.to_string()),
            proposer: Some(proposer_name.to_string()),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn governor_resolves_with_pk_proposer() {
        let gov_addr = "0x0000000000000000000000000000000000000099";
        let tl_addr = "0x0000000000000000000000000000000000000088";
        let mut senders = HashMap::new();
        senders.insert(
            "my-gov".to_string(),
            governor_config(gov_addr, Some(tl_addr), "deployer"),
        );
        senders.insert("deployer".to_string(), pk_config(ANVIL_KEY_0, None));

        let mut visited = HashSet::new();
        let result = resolve_sender(
            "my-gov",
            senders.get("my-gov").unwrap(),
            &senders,
            &mut visited,
        )
        .await
        .unwrap();

        match result {
            ResolvedSender::Governor {
                governor_address,
                timelock_address,
                proposer,
            } => {
                assert_eq!(governor_address, gov_addr.parse::<Address>().unwrap());
                assert_eq!(
                    timelock_address,
                    Some(tl_addr.parse::<Address>().unwrap())
                );
                match *proposer {
                    ResolvedSender::Wallet(ref ws) => {
                        assert_eq!(ws.address(), ANVIL_ADDR_0);
                    }
                    ref other => panic!("expected Wallet proposer, got {other:?}"),
                }
            }
            other => panic!("expected Governor variant, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn governor_without_timelock() {
        let gov_addr = "0x0000000000000000000000000000000000000099";
        let mut senders = HashMap::new();
        senders.insert(
            "my-gov".to_string(),
            governor_config(gov_addr, None, "deployer"),
        );
        senders.insert("deployer".to_string(), pk_config(ANVIL_KEY_0, None));

        let mut visited = HashSet::new();
        let result = resolve_sender(
            "my-gov",
            senders.get("my-gov").unwrap(),
            &senders,
            &mut visited,
        )
        .await
        .unwrap();

        match result {
            ResolvedSender::Governor {
                timelock_address, ..
            } => {
                assert_eq!(timelock_address, None);
            }
            other => panic!("expected Governor variant, got {other:?}"),
        }
    }

    // ---- Circular reference detection tests ----

    #[tokio::test]
    async fn circular_reference_detected() {
        // A -> B -> A creates a cycle
        let mut senders = HashMap::new();
        senders.insert(
            "a".to_string(),
            safe_config("0x0000000000000000000000000000000000000001", "b"),
        );
        senders.insert(
            "b".to_string(),
            safe_config("0x0000000000000000000000000000000000000002", "a"),
        );

        let mut visited = HashSet::new();
        let err = resolve_sender("a", senders.get("a").unwrap(), &senders, &mut visited)
            .await
            .unwrap_err();

        match err {
            TrebError::Config(msg) => {
                assert!(
                    msg.contains("circular"),
                    "should mention circular reference: {msg}"
                );
            }
            other => panic!("expected Config error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn self_referencing_sender_detected() {
        let mut senders = HashMap::new();
        senders.insert(
            "self-ref".to_string(),
            safe_config("0x0000000000000000000000000000000000000001", "self-ref"),
        );

        let mut visited = HashSet::new();
        let err = resolve_sender(
            "self-ref",
            senders.get("self-ref").unwrap(),
            &senders,
            &mut visited,
        )
        .await
        .unwrap_err();

        match err {
            TrebError::Config(msg) => {
                assert!(msg.contains("circular"), "should detect self-ref: {msg}");
            }
            other => panic!("expected Config error, got {other:?}"),
        }
    }

    // ---- resolve_all_senders tests ----

    #[tokio::test]
    async fn resolve_all_senders_handles_all_types() {
        let safe_addr = "0x0000000000000000000000000000000000000042";
        let gov_addr = "0x0000000000000000000000000000000000000099";

        let mut senders = HashMap::new();
        senders.insert("deployer".to_string(), pk_config(ANVIL_KEY_0, None));
        senders.insert(
            "my-safe".to_string(),
            safe_config(safe_addr, "deployer"),
        );
        senders.insert(
            "my-gov".to_string(),
            governor_config(gov_addr, None, "deployer"),
        );

        let resolved = resolve_all_senders(&senders).await.unwrap();
        assert_eq!(resolved.len(), 3);
        assert!(matches!(resolved.get("deployer"), Some(ResolvedSender::Wallet(_))));
        assert!(matches!(resolved.get("my-safe"), Some(ResolvedSender::Safe { .. })));
        assert!(matches!(resolved.get("my-gov"), Some(ResolvedSender::Governor { .. })));
    }
}
