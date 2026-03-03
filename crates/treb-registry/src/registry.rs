//! Registry facade — single entry point for all registry operations.
//!
//! Ties together deployment, transaction, and safe-transaction stores with the
//! lookup index and registry metadata. Provides migration detection on open.

use std::path::{Path, PathBuf};

use treb_core::types::{Deployment, GovernorProposal, SafeTransaction, Transaction};
use treb_core::TrebError;

use crate::io::{read_json_file, write_json_file};
use crate::lookup::LookupStore;
use crate::store::{DeploymentStore, GovernorProposalStore, SafeTransactionStore, TransactionStore};
use crate::types::{LookupIndex, RegistryMeta};
use crate::{REGISTRY_DIR, REGISTRY_FILE, REGISTRY_VERSION};

// ── MetaStore (internal) ─────────────────────────────────────────────────

/// Internal store for `registry.json` metadata.
struct MetaStore {
    path: PathBuf,
}

impl MetaStore {
    fn new(registry_dir: &Path) -> Self {
        Self {
            path: registry_dir.join(REGISTRY_FILE),
        }
    }

    fn load(&self) -> Result<RegistryMeta, TrebError> {
        read_json_file(&self.path)
    }

    fn save(&self, meta: &RegistryMeta) -> Result<(), TrebError> {
        write_json_file(&self.path, meta)
    }
}

// ── Registry ─────────────────────────────────────────────────────────────

/// Unified facade for the `.treb/` registry directory.
///
/// Holds all stores (deployments, transactions, safe transactions, governor
/// proposals, lookup index) and provides delegate methods for CRUD operations.
/// Deployment mutations automatically trigger a lookup index rebuild.
pub struct Registry {
    deployments: DeploymentStore,
    transactions: TransactionStore,
    safe_transactions: SafeTransactionStore,
    governor_proposals: GovernorProposalStore,
    lookup: LookupStore,
}

impl Registry {
    /// Open an existing registry at `<project_root>/.treb/`.
    ///
    /// Reads `registry.json` and checks the version. Returns an error if the
    /// registry version is greater than [`REGISTRY_VERSION`] (i.e. written by a
    /// newer tool). Returns `Ok` even if the `.treb/` directory doesn't exist
    /// (stores will simply be empty).
    pub fn open(project_root: &Path) -> Result<Self, TrebError> {
        let registry_dir = project_root.join(REGISTRY_DIR);
        let meta_store = MetaStore::new(&registry_dir);

        // Check version if registry.json exists.
        let meta_path = registry_dir.join(REGISTRY_FILE);
        if meta_path.exists() {
            let meta = meta_store.load()?;
            if meta.version > REGISTRY_VERSION {
                return Err(TrebError::Registry(format!(
                    "registry version {} is newer than supported version {}; please upgrade treb",
                    meta.version, REGISTRY_VERSION
                )));
            }
        }

        let mut deployments = DeploymentStore::new(&registry_dir);
        let mut transactions = TransactionStore::new(&registry_dir);
        let mut safe_transactions = SafeTransactionStore::new(&registry_dir);
        let mut governor_proposals = GovernorProposalStore::new(&registry_dir);
        let lookup = LookupStore::new(&registry_dir);

        // Load existing data (no-ops if files don't exist).
        deployments.load()?;
        transactions.load()?;
        safe_transactions.load()?;
        governor_proposals.load()?;

        Ok(Self {
            deployments,
            transactions,
            safe_transactions,
            governor_proposals,
            lookup,
        })
    }

    /// Initialise a new registry at `<project_root>/.treb/`.
    ///
    /// Creates the directory and `registry.json` if they don't already exist.
    /// Then delegates to [`open`](Self::open).
    pub fn init(project_root: &Path) -> Result<Self, TrebError> {
        let registry_dir = project_root.join(REGISTRY_DIR);
        std::fs::create_dir_all(&registry_dir)?;

        let meta_store = MetaStore::new(&registry_dir);
        if !meta_store.path.exists() {
            let meta = RegistryMeta::new();
            meta_store.save(&meta)?;
        }

        Self::open(project_root)
    }

    // ── Deployment delegates ─────────────────────────────────────────────

    /// Get a deployment by ID.
    pub fn get_deployment(&self, id: &str) -> Option<&Deployment> {
        self.deployments.get(id)
    }

    /// Insert a new deployment and rebuild the lookup index.
    pub fn insert_deployment(&mut self, deployment: Deployment) -> Result<(), TrebError> {
        self.deployments.insert(deployment)?;
        self.rebuild_lookup_index()?;
        Ok(())
    }

    /// Update an existing deployment and rebuild the lookup index.
    pub fn update_deployment(&mut self, deployment: Deployment) -> Result<(), TrebError> {
        self.deployments.update(deployment)?;
        self.rebuild_lookup_index()?;
        Ok(())
    }

    /// Remove a deployment by ID and rebuild the lookup index.
    pub fn remove_deployment(&mut self, id: &str) -> Result<Deployment, TrebError> {
        let removed = self.deployments.remove(id)?;
        self.rebuild_lookup_index()?;
        Ok(removed)
    }

    /// List all deployments sorted by `created_at`.
    pub fn list_deployments(&self) -> Vec<&Deployment> {
        self.deployments.list()
    }

    /// Return the number of deployments.
    pub fn deployment_count(&self) -> usize {
        self.deployments.count()
    }

    // ── Transaction delegates ────────────────────────────────────────────

    /// Get a transaction by ID.
    pub fn get_transaction(&self, id: &str) -> Option<&Transaction> {
        self.transactions.get(id)
    }

    /// Insert a new transaction.
    pub fn insert_transaction(&mut self, transaction: Transaction) -> Result<(), TrebError> {
        self.transactions.insert(transaction)
    }

    /// Update an existing transaction.
    pub fn update_transaction(&mut self, transaction: Transaction) -> Result<(), TrebError> {
        self.transactions.update(transaction)
    }

    /// Remove a transaction by ID.
    pub fn remove_transaction(&mut self, id: &str) -> Result<Transaction, TrebError> {
        self.transactions.remove(id)
    }

    /// List all transactions sorted by `created_at`.
    pub fn list_transactions(&self) -> Vec<&Transaction> {
        self.transactions.list()
    }

    /// Return the number of transactions.
    pub fn transaction_count(&self) -> usize {
        self.transactions.count()
    }

    // ── Safe transaction delegates ───────────────────────────────────────

    /// Get a safe transaction by hash.
    pub fn get_safe_transaction(&self, hash: &str) -> Option<&SafeTransaction> {
        self.safe_transactions.get(hash)
    }

    /// Insert a new safe transaction.
    pub fn insert_safe_transaction(
        &mut self,
        safe_tx: SafeTransaction,
    ) -> Result<(), TrebError> {
        self.safe_transactions.insert(safe_tx)
    }

    /// Update an existing safe transaction.
    pub fn update_safe_transaction(
        &mut self,
        safe_tx: SafeTransaction,
    ) -> Result<(), TrebError> {
        self.safe_transactions.update(safe_tx)
    }

    /// Remove a safe transaction by hash.
    pub fn remove_safe_transaction(
        &mut self,
        hash: &str,
    ) -> Result<SafeTransaction, TrebError> {
        self.safe_transactions.remove(hash)
    }

    /// List all safe transactions sorted by `proposed_at`.
    pub fn list_safe_transactions(&self) -> Vec<&SafeTransaction> {
        self.safe_transactions.list()
    }

    /// Return the number of safe transactions.
    pub fn safe_transaction_count(&self) -> usize {
        self.safe_transactions.count()
    }

    // ── Governor proposal delegates ───────────────────────────────────────

    /// Get a governor proposal by ID.
    pub fn get_governor_proposal(&self, proposal_id: &str) -> Option<&GovernorProposal> {
        self.governor_proposals.get(proposal_id)
    }

    /// Insert a new governor proposal.
    pub fn insert_governor_proposal(
        &mut self,
        proposal: GovernorProposal,
    ) -> Result<(), TrebError> {
        self.governor_proposals.insert(proposal)
    }

    /// Update an existing governor proposal.
    pub fn update_governor_proposal(
        &mut self,
        proposal: GovernorProposal,
    ) -> Result<(), TrebError> {
        self.governor_proposals.update(proposal)
    }

    /// Remove a governor proposal by ID.
    pub fn remove_governor_proposal(
        &mut self,
        proposal_id: &str,
    ) -> Result<GovernorProposal, TrebError> {
        self.governor_proposals.remove(proposal_id)
    }

    /// List all governor proposals sorted by `proposed_at` (descending).
    pub fn list_governor_proposals(&self) -> Vec<&GovernorProposal> {
        self.governor_proposals.list()
    }

    /// Return the number of governor proposals.
    pub fn governor_proposal_count(&self) -> usize {
        self.governor_proposals.count()
    }

    // ── Lookup index ─────────────────────────────────────────────────────

    /// Rebuild the lookup index from the current deployments and persist it.
    pub fn rebuild_lookup_index(&self) -> Result<LookupIndex, TrebError> {
        self.lookup.rebuild(self.deployments.data())
    }

    /// Load the current lookup index from disk.
    pub fn load_lookup_index(&self) -> Result<LookupIndex, TrebError> {
        self.lookup.load()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;
    use std::fs;

    use chrono::Utc;
    use tempfile::TempDir;
    use treb_core::types::{
        ArtifactInfo, DeploymentMethod, DeploymentStrategy, DeploymentType, TransactionStatus,
        VerificationInfo, VerificationStatus,
    };

    // ── Test helpers ─────────────────────────────────────────────────────

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
            environment: "testnet".to_string(),
            created_at: ts,
        }
    }

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

    // ── Integration tests ────────────────────────────────────────────────

    #[test]
    fn init_creates_registry_dir_and_meta() {
        let dir = TempDir::new().unwrap();
        let _registry = Registry::init(dir.path()).unwrap();

        let meta_path = dir.path().join(REGISTRY_DIR).join(REGISTRY_FILE);
        assert!(meta_path.exists(), "registry.json should be created");

        let meta: RegistryMeta = crate::io::read_json_file(&meta_path).unwrap();
        assert_eq!(meta.version, REGISTRY_VERSION);
    }

    #[test]
    fn open_with_no_treb_dir_returns_ok() {
        let dir = TempDir::new().unwrap();
        let registry = Registry::open(dir.path()).unwrap();

        assert_eq!(registry.deployment_count(), 0);
        assert_eq!(registry.transaction_count(), 0);
        assert_eq!(registry.safe_transaction_count(), 0);
    }

    #[test]
    fn newer_version_registry_returns_descriptive_error() {
        let dir = TempDir::new().unwrap();
        let registry_dir = dir.path().join(REGISTRY_DIR);
        fs::create_dir_all(&registry_dir).unwrap();

        let future_version = REGISTRY_VERSION + 1;
        let mut meta = RegistryMeta::new();
        meta.version = future_version;
        write_json_file(&registry_dir.join(REGISTRY_FILE), &meta).unwrap();

        let result = Registry::open(dir.path());
        let msg = match result {
            Err(e) => e.to_string(),
            Ok(_) => panic!("expected error for version {future_version} registry"),
        };
        assert!(
            msg.contains("newer than supported"),
            "expected descriptive version error, got: {msg}"
        );
    }

    #[test]
    fn insert_deployments_transactions_and_safe_txs_then_retrieve() {
        let dir = TempDir::new().unwrap();
        let mut registry = Registry::init(dir.path()).unwrap();

        // 3 deployments
        registry
            .insert_deployment(make_deployment("dep-1", 10))
            .unwrap();
        registry
            .insert_deployment(make_deployment("dep-2", 20))
            .unwrap();
        registry
            .insert_deployment(make_deployment("dep-3", 30))
            .unwrap();

        // 2 transactions
        registry
            .insert_transaction(make_transaction("tx-1", 10))
            .unwrap();
        registry
            .insert_transaction(make_transaction("tx-2", 20))
            .unwrap();

        // 1 safe transaction
        registry
            .insert_safe_transaction(make_safe_transaction("0xhash-1", 10))
            .unwrap();

        // Verify counts
        assert_eq!(registry.deployment_count(), 3);
        assert_eq!(registry.transaction_count(), 2);
        assert_eq!(registry.safe_transaction_count(), 1);

        // Verify retrieval
        assert!(registry.get_deployment("dep-1").is_some());
        assert!(registry.get_deployment("dep-2").is_some());
        assert!(registry.get_deployment("dep-3").is_some());
        assert!(registry.get_transaction("tx-1").is_some());
        assert!(registry.get_transaction("tx-2").is_some());
        assert!(registry.get_safe_transaction("0xhash-1").is_some());

        // Verify ordering
        let deps = registry.list_deployments();
        assert_eq!(deps[0].id, "dep-1");
        assert_eq!(deps[1].id, "dep-2");
        assert_eq!(deps[2].id, "dep-3");

        let txs = registry.list_transactions();
        assert_eq!(txs[0].id, "tx-1");
        assert_eq!(txs[1].id, "tx-2");

        let stxs = registry.list_safe_transactions();
        assert_eq!(stxs[0].safe_tx_hash, "0xhash-1");
    }

    #[test]
    fn insert_remove_deployment_updates_lookup_index() {
        let dir = TempDir::new().unwrap();
        let mut registry = Registry::init(dir.path()).unwrap();

        registry
            .insert_deployment(make_deployment("dep-1", 10))
            .unwrap();

        // Lookup should find the deployment
        let index = registry.load_lookup_index().unwrap();
        assert!(index.find_by_name("testcontract").is_some());
        let ids = index.find_by_name("testcontract").unwrap();
        assert!(ids.contains(&"dep-1".to_string()));

        // Remove and verify lookup is updated
        registry.remove_deployment("dep-1").unwrap();
        let index = registry.load_lookup_index().unwrap();
        assert!(
            index.find_by_name("testcontract").is_none(),
            "lookup should be empty after removing the only deployment"
        );
    }

    #[test]
    fn golden_file_integration_round_trip() {
        let fixture_dir =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../treb-core/tests/fixtures");

        // Load fixtures
        let deployments_json = fs::read_to_string(fixture_dir.join("deployments_map.json"))
            .expect("deployments fixture");
        let transactions_json = fs::read_to_string(fixture_dir.join("transactions_map.json"))
            .expect("transactions fixture");
        let safe_txs_json =
            fs::read_to_string(fixture_dir.join("safe_txs_map.json")).expect("safe_txs fixture");

        let deployments_value: serde_json::Value =
            serde_json::from_str(&deployments_json).unwrap();
        let transactions_value: serde_json::Value =
            serde_json::from_str(&transactions_json).unwrap();
        let safe_txs_value: serde_json::Value = serde_json::from_str(&safe_txs_json).unwrap();

        // Insert through registry
        let dir = TempDir::new().unwrap();
        let mut registry = Registry::init(dir.path()).unwrap();

        let deployments: HashMap<String, Deployment> =
            serde_json::from_value(deployments_value.clone()).unwrap();
        for (_, dep) in deployments {
            registry.insert_deployment(dep).unwrap();
        }

        let transactions: HashMap<String, Transaction> =
            serde_json::from_value(transactions_value.clone()).unwrap();
        for (_, tx) in transactions {
            registry.insert_transaction(tx).unwrap();
        }

        let safe_txs: HashMap<String, SafeTransaction> =
            serde_json::from_value(safe_txs_value.clone()).unwrap();
        for (_, stx) in safe_txs {
            registry.insert_safe_transaction(stx).unwrap();
        }

        // Re-read from disk and compare via serde_json::Value equality
        let treb_dir = dir.path().join(REGISTRY_DIR);

        let saved_deps_raw =
            fs::read_to_string(treb_dir.join(crate::DEPLOYMENTS_FILE)).unwrap();
        let saved_deps: serde_json::Value = serde_json::from_str(&saved_deps_raw).unwrap();
        assert_eq!(
            saved_deps, deployments_value,
            "deployments golden file round-trip"
        );

        let saved_txs_raw =
            fs::read_to_string(treb_dir.join(crate::TRANSACTIONS_FILE)).unwrap();
        let saved_txs: serde_json::Value = serde_json::from_str(&saved_txs_raw).unwrap();
        assert_eq!(
            saved_txs, transactions_value,
            "transactions golden file round-trip"
        );

        let saved_stxs_raw =
            fs::read_to_string(treb_dir.join(crate::SAFE_TXS_FILE)).unwrap();
        let saved_stxs: serde_json::Value = serde_json::from_str(&saved_stxs_raw).unwrap();
        assert_eq!(
            saved_stxs, safe_txs_value,
            "safe transactions golden file round-trip"
        );
    }

    #[test]
    fn init_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let mut registry = Registry::init(dir.path()).unwrap();
        registry
            .insert_deployment(make_deployment("dep-1", 10))
            .unwrap();

        // Init again — should not wipe existing data
        let registry2 = Registry::init(dir.path()).unwrap();
        assert_eq!(registry2.deployment_count(), 1);
        assert!(registry2.get_deployment("dep-1").is_some());
    }
}
