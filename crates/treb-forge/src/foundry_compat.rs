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

/// Preprocess a `ScriptArgs` into a `PreprocessedState`, abstracting over
/// the signature differences between foundry backends.
///
/// The return type is inferred — callers chain `.compile()` etc. without
/// needing to name the `PreprocessedState` type directly (which lives in
/// a private module in both old and new foundry).
#[cfg(feature = "foundry-nightly")]
pub async fn preprocess_script(
    args: forge_script::ScriptArgs,
) -> Result<
    forge_script::build::PreprocessedState<foundry_evm::core::evm::EthEvmNetwork>,
    eyre::Report,
> {
    use foundry_cli::utils::LoadConfig;
    let (config, evm_opts) = args.load_config_and_evm_opts()?;
    args.preprocess(config, evm_opts).await
}

#[cfg(feature = "foundry-v1-5-1")]
pub async fn preprocess_script(
    args: forge_script::ScriptArgs,
) -> Result<forge_script::build::PreprocessedState, eyre::Report> {
    args.preprocess().await
}

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
