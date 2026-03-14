//! Pipeline context, configuration, and result types.
//!
//! These types define the input/output contracts for the deployment recording
//! pipeline, ensuring well-defined boundaries between the orchestrator and
//! its constituent steps.

use std::{collections::HashMap, path::PathBuf};

use alloy_primitives::Address;
use treb_core::types::{
    GovernorProposal, deployment::Deployment, safe_transaction::SafeTransaction,
    transaction::Transaction,
};

use crate::{events::ExtractedCollision, sender::ResolvedSender};

// ---------------------------------------------------------------------------
// PipelineConfig
// ---------------------------------------------------------------------------

/// Configuration for a single pipeline execution.
pub struct PipelineConfig {
    /// Path to the forge script to execute (e.g., `script/Deploy.s.sol`).
    pub script_path: String,
    /// When true, the pipeline broadcasts transactions and writes to the registry.
    /// When false, the pipeline simulates only (dry run).
    pub broadcast: bool,
    /// The namespace for this deployment (e.g., `production`, `staging`).
    pub namespace: String,
    /// The target chain ID.
    pub chain_id: u64,
    /// The script function signature (e.g., `run()`, `deploy(uint256)`).
    pub script_sig: String,
    /// Positional arguments passed to the script function.
    pub script_args: Vec<String>,
    /// Environment variables to inject into the script execution.
    pub env_vars: HashMap<String, String>,
    /// Verbosity level for trace output (0 = none, 1+ = render traces).
    pub verbosity: u8,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            script_path: String::new(),
            broadcast: false,
            namespace: String::new(),
            chain_id: 0,
            script_sig: "run()".to_string(),
            script_args: Vec::new(),
            env_vars: HashMap::new(),
            verbosity: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// PipelineContext
// ---------------------------------------------------------------------------

/// Resolved execution context for a pipeline run.
///
/// Created from [`PipelineConfig`] by resolving paths, git state, and
/// project root before the pipeline begins execution.
pub struct PipelineContext {
    /// The original pipeline configuration.
    pub config: PipelineConfig,
    /// The fully resolved script path.
    pub script_path: PathBuf,
    /// The short git commit hash at the time of execution, or empty string.
    pub git_commit: String,
    /// The project root directory.
    pub project_root: PathBuf,
    /// The resolved deployer sender, used to detect Safe/Governor flows.
    pub deployer_sender: Option<ResolvedSender>,
    /// Sender address → role name mapping for trace labeling.
    pub sender_labels: HashMap<Address, String>,
    /// All sender role names (for senderId-based resolution).
    pub sender_role_names: Vec<String>,
}

// ---------------------------------------------------------------------------
// PipelineResult
// ---------------------------------------------------------------------------

/// Result of a completed pipeline execution.
pub struct PipelineResult {
    /// Deployments that were successfully recorded (or would be in dry-run).
    pub deployments: Vec<RecordedDeployment>,
    /// Transactions that were successfully recorded (or would be in dry-run).
    pub transactions: Vec<RecordedTransaction>,
    /// Whether the pipeline performed any registry writes.
    pub registry_updated: bool,
    /// Collision events reported during script execution.
    pub collisions: Vec<ExtractedCollision>,
    /// Deployments that were skipped due to duplicate detection.
    pub skipped: Vec<SkippedDeployment>,
    /// Whether this was a dry-run execution.
    pub dry_run: bool,
    /// Whether the pipeline completed successfully.
    pub success: bool,
    /// Total gas used by script execution.
    pub gas_used: u64,
    /// Number of decoded events from script execution.
    pub event_count: usize,
    /// Decoded console.log output from script execution.
    pub console_logs: Vec<String>,
    /// Governor proposals created during script execution.
    pub governor_proposals: Vec<GovernorProposal>,
    /// Pre-rendered execution traces (shown at `-v` and `-vv`).
    pub execution_traces: Option<String>,
    /// Pre-rendered setup traces (shown at `-vvv`).
    pub setup_traces: Option<String>,
}

// ---------------------------------------------------------------------------
// RecordedDeployment
// ---------------------------------------------------------------------------

/// A deployment that was successfully hydrated and recorded to the registry.
pub struct RecordedDeployment {
    /// The core domain deployment that was written.
    pub deployment: Deployment,
    /// The associated safe transaction, if the deployment was through a Safe.
    pub safe_transaction: Option<SafeTransaction>,
}

// ---------------------------------------------------------------------------
// RecordedTransaction
// ---------------------------------------------------------------------------

/// A transaction that was successfully hydrated and recorded to the registry.
pub struct RecordedTransaction {
    /// The core domain transaction that was written.
    pub transaction: Transaction,
    /// Sender role/name emitted during script execution, if available.
    pub sender_name: Option<String>,
    /// Per-transaction gas estimate or usage, if available from execution artifacts.
    pub gas_used: Option<u64>,
    /// Pre-rendered per-transaction trace sub-tree, if available.
    pub trace: Option<String>,
}

// ---------------------------------------------------------------------------
// SkippedDeployment
// ---------------------------------------------------------------------------

/// A deployment that was skipped due to duplicate detection.
#[derive(Debug)]
pub struct SkippedDeployment {
    /// The deployment that was not recorded.
    pub deployment: Deployment,
    /// Human-readable reason why it was skipped.
    pub reason: String,
}
