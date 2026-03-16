//! Pipeline context, configuration, and result types.
//!
//! These types define the input/output contracts for the deployment recording
//! pipeline, ensuring well-defined boundaries between the orchestrator and
//! its constituent steps.

use std::{collections::HashMap, path::PathBuf};

use alloy_primitives::Address;
use serde::{Deserialize, Serialize};
use treb_core::types::{
    GovernorProposal, deployment::Deployment, safe_transaction::SafeTransaction,
    transaction::Transaction,
};

use super::routing::RunResult;
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
    /// Whether the pipeline is running against an Anvil fork.
    pub is_fork: bool,
    /// The effective RPC URL for transaction routing.
    pub rpc_url: Option<String>,
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
            is_fork: false,
            rpc_url: None,
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
    /// All resolved senders keyed by role name.
    pub resolved_senders: HashMap<String, ResolvedSender>,
    /// Sender address → role name mapping for trace labeling.
    pub sender_labels: HashMap<Address, String>,
    /// Original sender configs (for key extraction in Safe/Governor routing).
    pub sender_configs: HashMap<String, treb_config::SenderConfig>,
    /// All sender role names (for transaction sender resolution).
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
    /// Safe transactions produced by routing (proposed to Safe Service).
    pub safe_transactions: Vec<SafeTransaction>,
    /// Raw routing results for CLI display (proposed vs broadcast per-run).
    pub proposed_results: Vec<ProposedResult>,
    /// Pre-rendered execution traces (shown at `-v` and `-vv`).
    pub execution_traces: Option<String>,
    /// Pre-rendered setup traces (shown at `-vvv`).
    pub setup_traces: Option<String>,
}

// ---------------------------------------------------------------------------
// ProposedResult
// ---------------------------------------------------------------------------

/// A routing result that was proposed (not broadcast on-chain).
///
/// These are surfaced to the CLI for display and optional polling.
#[derive(Debug, Clone)]
pub struct ProposedResult {
    /// The sender role that triggered this proposal.
    pub sender_role: String,
    /// The routing outcome.
    pub run_result: RunResult,
    /// Number of original user transactions covered by this proposal.
    pub tx_count: usize,
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

// ---------------------------------------------------------------------------
// ScriptEntry — input to SessionPipeline
// ---------------------------------------------------------------------------

/// A single script to execute in a session pipeline.
pub struct ScriptEntry {
    /// Human-readable name (script filename for run, component name for compose).
    pub name: String,
    /// Resolved execution context.
    pub context: PipelineContext,
    /// Forge script configuration.
    pub config: crate::script::ScriptConfig,
}

// ---------------------------------------------------------------------------
// ScriptResult — per-script output from SessionPipeline
// ---------------------------------------------------------------------------

/// Result of executing a single script within a session.
pub struct ScriptResult {
    /// The script name (matches `ScriptEntry::name`).
    pub name: String,
    /// The hydrated pipeline result.
    pub result: PipelineResult,
    /// Raw broadcastable transactions from simulation.
    /// Retained for compose replay between scripts.
    pub broadcastable_transactions: Option<foundry_cheatcodes::BroadcastableTransactions>,
}

// ---------------------------------------------------------------------------
// SessionPhase — progress reporting
// ---------------------------------------------------------------------------

/// Phases reported by the session pipeline.
#[derive(Debug, Clone)]
pub enum SessionPhase {
    /// Compiling the project (once).
    Compiling,
    /// Spawning an ephemeral Anvil fork (multi-script only).
    SpawningAnvil,
    /// Simulating a specific script.
    Simulating(String),
    /// All simulations complete.
    SimulationComplete,
    /// Broadcasting a specific script.
    Broadcasting(String),
    /// All broadcasts complete.
    BroadcastComplete,
}

/// Callback for session pipeline progress updates.
pub type SessionProgressCallback = Box<dyn Fn(SessionPhase) + Send>;

// ---------------------------------------------------------------------------
// SessionState — persistent session tracking
// ---------------------------------------------------------------------------

/// Persistent state for tracking session execution progress.
///
/// Written to `.treb/session-state.json` after each phase completion.
/// Used by `--resume` to skip already-completed scripts/phases.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionState {
    /// Hash of the configuration at session start, for change detection.
    pub config_hash: String,
    /// Per-script progress tracking.
    pub scripts: Vec<ScriptProgress>,
}

/// Progress of a single script in the session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptProgress {
    /// Script name (matches `ScriptEntry::name`).
    pub name: String,
    /// Path to the forge script file.
    pub script_path: String,
    /// Target chain ID.
    pub chain_id: u64,
    /// Script function signature.
    pub sig: String,
    /// Current phase of this script.
    pub phase: ScriptPhase,
    /// Number of deployments produced.
    pub deployments: usize,
    /// Number of transactions produced.
    pub transactions: usize,
}

/// Phase of a single script in the session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ScriptPhase {
    /// Not yet started.
    Pending,
    /// Simulation complete, ready for broadcast.
    Simulated,
    /// Broadcast complete.
    Broadcast,
    /// Failed during the named phase.
    Failed { phase: String },
}
