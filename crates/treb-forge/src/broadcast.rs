//! Broadcast file reading for historical deployment data.
//!
//! Reads forge broadcast JSON files to recover past deployment transactions
//! and contract addresses as a supplementary data source.

use alloy_network::Ethereum;
use forge_script_sequence::{BroadcastReader as FoundryBroadcastReader, ScriptSequence};
use foundry_config::Config;
use treb_core::error::TrebError;

/// Data from a forge broadcast file representing a deployment sequence.
pub struct BroadcastData {
    /// The underlying forge script sequence.
    pub sequence: ScriptSequence<Ethereum>,
    /// The chain ID this broadcast was executed on.
    pub chain_id: u64,
    /// Unix timestamp (milliseconds) of when this broadcast was executed.
    pub timestamp: u128,
}

/// A simplified view of a transaction from a broadcast file.
pub struct BroadcastTransaction {
    /// Transaction hash, if available.
    pub hash: Option<String>,
    /// Name of the contract involved, if available.
    pub contract_name: Option<String>,
    /// Deployed contract address, if available.
    pub contract_address: Option<String>,
    /// Function signature called, if available.
    pub function: Option<String>,
    /// Transaction type (e.g., "Create", "Call", "Create2").
    pub tx_type: String,
}

impl BroadcastData {
    /// Maps the sequence's transactions to simplified `BroadcastTransaction` structs.
    pub fn transactions(&self) -> Vec<BroadcastTransaction> {
        self.sequence
            .transactions
            .iter()
            .map(|tx| BroadcastTransaction {
                hash: tx.hash.map(|h| h.to_string()),
                contract_name: tx.contract_name.clone(),
                contract_address: tx.contract_address.map(|a| a.to_string()),
                function: tx.function.clone(),
                tx_type: format!("{:?}", tx.opcode),
            })
            .collect()
    }
}

/// Read the latest broadcast for a contract on a given chain.
///
/// Resolves the broadcast directory from the foundry config and reads the
/// most recent broadcast file matching the contract name and chain ID.
pub fn read_latest_broadcast(
    config: &Config,
    contract_name: &str,
    chain_id: u64,
) -> treb_core::Result<BroadcastData> {
    let broadcast_path = config.broadcast.clone();

    let reader = FoundryBroadcastReader::new(contract_name.to_string(), chain_id, &broadcast_path)
        .map_err(|e| {
            TrebError::Forge(format!(
                "failed to read broadcasts for '{}' (chain {}) in {}: {}",
                contract_name,
                chain_id,
                broadcast_path.display(),
                e
            ))
        })?;

    let sequence = reader.read_latest().map_err(|e| {
        TrebError::Forge(format!(
            "no broadcasts found for '{}' (chain {}) in {}: {}",
            contract_name,
            chain_id,
            broadcast_path.display(),
            e
        ))
    })?;

    Ok(BroadcastData { chain_id: sequence.chain, timestamp: sequence.timestamp, sequence })
}

/// Read all broadcasts for a contract on a given chain, sorted newest first.
///
/// Returns all broadcast sequences found in the broadcast directory that
/// match the contract name and chain ID.
pub fn read_all_broadcasts(
    config: &Config,
    contract_name: &str,
    chain_id: u64,
) -> treb_core::Result<Vec<BroadcastData>> {
    let broadcast_path = config.broadcast.clone();

    let reader = FoundryBroadcastReader::new(contract_name.to_string(), chain_id, &broadcast_path)
        .map_err(|e| {
            TrebError::Forge(format!(
                "failed to read broadcasts for '{}' (chain {}) in {}: {}",
                contract_name,
                chain_id,
                broadcast_path.display(),
                e
            ))
        })?;

    let sequences = reader.read().map_err(|e| {
        TrebError::Forge(format!(
            "failed to read broadcasts for '{}' (chain {}) in {}: {}",
            contract_name,
            chain_id,
            broadcast_path.display(),
            e
        ))
    })?;

    Ok(sequences
        .into_iter()
        .map(|seq| BroadcastData { chain_id: seq.chain, timestamp: seq.timestamp, sequence: seq })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_latest_broadcast_missing_dir_returns_forge_error() {
        let dir = tempfile::tempdir().unwrap();
        let foundry_toml = dir.path().join("foundry.toml");
        std::fs::write(&foundry_toml, "[profile.default]\nsrc = \"src\"\n").unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();

        let config = Config::load_with_root(dir.path()).expect("config should load");
        let result = read_latest_broadcast(&config, "Counter", 31337);

        match result {
            Err(TrebError::Forge(msg)) => {
                assert!(msg.contains("Counter"), "error should mention contract name, got: {msg}");
                assert!(msg.contains("31337"), "error should mention chain ID, got: {msg}");
                assert!(
                    msg.contains("broadcast"),
                    "error should mention broadcast dir, got: {msg}"
                );
            }
            Err(other) => panic!("expected TrebError::Forge, got: {other}"),
            Ok(_) => panic!("expected error for missing broadcast dir, got Ok"),
        }
    }

    #[test]
    fn read_all_broadcasts_missing_dir_returns_forge_error() {
        let dir = tempfile::tempdir().unwrap();
        let foundry_toml = dir.path().join("foundry.toml");
        std::fs::write(&foundry_toml, "[profile.default]\nsrc = \"src\"\n").unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();

        let config = Config::load_with_root(dir.path()).expect("config should load");
        let result = read_all_broadcasts(&config, "Deploy", 1);

        match result {
            Err(TrebError::Forge(msg)) => {
                assert!(msg.contains("Deploy"), "error should mention contract name, got: {msg}");
                assert!(
                    msg.contains("broadcast"),
                    "error should mention broadcast dir, got: {msg}"
                );
            }
            Err(other) => panic!("expected TrebError::Forge, got: {other}"),
            Ok(_) => panic!("expected error for missing broadcast dir, got Ok"),
        }
    }
}
