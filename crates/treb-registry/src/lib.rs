//! Registry system for treb — JSON-backed deployment registry with CRUD
//! operations, lookup index, and atomic file I/O.

use std::{
    fs,
    path::{Path, PathBuf},
};

use treb_core::TrebError;

pub mod io;
pub mod lookup;
pub mod registry;
pub mod store;
pub mod types;

// Re-export registry types at crate root for convenience.
pub use io::{VersionedStore, read_versioned_file, write_versioned_file};
pub use lookup::LookupStore;
pub use registry::Registry;
pub use store::{
    DeploymentStore, ForkStateStore, GovernorProposalStore, SafeTransactionStore, TransactionStore,
    fork_state::{remove_snapshot, restore_registry, snapshot_registry},
};
pub use types::LookupIndex;

// ── File-name constants ────────────────────────────────────────────────────

/// File storing the deployment map (`{id: Deployment}`).
pub const DEPLOYMENTS_FILE: &str = "deployments.json";

/// File storing the transaction map (`{id: Transaction}`).
pub const TRANSACTIONS_FILE: &str = "transactions.json";

/// File storing the safe-transaction map (`{hash: SafeTransaction}`).
pub const SAFE_TXS_FILE: &str = "safe-txs.json";

/// File storing the governor-proposal map (`{id: GovernorProposal}`).
pub const GOVERNOR_PROPOSALS_FILE: &str = "governor-txs.json";

/// File storing the lookup index.
pub const LOOKUP_FILE: &str = "lookup.json";

/// Directory name for the registry inside the project root.
pub const REGISTRY_DIR: &str = ".treb";

/// File storing fork-mode state (active forks, history).
pub const FORK_STATE_FILE: &str = "fork.json";

/// Current on-disk wrapper format for registry store files.
pub const STORE_FORMAT: &str = "treb-v1";

const LEGACY_SAFE_TXS_FILE: &str = "safe_txs.json";
const LEGACY_GOVERNOR_PROPOSALS_FILE: &str = "governor_proposals.json";
const LEGACY_FORK_STATE_FILE: &str = "fork-state.json";

const LEGACY_STORE_FILE_RENAMES: &[(&str, &str)] = &[
    (LEGACY_SAFE_TXS_FILE, SAFE_TXS_FILE),
    (LEGACY_GOVERNOR_PROPOSALS_FILE, GOVERNOR_PROPOSALS_FILE),
    (LEGACY_FORK_STATE_FILE, FORK_STATE_FILE),
];

pub(crate) fn legacy_registry_store_path(path: &Path) -> Option<PathBuf> {
    let legacy_name = match path.file_name()?.to_str()? {
        SAFE_TXS_FILE => LEGACY_SAFE_TXS_FILE,
        GOVERNOR_PROPOSALS_FILE => LEGACY_GOVERNOR_PROPOSALS_FILE,
        FORK_STATE_FILE => LEGACY_FORK_STATE_FILE,
        _ => return None,
    };

    Some(path.with_file_name(legacy_name))
}

/// Return legacy registry store files in `dir` that still need to be renamed.
pub fn detect_legacy_registry_store_files(dir: &Path) -> Vec<(PathBuf, PathBuf)> {
    LEGACY_STORE_FILE_RENAMES
        .iter()
        .filter_map(|(old, new)| {
            let old_path = dir.join(old);
            let new_path = dir.join(new);
            (old_path.exists() && !new_path.exists()).then_some((old_path, new_path))
        })
        .collect()
}

/// Rename legacy registry store files in `dir` to their current filenames.
pub fn rename_legacy_registry_store_files(
    dir: &Path,
) -> Result<Vec<(PathBuf, PathBuf)>, TrebError> {
    let pending = detect_legacy_registry_store_files(dir);
    for (old_path, new_path) in &pending {
        if let Some(parent) = new_path.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::rename(old_path, new_path).map_err(|e| {
            TrebError::Registry(format!(
                "failed to rename {} to {}: {e}",
                old_path.display(),
                new_path.display()
            ))
        })?;
    }

    Ok(pending)
}
