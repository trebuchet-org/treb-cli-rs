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
