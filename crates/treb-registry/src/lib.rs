//! Registry system for treb — JSON-backed deployment registry with CRUD
//! operations, lookup index, atomic file I/O, and migration detection.

pub mod io;
pub mod lookup;
pub mod registry;
pub mod store;
pub mod types;

// Re-export registry types at crate root for convenience.
pub use lookup::LookupStore;
pub use registry::Registry;
pub use store::{DeploymentStore, GovernorProposalStore, SafeTransactionStore, TransactionStore};
pub use types::{LookupIndex, RegistryMeta};

// ── File-name constants ────────────────────────────────────────────────────

/// File storing the deployment map (`{id: Deployment}`).
pub const DEPLOYMENTS_FILE: &str = "deployments.json";

/// File storing the transaction map (`{id: Transaction}`).
pub const TRANSACTIONS_FILE: &str = "transactions.json";

/// File storing the safe-transaction map (`{hash: SafeTransaction}`).
pub const SAFE_TXS_FILE: &str = "safe_txs.json";

/// File storing the governor-proposal map (`{id: GovernorProposal}`).
pub const GOVERNOR_PROPOSALS_FILE: &str = "governor_proposals.json";

/// File storing the lookup index.
pub const LOOKUP_FILE: &str = "lookup.json";

/// File storing the registry metadata.
pub const REGISTRY_FILE: &str = "registry.json";

/// Directory name for the registry inside the project root.
pub const REGISTRY_DIR: &str = ".treb";

/// File storing fork-mode state (active forks, history).
pub const FORK_STATE_FILE: &str = "fork-state.json";

/// Current registry format version.
pub const REGISTRY_VERSION: u32 = 1;
