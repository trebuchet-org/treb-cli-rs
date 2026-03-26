//! Registry system for treb — JSON-backed deployment registry with CRUD
//! operations, lookup index, and atomic file I/O.

use std::path::{Path, PathBuf};

pub mod io;
pub mod lookup;
pub mod registry;
pub mod solidity_registry;
pub mod store;
pub mod types;

// Re-export registry types at crate root for convenience.
pub use io::{VersionedStore, read_versioned_file, write_versioned_file};
pub use lookup::LookupStore;
pub use registry::Registry;
pub use solidity_registry::SolidityRegistryStore;
pub use store::{
    AddressbookStore, DeploymentStore, ForkStateStore, GovernorProposalStore, SafeTransactionStore,
    TransactionStore,
    fork_state::{remove_snapshot, restore_registry, snapshot_registry},
    queued_index::QueuedIndexStore,
};
pub use types::{LookupIndex, QueuedIndex, QueuedIndexEntry};

// ── File-name constants ────────────────────────────────────────────────────

/// File storing named addresses scoped by chain ID.
pub const ADDRESSBOOK_FILE: &str = "addressbook.json";

/// File storing the deployment map (`{id: Deployment}`).
pub const DEPLOYMENTS_FILE: &str = "deployments.json";

/// Directory containing canonical project deployment metadata.
pub const DEPLOYMENTS_DIR: &str = "deployments";

/// File storing the transaction map (`{id: Transaction}`).
pub const TRANSACTIONS_FILE: &str = "transactions.json";

/// File storing the safe-transaction map (`{hash: SafeTransaction}`).
pub const SAFE_TXS_FILE: &str = "safe-txs.json";

/// File storing the governor-proposal map (`{id: GovernorProposal}`).
pub const GOVERNOR_PROPOSALS_FILE: &str = "governor-txs.json";

/// File storing the lookup index.
pub const LOOKUP_FILE: &str = "lookup.json";

/// File storing the active queued-work index.
pub const QUEUED_FILE: &str = "queued.json";

/// Directory name for the registry inside the project root.
pub const REGISTRY_DIR: &str = ".treb";

/// File storing fork-mode state (active forks, history).
pub const FORK_STATE_FILE: &str = "fork.json";

/// File storing the Solidity registry for cross-contract address lookups.
pub const SOLIDITY_REGISTRY_FILE: &str = "registry.json";

/// Current on-disk wrapper format for registry store files.
pub const STORE_FORMAT: &str = "treb-v1";

const LEGACY_SAFE_TXS_FILE: &str = "safe_txs.json";
const LEGACY_GOVERNOR_PROPOSALS_FILE: &str = "governor_proposals.json";
const LEGACY_FORK_STATE_FILE: &str = "fork-state.json";

/// Return the canonical deployments root for a project.
pub fn deployments_dir(project_root: &Path) -> PathBuf {
    project_root.join(DEPLOYMENTS_DIR)
}

/// Return the canonical addressbook path for a project.
pub fn addressbook_path(project_root: &Path) -> PathBuf {
    deployments_dir(project_root).join(ADDRESSBOOK_FILE)
}

/// Return the canonical Solidity registry path for a project.
pub fn solidity_registry_path(project_root: &Path) -> PathBuf {
    deployments_dir(project_root).join(SOLIDITY_REGISTRY_FILE)
}

/// Return the canonical queued index path for a project.
pub fn queued_index_path(project_root: &Path) -> PathBuf {
    deployments_dir(project_root).join(QUEUED_FILE)
}

/// Return the legacy registry directory for a project.
pub fn registry_dir(project_root: &Path) -> PathBuf {
    project_root.join(REGISTRY_DIR)
}

pub(crate) fn legacy_registry_store_path(path: &Path) -> Option<PathBuf> {
    let legacy_name = match path.file_name()?.to_str()? {
        SAFE_TXS_FILE => LEGACY_SAFE_TXS_FILE,
        GOVERNOR_PROPOSALS_FILE => LEGACY_GOVERNOR_PROPOSALS_FILE,
        FORK_STATE_FILE => LEGACY_FORK_STATE_FILE,
        _ => return None,
    };

    Some(path.with_file_name(legacy_name))
}
