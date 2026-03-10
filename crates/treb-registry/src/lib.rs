//! Registry system for treb — JSON-backed deployment registry with CRUD
//! operations, lookup index, and atomic file I/O.

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
