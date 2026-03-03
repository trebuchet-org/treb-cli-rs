//! Sender/wallet resolution for treb.
//!
//! Bridges treb's `SenderConfig` definitions with foundry's `WalletSigner`
//! instances. Each sender type (PrivateKey, Ledger, Trezor, Safe, Governor)
//! is resolved into a `ResolvedSender` that can be wired into `ScriptArgs`
//! for in-process forge execution.

use std::collections::{HashMap, HashSet};

use alloy_primitives::Address;
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
    _name: &str,
    _config: &SenderConfig,
) -> treb_core::Result<ResolvedSender> {
    todo!("US-002: PrivateKey sender resolution")
}

async fn resolve_ledger(
    _name: &str,
    _config: &SenderConfig,
) -> treb_core::Result<ResolvedSender> {
    todo!("US-002: Ledger sender resolution")
}

async fn resolve_trezor(
    _name: &str,
    _config: &SenderConfig,
) -> treb_core::Result<ResolvedSender> {
    todo!("US-002: Trezor sender resolution")
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
