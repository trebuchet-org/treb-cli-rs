//! Thin compatibility helpers for Foundry/Alloy API drift across supported
//! backend versions.

use alloy_primitives::Address;

pub type BroadcastableTransaction = foundry_cheatcodes::BroadcastableTransaction;

#[cfg(feature = "foundry-nightly")]
pub type BroadcastableTransactions =
    foundry_cheatcodes::BroadcastableTransactions<alloy_network::Ethereum>;

#[cfg(feature = "foundry-v1-5-1")]
pub type BroadcastableTransactions = foundry_cheatcodes::BroadcastableTransactions;

#[cfg(feature = "foundry-nightly")]
pub type ScriptSequence = forge_script_sequence::ScriptSequence<alloy_network::Ethereum>;

#[cfg(feature = "foundry-v1-5-1")]
pub type ScriptSequence = forge_script_sequence::ScriptSequence;

#[cfg(feature = "foundry-nightly")]
pub type TransactionMaybeSigned = foundry_common::TransactionMaybeSigned<alloy_network::Ethereum>;

#[cfg(feature = "foundry-v1-5-1")]
pub type TransactionMaybeSigned = foundry_common::TransactionMaybeSigned;

#[cfg(all(feature = "foundry-nightly", feature = "foundry-v1-5-1"))]
compile_error!("foundry-nightly and foundry-v1-5-1 cannot both be enabled");

#[cfg(not(any(feature = "foundry-nightly", feature = "foundry-v1-5-1")))]
compile_error!("one Foundry backend feature must be enabled");

pub fn make_tx_maybe_signed(
    request: alloy_rpc_types::TransactionRequest,
) -> TransactionMaybeSigned {
    foundry_common::TransactionMaybeSigned::new(request.into())
}

#[cfg(feature = "foundry-nightly")]
pub fn broadcast_tx_to_address(tx: &BroadcastableTransaction) -> Option<Address> {
    tx.transaction.to()
}

#[cfg(feature = "foundry-v1-5-1")]
pub fn broadcast_tx_to_address(tx: &BroadcastableTransaction) -> Option<Address> {
    match tx.transaction.to() {
        Some(alloy_primitives::TxKind::Call(addr)) => Some(addr),
        Some(alloy_primitives::TxKind::Create) | None => None,
    }
}

pub fn broadcast_tx_is_create(tx: &BroadcastableTransaction) -> bool {
    broadcast_tx_to_address(tx).is_none()
}

// ---------------------------------------------------------------------------
// Script pipeline: preprocess entry point
// ---------------------------------------------------------------------------
//
// Foundry nightly (post b68f4e28) made `ScriptArgs::preprocess()` private and
// changed its signature to take `config` and `evm_opts` as arguments. We wrap
// it here with the original zero-argument calling convention, resolving config
// internally using the same `LoadConfig` trait foundry uses.
//
// The nightly preprocess is also generic over `FoundryEvmNetwork`; we
// specialize on `EthEvmNetwork` here (Tempo network support will be added
// when treb's pipeline is made network-generic).

/// Preprocess `ScriptArgs` into a `PreprocessedState`.
///
/// This macro abstracts over the API differences between foundry backends:
/// - **Nightly**: `preprocess()` is private and takes `(config, evm_opts)` + a `FoundryEvmNetwork`
///   generic. We resolve config via `LoadConfig` and specialize on `EthEvmNetwork`.
/// - **v1.5.1 / v1.6.0-rc1**: `preprocess()` is public on `ScriptArgs`, so we call it directly.
///   Using a macro avoids naming the `PreprocessedState` type which lives in a private module.
macro_rules! preprocess_script {
    ($args:expr) => {{
        #[cfg(feature = "foundry-nightly")]
        {
            use foundry_cli::utils::LoadConfig as _;
            async {
                let (config, evm_opts) = $args.load_config_and_evm_opts()?;
                $args.preprocess::<foundry_evm::core::evm::EthEvmNetwork>(config, evm_opts).await
            }
            .await
        }
        #[cfg(feature = "foundry-v1-5-1")]
        {
            $args.preprocess().await
        }
    }};
}
pub(crate) use preprocess_script;

// ---------------------------------------------------------------------------
// Transaction metadata: opcode → call_kind rename
// ---------------------------------------------------------------------------
//
// Foundry nightly (791b10e0) renamed `TransactionWithMetadata::opcode` to
// `call_kind`. These helpers abstract the field access.

// Re-export CallKind from foundry_evm::traces which re-exports from the same
// version of revm-inspectors that forge-script-sequence uses.
pub use foundry_evm::traces::CallKind;

#[cfg(feature = "foundry-nightly")]
pub type TransactionWithMetadata =
    forge_script_sequence::TransactionWithMetadata<alloy_network::Ethereum>;

#[cfg(feature = "foundry-v1-5-1")]
pub type TransactionWithMetadata = forge_script_sequence::TransactionWithMetadata;

#[cfg(feature = "foundry-nightly")]
pub fn tx_meta_call_kind(tx: &TransactionWithMetadata) -> CallKind {
    tx.call_kind
}

#[cfg(feature = "foundry-v1-5-1")]
pub fn tx_meta_call_kind(tx: &TransactionWithMetadata) -> CallKind {
    tx.opcode
}

#[cfg(feature = "foundry-nightly")]
pub fn set_tx_meta_call_kind(tx: &mut TransactionWithMetadata, kind: CallKind) {
    tx.call_kind = kind;
}

#[cfg(feature = "foundry-v1-5-1")]
pub fn set_tx_meta_call_kind(tx: &mut TransactionWithMetadata, kind: CallKind) {
    tx.opcode = kind;
}
