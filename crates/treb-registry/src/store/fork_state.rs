//! Persistent store for fork-mode state and registry snapshot/restore.

use std::fs;
use std::path::{Path, PathBuf};

use treb_core::types::fork::{ForkEntry, ForkHistoryEntry, ForkState};
use treb_core::TrebError;

use crate::io::{read_json_file_or_default, with_file_lock, write_json_file};
use crate::{
    DEPLOYMENTS_FILE, FORK_STATE_FILE, GOVERNOR_PROPOSALS_FILE, LOOKUP_FILE, SAFE_TXS_FILE,
    TRANSACTIONS_FILE,
};

/// Registry JSON files that are snapshotted/restored during fork mode.
const SNAPSHOT_FILES: &[&str] = &[
    DEPLOYMENTS_FILE,
    TRANSACTIONS_FILE,
    SAFE_TXS_FILE,
    GOVERNOR_PROPOSALS_FILE,
    LOOKUP_FILE,
];

/// Maximum number of history entries retained.
const MAX_HISTORY: usize = 100;

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
    for entry in fs::read_dir(snapshot_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            if let Some(name) = path.file_name() {
                fs::copy(&path, registry_dir.join(name))?;
            }
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

/// Persistent store for fork-mode state backed by `fork-state.json`.
pub struct ForkStateStore {
    path: PathBuf,
    data: ForkState,
}

impl ForkStateStore {
    /// Create a new store pointing at `<registry_dir>/fork-state.json`.
    /// Call [`load`](Self::load) to read existing data from disk.
    pub fn new(registry_dir: &Path) -> Self {
        Self {
            path: registry_dir.join(FORK_STATE_FILE),
            data: ForkState::default(),
        }
    }

    /// Load fork state from disk, replacing any in-memory data.
    pub fn load(&mut self) -> Result<(), TrebError> {
        self.data = read_json_file_or_default(&self.path)?;
        Ok(())
    }

    /// Atomically save fork state to disk under a file lock.
    pub fn save(&self) -> Result<(), TrebError> {
        with_file_lock(&self.path, || write_json_file(&self.path, &self.data))
    }

    /// Get an active fork entry by network name.
    pub fn get_active_fork(&self, network: &str) -> Option<&ForkEntry> {
        self.data.active_forks.get(network)
    }

    /// Insert a new active fork entry. Returns an error if the network is
    /// already forked.
    pub fn insert_active_fork(&mut self, entry: ForkEntry) -> Result<(), TrebError> {
        if self.data.active_forks.contains_key(&entry.network) {
            return Err(TrebError::Fork(format!(
                "network already forked: {}",
                entry.network
            )));
        }
        self.data
            .active_forks
            .insert(entry.network.clone(), entry);
        self.save()
    }

    /// Update an existing active fork entry in place.
    ///
    /// Returns an error if the network is not actively forked.
    pub fn update_active_fork(&mut self, entry: ForkEntry) -> Result<(), TrebError> {
        if !self.data.active_forks.contains_key(&entry.network) {
            return Err(TrebError::Fork(format!(
                "network is not actively forked: {}",
                entry.network
            )));
        }
        self.data.active_forks.insert(entry.network.clone(), entry);
        self.save()
    }

    /// Remove an active fork entry by network name. Returns an error if the
    /// network is not actively forked.
    pub fn remove_active_fork(&mut self, network: &str) -> Result<ForkEntry, TrebError> {
        let entry = self.data.active_forks.remove(network).ok_or_else(|| {
            TrebError::Fork(format!("network is not actively forked: {network}"))
        })?;
        self.save()?;
        Ok(entry)
    }

    /// List all active fork entries.
    pub fn list_active_forks(&self) -> Vec<&ForkEntry> {
        self.data.active_forks.values().collect()
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
    use tempfile::TempDir;

    fn sample_fork_entry(network: &str) -> ForkEntry {
        ForkEntry {
            network: network.into(),
            rpc_url: "http://127.0.0.1:8545".into(),
            port: 8545,
            chain_id: 1,
            fork_url: "https://eth.llamarpc.com".into(),
            fork_block_number: Some(19_000_000),
            snapshot_dir: format!(".treb/snapshots/{network}"),
            started_at: Utc.with_ymd_and_hms(2026, 3, 3, 12, 0, 0).unwrap(),
            evm_snapshot_id: None,
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
        fs::write(
            reg_dir.path().join(DEPLOYMENTS_FILE),
            r#"{"dep1": {}}"#,
        )
        .unwrap();
        fs::write(
            reg_dir.path().join(TRANSACTIONS_FILE),
            r#"{"tx1": {}}"#,
        )
        .unwrap();

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

    // ── restore_registry tests ──────────────────────────────────────────

    #[test]
    fn restore_overwrites_registry() {
        let reg_dir = TempDir::new().unwrap();
        let snap_dir = TempDir::new().unwrap();

        // Current registry state
        fs::write(reg_dir.path().join(DEPLOYMENTS_FILE), r#"{"new": true}"#).unwrap();

        // Snapshot with old state
        fs::write(
            snap_dir.path().join(DEPLOYMENTS_FILE),
            r#"{"old": true}"#,
        )
        .unwrap();

        restore_registry(snap_dir.path(), reg_dir.path()).unwrap();

        let content = fs::read_to_string(reg_dir.path().join(DEPLOYMENTS_FILE)).unwrap();
        assert_eq!(content, r#"{"old": true}"#);
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

        store
            .insert_active_fork(sample_fork_entry("mainnet"))
            .unwrap();
        let result = store.insert_active_fork(sample_fork_entry("mainnet"));
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("already forked"),
            "expected 'already forked' in: {msg}"
        );
    }

    #[test]
    fn remove_active_fork_returns_entry() {
        let dir = TempDir::new().unwrap();
        let mut store = ForkStateStore::new(dir.path());

        store
            .insert_active_fork(sample_fork_entry("mainnet"))
            .unwrap();
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
        assert!(
            msg.contains("not actively forked"),
            "expected 'not actively forked' in: {msg}"
        );
    }

    #[test]
    fn list_active_forks_returns_all() {
        let dir = TempDir::new().unwrap();
        let mut store = ForkStateStore::new(dir.path());

        store
            .insert_active_fork(sample_fork_entry("mainnet"))
            .unwrap();

        let mut entry2 = sample_fork_entry("sepolia");
        entry2.port = 8546;
        entry2.chain_id = 11155111;
        store.insert_active_fork(entry2).unwrap();

        let forks = store.list_active_forks();
        assert_eq!(forks.len(), 2);
    }

    #[test]
    fn add_history_prepends() {
        let dir = TempDir::new().unwrap();
        let mut store = ForkStateStore::new(dir.path());

        store
            .add_history(sample_history_entry("enter", "mainnet"))
            .unwrap();
        store
            .add_history(sample_history_entry("exit", "mainnet"))
            .unwrap();

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
            store
                .add_history(sample_history_entry(
                    &format!("action-{i}"),
                    "mainnet",
                ))
                .unwrap();
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
            store
                .insert_active_fork(sample_fork_entry("mainnet"))
                .unwrap();
            store
                .add_history(sample_history_entry("enter", "mainnet"))
                .unwrap();
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
}
