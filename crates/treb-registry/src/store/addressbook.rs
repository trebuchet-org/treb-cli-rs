//! Persistent store for addressbook entries backed by `addressbook.json`.

use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
};

use treb_core::TrebError;

use crate::{
    ADDRESSBOOK_FILE,
    io::{read_versioned_file, write_versioned_file},
};

/// CRUD store for addressbook entries scoped by chain ID and persisted as a
/// nested `HashMap<String, HashMap<String, String>>`.
pub struct AddressbookStore {
    path: PathBuf,
    data: HashMap<String, HashMap<String, String>>,
}

impl AddressbookStore {
    /// Create a new store pointing at `<registry_dir>/addressbook.json`.
    /// Call [`load`](Self::load) to read existing data from disk.
    pub fn new(registry_dir: &std::path::Path) -> Self {
        Self { path: registry_dir.join(ADDRESSBOOK_FILE), data: HashMap::new() }
    }

    /// Load addressbook entries from disk, replacing any in-memory data.
    pub fn load(&mut self) -> Result<(), TrebError> {
        self.data = read_versioned_file(&self.path)?;
        Ok(())
    }

    /// Atomically save all addressbook entries to disk under a file lock.
    pub fn save(&self) -> Result<(), TrebError> {
        let sorted: BTreeMap<String, BTreeMap<String, String>> = self
            .data
            .iter()
            .map(|(chain_id, entries)| {
                (
                    chain_id.clone(),
                    entries.iter().map(|(name, address)| (name.clone(), address.clone())).collect(),
                )
            })
            .collect();

        write_versioned_file(&self.path, &sorted)
    }

    /// Return `true` when the given chain/name entry exists.
    pub fn has_entry(&self, chain_id: &str, name: &str) -> bool {
        self.data.get(chain_id).is_some_and(|entries| entries.contains_key(name))
    }

    /// List all entries for a chain, sorted by name.
    pub fn list_entries(&self, chain_id: &str) -> Vec<(String, String)> {
        let mut entries: Vec<(String, String)> = self
            .data
            .get(chain_id)
            .into_iter()
            .flat_map(|chain_entries| chain_entries.iter())
            .map(|(name, address)| (name.clone(), address.clone()))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        entries
    }

    /// Set or replace an addressbook entry for the given chain.
    pub fn set_entry(
        &mut self,
        chain_id: &str,
        name: &str,
        address: &str,
    ) -> Result<(), TrebError> {
        self.data
            .entry(chain_id.to_string())
            .or_default()
            .insert(name.to_string(), address.to_string());
        self.save()
    }

    /// Remove an addressbook entry for the given chain.
    pub fn remove_entry(&mut self, chain_id: &str, name: &str) -> Result<(), TrebError> {
        let chain_entries = self.data.get_mut(chain_id).ok_or_else(|| {
            TrebError::Registry(format!("addressbook entry not found: {name} on chain {chain_id}"))
        })?;

        if chain_entries.remove(name).is_none() {
            return Err(TrebError::Registry(format!(
                "addressbook entry not found: {name} on chain {chain_id}"
            )));
        }

        if chain_entries.is_empty() {
            self.data.remove(chain_id);
        }

        self.save()
    }

    /// Return a reference to the underlying data map.
    pub fn data(&self) -> &HashMap<String, HashMap<String, String>> {
        &self.data
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn load_round_trip_reads_and_writes_bare_json() {
        let dir = TempDir::new().unwrap();
        let mut store = AddressbookStore::new(dir.path());

        store.set_entry("1", "Treasury", "0x1111111111111111111111111111111111111111").unwrap();
        store.set_entry("42220", "Guardian", "0x2222222222222222222222222222222222222222").unwrap();

        let mut reloaded = AddressbookStore::new(dir.path());
        reloaded.load().unwrap();

        assert_eq!(reloaded.data(), store.data());

        let raw = fs::read_to_string(dir.path().join(ADDRESSBOOK_FILE)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            value,
            json!({
                "1": {
                    "Treasury": "0x1111111111111111111111111111111111111111"
                },
                "42220": {
                    "Guardian": "0x2222222222222222222222222222222222222222"
                }
            })
        );
    }

    #[test]
    fn load_accepts_wrapped_json_and_missing_file_defaults_empty() {
        let dir = TempDir::new().unwrap();
        let mut store = AddressbookStore::new(dir.path());
        store.load().unwrap();
        assert!(store.data().is_empty());

        let path = dir.path().join(ADDRESSBOOK_FILE);
        fs::write(
            &path,
            serde_json::to_string_pretty(&json!({
                "_format": crate::STORE_FORMAT,
                "entries": {
                    "10": {
                        "Alice": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let mut wrapped = AddressbookStore::new(dir.path());
        wrapped.load().unwrap();
        assert!(wrapped.has_entry("10", "Alice"));
    }

    #[test]
    fn save_orders_outer_and_inner_keys_deterministically() {
        let dir = TempDir::new().unwrap();
        let mut store = AddressbookStore::new(dir.path());

        store.set_entry("42220", "Zulu", "0x3333333333333333333333333333333333333333").unwrap();
        store.set_entry("1", "Bravo", "0x1111111111111111111111111111111111111111").unwrap();
        store.set_entry("1", "Alpha", "0x0000000000000000000000000000000000000000").unwrap();

        let raw = fs::read_to_string(dir.path().join(ADDRESSBOOK_FILE)).unwrap();
        let alpha = raw.find("\"Alpha\"").unwrap();
        let bravo = raw.find("\"Bravo\"").unwrap();
        let chain_one = raw.find("\"1\"").unwrap();
        let chain_celo = raw.find("\"42220\"").unwrap();

        assert!(chain_one < chain_celo, "outer keys should be sorted: {raw}");
        assert!(alpha < bravo, "inner keys should be sorted: {raw}");
    }

    #[test]
    fn remove_entry_cleans_up_empty_chain_maps() {
        let dir = TempDir::new().unwrap();
        let mut store = AddressbookStore::new(dir.path());

        store.set_entry("8453", "Ops", "0x4444444444444444444444444444444444444444").unwrap();
        store.remove_entry("8453", "Ops").unwrap();

        assert!(!store.data().contains_key("8453"));

        let path = dir.path().join(ADDRESSBOOK_FILE);
        assert_json_eq(&path, &json!({}));
    }

    #[test]
    fn list_entries_returns_names_sorted_and_remove_missing_errors() {
        let dir = TempDir::new().unwrap();
        let mut store = AddressbookStore::new(dir.path());

        store.set_entry("1", "Zulu", "0x9999999999999999999999999999999999999999").unwrap();
        store.set_entry("1", "Alpha", "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();

        assert_eq!(
            store.list_entries("1"),
            vec![
                ("Alpha".to_string(), "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
                ("Zulu".to_string(), "0x9999999999999999999999999999999999999999".to_string()),
            ]
        );

        let err = store.remove_entry("1", "Missing").unwrap_err().to_string();
        assert!(err.contains("addressbook entry not found"));
    }

    fn assert_json_eq(path: &Path, expected: &serde_json::Value) {
        let raw = fs::read_to_string(path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(&value, expected);
    }
}
