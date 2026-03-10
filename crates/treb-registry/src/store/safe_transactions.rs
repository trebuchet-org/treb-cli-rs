//! Persistent store for safe transactions backed by `safe-txs.json`.

use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
};

use treb_core::{TrebError, types::SafeTransaction};

use crate::{
    SAFE_TXS_FILE,
    io::{read_versioned_file_compat, write_versioned_file},
};

/// CRUD store for safe transactions, persisted as a
/// `HashMap<String, SafeTransaction>` in `safe-txs.json` inside the registry
/// directory. Keyed by `safe_tx_hash`.
pub struct SafeTransactionStore {
    path: PathBuf,
    data: HashMap<String, SafeTransaction>,
}

impl SafeTransactionStore {
    /// Create a new store pointing at `<registry_dir>/safe-txs.json`.
    /// Call [`load`](Self::load) to read existing data from disk.
    pub fn new(registry_dir: &std::path::Path) -> Self {
        Self { path: registry_dir.join(SAFE_TXS_FILE), data: HashMap::new() }
    }

    /// Load safe transactions from disk, replacing any in-memory data.
    pub fn load(&mut self) -> Result<(), TrebError> {
        self.data = read_versioned_file_compat(&self.path)?;
        Ok(())
    }

    /// Atomically save all safe transactions to disk under a file lock.
    pub fn save(&self) -> Result<(), TrebError> {
        let sorted: BTreeMap<String, SafeTransaction> =
            self.data.iter().map(|(hash, tx)| (hash.clone(), tx.clone())).collect();
        write_versioned_file(&self.path, &sorted)
    }

    /// Get a safe transaction by its `safe_tx_hash`.
    pub fn get(&self, safe_tx_hash: &str) -> Option<&SafeTransaction> {
        self.data.get(safe_tx_hash)
    }

    /// Insert a new safe transaction. Returns an error if the hash already exists.
    pub fn insert(&mut self, safe_tx: SafeTransaction) -> Result<(), TrebError> {
        if self.data.contains_key(&safe_tx.safe_tx_hash) {
            return Err(TrebError::Registry(format!(
                "safe transaction already exists: {}",
                safe_tx.safe_tx_hash
            )));
        }
        self.data.insert(safe_tx.safe_tx_hash.clone(), safe_tx);
        self.save()
    }

    /// Update an existing safe transaction.
    /// Returns an error if the hash is not found.
    pub fn update(&mut self, safe_tx: SafeTransaction) -> Result<(), TrebError> {
        if !self.data.contains_key(&safe_tx.safe_tx_hash) {
            return Err(TrebError::Registry(format!(
                "safe transaction not found: {}",
                safe_tx.safe_tx_hash
            )));
        }
        self.data.insert(safe_tx.safe_tx_hash.clone(), safe_tx);
        self.save()
    }

    /// Remove a safe transaction by hash, returning it if found.
    pub fn remove(&mut self, safe_tx_hash: &str) -> Result<SafeTransaction, TrebError> {
        let safe_tx = self.data.remove(safe_tx_hash).ok_or_else(|| {
            TrebError::Registry(format!("safe transaction not found: {safe_tx_hash}"))
        })?;
        self.save()?;
        Ok(safe_tx)
    }

    /// List all safe transactions sorted by `proposed_at` (ascending).
    pub fn list(&self) -> Vec<&SafeTransaction> {
        let mut entries: Vec<&SafeTransaction> = self.data.values().collect();
        entries.sort_by_key(|s| s.proposed_at);
        entries
    }

    /// Return the number of safe transactions in the store.
    pub fn count(&self) -> usize {
        self.data.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::fs;
    use tempfile::TempDir;
    use treb_core::types::TransactionStatus;

    use crate::{
        STORE_FORMAT,
        io::{VersionedStore, read_json_file, write_json_file},
    };

    /// Helper to create a minimal safe transaction with the given hash and
    /// proposed_at offset in seconds.
    fn make_safe_transaction(hash: &str, proposed_at_offset_secs: i64) -> SafeTransaction {
        let base = chrono::DateTime::parse_from_rfc3339("2026-03-02T19:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let ts = base + chrono::Duration::seconds(proposed_at_offset_secs);
        SafeTransaction {
            safe_tx_hash: hash.to_string(),
            safe_address: "0x32CB58b145d3f7e28c45cE4B2Cc31fa94248b23F".to_string(),
            chain_id: 42220,
            status: TransactionStatus::Queued,
            nonce: 0,
            transactions: vec![],
            transaction_ids: vec![],
            proposed_by: "0x56fD3F2bEE130e9867942D0F463a16fBE49B8d81".to_string(),
            proposed_at: ts,
            confirmations: vec![],
            executed_at: None,
            execution_tx_hash: String::new(),
        }
    }

    #[test]
    fn insert_then_get() {
        let dir = TempDir::new().unwrap();
        let mut store = SafeTransactionStore::new(dir.path());

        let stx = make_safe_transaction("0xabc", 0);
        store.insert(stx.clone()).unwrap();

        let got = store.get("0xabc").unwrap();
        assert_eq!(got.safe_tx_hash, "0xabc");
        assert_eq!(got.chain_id, 42220);
    }

    #[test]
    fn duplicate_insert_error() {
        let dir = TempDir::new().unwrap();
        let mut store = SafeTransactionStore::new(dir.path());

        let stx = make_safe_transaction("0xabc", 0);
        store.insert(stx.clone()).unwrap();

        let result = store.insert(stx);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("already exists"), "expected 'already exists' in: {msg}");
    }

    #[test]
    fn update_success() {
        let dir = TempDir::new().unwrap();
        let mut store = SafeTransactionStore::new(dir.path());

        let stx = make_safe_transaction("0xabc", 0);
        store.insert(stx.clone()).unwrap();

        let mut modified = stx;
        modified.status = TransactionStatus::Executed;
        store.update(modified).unwrap();

        let got = store.get("0xabc").unwrap();
        assert_eq!(got.status, TransactionStatus::Executed);
    }

    #[test]
    fn update_nonexistent_error() {
        let dir = TempDir::new().unwrap();
        let mut store = SafeTransactionStore::new(dir.path());

        let stx = make_safe_transaction("0xmissing", 0);
        let result = store.update(stx);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("not found"), "expected 'not found' in: {msg}");
    }

    #[test]
    fn remove_returns_safe_transaction() {
        let dir = TempDir::new().unwrap();
        let mut store = SafeTransactionStore::new(dir.path());

        let stx = make_safe_transaction("0xabc", 0);
        store.insert(stx).unwrap();

        let removed = store.remove("0xabc").unwrap();
        assert_eq!(removed.safe_tx_hash, "0xabc");
        assert!(store.get("0xabc").is_none());
        assert_eq!(store.count(), 0);
    }

    #[test]
    fn list_sorted_by_proposed_at() {
        let dir = TempDir::new().unwrap();
        let mut store = SafeTransactionStore::new(dir.path());

        // Insert in reverse order
        store.insert(make_safe_transaction("0xhash-3", 30)).unwrap();
        store.insert(make_safe_transaction("0xhash-1", 10)).unwrap();
        store.insert(make_safe_transaction("0xhash-2", 20)).unwrap();

        let list = store.list();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].safe_tx_hash, "0xhash-1");
        assert_eq!(list[1].safe_tx_hash, "0xhash-2");
        assert_eq!(list[2].safe_tx_hash, "0xhash-3");
    }

    #[test]
    fn empty_store_operations() {
        let dir = TempDir::new().unwrap();
        let store = SafeTransactionStore::new(dir.path());

        assert_eq!(store.count(), 0);
        assert!(store.list().is_empty());
        assert!(store.get("anything").is_none());
    }

    #[test]
    fn load_reads_legacy_bare_map() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(SAFE_TXS_FILE);
        let mut safe_txs = BTreeMap::new();
        safe_txs.insert("0xhash-1".to_string(), make_safe_transaction("0xhash-1", 0));
        safe_txs.insert("0xhash-2".to_string(), make_safe_transaction("0xhash-2", 1));

        write_json_file(&path, &safe_txs).unwrap();

        let mut store = SafeTransactionStore::new(dir.path());
        store.load().unwrap();

        assert_eq!(store.count(), 2);
        assert_eq!(store.get("0xhash-1").unwrap().safe_tx_hash, "0xhash-1");
        assert_eq!(store.get("0xhash-2").unwrap().safe_tx_hash, "0xhash-2");
    }

    #[test]
    fn load_reads_wrapped_format() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(SAFE_TXS_FILE);
        let mut safe_txs = BTreeMap::new();
        safe_txs.insert("0xhash-1".to_string(), make_safe_transaction("0xhash-1", 0));
        safe_txs.insert("0xhash-2".to_string(), make_safe_transaction("0xhash-2", 1));

        write_json_file(&path, &VersionedStore::new(safe_txs)).unwrap();

        let mut store = SafeTransactionStore::new(dir.path());
        store.load().unwrap();

        assert_eq!(store.count(), 2);
        assert_eq!(store.get("0xhash-1").unwrap().safe_tx_hash, "0xhash-1");
        assert_eq!(store.get("0xhash-2").unwrap().safe_tx_hash, "0xhash-2");
    }

    #[test]
    fn golden_file_round_trip() {
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../treb-core/tests/fixtures/safe_txs_map.json");
        let fixture_json = fs::read_to_string(&fixture_path)
            .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", fixture_path.display()));
        let fixture_value: serde_json::Value =
            serde_json::from_str(&fixture_json).expect("fixture is valid JSON");

        // Load fixture into store
        let dir = TempDir::new().unwrap();
        let safe_txs: HashMap<String, SafeTransaction> =
            serde_json::from_value(fixture_value.clone()).expect("fixture deserializes");

        let mut store = SafeTransactionStore::new(dir.path());
        for (_, stx) in safe_txs {
            store.insert(stx).unwrap();
        }

        // Re-read from disk and compare via serde_json::Value
        let saved_raw = fs::read_to_string(dir.path().join(SAFE_TXS_FILE)).unwrap();
        let saved_value: serde_json::Value = serde_json::from_str(&saved_raw).unwrap();

        assert_eq!(
            saved_value,
            serde_json::json!({
                "_format": STORE_FORMAT,
                "entries": fixture_value,
            }),
            "golden file round-trip: saved JSON must wrap fixture entries"
        );
    }

    #[test]
    fn save_writes_wrapped_format() {
        let dir = TempDir::new().unwrap();
        let mut store = SafeTransactionStore::new(dir.path());

        store.insert(make_safe_transaction("0xhash-1", 0)).unwrap();

        let saved: serde_json::Value = read_json_file(&dir.path().join(SAFE_TXS_FILE)).unwrap();
        assert_eq!(saved["_format"], STORE_FORMAT);
        assert!(saved["entries"].get("0xhash-1").is_some());
    }
}
