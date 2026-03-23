//! Solidity registry: build and persist the `registry.json` file read by
//! `Registry.sol` for cross-contract address lookups.
//!
//! Format: `{ chainId: { namespace: { identifier: address } } }`
//!
//! Identifiers use `contract_display_name()` — plain contract name when the
//! label is empty, or `ContractName:Label` otherwise.

use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
};

use treb_core::{
    TrebError,
    types::{Deployment, contract_display_name},
};

use crate::{SOLIDITY_REGISTRY_FILE, io::write_json_file};

/// Nested map: chain_id → namespace → identifier → address.
type SolidityRegistry = BTreeMap<String, BTreeMap<String, BTreeMap<String, String>>>;

/// Build a [`SolidityRegistry`] from a map of deployments.
pub fn build_solidity_registry(deployments: &HashMap<String, Deployment>) -> SolidityRegistry {
    let mut registry = SolidityRegistry::new();

    for deployment in deployments.values() {
        if deployment.address.is_empty() {
            continue;
        }

        let chain_id = deployment.chain_id.to_string();
        let identifier = contract_display_name(&deployment.contract_name, &deployment.label);

        registry
            .entry(chain_id)
            .or_default()
            .entry(deployment.namespace.clone())
            .or_default()
            .insert(identifier, deployment.address.clone());
    }

    registry
}

// ── SolidityRegistryStore ────────────────────────────────────────────────

/// Persistent store for the Solidity registry, backed by `registry.json`.
pub struct SolidityRegistryStore {
    path: PathBuf,
}

impl SolidityRegistryStore {
    /// Create a new store pointing at `<registry_dir>/registry.json`.
    pub fn new(registry_dir: &std::path::Path) -> Self {
        Self { path: registry_dir.join(SOLIDITY_REGISTRY_FILE) }
    }

    /// Rebuild the Solidity registry from the given deployments and save to disk.
    pub fn rebuild(&self, deployments: &HashMap<String, Deployment>) -> Result<(), TrebError> {
        let registry = build_solidity_registry(deployments);
        write_json_file(&self.path, &registry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;

    use chrono::Utc;
    use tempfile::TempDir;
    use treb_core::types::{
        ArtifactInfo, DeploymentMethod, DeploymentStrategy, DeploymentType, VerificationInfo,
        VerificationStatus,
    };

    use crate::io::read_json_file;

    fn make_deployment(
        id: &str,
        contract_name: &str,
        label: &str,
        address: &str,
        namespace: &str,
        chain_id: u64,
    ) -> Deployment {
        let now = Utc::now();
        Deployment {
            id: id.to_string(),
            namespace: namespace.to_string(),
            chain_id,
            contract_name: contract_name.to_string(),
            label: label.to_string(),
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
            tags: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn build_basic_registry() {
        let mut deployments = HashMap::new();
        deployments.insert(
            "default/31337/Counter".to_string(),
            make_deployment(
                "default/31337/Counter",
                "Counter",
                "",
                "0x1234567890123456789012345678901234567890",
                "default",
                31337,
            ),
        );

        let registry = build_solidity_registry(&deployments);

        assert_eq!(
            registry["31337"]["default"]["Counter"],
            "0x1234567890123456789012345678901234567890"
        );
    }

    #[test]
    fn build_with_label() {
        let mut deployments = HashMap::new();
        deployments.insert(
            "default/31337/Counter:v2".to_string(),
            make_deployment(
                "default/31337/Counter:v2",
                "Counter",
                "v2",
                "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "default",
                31337,
            ),
        );

        let registry = build_solidity_registry(&deployments);

        assert_eq!(
            registry["31337"]["default"]["Counter:v2"],
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
    }

    #[test]
    fn build_multiple_chains_and_namespaces() {
        let mut deployments = HashMap::new();
        deployments.insert(
            "default/1/Token".to_string(),
            make_deployment(
                "default/1/Token",
                "Token",
                "",
                "0x1111111111111111111111111111111111111111",
                "default",
                1,
            ),
        );
        deployments.insert(
            "staging/1/Token".to_string(),
            make_deployment(
                "staging/1/Token",
                "Token",
                "",
                "0x2222222222222222222222222222222222222222",
                "staging",
                1,
            ),
        );
        deployments.insert(
            "default/42161/Token".to_string(),
            make_deployment(
                "default/42161/Token",
                "Token",
                "",
                "0x3333333333333333333333333333333333333333",
                "default",
                42161,
            ),
        );

        let registry = build_solidity_registry(&deployments);

        assert_eq!(registry.len(), 2); // chains 1 and 42161
        assert_eq!(registry["1"].len(), 2); // default and staging
        assert_eq!(registry["1"]["default"]["Token"], "0x1111111111111111111111111111111111111111");
        assert_eq!(registry["1"]["staging"]["Token"], "0x2222222222222222222222222222222222222222");
        assert_eq!(
            registry["42161"]["default"]["Token"],
            "0x3333333333333333333333333333333333333333"
        );
    }

    #[test]
    fn build_skips_empty_address() {
        let mut deployments = HashMap::new();
        deployments.insert(
            "default/31337/Counter".to_string(),
            make_deployment("default/31337/Counter", "Counter", "", "", "default", 31337),
        );

        let registry = build_solidity_registry(&deployments);
        assert!(registry.is_empty());
    }

    #[test]
    fn empty_deployments_produces_empty_registry() {
        let deployments: HashMap<String, Deployment> = HashMap::new();
        let registry = build_solidity_registry(&deployments);
        assert!(registry.is_empty());
    }

    #[test]
    fn store_rebuild_writes_sorted_json() {
        let dir = TempDir::new().unwrap();
        let store = SolidityRegistryStore::new(dir.path());

        let mut deployments = HashMap::new();
        deployments.insert(
            "default/31337/Counter".to_string(),
            make_deployment(
                "default/31337/Counter",
                "Counter",
                "",
                "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "default",
                31337,
            ),
        );
        deployments.insert(
            "default/31337/Token:v2".to_string(),
            make_deployment(
                "default/31337/Token:v2",
                "Token",
                "v2",
                "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "default",
                31337,
            ),
        );

        store.rebuild(&deployments).unwrap();

        let saved: serde_json::Value =
            read_json_file(&dir.path().join(crate::SOLIDITY_REGISTRY_FILE)).unwrap();

        assert_eq!(
            saved["31337"]["default"]["Counter"],
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(
            saved["31337"]["default"]["Token:v2"],
            "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        );
    }
}
