//! Persistent store for deployments backed by `deployments/<namespace>/<chain>.json`.

use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
};

use chrono::Utc;
use treb_core::{TrebError, types::Deployment};

use crate::{
    ADDRESSBOOK_FILE, DEPLOYMENTS_FILE, LOOKUP_FILE, QUEUED_FILE, SOLIDITY_REGISTRY_FILE,
    io::{read_versioned_file, write_versioned_file},
};

/// CRUD store for deployments, persisted as grouped `HashMap<String, Deployment>` files under
/// `deployments/<namespace>/<chain>.json`.
pub struct DeploymentStore {
    root: PathBuf,
    legacy_path: Option<PathBuf>,
    data: HashMap<String, Deployment>,
}

impl DeploymentStore {
    /// Create a new store rooted at the canonical `deployments/` directory.
    pub fn new(deployments_root: &Path) -> Self {
        Self { root: deployments_root.to_path_buf(), legacy_path: None, data: HashMap::new() }
    }

    /// Create a new store rooted at the canonical `deployments/` directory with
    /// a fallback path for loading legacy `.treb/deployments.json`.
    pub fn with_legacy_path(deployments_root: &Path, legacy_path: &Path) -> Self {
        Self {
            root: deployments_root.to_path_buf(),
            legacy_path: Some(legacy_path.to_path_buf()),
            data: HashMap::new(),
        }
    }

    /// Load deployments from disk, replacing any in-memory data.
    ///
    /// Canonical `deployments/` files win. Legacy flat-file input is only used
    /// when no canonical deployment group files exist yet.
    pub fn load(&mut self) -> Result<(), TrebError> {
        self.data.clear();

        if self.has_canonical_groups()? {
            self.load_canonical_groups()
        } else if self.root.join(DEPLOYMENTS_FILE).exists() {
            self.data = read_versioned_file(&self.root.join(DEPLOYMENTS_FILE))?;
            Ok(())
        } else if let Some(legacy_path) = &self.legacy_path {
            self.data = read_versioned_file(legacy_path)?;
            Ok(())
        } else {
            Ok(())
        }
    }

    /// Atomically save all deployments to grouped files under `deployments/`.
    pub fn save(&self) -> Result<(), TrebError> {
        fs::create_dir_all(&self.root)?;
        self.remove_existing_group_files()?;

        let mut grouped: BTreeMap<PathBuf, BTreeMap<String, Deployment>> = BTreeMap::new();
        for deployment in self.data.values() {
            grouped
                .entry(group_path(&self.root, &deployment.namespace, deployment.chain_id))
                .or_default()
                .insert(deployment.id.clone(), deployment.clone());
        }

        for (path, deployments) in grouped {
            write_versioned_file(&path, &deployments)?;
        }

        Ok(())
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
            return Err(TrebError::Registry(format!("deployment not found: {}", deployment.id)));
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

    /// List all deployments sorted by `created_at` (ascending), then by `id`.
    pub fn list(&self) -> Vec<&Deployment> {
        let mut entries: Vec<&Deployment> = self.data.values().collect();
        entries.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        entries
    }

    /// Return the number of deployments in the store.
    pub fn count(&self) -> usize {
        self.data.len()
    }

    /// Return a reference to the underlying data map.
    pub fn data(&self) -> &HashMap<String, Deployment> {
        &self.data
    }

    /// Replace the full in-memory deployment set and persist it.
    pub fn replace_all(
        &mut self,
        deployments: HashMap<String, Deployment>,
    ) -> Result<(), TrebError> {
        self.data = deployments;
        self.save()
    }

    fn has_canonical_groups(&self) -> Result<bool, TrebError> {
        if !self.root.exists() {
            return Ok(false);
        }
        for path in collect_group_files(&self.root)? {
            if path.exists() {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn load_canonical_groups(&mut self) -> Result<(), TrebError> {
        for path in collect_group_files(&self.root)? {
            let deployments: HashMap<String, Deployment> = read_versioned_file(&path)?;
            for (id, deployment) in deployments {
                if self.data.insert(id.clone(), deployment).is_some() {
                    return Err(TrebError::Registry(format!(
                        "duplicate deployment id '{id}' found while loading {}",
                        path.display()
                    )));
                }
            }
        }
        Ok(())
    }

    fn remove_existing_group_files(&self) -> Result<(), TrebError> {
        if !self.root.exists() {
            return Ok(());
        }

        for path in collect_group_files(&self.root)? {
            if path.exists() {
                fs::remove_file(path)?;
            }
        }

        let legacy_flat = self.root.join(DEPLOYMENTS_FILE);
        if legacy_flat.exists() {
            fs::remove_file(legacy_flat)?;
        }

        Ok(())
    }
}

fn collect_group_files(root: &Path) -> Result<Vec<PathBuf>, TrebError> {
    let mut result = Vec::new();
    if !root.exists() {
        return Ok(result);
    }

    fn visit(dir: &Path, root: &Path, result: &mut Vec<PathBuf>) -> Result<(), TrebError> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                visit(&path, root, result)?;
                continue;
            }

            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }

            if path.parent() == Some(root) {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if matches!(
                    name.as_ref(),
                    ADDRESSBOOK_FILE
                        | SOLIDITY_REGISTRY_FILE
                        | LOOKUP_FILE
                        | QUEUED_FILE
                        | DEPLOYMENTS_FILE
                ) {
                    continue;
                }
            }

            result.push(path);
        }
        Ok(())
    }

    visit(root, root, &mut result)?;
    result.sort();
    Ok(result)
}

fn group_path(root: &Path, namespace: &str, chain_id: u64) -> PathBuf {
    root.join(namespace).join(format!("{chain_id}.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;
    use treb_core::types::{
        ArtifactInfo, DeploymentMethod, DeploymentStrategy, DeploymentType, ExecutionKind,
        ExecutionRef, ExecutionStatus, VerificationInfo, VerificationStatus,
    };

    use crate::io::{VersionedStore, read_json_file, write_json_file};

    /// Helper to create a minimal deployment with the given ID and created_at offset in seconds.
    fn make_deployment(
        id: &str,
        namespace: &str,
        chain_id: u64,
        created_at_offset_secs: i64,
    ) -> Deployment {
        let base = chrono::DateTime::parse_from_rfc3339("2026-03-02T19:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let ts = base + chrono::Duration::seconds(created_at_offset_secs);
        Deployment {
            id: id.to_string(),
            namespace: namespace.to_string(),
            chain_id,
            contract_name: "TestContract".to_string(),
            label: "v1.0.0".to_string(),
            address: format!("0x{:040x}", created_at_offset_secs.unsigned_abs()),
            deployment_type: DeploymentType::Singleton,
            execution: Some(ExecutionRef {
                status: ExecutionStatus::Broadcast,
                kind: ExecutionKind::Tx,
                artifact_file: format!("broadcast/{id}.json"),
                tx_hash: Some(format!("0x{:064x}", created_at_offset_secs.unsigned_abs())),
                safe_tx_hash: None,
                proposal_id: None,
                propose_safe_tx_hash: None,
                script_tx_index: Some(0),
            }),
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

        let dep = make_deployment("dep-1", "mainnet", 42220, 0);
        store.insert(dep.clone()).unwrap();

        let got = store.get("dep-1").unwrap();
        assert_eq!(got.id, "dep-1");
        assert_eq!(got.contract_name, dep.contract_name);
    }

    #[test]
    fn saves_grouped_by_namespace_and_chain() {
        let dir = TempDir::new().unwrap();
        let mut store = DeploymentStore::new(dir.path());

        store.insert(make_deployment("dep-a", "mainnet", 1, 0)).unwrap();
        store.insert(make_deployment("dep-b", "staging", 1, 1)).unwrap();
        store.insert(make_deployment("dep-c", "fork/42220", 31337, 2)).unwrap();

        assert!(dir.path().join("mainnet/1.json").exists());
        assert!(dir.path().join("staging/1.json").exists());
        assert!(dir.path().join("fork/42220/31337.json").exists());
    }

    #[test]
    fn load_reads_grouped_files() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("mainnet")).unwrap();
        fs::create_dir_all(dir.path().join("fork/42220")).unwrap();

        let mut group_a = BTreeMap::new();
        group_a.insert("dep-1".to_string(), make_deployment("dep-1", "mainnet", 42220, 0));
        let mut group_b = BTreeMap::new();
        group_b.insert("dep-2".to_string(), make_deployment("dep-2", "fork/42220", 31337, 1));

        write_json_file(&dir.path().join("mainnet/42220.json"), &group_a).unwrap();
        write_json_file(&dir.path().join("fork/42220/31337.json"), &group_b).unwrap();

        let mut store = DeploymentStore::new(dir.path());
        store.load().unwrap();

        assert_eq!(store.count(), 2);
        assert_eq!(store.get("dep-1").unwrap().namespace, "mainnet");
        assert_eq!(store.get("dep-2").unwrap().namespace, "fork/42220");
    }

    #[test]
    fn load_falls_back_to_legacy_flat_file() {
        let dir = TempDir::new().unwrap();
        let legacy_dir = TempDir::new().unwrap();
        let legacy_path = legacy_dir.path().join(DEPLOYMENTS_FILE);
        let mut deployments = BTreeMap::new();
        deployments.insert("dep-1".to_string(), make_deployment("dep-1", "mainnet", 42220, 0));

        write_json_file(&legacy_path, &deployments).unwrap();

        let mut store = DeploymentStore::with_legacy_path(dir.path(), &legacy_path);
        store.load().unwrap();

        assert_eq!(store.count(), 1);
        assert_eq!(store.get("dep-1").unwrap().id, "dep-1");
    }

    #[test]
    fn save_rewrites_group_files_and_ignores_reserved_root_files() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(ADDRESSBOOK_FILE), "{}\n").unwrap();
        fs::create_dir_all(dir.path().join("mainnet")).unwrap();
        fs::write(dir.path().join("mainnet/1.json"), "{}\n").unwrap();

        let mut store = DeploymentStore::new(dir.path());
        store.insert(make_deployment("dep-1", "mainnet", 42220, 0)).unwrap();

        assert!(dir.path().join(ADDRESSBOOK_FILE).exists());
        assert!(!dir.path().join("mainnet/1.json").exists());
        assert!(dir.path().join("mainnet/42220.json").exists());
    }

    #[test]
    fn load_reads_wrapped_format_from_group_file() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("mainnet")).unwrap();

        let mut deployments = BTreeMap::new();
        deployments.insert("dep-1".to_string(), make_deployment("dep-1", "mainnet", 1, 0));
        write_json_file(&dir.path().join("mainnet/1.json"), &VersionedStore::new(deployments))
            .unwrap();

        let mut store = DeploymentStore::new(dir.path());
        store.load().unwrap();
        assert!(store.get("dep-1").is_some());
    }

    #[test]
    fn list_sorted_by_created_at() {
        let dir = TempDir::new().unwrap();
        let mut store = DeploymentStore::new(dir.path());

        store.insert(make_deployment("dep-3", "mainnet", 1, 30)).unwrap();
        store.insert(make_deployment("dep-1", "mainnet", 1, 10)).unwrap();
        store.insert(make_deployment("dep-2", "mainnet", 1, 20)).unwrap();

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
    fn save_writes_bare_format_in_group_file() {
        let dir = TempDir::new().unwrap();
        let mut store = DeploymentStore::new(dir.path());

        store.insert(make_deployment("dep-1", "mainnet", 1, 0)).unwrap();

        let saved: serde_json::Value = read_json_file(&dir.path().join("mainnet/1.json")).unwrap();
        assert!(saved.get("_format").is_none());
        assert!(saved.get("entries").is_none());
        assert!(saved.get("dep-1").is_some());
    }
}
