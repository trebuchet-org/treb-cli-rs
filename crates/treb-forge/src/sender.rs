//! Sender/wallet resolution for treb.
//!
//! Bridges treb's `SenderConfig` definitions with foundry's `WalletSigner`
//! instances. Each sender type (PrivateKey, Ledger, Trezor, Safe, Governor)
//! is resolved into a `ResolvedSender` that can be wired into `ScriptArgs`
//! for in-process forge execution.

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    pin::Pin,
};

use alloy_network::EthereumWallet;
use alloy_primitives::{Address, B256, hex};
use alloy_signer::Signer;
use alloy_signer_ledger::HDPath as LedgerHDPath;
use alloy_signer_trezor::HDPath as TrezorHDPath;
use foundry_wallets::WalletSigner;
use treb_config::SenderConfig;
use treb_core::error::TrebError;

/// Classification of a resolved sender for transaction routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SenderCategory {
    /// Direct wallet sender (PrivateKey, Ledger, Trezor, or InMemory).
    Wallet,
    /// Safe multisig sender — transactions are proposed via Safe Transaction Service.
    Safe,
    /// OZ Governor sender — transactions are submitted as governance proposals.
    Governor,
}

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
    Safe { safe_address: Address, signer: Box<ResolvedSender> },

    /// A governance sender — holds governor/timelock addresses and the resolved proposer.
    /// Actual governance proposal creation is deferred to routing.
    Governor {
        governor_address: Address,
        timelock_address: Option<Address>,
        proposer: Box<ResolvedSender>,
        proposer_script: Option<String>,
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
            Self::Governor { governor_address, .. } => *governor_address,
        }
    }

    /// Returns the address that `vm.broadcast()` should use for this sender.
    ///
    /// For most senders this equals `sender_address()`. The difference is for
    /// Governor senders with a timelock: the **timelock** is the on-chain
    /// executor (it's the `msg.sender` when the proposal executes), so the
    /// script must `vm.broadcast(timelockAddress)`.
    pub fn broadcast_address(&self) -> Address {
        match self {
            Self::Governor { timelock_address: Some(tl), .. } => *tl,
            _ => self.sender_address(),
        }
    }

    /// Returns `true` if this is a Safe multisig sender.
    pub fn is_safe(&self) -> bool {
        matches!(self, Self::Safe { .. })
    }

    /// Returns `true` if this is a governance sender.
    pub fn is_governor(&self) -> bool {
        matches!(self, Self::Governor { .. })
    }

    /// Returns the inner sub-signer for composite sender types.
    ///
    /// For Safe senders, this is the signing wallet.
    /// For Governor senders, this is the proposer wallet.
    /// For Wallet/InMemory senders, returns `self`.
    pub fn sub_signer(&self) -> &ResolvedSender {
        match self {
            Self::Safe { signer, .. } => signer,
            Self::Governor { proposer, .. } => proposer,
            _ => self,
        }
    }

    /// Returns the underlying `WalletSigner` if this is a Wallet or InMemory sender.
    ///
    /// Returns `None` for Safe or Governor senders — use `sub_signer().wallet_signer()`
    /// to reach the leaf signer.
    pub fn wallet_signer(&self) -> Option<&WalletSigner> {
        match self {
            Self::Wallet(ws) | Self::InMemory(ws) => Some(ws),
            _ => None,
        }
    }

    /// Returns the Safe multisig address if this is a Safe sender.
    pub fn safe_address(&self) -> Option<Address> {
        match self {
            Self::Safe { safe_address, .. } => Some(*safe_address),
            _ => None,
        }
    }

    /// Returns the Governor contract address if this is a Governor sender.
    pub fn governor_address(&self) -> Option<Address> {
        match self {
            Self::Governor { governor_address, .. } => Some(*governor_address),
            _ => None,
        }
    }

    /// Returns the timelock address if this is a Governor sender with a timelock.
    pub fn timelock_address(&self) -> Option<Address> {
        match self {
            Self::Governor { timelock_address, .. } => *timelock_address,
            _ => None,
        }
    }

    /// Returns the sender category for transaction routing.
    pub fn category(&self) -> SenderCategory {
        match self {
            Self::Wallet(_) | Self::InMemory(_) => SenderCategory::Wallet,
            Self::Safe { .. } => SenderCategory::Safe,
            Self::Governor { .. } => SenderCategory::Governor,
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
            treb_config::SenderType::Safe => resolve_safe(name, config, all_senders, visited).await,
            treb_config::SenderType::Governance => {
                resolve_governance(name, config, all_senders, visited).await
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

/// Extract the private key hex string for the leaf signing wallet of a sender.
///
/// For Wallet/InMemory senders, returns the key directly if available in the config.
/// For Safe/Governor senders, follows the sub-signer chain to the leaf wallet.
pub fn extract_signing_key<'a>(
    role: &str,
    sender: &ResolvedSender,
    sender_configs: &'a HashMap<String, SenderConfig>,
) -> Option<&'a str> {
    match sender {
        ResolvedSender::Wallet(_) | ResolvedSender::InMemory(_) => {
            sender_configs.get(role).and_then(|c| c.private_key.as_deref())
        }
        ResolvedSender::Safe { signer, .. } => {
            // Find the signer's role name in the config
            let signer_name = sender_configs.get(role).and_then(|c| c.signer.as_deref())?;
            extract_signing_key(signer_name, signer, sender_configs)
        }
        ResolvedSender::Governor { proposer, .. } => {
            let proposer_name = sender_configs.get(role).and_then(|c| c.proposer.as_deref())?;
            extract_signing_key(proposer_name, proposer, sender_configs)
        }
    }
}

/// Resolve an [`EthereumWallet`] for a given on-chain address.
///
/// Looks up the `ResolvedSender` whose [`broadcast_address()`] matches `address`,
/// walks the sender chain (Safe→signer, Governor→proposer) to the leaf wallet,
/// and wraps the underlying signer in an `EthereumWallet` for use with alloy providers.
pub fn resolve_wallet_for_address(
    address: Address,
    resolved_senders: &HashMap<String, ResolvedSender>,
) -> Result<EthereumWallet, TrebError> {
    let sender = resolved_senders
        .values()
        .find(|s| s.broadcast_address() == address)
        .ok_or_else(|| {
            TrebError::Forge(format!("no resolved sender found for address {address}"))
        })?;

    leaf_ethereum_wallet(sender)
}

/// Walk a sender chain to the leaf wallet and wrap it in an [`EthereumWallet`].
fn leaf_ethereum_wallet(sender: &ResolvedSender) -> Result<EthereumWallet, TrebError> {
    match sender {
        ResolvedSender::Wallet(ws) | ResolvedSender::InMemory(ws) => {
            wallet_signer_to_ethereum_wallet(ws)
        }
        ResolvedSender::Safe { signer, .. } => leaf_ethereum_wallet(signer),
        ResolvedSender::Governor { proposer, .. } => leaf_ethereum_wallet(proposer),
    }
}

/// Convert a [`WalletSigner`] reference into an owned [`EthereumWallet`].
///
/// Currently supports `Local` (private-key / mnemonic) signers, which are `Clone`.
/// Hardware wallet signers (Ledger, Trezor) are not yet supported for live signing.
fn wallet_signer_to_ethereum_wallet(ws: &WalletSigner) -> Result<EthereumWallet, TrebError> {
    match ws {
        WalletSigner::Local(pk) => Ok(EthereumWallet::new(pk.clone())),
        _ => Err(TrebError::Forge(
            "live signing is not yet supported for hardware wallet signers (Ledger/Trezor)"
                .to_string(),
        )),
    }
}

// ---------------------------------------------------------------------------
// Per-type resolution helpers
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

    let key_bytes: B256 = hex::FromHex::from_hex(key_hex)
        .map_err(|e| TrebError::Config(format!("sender '{name}': invalid private key: {e}")))?;

    let signer = WalletSigner::from_private_key(&key_bytes)
        .map_err(|e| TrebError::Config(format!("sender '{name}': invalid private key: {e}")))?;

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

async fn resolve_ledger(name: &str, config: &SenderConfig) -> treb_core::Result<ResolvedSender> {
    let path = parse_ledger_path(config.derivation_path.as_deref());

    let signer = WalletSigner::from_ledger_path(path).await.map_err(|e| {
        TrebError::Forge(format!("sender '{name}': failed to connect to Ledger device: {e}"))
    })?;

    Ok(ResolvedSender::Wallet(signer))
}

fn parse_trezor_path(derivation_path: Option<&str>) -> TrezorHDPath {
    match derivation_path {
        Some(path) => TrezorHDPath::Other(path.to_string()),
        None => TrezorHDPath::TrezorLive(0),
    }
}

async fn resolve_trezor(name: &str, config: &SenderConfig) -> treb_core::Result<ResolvedSender> {
    let path = parse_trezor_path(config.derivation_path.as_deref());

    let signer = WalletSigner::from_trezor_path(path).await.map_err(|e| {
        TrebError::Forge(format!("sender '{name}': failed to connect to Trezor device: {e}"))
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
        .map_err(|e| TrebError::Config(format!("sender '{name}': invalid safe address: {e}")))?;

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

    Ok(ResolvedSender::Safe { safe_address, signer: Box::new(signer) })
}

async fn resolve_governance(
    name: &str,
    config: &SenderConfig,
    all_senders: &HashMap<String, SenderConfig>,
    visited: &mut HashSet<String>,
) -> treb_core::Result<ResolvedSender> {
    let governor_address: Address = config
        .address
        .as_deref()
        .ok_or_else(|| {
            TrebError::Config(format!(
                "sender '{name}' of type Governance is missing required 'address' field"
            ))
        })?
        .parse()
        .map_err(|e| {
            TrebError::Config(format!("sender '{name}': invalid governance address: {e}"))
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
            "sender '{name}' of type Governance is missing required 'proposer' field"
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
        proposer_script: config.proposer_script.clone(),
    })
}

/// Anvil's default HD wallet mnemonic.
const ANVIL_MNEMONIC: &str = "test test test test test test test test test test test junk";

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
    use alloy_network::{Ethereum, NetworkWallet};
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
        let config = pk_config(ANVIL_KEY_0, Some("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));
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
                assert!(msg.contains(wrong_address), "should mention expected address: {msg}");
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
        let config = SenderConfig { type_: Some(SenderType::PrivateKey), ..Default::default() };
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
        let result =
            resolve_sender("my-safe", senders.get("my-safe").unwrap(), &senders, &mut visited)
                .await
                .unwrap();

        match result {
            ResolvedSender::Safe { safe_address, signer } => {
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
        let err = resolve_sender("my-safe", &config, &senders, &mut visited).await.unwrap_err();

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
        let err = resolve_sender("my-safe", &config, &senders, &mut visited).await.unwrap_err();

        match err {
            TrebError::Config(msg) => {
                assert!(msg.contains("nonexistent"), "should mention missing sender name: {msg}");
                assert!(msg.contains("not found"), "should say not found: {msg}");
            }
            other => panic!("expected Config error, got {other:?}"),
        }
    }

    // ---- Governor sender resolution tests ----

    fn governance_config(
        address: &str,
        timelock_addr: Option<&str>,
        proposer_name: &str,
    ) -> SenderConfig {
        SenderConfig {
            type_: Some(SenderType::Governance),
            address: Some(address.to_string()),
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
        senders.insert("my-gov".to_string(), governance_config(gov_addr, Some(tl_addr), "deployer"));
        senders.insert("deployer".to_string(), pk_config(ANVIL_KEY_0, None));

        let mut visited = HashSet::new();
        let result =
            resolve_sender("my-gov", senders.get("my-gov").unwrap(), &senders, &mut visited)
                .await
                .unwrap();

        match result {
            ResolvedSender::Governor { governor_address, timelock_address, proposer, .. } => {
                assert_eq!(governor_address, gov_addr.parse::<Address>().unwrap());
                assert_eq!(timelock_address, Some(tl_addr.parse::<Address>().unwrap()));
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
        senders.insert("my-gov".to_string(), governance_config(gov_addr, None, "deployer"));
        senders.insert("deployer".to_string(), pk_config(ANVIL_KEY_0, None));

        let mut visited = HashSet::new();
        let result =
            resolve_sender("my-gov", senders.get("my-gov").unwrap(), &senders, &mut visited)
                .await
                .unwrap();

        match result {
            ResolvedSender::Governor { timelock_address, .. } => {
                assert!(timelock_address.is_none());
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
                assert!(msg.contains("circular"), "should mention circular reference: {msg}");
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
        let err =
            resolve_sender("self-ref", senders.get("self-ref").unwrap(), &senders, &mut visited)
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
        senders.insert("my-safe".to_string(), safe_config(safe_addr, "deployer"));
        senders.insert("my-gov".to_string(), governance_config(gov_addr, None, "deployer"));

        let resolved = resolve_all_senders(&senders).await.unwrap();
        assert_eq!(resolved.len(), 3);
        assert!(matches!(resolved.get("deployer"), Some(ResolvedSender::Wallet(_))));
        assert!(matches!(resolved.get("my-safe"), Some(ResolvedSender::Safe { .. })));
        assert!(matches!(resolved.get("my-gov"), Some(ResolvedSender::Governor { .. })));
    }

    // ---- is_safe / is_governor / sub_signer / wallet_signer / safe_address tests ----

    #[tokio::test]
    async fn is_safe_returns_true_for_safe_sender() {
        let signer = ResolvedSender::Wallet(in_memory_signer(0).unwrap());
        let safe = ResolvedSender::Safe {
            safe_address: address!("0000000000000000000000000000000000000042"),
            signer: Box::new(signer),
        };
        assert!(safe.is_safe());
        assert!(!safe.is_governor());
    }

    #[tokio::test]
    async fn is_governor_returns_true_for_governor_sender() {
        let proposer = ResolvedSender::Wallet(in_memory_signer(0).unwrap());
        let gov = ResolvedSender::Governor {
            governor_address: address!("0000000000000000000000000000000000000099"),
            timelock_address: None,
            proposer: Box::new(proposer),
            proposer_script: None,
        };
        assert!(gov.is_governor());
        assert!(!gov.is_safe());
    }

    #[test]
    fn is_safe_and_governor_false_for_wallet() {
        let wallet = ResolvedSender::Wallet(in_memory_signer(0).unwrap());
        assert!(!wallet.is_safe());
        assert!(!wallet.is_governor());
    }

    #[test]
    fn sub_signer_returns_inner_for_safe() {
        let ws = in_memory_signer(0).unwrap();
        let expected_addr = ws.address();
        let safe = ResolvedSender::Safe {
            safe_address: address!("0000000000000000000000000000000000000042"),
            signer: Box::new(ResolvedSender::Wallet(ws)),
        };
        let sub = safe.sub_signer();
        assert!(matches!(sub, ResolvedSender::Wallet(_)));
        assert_eq!(sub.sender_address(), expected_addr);
    }

    #[test]
    fn sub_signer_returns_inner_for_governor() {
        let ws = in_memory_signer(0).unwrap();
        let expected_addr = ws.address();
        let gov = ResolvedSender::Governor {
            governor_address: address!("0000000000000000000000000000000000000099"),
            timelock_address: None,
            proposer: Box::new(ResolvedSender::Wallet(ws)),
            proposer_script: None,
        };
        let sub = gov.sub_signer();
        assert!(matches!(sub, ResolvedSender::Wallet(_)));
        assert_eq!(sub.sender_address(), expected_addr);
    }

    #[test]
    fn sub_signer_returns_self_for_wallet() {
        let ws = in_memory_signer(0).unwrap();
        let expected_addr = ws.address();
        let wallet = ResolvedSender::Wallet(ws);
        let sub = wallet.sub_signer();
        assert_eq!(sub.sender_address(), expected_addr);
    }

    #[test]
    fn wallet_signer_returns_some_for_wallet() {
        let ws = in_memory_signer(0).unwrap();
        let expected_addr = ws.address();
        let wallet = ResolvedSender::Wallet(ws);
        let ws_ref = wallet.wallet_signer().expect("should return Some");
        assert_eq!(ws_ref.address(), expected_addr);
    }

    #[test]
    fn wallet_signer_returns_none_for_safe() {
        let safe = ResolvedSender::Safe {
            safe_address: address!("0000000000000000000000000000000000000042"),
            signer: Box::new(ResolvedSender::Wallet(in_memory_signer(0).unwrap())),
        };
        assert!(safe.wallet_signer().is_none());
    }

    #[test]
    fn safe_address_returns_some_for_safe_sender() {
        let addr = address!("0000000000000000000000000000000000000042");
        let safe = ResolvedSender::Safe {
            safe_address: addr,
            signer: Box::new(ResolvedSender::Wallet(in_memory_signer(0).unwrap())),
        };
        assert_eq!(safe.safe_address(), Some(addr));
    }

    #[test]
    fn safe_address_returns_none_for_wallet() {
        let wallet = ResolvedSender::Wallet(in_memory_signer(0).unwrap());
        assert!(wallet.safe_address().is_none());
    }

    #[test]
    fn governor_address_returns_some_for_governor_sender() {
        let addr = address!("0000000000000000000000000000000000000099");
        let gov = ResolvedSender::Governor {
            governor_address: addr,
            timelock_address: None,
            proposer: Box::new(ResolvedSender::Wallet(in_memory_signer(0).unwrap())),
            proposer_script: None,
        };
        assert_eq!(gov.governor_address(), Some(addr));
    }

    #[test]
    fn governor_address_returns_none_for_wallet() {
        let wallet = ResolvedSender::Wallet(in_memory_signer(0).unwrap());
        assert!(wallet.governor_address().is_none());
    }

    #[test]
    fn timelock_address_returns_some_when_present() {
        let tl = address!("0000000000000000000000000000000000000088");
        let gov = ResolvedSender::Governor {
            governor_address: address!("0000000000000000000000000000000000000099"),
            timelock_address: Some(tl),
            proposer: Box::new(ResolvedSender::Wallet(in_memory_signer(0).unwrap())),
            proposer_script: None,
        };
        assert_eq!(gov.timelock_address(), Some(tl));
    }

    #[test]
    fn timelock_address_returns_none_when_absent() {
        let gov = ResolvedSender::Governor {
            governor_address: address!("0000000000000000000000000000000000000099"),
            timelock_address: None,
            proposer: Box::new(ResolvedSender::Wallet(in_memory_signer(0).unwrap())),
            proposer_script: None,
        };
        assert!(gov.timelock_address().is_none());
    }

    #[test]
    fn timelock_address_returns_none_for_wallet() {
        let wallet = ResolvedSender::Wallet(in_memory_signer(0).unwrap());
        assert!(wallet.timelock_address().is_none());
    }

    // ---- broadcast_address tests ----

    #[test]
    fn broadcast_address_returns_timelock_when_present() {
        let tl = address!("0000000000000000000000000000000000000088");
        let gov = ResolvedSender::Governor {
            governor_address: address!("0000000000000000000000000000000000000099"),
            timelock_address: Some(tl),
            proposer: Box::new(ResolvedSender::Wallet(in_memory_signer(0).unwrap())),
            proposer_script: None,
        };
        assert_eq!(gov.broadcast_address(), tl);
    }

    #[test]
    fn broadcast_address_returns_governor_when_no_timelock() {
        let gov_addr = address!("0000000000000000000000000000000000000099");
        let gov = ResolvedSender::Governor {
            governor_address: gov_addr,
            timelock_address: None,
            proposer: Box::new(ResolvedSender::Wallet(in_memory_signer(0).unwrap())),
            proposer_script: None,
        };
        assert_eq!(gov.broadcast_address(), gov_addr);
    }

    #[test]
    fn broadcast_address_equals_sender_address_for_wallet() {
        let ws = in_memory_signer(0).unwrap();
        let addr = ws.address();
        let wallet = ResolvedSender::Wallet(ws);
        assert_eq!(wallet.broadcast_address(), addr);
        assert_eq!(wallet.broadcast_address(), wallet.sender_address());
    }

    #[test]
    fn broadcast_address_equals_sender_address_for_safe() {
        let safe_addr = address!("0000000000000000000000000000000000000042");
        let safe = ResolvedSender::Safe {
            safe_address: safe_addr,
            signer: Box::new(ResolvedSender::Wallet(in_memory_signer(0).unwrap())),
        };
        assert_eq!(safe.broadcast_address(), safe_addr);
        assert_eq!(safe.broadcast_address(), safe.sender_address());
    }

    // ---- resolve_wallet_for_address tests ----

    #[test]
    fn resolve_wallet_for_address_direct_wallet() {
        let ws = in_memory_signer(0).unwrap();
        let addr = ws.address();
        let mut senders = HashMap::new();
        senders.insert("deployer".to_string(), ResolvedSender::Wallet(ws));

        let wallet = resolve_wallet_for_address(addr, &senders).unwrap();
        assert_eq!(NetworkWallet::<Ethereum>::default_signer_address(&wallet), addr);
    }

    #[test]
    fn resolve_wallet_for_address_in_memory() {
        let ws = in_memory_signer(1).unwrap();
        let addr = ws.address();
        let mut senders = HashMap::new();
        senders.insert("test".to_string(), ResolvedSender::InMemory(ws));

        let wallet = resolve_wallet_for_address(addr, &senders).unwrap();
        assert_eq!(NetworkWallet::<Ethereum>::default_signer_address(&wallet), addr);
    }

    #[test]
    fn resolve_wallet_for_address_safe_chain() {
        let ws = in_memory_signer(0).unwrap();
        let wallet_addr = ws.address();
        let safe_addr = address!("0000000000000000000000000000000000000042");
        let mut senders = HashMap::new();
        senders.insert(
            "my-safe".to_string(),
            ResolvedSender::Safe {
                safe_address: safe_addr,
                signer: Box::new(ResolvedSender::Wallet(ws)),
            },
        );

        // Safe's broadcast_address() == safe_address
        let wallet = resolve_wallet_for_address(safe_addr, &senders).unwrap();
        // The EthereumWallet wraps the leaf signer's address
        assert_eq!(NetworkWallet::<Ethereum>::default_signer_address(&wallet), wallet_addr);
    }

    #[test]
    fn resolve_wallet_for_address_governor_chain() {
        let ws = in_memory_signer(0).unwrap();
        let wallet_addr = ws.address();
        let gov_addr = address!("0000000000000000000000000000000000000099");
        let mut senders = HashMap::new();
        senders.insert(
            "my-gov".to_string(),
            ResolvedSender::Governor {
                governor_address: gov_addr,
                timelock_address: None,
                proposer: Box::new(ResolvedSender::Wallet(ws)),
                proposer_script: None,
            },
        );

        // Governor without timelock: broadcast_address() == governor_address
        let wallet = resolve_wallet_for_address(gov_addr, &senders).unwrap();
        assert_eq!(NetworkWallet::<Ethereum>::default_signer_address(&wallet), wallet_addr);
    }

    #[test]
    fn resolve_wallet_for_address_governor_with_timelock() {
        let ws = in_memory_signer(0).unwrap();
        let wallet_addr = ws.address();
        let gov_addr = address!("0000000000000000000000000000000000000099");
        let tl_addr = address!("0000000000000000000000000000000000000088");
        let mut senders = HashMap::new();
        senders.insert(
            "my-gov".to_string(),
            ResolvedSender::Governor {
                governor_address: gov_addr,
                timelock_address: Some(tl_addr),
                proposer: Box::new(ResolvedSender::Wallet(ws)),
                proposer_script: None,
            },
        );

        // Governor with timelock: broadcast_address() == timelock_address
        let wallet = resolve_wallet_for_address(tl_addr, &senders).unwrap();
        assert_eq!(NetworkWallet::<Ethereum>::default_signer_address(&wallet), wallet_addr);
    }

    #[test]
    fn resolve_wallet_for_address_missing_address_error() {
        let senders = HashMap::new();
        let addr = address!("0000000000000000000000000000000000000001");
        let err = resolve_wallet_for_address(addr, &senders).unwrap_err();

        match err {
            TrebError::Forge(msg) => {
                assert!(
                    msg.contains("no resolved sender found"),
                    "should mention missing sender: {msg}"
                );
                assert!(msg.contains(&format!("{addr}")), "should include the address: {msg}");
            }
            other => panic!("expected Forge error, got {other:?}"),
        }
    }
}
