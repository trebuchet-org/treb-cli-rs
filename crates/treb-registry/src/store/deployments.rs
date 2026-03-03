//! Persistent store for deployments backed by `deployments.json`.

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::Utc;
use treb_core::types::Deployment;
use treb_core::TrebError;

use crate::io::{read_json_file_or_default, with_file_lock, write_json_file};
use crate::DEPLOYMENTS_FILE;

/// CRUD store for deployments, persisted as a `HashMap<String, Deployment>` in
/// `deployments.json` inside the registry directory.
pub struct DeploymentStore {
    path: PathBuf,
    data: HashMap<String, Deployment>,
}

impl DeploymentStore {
    /// Create a new store pointing at `<registry_dir>/deployments.json`.
    /// Call [`load`](Self::load) to read existing data from disk.
    pub fn new(registry_dir: &std::path::Path) -> Self {
        Self {
            path: registry_dir.join(DEPLOYMENTS_FILE),
            data: HashMap::new(),
        }
    }

    /// Load deployments from disk, replacing any in-memory data.
    pub fn load(&mut self) -> Result<(), TrebError> {
        self.data = read_json_file_or_default(&self.path)?;
        Ok(())
    }

    /// Atomically save all deployments to disk under a file lock.
    pub fn save(&self) -> Result<(), TrebError> {
        with_file_lock(&self.path, || write_json_file(&self.path, &self.data))
    }

    /// Get a deployment by ID.
    pub fn get(&self, id: &str) -> Option<&Deployment> {
        self.data.get(id)
    }

    /// Insert a new deployment. Returns an error if the ID already exists.
    pub fn insert(&mut self, deployment: Deployment) -> Result<(), TrebError> {
        if self.data.contains_key(&deployment.id) {
            return Err(TrebError::Registry(format!(
                "deployment already exists: {}",
                deployment.id
            )));
        }
        self.data.insert(deployment.id.clone(), deployment);
        self.save()
    }

    /// Update an existing deployment. Sets `updated_at` to now.
    /// Returns an error if the ID is not found.
    pub fn update(&mut self, mut deployment: Deployment) -> Result<(), TrebError> {
        if !self.data.contains_key(&deployment.id) {
            return Err(TrebError::Registry(format!(
                "deployment not found: {}",
                deployment.id
            )));
        }
        deployment.updated_at = Utc::now();
        self.data.insert(deployment.id.clone(), deployment);
        self.save()
    }

    /// Remove a deployment by ID, returning it if found.
    pub fn remove(&mut self, id: &str) -> Result<Deployment, TrebError> {
        let deployment = self
            .data
            .remove(id)
            .ok_or_else(|| TrebError::Registry(format!("deployment not found: {id}")))?;
        self.save()?;
        Ok(deployment)
    }

    /// List all deployments sorted by `created_at` (ascending).
    pub fn list(&self) -> Vec<&Deployment> {
        let mut entries: Vec<&Deployment> = self.data.values().collect();
        entries.sort_by_key(|d| d.created_at);
        entries
    }

    /// Return the number of deployments in the store.
    pub fn count(&self) -> usize {
        self.data.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;
    use treb_core::types::{
        ArtifactInfo, DeploymentMethod, DeploymentStrategy, DeploymentType, VerificationInfo,
        VerificationStatus,
    };

    /// Helper to create a minimal deployment with the given ID and created_at offset in seconds.
    fn make_deployment(id: &str, created_at_offset_secs: i64) -> Deployment {
        let base = chrono::DateTime::parse_from_rfc3339("2026-03-02T19:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let ts = base + chrono::Duration::seconds(created_at_offset_secs);
        Deployment {
            id: id.to_string(),
            namespace: "mainnet".to_string(),
            chain_id: 42220,
            contract_name: "TestContract".to_string(),
            label: "v1.0.0".to_string(),
            address: format!("0x{:040x}", created_at_offset_secs.unsigned_abs()),
            deployment_type: DeploymentType::Singleton,
            transaction_id: String::new(),
            deployment_strategy: DeploymentStrategy {
                method: DeploymentMethod::Create,
                salt: String::new(),
                init_code_hash: String::new(),
                factory: String::new(),
                constructor_args: String::new(),
                entropy: String::new(),
            },
            proxy_info: None,
            artifact: ArtifactInfo {
                path: "contracts/Test.sol".to_string(),
                compiler_version: "0.8.24".to_string(),
                bytecode_hash: "0xabc".to_string(),
                script_path: "script/Deploy.s.sol".to_string(),
                git_commit: "abc123".to_string(),
            },
            verification: VerificationInfo {
                status: VerificationStatus::Unverified,
                etherscan_url: String::new(),
                verified_at: None,
                reason: String::new(),
                verifiers: HashMap::new(),
            },
            tags: None,
            created_at: ts,
            updated_at: ts,
        }
    }

    #[test]
    fn insert_then_get() {
        let dir = TempDir::new().unwrap();
        let mut store = DeploymentStore::new(dir.path());

        let dep = make_deployment("dep-1", 0);
        store.insert(dep.clone()).unwrap();

        let got = store.get("dep-1").unwrap();
        assert_eq!(got.id, "dep-1");
        assert_eq!(got.contract_name, dep.contract_name);
    }

    #[test]
    fn duplicate_insert_error() {
        let dir = TempDir::new().unwrap();
        let mut store = DeploymentStore::new(dir.path());

        let dep = make_deployment("dep-1", 0);
        store.insert(dep.clone()).unwrap();

        let result = store.insert(dep);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("already exists"),
            "expected 'already exists' in: {msg}"
        );
    }

    #[test]
    fn update_success_and_updated_at_changes() {
        let dir = TempDir::new().unwrap();
        let mut store = DeploymentStore::new(dir.path());

        let dep = make_deployment("dep-1", 0);
        let original_updated_at = dep.updated_at;
        store.insert(dep.clone()).unwrap();

        // Small sleep to ensure updated_at changes
        std::thread::sleep(std::time::Duration::from_millis(10));

        let mut modified = dep;
        modified.label = "v2.0.0".to_string();
        store.update(modified).unwrap();

        let got = store.get("dep-1").unwrap();
        assert_eq!(got.label, "v2.0.0");
        assert!(got.updated_at > original_updated_at);
    }

    #[test]
    fn update_nonexistent_error() {
        let dir = TempDir::new().unwrap();
        let mut store = DeploymentStore::new(dir.path());

        let dep = make_deployment("missing", 0);
        let result = store.update(dep);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("not found"),
            "expected 'not found' in: {msg}"
        );
    }

    #[test]
    fn remove_returns_deployment() {
        let dir = TempDir::new().unwrap();
        let mut store = DeploymentStore::new(dir.path());

        let dep = make_deployment("dep-1", 0);
        store.insert(dep).unwrap();

        let removed = store.remove("dep-1").unwrap();
        assert_eq!(removed.id, "dep-1");
        assert!(store.get("dep-1").is_none());
        assert_eq!(store.count(), 0);
    }

    #[test]
    fn list_sorted_by_created_at() {
        let dir = TempDir::new().unwrap();
        let mut store = DeploymentStore::new(dir.path());

        // Insert in reverse order
        store.insert(make_deployment("dep-3", 30)).unwrap();
        store.insert(make_deployment("dep-1", 10)).unwrap();
        store.insert(make_deployment("dep-2", 20)).unwrap();

        let list = store.list();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].id, "dep-1");
        assert_eq!(list[1].id, "dep-2");
        assert_eq!(list[2].id, "dep-3");
    }

    #[test]
    fn empty_store_operations() {
        let dir = TempDir::new().unwrap();
        let store = DeploymentStore::new(dir.path());

        assert_eq!(store.count(), 0);
        assert!(store.list().is_empty());
        assert!(store.get("anything").is_none());
    }

    #[test]
    fn golden_file_round_trip() {
        let fixture_path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../treb-core/tests/fixtures/deployments_map.json");
        let fixture_json = fs::read_to_string(&fixture_path)
            .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", fixture_path.display()));
        let fixture_value: serde_json::Value =
            serde_json::from_str(&fixture_json).expect("fixture is valid JSON");

        // Load fixture into store
        let dir = TempDir::new().unwrap();
        let deployments: HashMap<String, Deployment> =
            serde_json::from_value(fixture_value.clone()).expect("fixture deserializes");

        let mut store = DeploymentStore::new(dir.path());
        for (_, dep) in deployments {
            store.insert(dep).unwrap();
        }

        // Re-read from disk and compare via serde_json::Value
        let saved_raw = fs::read_to_string(dir.path().join(DEPLOYMENTS_FILE)).unwrap();
        let saved_value: serde_json::Value = serde_json::from_str(&saved_raw).unwrap();

        assert_eq!(
            saved_value, fixture_value,
            "golden file round-trip: saved JSON must equal fixture"
        );
    }
}
