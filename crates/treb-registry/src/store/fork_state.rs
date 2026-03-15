//! Persistent store for fork-mode state and registry snapshot/restore.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use treb_core::{
    TrebError,
    types::fork::{ForkEntry, ForkHistoryEntry, ForkState},
};

use crate::{
    DEPLOYMENTS_FILE, FORK_STATE_FILE, GOVERNOR_PROPOSALS_FILE, LOOKUP_FILE, SAFE_TXS_FILE,
    SOLIDITY_REGISTRY_FILE, TRANSACTIONS_FILE,
    io::{read_versioned_file_compat, write_versioned_file},
};

/// Registry JSON files that are snapshotted/restored during fork mode.
const SNAPSHOT_FILES: &[&str] = &[
    DEPLOYMENTS_FILE,
    TRANSACTIONS_FILE,
    SAFE_TXS_FILE,
    GOVERNOR_PROPOSALS_FILE,
    LOOKUP_FILE,
    SOLIDITY_REGISTRY_FILE,
];

/// Maximum number of history entries retained.
const MAX_HISTORY: usize = 100;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PersistedForkState {
    forks: BTreeMap<String, ForkEntry>,
    history: Vec<ForkHistoryEntry>,
}

impl From<&ForkState> for PersistedForkState {
    fn from(state: &ForkState) -> Self {
        Self {
            forks: state.forks.iter().map(|(key, entry)| (key.clone(), entry.clone())).collect(),
            history: state.history.clone(),
        }
    }
}

fn active_fork_key(network: &str, instance_name: Option<&str>) -> String {
    match instance_name {
        Some(name) if name != network => format!("{network}:{name}"),
        _ => network.to_string(),
    }
}

fn active_fork_entry_key(entry: &ForkEntry) -> String {
    active_fork_key(&entry.network, entry.instance_name.as_deref())
}

// ── Snapshot / Restore ──────────────────────────────────────────────────────

/// Copy registry JSON files from `registry_dir` to `snapshot_dir`.
///
/// Missing files are silently skipped. Creates `snapshot_dir` if it does not
/// exist.
pub fn snapshot_registry(registry_dir: &Path, snapshot_dir: &Path) -> Result<(), TrebError> {
    fs::create_dir_all(snapshot_dir)?;
    for &file in SNAPSHOT_FILES {
        let src = registry_dir.join(file);
        if src.exists() {
            fs::copy(&src, snapshot_dir.join(file))?;
        } else if let Some(legacy_src) = crate::legacy_registry_store_path(&src) {
            if legacy_src.exists() {
                fs::copy(&legacy_src, snapshot_dir.join(file))?;
            }
        }
    }
    Ok(())
}

/// Restore all `.json` files from `snapshot_dir` back to `registry_dir`,
/// overwriting current state.
///
/// Returns `TrebError::Fork` if `snapshot_dir` does not exist.
pub fn restore_registry(snapshot_dir: &Path, registry_dir: &Path) -> Result<(), TrebError> {
    if !snapshot_dir.exists() {
        return Err(TrebError::Fork(format!(
            "snapshot directory does not exist: {}",
            snapshot_dir.display()
        )));
    }
    for &file in SNAPSHOT_FILES {
        let snapshot_file = snapshot_dir.join(file);
        let registry_file = registry_dir.join(file);
        if snapshot_file.exists() {
            fs::copy(&snapshot_file, &registry_file)?;
        } else if let Some(legacy_snapshot_file) = crate::legacy_registry_store_path(&snapshot_file)
        {
            if legacy_snapshot_file.exists() {
                fs::copy(&legacy_snapshot_file, &registry_file)?;
            } else if registry_file.exists() {
                fs::remove_file(&registry_file)?;
            }
        } else if registry_file.exists() {
            fs::remove_file(&registry_file)?;
        }
    }
    Ok(())
}

/// Remove the snapshot directory and all its contents.
pub fn remove_snapshot(snapshot_dir: &Path) -> Result<(), TrebError> {
    if snapshot_dir.exists() {
        fs::remove_dir_all(snapshot_dir)?;
    }
    Ok(())
}

// ── ForkStateStore ──────────────────────────────────────────────────────────

/// Persistent store for fork-mode state backed by `fork.json`.
pub struct ForkStateStore {
    path: PathBuf,
    data: ForkState,
}

impl ForkStateStore {
    /// Create a new store pointing at `<registry_dir>/fork.json`.
    /// Call [`load`](Self::load) to read existing data from disk.
    pub fn new(registry_dir: &Path) -> Self {
        Self { path: registry_dir.join(FORK_STATE_FILE), data: ForkState::default() }
    }

    /// Load fork state from disk, replacing any in-memory data.
    pub fn load(&mut self) -> Result<(), TrebError> {
        self.data = read_versioned_file_compat(&self.path)?;
        Ok(())
    }

    /// Atomically save fork state to disk under the versioned store wrapper.
    pub fn save(&self) -> Result<(), TrebError> {
        let persisted = PersistedForkState::from(&self.data);
        write_versioned_file(&self.path, &persisted)
    }

    /// Get the default active fork entry by network name.
    pub fn get_active_fork(&self, network: &str) -> Option<&ForkEntry> {
        self.data.forks.get(network)
    }

    /// Get an active fork entry by network + instance identifier.
    pub fn get_active_fork_instance(
        &self,
        network: &str,
        instance_name: &str,
    ) -> Option<&ForkEntry> {
        self.data.forks.get(&active_fork_key(network, Some(instance_name)))
    }

    /// Insert a new active fork entry. Returns an error if the key is already
    /// present.
    pub fn insert_active_fork(&mut self, entry: ForkEntry) -> Result<(), TrebError> {
        let key = active_fork_entry_key(&entry);
        if self.data.forks.contains_key(&key) {
            let msg = if key == entry.network {
                format!("network already forked: {}", entry.network)
            } else {
                format!("fork already tracked for key '{}'", key)
            };
            return Err(TrebError::Fork(msg));
        }
        self.data.forks.insert(key, entry);
        self.save()
    }

    /// Update an existing active fork entry in place.
    ///
    /// Returns an error if the key is not actively forked.
    pub fn update_active_fork(&mut self, entry: ForkEntry) -> Result<(), TrebError> {
        let key = active_fork_entry_key(&entry);
        if !self.data.forks.contains_key(&key) {
            return Err(TrebError::Fork(format!("fork is not actively tracked: {key}")));
        }
        self.data.forks.insert(key, entry);
        self.save()
    }

    /// Insert or update an active fork entry in place.
    pub fn upsert_active_fork(&mut self, entry: ForkEntry) -> Result<(), TrebError> {
        let key = active_fork_entry_key(&entry);
        self.data.forks.insert(key, entry);
        self.save()
    }

    /// Remove the default active fork entry by network name. Returns an error
    /// if the network is not actively forked.
    pub fn remove_active_fork(&mut self, network: &str) -> Result<ForkEntry, TrebError> {
        let entry =
            self.data.forks.remove(&active_fork_key(network, None)).ok_or_else(|| {
                TrebError::Fork(format!("network is not actively forked: {network}"))
            })?;
        self.save()?;
        Ok(entry)
    }

    /// Remove an active fork entry by network + instance identifier.
    pub fn remove_active_fork_instance(
        &mut self,
        network: &str,
        instance_name: &str,
    ) -> Result<ForkEntry, TrebError> {
        let key = active_fork_key(network, Some(instance_name));
        let entry = self
            .data
            .forks
            .remove(&key)
            .ok_or_else(|| TrebError::Fork(format!("fork is not actively tracked: {key}")))?;
        self.save()?;
        Ok(entry)
    }

    /// Remove every active fork entry associated with a network.
    pub fn remove_active_forks_for_network(
        &mut self,
        network: &str,
    ) -> Result<Vec<ForkEntry>, TrebError> {
        let keys: Vec<String> = self
            .data
            .forks
            .iter()
            .filter(|(_, entry)| entry.network == network)
            .map(|(key, _)| key.clone())
            .collect();

        if keys.is_empty() {
            return Err(TrebError::Fork(format!("network is not actively forked: {network}")));
        }

        let removed =
            keys.into_iter().filter_map(|key| self.data.forks.remove(&key)).collect::<Vec<_>>();
        self.save()?;
        Ok(removed)
    }

    /// List all active fork entries.
    pub fn list_active_forks(&self) -> Vec<&ForkEntry> {
        self.data.forks.values().collect()
    }

    /// List all active fork entries for a single network.
    pub fn list_active_forks_for_network(&self, network: &str) -> Vec<&ForkEntry> {
        self.data.forks.values().filter(|entry| entry.network == network).collect()
    }

    /// List the session-level active fork entries (one default entry per network).
    pub fn list_active_fork_sessions(&self) -> Vec<&ForkEntry> {
        self.data.forks.values().filter(|entry| entry.instance_name.is_none()).collect()
    }

    /// Return whether any fork state is tracked for a network.
    pub fn has_active_fork_network(&self, network: &str) -> bool {
        self.data.forks.values().any(|entry| entry.network == network)
    }

    /// List active network names with duplicates removed.
    pub fn list_active_networks(&self) -> Vec<String> {
        self.data
            .forks
            .values()
            .map(|entry| entry.network.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// Prepend a history entry. Caps history at [`MAX_HISTORY`] entries.
    pub fn add_history(&mut self, entry: ForkHistoryEntry) -> Result<(), TrebError> {
        self.data.history.insert(0, entry);
        self.data.history.truncate(MAX_HISTORY);
        self.save()
    }

    /// Return a reference to the underlying fork state.
    pub fn data(&self) -> &ForkState {
        &self.data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::{TimeZone, Utc};
    use serde_json::json;
    use tempfile::TempDir;

    use crate::{
        STORE_FORMAT,
        io::{VersionedStore, read_json_file, write_json_file},
    };

    fn sample_fork_entry(network: &str) -> ForkEntry {
        let ts = Utc.with_ymd_and_hms(2026, 3, 3, 12, 0, 0).unwrap();
        ForkEntry {
            network: network.into(),
            instance_name: None,
            rpc_url: "http://127.0.0.1:8545".into(),
            port: 8545,
            chain_id: 1,
            fork_url: "https://eth.llamarpc.com".into(),
            fork_block_number: Some(19_000_000),
            snapshot_dir: format!(".treb/snapshots/{network}"),
            started_at: ts,
            env_var_name: String::new(),
            original_rpc: String::new(),
            anvil_pid: 0,
            pid_file: String::new(),
            log_file: String::new(),
            entered_at: ts,
            snapshots: vec![],
        }
    }

    fn sample_history_entry(action: &str, network: &str) -> ForkHistoryEntry {
        ForkHistoryEntry {
            action: action.into(),
            network: network.into(),
            timestamp: Utc.with_ymd_and_hms(2026, 3, 3, 12, 0, 0).unwrap(),
            details: None,
        }
    }

    // ── snapshot_registry tests ─────────────────────────────────────────

    #[test]
    fn snapshot_copies_existing_files() {
        let reg_dir = TempDir::new().unwrap();
        let snap_dir = TempDir::new().unwrap();
        let snap_path = snap_dir.path().join("snapshot");

        // Create some registry files
        fs::write(reg_dir.path().join(DEPLOYMENTS_FILE), r#"{"dep1": {}}"#).unwrap();
        fs::write(reg_dir.path().join(TRANSACTIONS_FILE), r#"{"tx1": {}}"#).unwrap();

        snapshot_registry(reg_dir.path(), &snap_path).unwrap();

        assert!(snap_path.join(DEPLOYMENTS_FILE).exists());
        assert!(snap_path.join(TRANSACTIONS_FILE).exists());
        assert_eq!(
            fs::read_to_string(snap_path.join(DEPLOYMENTS_FILE)).unwrap(),
            r#"{"dep1": {}}"#
        );
    }

    #[test]
    fn snapshot_skips_missing_files() {
        let reg_dir = TempDir::new().unwrap();
        let snap_dir = TempDir::new().unwrap();
        let snap_path = snap_dir.path().join("snapshot");

        // Only create one file
        fs::write(reg_dir.path().join(DEPLOYMENTS_FILE), "{}").unwrap();

        snapshot_registry(reg_dir.path(), &snap_path).unwrap();

        assert!(snap_path.join(DEPLOYMENTS_FILE).exists());
        assert!(!snap_path.join(TRANSACTIONS_FILE).exists());
        assert!(!snap_path.join(SAFE_TXS_FILE).exists());
    }

    #[test]
    fn snapshot_copies_legacy_safe_transactions_file_using_current_name() {
        let reg_dir = TempDir::new().unwrap();
        let snap_dir = TempDir::new().unwrap();
        let snap_path = snap_dir.path().join("snapshot");

        fs::write(reg_dir.path().join("safe_txs.json"), r#"{"legacy": true}"#).unwrap();

        snapshot_registry(reg_dir.path(), &snap_path).unwrap();

        assert_eq!(
            fs::read_to_string(snap_path.join(SAFE_TXS_FILE)).unwrap(),
            r#"{"legacy": true}"#
        );
    }

    // ── restore_registry tests ──────────────────────────────────────────

    #[test]
    fn restore_overwrites_registry() {
        let reg_dir = TempDir::new().unwrap();
        let snap_dir = TempDir::new().unwrap();

        // Current registry state
        fs::write(reg_dir.path().join(DEPLOYMENTS_FILE), r#"{"new": true}"#).unwrap();

        // Snapshot with old state
        fs::write(snap_dir.path().join(DEPLOYMENTS_FILE), r#"{"old": true}"#).unwrap();

        restore_registry(snap_dir.path(), reg_dir.path()).unwrap();

        let content = fs::read_to_string(reg_dir.path().join(DEPLOYMENTS_FILE)).unwrap();
        assert_eq!(content, r#"{"old": true}"#);
    }

    #[test]
    fn restore_removes_files_missing_from_snapshot() {
        let reg_dir = TempDir::new().unwrap();
        let snap_dir = TempDir::new().unwrap();

        fs::write(reg_dir.path().join(DEPLOYMENTS_FILE), r#"{"fork_only": true}"#).unwrap();
        fs::write(snap_dir.path().join(TRANSACTIONS_FILE), "{}").unwrap();

        restore_registry(snap_dir.path(), reg_dir.path()).unwrap();

        assert!(
            !reg_dir.path().join(DEPLOYMENTS_FILE).exists(),
            "restore should remove registry files that were absent from the snapshot"
        );
        assert!(
            reg_dir.path().join(TRANSACTIONS_FILE).exists(),
            "restore should still copy files that do exist in the snapshot"
        );
    }

    #[test]
    fn restore_reads_legacy_snapshot_filename() {
        let reg_dir = TempDir::new().unwrap();
        let snap_dir = TempDir::new().unwrap();

        fs::write(reg_dir.path().join(SAFE_TXS_FILE), r#"{"new": true}"#).unwrap();
        fs::write(snap_dir.path().join("safe_txs.json"), r#"{"legacy": true}"#).unwrap();

        restore_registry(snap_dir.path(), reg_dir.path()).unwrap();

        assert_eq!(
            fs::read_to_string(reg_dir.path().join(SAFE_TXS_FILE)).unwrap(),
            r#"{"legacy": true}"#
        );
    }

    #[test]
    fn restore_errors_on_missing_snapshot_dir() {
        let reg_dir = TempDir::new().unwrap();
        let missing = reg_dir.path().join("nonexistent");

        let result = restore_registry(&missing, reg_dir.path());
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("snapshot directory does not exist"),
            "expected snapshot error, got: {msg}"
        );
    }

    // ── remove_snapshot tests ───────────────────────────────────────────

    #[test]
    fn remove_cleans_up_snapshot_dir() {
        let dir = TempDir::new().unwrap();
        let snap_dir = dir.path().join("snapshot");
        fs::create_dir_all(&snap_dir).unwrap();
        fs::write(snap_dir.join("test.json"), "{}").unwrap();

        remove_snapshot(&snap_dir).unwrap();
        assert!(!snap_dir.exists());
    }

    #[test]
    fn remove_noop_for_missing_dir() {
        let dir = TempDir::new().unwrap();
        let snap_dir = dir.path().join("nonexistent");
        // Should not error
        remove_snapshot(&snap_dir).unwrap();
    }

    // ── ForkStateStore tests ────────────────────────────────────────────

    #[test]
    fn insert_and_get_active_fork() {
        let dir = TempDir::new().unwrap();
        let mut store = ForkStateStore::new(dir.path());

        let entry = sample_fork_entry("mainnet");
        store.insert_active_fork(entry.clone()).unwrap();

        let got = store.get_active_fork("mainnet").unwrap();
        assert_eq!(got, &entry);
    }

    #[test]
    fn duplicate_insert_errors() {
        let dir = TempDir::new().unwrap();
        let mut store = ForkStateStore::new(dir.path());

        store.insert_active_fork(sample_fork_entry("mainnet")).unwrap();
        let result = store.insert_active_fork(sample_fork_entry("mainnet"));
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("already forked"), "expected 'already forked' in: {msg}");
    }

    #[test]
    fn named_instances_use_composite_keys() {
        let dir = TempDir::new().unwrap();
        let mut store = ForkStateStore::new(dir.path());

        store.insert_active_fork(sample_fork_entry("mainnet")).unwrap();

        let mut alpha = sample_fork_entry("mainnet");
        alpha.instance_name = Some("alpha".into());
        alpha.port = 8546;
        store.insert_active_fork(alpha.clone()).unwrap();

        let mut beta = sample_fork_entry("mainnet");
        beta.instance_name = Some("beta".into());
        beta.port = 8547;
        store.insert_active_fork(beta.clone()).unwrap();

        assert_eq!(store.get_active_fork("mainnet").unwrap().port, 8545);
        assert_eq!(store.get_active_fork_instance("mainnet", "alpha"), Some(&alpha));
        assert_eq!(store.get_active_fork_instance("mainnet", "beta"), Some(&beta));
    }

    #[test]
    fn remove_active_fork_returns_entry() {
        let dir = TempDir::new().unwrap();
        let mut store = ForkStateStore::new(dir.path());

        store.insert_active_fork(sample_fork_entry("mainnet")).unwrap();
        let removed = store.remove_active_fork("mainnet").unwrap();
        assert_eq!(removed.network, "mainnet");
        assert!(store.get_active_fork("mainnet").is_none());
    }

    #[test]
    fn remove_nonexistent_fork_errors() {
        let dir = TempDir::new().unwrap();
        let mut store = ForkStateStore::new(dir.path());

        let result = store.remove_active_fork("mainnet");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("not actively forked"), "expected 'not actively forked' in: {msg}");
    }

    #[test]
    fn remove_named_fork_returns_entry() {
        let dir = TempDir::new().unwrap();
        let mut store = ForkStateStore::new(dir.path());

        let mut entry = sample_fork_entry("mainnet");
        entry.instance_name = Some("alpha".into());
        store.insert_active_fork(entry).unwrap();

        let removed = store.remove_active_fork_instance("mainnet", "alpha").unwrap();
        assert_eq!(removed.instance_name.as_deref(), Some("alpha"));
        assert!(store.get_active_fork_instance("mainnet", "alpha").is_none());
    }

    #[test]
    fn remove_active_forks_for_network_removes_related_entries() {
        let dir = TempDir::new().unwrap();
        let mut store = ForkStateStore::new(dir.path());

        store.insert_active_fork(sample_fork_entry("mainnet")).unwrap();

        let mut named = sample_fork_entry("mainnet");
        named.instance_name = Some("alpha".into());
        store.insert_active_fork(named).unwrap();

        let removed = store.remove_active_forks_for_network("mainnet").unwrap();
        assert_eq!(removed.len(), 2);
        assert!(!store.has_active_fork_network("mainnet"));
    }

    #[test]
    fn list_active_forks_returns_all() {
        let dir = TempDir::new().unwrap();
        let mut store = ForkStateStore::new(dir.path());

        store.insert_active_fork(sample_fork_entry("mainnet")).unwrap();

        let mut entry2 = sample_fork_entry("sepolia");
        entry2.port = 8546;
        entry2.chain_id = 11155111;
        store.insert_active_fork(entry2).unwrap();

        let forks = store.list_active_forks();
        assert_eq!(forks.len(), 2);
    }

    #[test]
    fn list_active_fork_sessions_ignores_named_entries() {
        let dir = TempDir::new().unwrap();
        let mut store = ForkStateStore::new(dir.path());

        store.insert_active_fork(sample_fork_entry("mainnet")).unwrap();

        let mut named = sample_fork_entry("mainnet");
        named.instance_name = Some("alpha".into());
        store.insert_active_fork(named).unwrap();

        let sessions = store.list_active_fork_sessions();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].network, "mainnet");
        assert_eq!(sessions[0].instance_name, None);
    }

    #[test]
    fn list_active_networks_deduplicates_named_entries() {
        let dir = TempDir::new().unwrap();
        let mut store = ForkStateStore::new(dir.path());

        store.insert_active_fork(sample_fork_entry("mainnet")).unwrap();

        let mut named = sample_fork_entry("mainnet");
        named.instance_name = Some("alpha".into());
        store.insert_active_fork(named).unwrap();

        let mut sepolia = sample_fork_entry("sepolia");
        sepolia.chain_id = 11155111;
        store.insert_active_fork(sepolia).unwrap();

        let networks = store.list_active_networks();
        assert_eq!(networks, vec!["mainnet".to_string(), "sepolia".to_string()]);
    }

    #[test]
    fn add_history_prepends() {
        let dir = TempDir::new().unwrap();
        let mut store = ForkStateStore::new(dir.path());

        store.add_history(sample_history_entry("enter", "mainnet")).unwrap();
        store.add_history(sample_history_entry("exit", "mainnet")).unwrap();

        let data = store.data();
        assert_eq!(data.history.len(), 2);
        assert_eq!(data.history[0].action, "exit"); // Most recent first
        assert_eq!(data.history[1].action, "enter");
    }

    #[test]
    fn history_caps_at_100() {
        let dir = TempDir::new().unwrap();
        let mut store = ForkStateStore::new(dir.path());

        for i in 0..110 {
            store.add_history(sample_history_entry(&format!("action-{i}"), "mainnet")).unwrap();
        }

        assert_eq!(store.data().history.len(), MAX_HISTORY);
        // Most recent is action-109 (the last one added)
        assert_eq!(store.data().history[0].action, "action-109");
    }

    #[test]
    fn save_load_persistence_round_trip() {
        let dir = TempDir::new().unwrap();

        // Write data
        {
            let mut store = ForkStateStore::new(dir.path());
            store.insert_active_fork(sample_fork_entry("mainnet")).unwrap();
            store.add_history(sample_history_entry("enter", "mainnet")).unwrap();
        }

        // Read back in a fresh store
        {
            let mut store = ForkStateStore::new(dir.path());
            store.load().unwrap();

            let fork = store.get_active_fork("mainnet").unwrap();
            assert_eq!(fork.network, "mainnet");
            assert_eq!(store.data().history.len(), 1);
            assert_eq!(store.data().history[0].action, "enter");
        }
    }

    #[test]
    fn load_reads_legacy_bare_state() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FORK_STATE_FILE);
        let mut state = ForkState::default();
        state.forks.insert("mainnet".into(), sample_fork_entry("mainnet"));
        state.history.push(sample_history_entry("enter", "mainnet"));

        write_json_file(&path, &state).unwrap();

        let mut store = ForkStateStore::new(dir.path());
        store.load().unwrap();

        assert_eq!(store.data(), &state);
    }

    #[test]
    fn load_reads_wrapped_format() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FORK_STATE_FILE);
        let mut state = ForkState::default();
        state.forks.insert("mainnet".into(), sample_fork_entry("mainnet"));
        state.history.push(sample_history_entry("enter", "mainnet"));

        write_json_file(&path, &VersionedStore::new(state.clone())).unwrap();

        let mut store = ForkStateStore::new(dir.path());
        store.load().unwrap();

        assert_eq!(store.data(), &state);
    }

    #[test]
    fn load_reads_legacy_fork_state_filename() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("fork-state.json");
        let mut state = ForkState::default();
        state.forks.insert("mainnet".into(), sample_fork_entry("mainnet"));
        state.history.push(sample_history_entry("enter", "mainnet"));

        write_json_file(&path, &VersionedStore::new(state.clone())).unwrap();

        let mut store = ForkStateStore::new(dir.path());
        store.load().unwrap();

        assert_eq!(store.data(), &state);
    }

    #[test]
    fn save_writes_bare_format() {
        let dir = TempDir::new().unwrap();
        let mut store = ForkStateStore::new(dir.path());

        store.insert_active_fork(sample_fork_entry("mainnet")).unwrap();
        store.add_history(sample_history_entry("enter", "mainnet")).unwrap();

        let saved: serde_json::Value = read_json_file(&dir.path().join(FORK_STATE_FILE)).unwrap();
        assert!(saved.get("_format").is_none());
        assert!(saved.get("entries").is_none());
        assert!(saved["forks"].get("mainnet").is_some());
        assert_eq!(saved["history"][0]["action"], "enter");
    }

    #[test]
    fn snapshot_and_restore_preserve_wrapped_registry_files() {
        let reg_dir = TempDir::new().unwrap();
        let snap_dir = TempDir::new().unwrap();
        let deployments_path = reg_dir.path().join(DEPLOYMENTS_FILE);

        let wrapped = json!({
            "_format": STORE_FORMAT,
            "entries": {
                "dep-1": {
                    "wrapped": true
                }
            }
        });
        write_json_file(&deployments_path, &wrapped).unwrap();

        snapshot_registry(reg_dir.path(), snap_dir.path()).unwrap();
        let snap_value: serde_json::Value =
            read_json_file(&snap_dir.path().join(DEPLOYMENTS_FILE)).unwrap();
        assert_eq!(snap_value, wrapped);

        write_json_file(
            &deployments_path,
            &json!({
                "_format": STORE_FORMAT,
                "entries": {
                    "dep-2": {
                        "wrapped": false
                    }
                }
            }),
        )
        .unwrap();

        restore_registry(snap_dir.path(), reg_dir.path()).unwrap();
        let restored: serde_json::Value = read_json_file(&deployments_path).unwrap();
        assert_eq!(restored, wrapped);
    }
}
