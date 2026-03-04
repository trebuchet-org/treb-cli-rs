//! Lookup index: build, persist, and query deployment lookups by name,
//! address, or tag.

use std::{collections::HashMap, path::PathBuf};

use treb_core::{TrebError, types::Deployment};

use crate::{
    LOOKUP_FILE,
    io::{read_json_file_or_default, with_file_lock, write_json_file},
    types::LookupIndex,
};

/// Build a [`LookupIndex`] from a map of deployments.
///
/// - `by_name`: lowercase `contract_name` → list of deployment IDs
/// - `by_address`: lowercase `address` → deployment ID (skips empty addresses)
/// - `by_tag`: each tag → list of deployment IDs
pub fn build_lookup_index(deployments: &HashMap<String, Deployment>) -> LookupIndex {
    let mut index = LookupIndex::default();

    for deployment in deployments.values() {
        // by_name: lowercase contract_name → IDs
        let name_key = deployment.contract_name.to_lowercase();
        index.by_name.entry(name_key).or_default().push(deployment.id.clone());

        // by_address: lowercase address → ID (skip empty)
        if !deployment.address.is_empty() {
            let addr_key = deployment.address.to_lowercase();
            index.by_address.insert(addr_key, deployment.id.clone());
        }

        // by_tag: each tag → IDs
        if let Some(tags) = &deployment.tags {
            for tag in tags {
                index.by_tag.entry(tag.clone()).or_default().push(deployment.id.clone());
            }
        }
    }

    index
}

// ── Query methods on LookupIndex ─────────────────────────────────────────

impl LookupIndex {
    /// Find deployment IDs by contract name (case-insensitive).
    pub fn find_by_name(&self, name: &str) -> Option<&Vec<String>> {
        self.by_name.get(&name.to_lowercase())
    }

    /// Find a deployment ID by address (case-insensitive).
    pub fn find_by_address(&self, address: &str) -> Option<&String> {
        self.by_address.get(&address.to_lowercase())
    }

    /// Find deployment IDs by tag (exact match).
    pub fn find_by_tag(&self, tag: &str) -> Option<&Vec<String>> {
        self.by_tag.get(tag)
    }
}

// ── LookupStore ──────────────────────────────────────────────────────────

/// Persistent store for the lookup index, backed by `lookup.json`.
pub struct LookupStore {
    path: PathBuf,
}

impl LookupStore {
    /// Create a new store pointing at `<registry_dir>/lookup.json`.
    pub fn new(registry_dir: &std::path::Path) -> Self {
        Self { path: registry_dir.join(LOOKUP_FILE) }
    }

    /// Load the lookup index from disk, returning a default (empty) index if
    /// the file does not exist.
    pub fn load(&self) -> Result<LookupIndex, TrebError> {
        read_json_file_or_default(&self.path)
    }

    /// Atomically save the lookup index to disk under a file lock.
    pub fn save(&self, index: &LookupIndex) -> Result<(), TrebError> {
        with_file_lock(&self.path, || write_json_file(&self.path, index))
    }

    /// Rebuild the lookup index from the given deployments, save it to disk,
    /// and return the new index.
    pub fn rebuild(
        &self,
        deployments: &HashMap<String, Deployment>,
    ) -> Result<LookupIndex, TrebError> {
        let index = build_lookup_index(deployments);
        self.save(&index)?;
        Ok(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::{collections::HashMap, fs};

    use chrono::Utc;
    use tempfile::TempDir;
    use treb_core::types::{
        ArtifactInfo, DeploymentMethod, DeploymentStrategy, DeploymentType, VerificationInfo,
        VerificationStatus,
    };

    /// Helper to create a minimal deployment with configurable fields for
    /// lookup testing.
    fn make_deployment(
        id: &str,
        contract_name: &str,
        address: &str,
        tags: Option<Vec<String>>,
    ) -> Deployment {
        let now = Utc::now();
        Deployment {
            id: id.to_string(),
            namespace: "mainnet".to_string(),
            chain_id: 42220,
            contract_name: contract_name.to_string(),
            label: "v1".to_string(),
            address: address.to_string(),
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
            tags,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn build_from_fixture_deployments() {
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../treb-core/tests/fixtures/deployments_map.json");
        let fixture_json = fs::read_to_string(&fixture_path)
            .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", fixture_path.display()));
        let deployments: HashMap<String, Deployment> =
            serde_json::from_str(&fixture_json).expect("fixture deserializes");

        let index = build_lookup_index(&deployments);

        // 3 deployments, 2 unique contract names (FPMM, FPMMFactory, TransparentUpgradeableProxy)
        assert_eq!(index.by_name.len(), 3);
        assert!(index.by_name.contains_key("fpmm"));
        assert!(index.by_name.contains_key("fpmmfactory"));
        assert!(index.by_name.contains_key("transparentupgradeableproxy"));

        // All 3 have addresses
        assert_eq!(index.by_address.len(), 3);
        assert!(index.by_address.contains_key("0x42eddd7dc046da254a93659ca9b02f294606833d"));

        // No tags in the fixture
        assert!(index.by_tag.is_empty());
    }

    #[test]
    fn build_with_tags() {
        let mut deployments = HashMap::new();
        deployments.insert(
            "dep-1".to_string(),
            make_deployment("dep-1", "Token", "0xAAA", Some(vec!["core".into(), "v2".into()])),
        );
        deployments.insert(
            "dep-2".to_string(),
            make_deployment("dep-2", "Factory", "0xBBB", Some(vec!["core".into()])),
        );

        let index = build_lookup_index(&deployments);

        let core_ids = index.by_tag.get("core").unwrap();
        assert_eq!(core_ids.len(), 2);

        let v2_ids = index.by_tag.get("v2").unwrap();
        assert_eq!(v2_ids.len(), 1);
        assert!(v2_ids.contains(&"dep-1".to_string()));
    }

    #[test]
    fn build_skips_empty_address() {
        let mut deployments = HashMap::new();
        deployments.insert("dep-1".to_string(), make_deployment("dep-1", "Token", "", None));

        let index = build_lookup_index(&deployments);
        assert!(index.by_address.is_empty());
        assert_eq!(index.by_name.len(), 1);
    }

    #[test]
    fn empty_deployments_produces_empty_index() {
        let deployments: HashMap<String, Deployment> = HashMap::new();
        let index = build_lookup_index(&deployments);

        assert!(index.by_name.is_empty());
        assert!(index.by_address.is_empty());
        assert!(index.by_tag.is_empty());
    }

    #[test]
    fn find_by_name_case_insensitive() {
        let mut deployments = HashMap::new();
        deployments.insert("dep-1".to_string(), make_deployment("dep-1", "MyToken", "0xAAA", None));

        let index = build_lookup_index(&deployments);

        assert!(index.find_by_name("mytoken").is_some());
        assert!(index.find_by_name("MYTOKEN").is_some());
        assert!(index.find_by_name("MyToken").is_some());
        assert!(index.find_by_name("unknown").is_none());
    }

    #[test]
    fn find_by_address_case_insensitive() {
        let mut deployments = HashMap::new();
        deployments
            .insert("dep-1".to_string(), make_deployment("dep-1", "Token", "0xAbCdEf", None));

        let index = build_lookup_index(&deployments);

        assert_eq!(index.find_by_address("0xabcdef"), Some(&"dep-1".to_string()));
        assert_eq!(index.find_by_address("0xABCDEF"), Some(&"dep-1".to_string()));
        assert_eq!(index.find_by_address("0xAbCdEf"), Some(&"dep-1".to_string()));
        assert!(index.find_by_address("0x000").is_none());
    }

    #[test]
    fn find_by_tag_exact_match() {
        let mut deployments = HashMap::new();
        deployments.insert(
            "dep-1".to_string(),
            make_deployment("dep-1", "Token", "0xAAA", Some(vec!["Core".into()])),
        );

        let index = build_lookup_index(&deployments);

        assert!(index.find_by_tag("Core").is_some());
        assert!(index.find_by_tag("core").is_none()); // exact match, not case-insensitive
    }

    #[test]
    fn rebuild_save_reload_round_trip() {
        let dir = TempDir::new().unwrap();
        let store = LookupStore::new(dir.path());

        let mut deployments = HashMap::new();
        deployments.insert(
            "dep-1".to_string(),
            make_deployment("dep-1", "Token", "0xAAA", Some(vec!["core".into()])),
        );
        deployments.insert("dep-2".to_string(), make_deployment("dep-2", "Factory", "0xBBB", None));

        // Rebuild writes to disk and returns index
        let built = store.rebuild(&deployments).unwrap();

        // Load from disk
        let loaded = store.load().unwrap();

        assert_eq!(built, loaded);
        assert_eq!(loaded.by_name.len(), 2);
        assert_eq!(loaded.by_address.len(), 2);
        assert_eq!(loaded.by_tag.len(), 1);
    }

    #[test]
    fn load_nonexistent_returns_empty() {
        let dir = TempDir::new().unwrap();
        let store = LookupStore::new(dir.path());

        let index = store.load().unwrap();
        assert_eq!(index, LookupIndex::default());
    }

    #[test]
    fn build_indexes_both_id_formats() {
        let mut deployments = HashMap::new();
        // Go-compatible: no label → ID has no colon
        deployments.insert(
            "default/31337/Counter".to_string(),
            make_deployment("default/31337/Counter", "Counter", "0xAAA", None),
        );
        // With label → ID has colon
        deployments.insert(
            "default/31337/Counter:v2".to_string(),
            make_deployment("default/31337/Counter:v2", "Counter", "0xBBB", None),
        );
        // Old-style: label equals contract name
        deployments.insert(
            "default/31337/Token:Token".to_string(),
            make_deployment("default/31337/Token:Token", "Token", "0xCCC", None),
        );

        let index = build_lookup_index(&deployments);

        // Both Counter deployments indexed under "counter"
        let counter_ids = index.find_by_name("Counter").unwrap();
        assert_eq!(counter_ids.len(), 2);
        assert!(counter_ids.contains(&"default/31337/Counter".to_string()));
        assert!(counter_ids.contains(&"default/31337/Counter:v2".to_string()));

        // Token indexed under "token"
        let token_ids = index.find_by_name("Token").unwrap();
        assert_eq!(token_ids.len(), 1);
        assert!(token_ids.contains(&"default/31337/Token:Token".to_string()));

        // All 3 addresses indexed
        assert_eq!(index.by_address.len(), 3);
    }
}
