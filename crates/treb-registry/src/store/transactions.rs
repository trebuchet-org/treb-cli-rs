//! Persistent store for transactions backed by `transactions.json`.

use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
};

use treb_core::{TrebError, types::Transaction};

use crate::{
    TRANSACTIONS_FILE,
    io::{read_versioned_file, write_versioned_file},
};

/// CRUD store for transactions, persisted as a `HashMap<String, Transaction>` in
/// `transactions.json` inside the registry directory.
pub struct TransactionStore {
    path: PathBuf,
    data: HashMap<String, Transaction>,
}

impl TransactionStore {
    /// Create a new store pointing at `<registry_dir>/transactions.json`.
    /// Call [`load`](Self::load) to read existing data from disk.
    pub fn new(registry_dir: &std::path::Path) -> Self {
        Self { path: registry_dir.join(TRANSACTIONS_FILE), data: HashMap::new() }
    }

    /// Load transactions from disk, replacing any in-memory data.
    pub fn load(&mut self) -> Result<(), TrebError> {
        self.data = read_versioned_file(&self.path)?;
        Ok(())
    }

    /// Atomically save all transactions to disk under a file lock.
    pub fn save(&self) -> Result<(), TrebError> {
        let sorted: BTreeMap<String, Transaction> =
            self.data.iter().map(|(id, transaction)| (id.clone(), transaction.clone())).collect();
        write_versioned_file(&self.path, &sorted)
    }

    /// Get a transaction by ID.
    pub fn get(&self, id: &str) -> Option<&Transaction> {
        self.data.get(id)
    }

    /// Insert a new transaction. Returns an error if the ID already exists.
    pub fn insert(&mut self, transaction: Transaction) -> Result<(), TrebError> {
        if self.data.contains_key(&transaction.id) {
            return Err(TrebError::Registry(format!(
                "transaction already exists: {}",
                transaction.id
            )));
        }
        self.data.insert(transaction.id.clone(), transaction);
        self.save()
    }

    /// Update an existing transaction. Sets `created_at` to now (acts as updated_at).
    /// Returns an error if the ID is not found.
    pub fn update(&mut self, transaction: Transaction) -> Result<(), TrebError> {
        if !self.data.contains_key(&transaction.id) {
            return Err(TrebError::Registry(format!("transaction not found: {}", transaction.id)));
        }
        self.data.insert(transaction.id.clone(), transaction);
        self.save()
    }

    /// Remove a transaction by ID, returning it if found.
    pub fn remove(&mut self, id: &str) -> Result<Transaction, TrebError> {
        let transaction = self
            .data
            .remove(id)
            .ok_or_else(|| TrebError::Registry(format!("transaction not found: {id}")))?;
        self.save()?;
        Ok(transaction)
    }

    /// List all transactions sorted by `created_at` (ascending).
    pub fn list(&self) -> Vec<&Transaction> {
        let mut entries: Vec<&Transaction> = self.data.values().collect();
        entries.sort_by_key(|t| t.created_at);
        entries
    }

    /// Return the number of transactions in the store.
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

    use crate::io::{VersionedStore, read_json_file, write_json_file};

    /// Helper to create a minimal transaction with the given ID and created_at offset in seconds.
    fn make_transaction(id: &str, created_at_offset_secs: i64) -> Transaction {
        let base = chrono::DateTime::parse_from_rfc3339("2026-03-02T19:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let ts = base + chrono::Duration::seconds(created_at_offset_secs);
        Transaction {
            id: id.to_string(),
            chain_id: 42220,
            hash: format!("0x{:064x}", created_at_offset_secs.unsigned_abs()),
            status: TransactionStatus::Executed,
            block_number: 1000,
            sender: "0x56fD3F2bEE130e9867942D0F463a16fBE49B8d81".to_string(),
            nonce: 0,
            deployments: vec![],
            operations: vec![],
            safe_context: None,
            broadcast_file: None,
            environment: "testnet".to_string(),
            created_at: ts,
        }
    }

    #[test]
    fn insert_then_get() {
        let dir = TempDir::new().unwrap();
        let mut store = TransactionStore::new(dir.path());

        let tx = make_transaction("tx-1", 0);
        store.insert(tx.clone()).unwrap();

        let got = store.get("tx-1").unwrap();
        assert_eq!(got.id, "tx-1");
        assert_eq!(got.chain_id, tx.chain_id);
    }

    #[test]
    fn duplicate_insert_error() {
        let dir = TempDir::new().unwrap();
        let mut store = TransactionStore::new(dir.path());

        let tx = make_transaction("tx-1", 0);
        store.insert(tx.clone()).unwrap();

        let result = store.insert(tx);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("already exists"), "expected 'already exists' in: {msg}");
    }

    #[test]
    fn update_success() {
        let dir = TempDir::new().unwrap();
        let mut store = TransactionStore::new(dir.path());

        let tx = make_transaction("tx-1", 0);
        store.insert(tx.clone()).unwrap();

        let mut modified = tx;
        modified.environment = "mainnet".to_string();
        store.update(modified).unwrap();

        let got = store.get("tx-1").unwrap();
        assert_eq!(got.environment, "mainnet");
    }

    #[test]
    fn update_nonexistent_error() {
        let dir = TempDir::new().unwrap();
        let mut store = TransactionStore::new(dir.path());

        let tx = make_transaction("missing", 0);
        let result = store.update(tx);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("not found"), "expected 'not found' in: {msg}");
    }

    #[test]
    fn remove_returns_transaction() {
        let dir = TempDir::new().unwrap();
        let mut store = TransactionStore::new(dir.path());

        let tx = make_transaction("tx-1", 0);
        store.insert(tx).unwrap();

        let removed = store.remove("tx-1").unwrap();
        assert_eq!(removed.id, "tx-1");
        assert!(store.get("tx-1").is_none());
        assert_eq!(store.count(), 0);
    }

    #[test]
    fn list_sorted_by_created_at() {
        let dir = TempDir::new().unwrap();
        let mut store = TransactionStore::new(dir.path());

        // Insert in reverse order
        store.insert(make_transaction("tx-3", 30)).unwrap();
        store.insert(make_transaction("tx-1", 10)).unwrap();
        store.insert(make_transaction("tx-2", 20)).unwrap();

        let list = store.list();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].id, "tx-1");
        assert_eq!(list[1].id, "tx-2");
        assert_eq!(list[2].id, "tx-3");
    }

    #[test]
    fn empty_store_operations() {
        let dir = TempDir::new().unwrap();
        let store = TransactionStore::new(dir.path());

        assert_eq!(store.count(), 0);
        assert!(store.list().is_empty());
        assert!(store.get("anything").is_none());
    }

    #[test]
    fn load_reads_legacy_bare_map() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(TRANSACTIONS_FILE);
        let mut transactions = BTreeMap::new();
        transactions.insert("tx-1".to_string(), make_transaction("tx-1", 0));
        transactions.insert("tx-2".to_string(), make_transaction("tx-2", 1));

        write_json_file(&path, &transactions).unwrap();

        let mut store = TransactionStore::new(dir.path());
        store.load().unwrap();

        assert_eq!(store.count(), 2);
        assert_eq!(store.get("tx-1").unwrap().id, "tx-1");
        assert_eq!(store.get("tx-2").unwrap().id, "tx-2");
    }

    #[test]
    fn load_reads_wrapped_format() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(TRANSACTIONS_FILE);
        let mut transactions = BTreeMap::new();
        transactions.insert("tx-1".to_string(), make_transaction("tx-1", 0));
        transactions.insert("tx-2".to_string(), make_transaction("tx-2", 1));

        write_json_file(&path, &VersionedStore::new(transactions)).unwrap();

        let mut store = TransactionStore::new(dir.path());
        store.load().unwrap();

        assert_eq!(store.count(), 2);
        assert_eq!(store.get("tx-1").unwrap().id, "tx-1");
        assert_eq!(store.get("tx-2").unwrap().id, "tx-2");
    }

    #[test]
    fn golden_file_round_trip() {
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../treb-core/tests/fixtures/transactions_map.json");
        let fixture_json = fs::read_to_string(&fixture_path)
            .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", fixture_path.display()));
        let fixture_value: serde_json::Value =
            serde_json::from_str(&fixture_json).expect("fixture is valid JSON");

        // Load fixture into store
        let dir = TempDir::new().unwrap();
        let transactions: HashMap<String, Transaction> =
            serde_json::from_value(fixture_value.clone()).expect("fixture deserializes");

        let mut store = TransactionStore::new(dir.path());
        for (_, tx) in transactions {
            store.insert(tx).unwrap();
        }

        // Re-read from disk and compare via serde_json::Value
        let saved_raw = fs::read_to_string(dir.path().join(TRANSACTIONS_FILE)).unwrap();
        let saved_value: serde_json::Value = serde_json::from_str(&saved_raw).unwrap();

        assert_eq!(
            saved_value, fixture_value,
            "golden file round-trip: saved JSON must preserve fixture entries as bare JSON"
        );
    }

    #[test]
    fn save_writes_bare_format() {
        let dir = TempDir::new().unwrap();
        let mut store = TransactionStore::new(dir.path());

        store.insert(make_transaction("tx-1", 0)).unwrap();

        let saved: serde_json::Value = read_json_file(&dir.path().join(TRANSACTIONS_FILE)).unwrap();
        assert!(saved.get("_format").is_none());
        assert!(saved.get("entries").is_none());
        assert!(saved.get("tx-1").is_some());
    }
}
