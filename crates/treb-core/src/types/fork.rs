//! Fork-mode state types.
//!
//! These types track active Anvil fork instances, their configuration,
//! and a history of fork-mode actions for auditing.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// ForkEntry
// ---------------------------------------------------------------------------

/// An active fork instance tracked in fork state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkEntry {
    /// Network name this fork targets.
    pub network: String,
    /// Local RPC URL of the Anvil instance.
    pub rpc_url: String,
    /// Port the Anvil instance is listening on.
    pub port: u16,
    /// Chain ID of the forked network.
    pub chain_id: u64,
    /// Upstream RPC URL being forked.
    pub fork_url: String,
    /// Block number the fork was taken from (if specified).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_block_number: Option<u64>,
    /// Directory containing the registry snapshot.
    pub snapshot_dir: String,
    /// When the fork was started.
    pub started_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// ForkHistoryEntry
// ---------------------------------------------------------------------------

/// A record of a fork-mode action for auditing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkHistoryEntry {
    /// The action that was performed (e.g. "enter", "exit", "revert", "restart").
    pub action: String,
    /// Network the action was performed on.
    pub network: String,
    /// When the action occurred.
    pub timestamp: DateTime<Utc>,
    /// Optional details about the action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

// ---------------------------------------------------------------------------
// ForkState
// ---------------------------------------------------------------------------

/// Top-level fork state persisted to `fork-state.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkState {
    /// Active forks keyed by network name.
    pub active_forks: HashMap<String, ForkEntry>,
    /// History of fork actions (most recent first).
    pub history: Vec<ForkHistoryEntry>,
}

impl Default for ForkState {
    fn default() -> Self {
        Self {
            active_forks: HashMap::new(),
            history: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn sample_fork_entry() -> ForkEntry {
        ForkEntry {
            network: "mainnet".into(),
            rpc_url: "http://127.0.0.1:8545".into(),
            port: 8545,
            chain_id: 1,
            fork_url: "https://eth.llamarpc.com".into(),
            fork_block_number: Some(19_000_000),
            snapshot_dir: ".treb/snapshots/mainnet".into(),
            started_at: Utc.with_ymd_and_hms(2026, 3, 3, 12, 0, 0).unwrap(),
        }
    }

    fn sample_history_entry() -> ForkHistoryEntry {
        ForkHistoryEntry {
            action: "enter".into(),
            network: "mainnet".into(),
            timestamp: Utc.with_ymd_and_hms(2026, 3, 3, 12, 0, 0).unwrap(),
            details: None,
        }
    }

    #[test]
    fn fork_entry_serde_round_trip() {
        let entry = sample_fork_entry();
        let json_str = serde_json::to_string_pretty(&entry).unwrap();
        let deserialized: ForkEntry = serde_json::from_str(&json_str).unwrap();
        assert_eq!(entry, deserialized);
    }

    #[test]
    fn fork_entry_camel_case_keys() {
        let entry = sample_fork_entry();
        let json = serde_json::to_value(&entry).unwrap();
        let obj = json.as_object().unwrap();

        assert!(obj.contains_key("network"));
        assert!(obj.contains_key("rpcUrl"));
        assert!(obj.contains_key("port"));
        assert!(obj.contains_key("chainId"));
        assert!(obj.contains_key("forkUrl"));
        assert!(obj.contains_key("forkBlockNumber"));
        assert!(obj.contains_key("snapshotDir"));
        assert!(obj.contains_key("startedAt"));

        // No snake_case
        assert!(!obj.contains_key("rpc_url"));
        assert!(!obj.contains_key("chain_id"));
        assert!(!obj.contains_key("fork_url"));
        assert!(!obj.contains_key("fork_block_number"));
        assert!(!obj.contains_key("snapshot_dir"));
        assert!(!obj.contains_key("started_at"));
    }

    #[test]
    fn fork_entry_optional_block_number_omitted() {
        let mut entry = sample_fork_entry();
        entry.fork_block_number = None;
        let json = serde_json::to_value(&entry).unwrap();
        let obj = json.as_object().unwrap();
        assert!(!obj.contains_key("forkBlockNumber"));
    }

    #[test]
    fn fork_history_entry_serde_round_trip() {
        let entry = sample_history_entry();
        let json_str = serde_json::to_string_pretty(&entry).unwrap();
        let deserialized: ForkHistoryEntry = serde_json::from_str(&json_str).unwrap();
        assert_eq!(entry, deserialized);
    }

    #[test]
    fn fork_history_entry_details_omitted_when_none() {
        let entry = sample_history_entry();
        let json = serde_json::to_value(&entry).unwrap();
        let obj = json.as_object().unwrap();
        assert!(!obj.contains_key("details"));
    }

    #[test]
    fn fork_history_entry_details_present_when_some() {
        let mut entry = sample_history_entry();
        entry.details = Some("reverted to snapshot 0x1".into());
        let json = serde_json::to_value(&entry).unwrap();
        assert_eq!(json["details"], "reverted to snapshot 0x1");
    }

    #[test]
    fn fork_state_serde_round_trip() {
        let mut state = ForkState::default();
        state
            .active_forks
            .insert("mainnet".into(), sample_fork_entry());
        state.history.push(sample_history_entry());

        let json_str = serde_json::to_string_pretty(&state).unwrap();
        let deserialized: ForkState = serde_json::from_str(&json_str).unwrap();
        assert_eq!(state, deserialized);
    }

    #[test]
    fn fork_state_default_produces_empty_collections() {
        let state = ForkState::default();
        assert!(state.active_forks.is_empty());
        assert!(state.history.is_empty());
    }
}
