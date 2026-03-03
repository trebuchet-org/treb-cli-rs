//! Fork-mode state types.
//!
//! These types track active Anvil fork instances, their configuration,
//! and a history of fork-mode actions for auditing.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// SnapshotEntry
// ---------------------------------------------------------------------------

/// An EVM snapshot taken during a fork session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotEntry {
    /// Sequential index of this snapshot within the fork session.
    pub index: u32,
    /// The EVM snapshot ID returned by `evm_snapshot` (hex string).
    pub snapshot_id: String,
    /// The command/action that triggered this snapshot (e.g. "enter", "revert", "restart").
    pub command: String,
    /// When the snapshot was taken.
    pub timestamp: DateTime<Utc>,
}

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
    /// Environment variable name for this fork's RPC URL (Go-compatible).
    #[serde(default)]
    pub env_var_name: String,
    /// Original RPC URL before fork mode was entered (Go-compatible).
    #[serde(default)]
    pub original_rpc: String,
    /// PID of the Anvil process (Go-compatible).
    #[serde(default)]
    pub anvil_pid: i32,
    /// Path to the PID file for the Anvil process (Go-compatible).
    #[serde(default)]
    pub pid_file: String,
    /// Path to the log file for the Anvil process (Go-compatible).
    #[serde(default)]
    pub log_file: String,
    /// When fork mode was entered (Go-compatible).
    pub entered_at: DateTime<Utc>,
    /// EVM snapshots taken during this fork session.
    #[serde(default)]
    pub snapshots: Vec<SnapshotEntry>,
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

/// Top-level fork state persisted to `fork.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkState {
    /// Active forks keyed by network name.
    pub forks: HashMap<String, ForkEntry>,
    /// History of fork actions (most recent first).
    pub history: Vec<ForkHistoryEntry>,
}

impl Default for ForkState {
    fn default() -> Self {
        Self {
            forks: HashMap::new(),
            history: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn sample_fork_entry() -> ForkEntry {
        let ts = Utc.with_ymd_and_hms(2026, 3, 3, 12, 0, 0).unwrap();
        ForkEntry {
            network: "mainnet".into(),
            rpc_url: "http://127.0.0.1:8545".into(),
            port: 8545,
            chain_id: 1,
            fork_url: "https://eth.llamarpc.com".into(),
            fork_block_number: Some(19_000_000),
            snapshot_dir: ".treb/snapshots/mainnet".into(),
            started_at: ts,
            env_var_name: "ETH_RPC_URL".into(),
            original_rpc: "https://eth.llamarpc.com".into(),
            anvil_pid: 0,
            pid_file: String::new(),
            log_file: String::new(),
            entered_at: ts,
            snapshots: vec![],
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

        // Existing fields
        assert!(obj.contains_key("network"));
        assert!(obj.contains_key("rpcUrl"));
        assert!(obj.contains_key("port"));
        assert!(obj.contains_key("chainId"));
        assert!(obj.contains_key("forkUrl"));
        assert!(obj.contains_key("forkBlockNumber"));
        assert!(obj.contains_key("snapshotDir"));
        assert!(obj.contains_key("startedAt"));

        // Go-compatible fields
        assert!(obj.contains_key("envVarName"));
        assert!(obj.contains_key("originalRpc"));
        assert!(obj.contains_key("anvilPid"));
        assert!(obj.contains_key("pidFile"));
        assert!(obj.contains_key("logFile"));
        assert!(obj.contains_key("enteredAt"));
        assert!(obj.contains_key("snapshots"));

        // No snake_case
        assert!(!obj.contains_key("rpc_url"));
        assert!(!obj.contains_key("chain_id"));
        assert!(!obj.contains_key("fork_url"));
        assert!(!obj.contains_key("fork_block_number"));
        assert!(!obj.contains_key("snapshot_dir"));
        assert!(!obj.contains_key("started_at"));
        assert!(!obj.contains_key("env_var_name"));
        assert!(!obj.contains_key("original_rpc"));
        assert!(!obj.contains_key("anvil_pid"));
        assert!(!obj.contains_key("pid_file"));
        assert!(!obj.contains_key("log_file"));
        assert!(!obj.contains_key("entered_at"));
        assert!(!obj.contains_key("evm_snapshot_id"));
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
            .forks
            .insert("mainnet".into(), sample_fork_entry());
        state.history.push(sample_history_entry());

        let json_str = serde_json::to_string_pretty(&state).unwrap();
        let deserialized: ForkState = serde_json::from_str(&json_str).unwrap();
        assert_eq!(state, deserialized);
    }

    #[test]
    fn fork_state_top_level_keys_match_go_schema() {
        let mut state = ForkState::default();
        state
            .forks
            .insert("mainnet".into(), sample_fork_entry());
        state.history.push(sample_history_entry());

        let json = serde_json::to_value(&state).unwrap();
        let obj = json.as_object().unwrap();

        // Go CLI uses "forks" and "history" as top-level keys
        assert!(obj.contains_key("forks"), "expected 'forks' key");
        assert!(obj.contains_key("history"), "expected 'history' key");
        assert!(!obj.contains_key("activeForks"), "should not have 'activeForks' key");
    }

    #[test]
    fn fork_state_default_produces_empty_collections() {
        let state = ForkState::default();
        assert!(state.forks.is_empty());
        assert!(state.history.is_empty());
    }
}
