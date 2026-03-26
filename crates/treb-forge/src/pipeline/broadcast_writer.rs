//! Broadcast file writer — Foundry-compatible `ScriptSequence` construction and persistence.
//!
//! After routing, this module constructs `run-latest.json` in Foundry's exact
//! `ScriptSequence` format for on-chain transactions, and a companion
//! `run-latest.queued.json` for pending Safe/Governor operations.
//!
//! File layout matches Foundry conventions:
//! ```text
//! broadcast/<script_filename>/<chain_id>/<sig>-latest.json
//! cache/<script_filename>/<chain_id>/<sig>-latest.json
//! ```

use std::{
    collections::VecDeque,
    fs,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

// Alloy's HashMap (FxBuildHasher-based) is used by ScriptSequence fields.
use alloy_primitives::map::HashMap as AlloyHashMap;

use alloy_primitives::B256;
use forge_script_sequence::{ScriptSequence, TransactionWithMetadata, sig_to_file_name};
use foundry_evm::traces::CallKind;
use serde::{Deserialize, Serialize};
use treb_core::error::TrebError;

use super::{
    PipelineContext,
    routing::{
        QueuedExecution, RoutingAction, RoutingPlan, RunResult, TransactionRun,
        compute_safe_tx_hash_for_ops,
    },
    types::RecordedTransaction,
};

// ---------------------------------------------------------------------------
// BroadcastableTransaction → TransactionRequest helper
// ---------------------------------------------------------------------------

/// Convert a `BroadcastableTransaction` (network-agnostic) into an Ethereum
/// `TransactionRequest` suitable for wrapping in `TransactionMaybeSigned`.
fn btx_to_transaction_request(
    btx: &foundry_cheatcodes::BroadcastableTransaction,
) -> alloy_rpc_types::TransactionRequest {
    use alloy_rpc_types::{TransactionInput, TransactionRequest};

    let from = btx.transaction.from().unwrap_or_default();
    let mut tx = TransactionRequest::default().from(from);

    if let Some(alloy_primitives::TxKind::Call(to)) = btx.transaction.to() {
        tx = tx.to(to);
    }

    let input = btx.transaction.input().cloned().unwrap_or_default();
    if !input.is_empty() {
        tx.input = TransactionInput::new(input);
    }

    let value = btx.transaction.value().unwrap_or_default();
    if !value.is_zero() {
        tx = tx.value(value);
    }

    if let Some(gas) = btx.transaction.gas() {
        tx.gas = Some(gas as u64);
    }

    tx.nonce = btx.transaction.nonce();

    tx
}

// ---------------------------------------------------------------------------
// Queued operations (treb extension)
// ---------------------------------------------------------------------------

/// Pending operations that haven't hit the chain yet.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedOperations {
    pub timestamp: u128,
    pub chain: u64,
    pub commit: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub safe_proposals: Vec<QueuedSafeProposal>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub governor_proposals: Vec<QueuedGovernorProposal>,
}

/// A Safe proposal awaiting multi-sig execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedSafeProposal {
    pub safe_tx_hash: String,
    pub safe_address: String,
    pub nonce: u64,
    pub chain_id: u64,
    pub sender_role: String,
    pub transaction_ids: Vec<String>,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_tx_hash: Option<String>,
}

/// A Governor proposal awaiting execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedGovernorProposal {
    pub proposal_id: String,
    pub governor_address: String,
    pub sender_role: String,
    pub transaction_ids: Vec<String>,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub propose_tx_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub propose_safe_tx_hash: Option<String>,
}

#[allow(dead_code)]
pub type DeferredOperations = QueuedOperations;
#[allow(dead_code)]
pub type DeferredSafeProposal = QueuedSafeProposal;
#[allow(dead_code)]
pub type DeferredGovernorProposal = QueuedGovernorProposal;

// ---------------------------------------------------------------------------
// Resume state
// ---------------------------------------------------------------------------

/// State loaded from existing broadcast files for `--resume`.
pub struct ResumeState {
    pub sequence: ScriptSequence,
    pub queued: Option<QueuedOperations>,
    /// Tx hashes of wallet txs that already have on-chain receipts.
    pub completed_tx_hashes: std::collections::HashSet<B256>,
    /// Tx hashes that were sent but have no on-chain receipt yet.
    pub pending_tx_hashes: std::collections::HashSet<B256>,
    /// safeTxHash values already proposed.
    pub completed_safe_hashes: std::collections::HashSet<String>,
    /// Governor proposal IDs already submitted.
    pub completed_gov_ids: std::collections::HashSet<String>,
}

// ---------------------------------------------------------------------------
// Path computation
// ---------------------------------------------------------------------------

/// Compute broadcast file paths matching Foundry's convention.
///
/// Returns `(broadcast_dir, broadcast_path, cache_path)`.
pub fn compute_broadcast_paths(
    project_root: &Path,
    script_path: &str,
    chain_id: u64,
    sig: &str,
) -> (PathBuf, PathBuf, PathBuf) {
    let script_filename = Path::new(script_path)
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| script_path.to_string());

    let sig_name = sig_to_file_name(sig);
    let latest_name = format!("{sig_name}-latest.json");

    let broadcast_dir =
        project_root.join("broadcast").join(&script_filename).join(chain_id.to_string());
    let broadcast_path = broadcast_dir.join(&latest_name);

    let cache_dir = project_root.join("cache").join(&script_filename).join(chain_id.to_string());
    let cache_path = cache_dir.join(&latest_name);

    (broadcast_dir, broadcast_path, cache_path)
}

/// Compute the queued file path from the broadcast path.
pub fn queued_path_from(broadcast_path: &Path) -> PathBuf {
    let stem =
        broadcast_path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    broadcast_path.with_file_name(format!("{stem}.queued.json"))
}

fn legacy_deferred_path_from(broadcast_path: &Path) -> PathBuf {
    let stem =
        broadcast_path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    broadcast_path.with_file_name(format!("{stem}.deferred.json"))
}

/// Build a relative path from project root to the broadcast file.
pub fn relative_broadcast_path(project_root: &Path, broadcast_path: &Path) -> String {
    broadcast_path
        .strip_prefix(project_root)
        .unwrap_or(broadcast_path)
        .to_string_lossy()
        .to_string()
}

/// Compute the immutable archived path corresponding to a `*-latest` artifact.
pub fn timestamped_path_from_latest(path: &Path, timestamp: u128) -> PathBuf {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
    let archived_stem = if stem.contains("-latest") {
        stem.replacen("-latest", &format!("-{timestamp}"), 1)
    } else {
        format!("{stem}-{timestamp}")
    };

    match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) => path.with_file_name(format!("{archived_stem}.{ext}")),
        None => path.with_file_name(archived_stem),
    }
}

fn timestamped_run_path(path: &Path, timestamp: u128) -> PathBuf {
    path.with_file_name(format!("run-{timestamp}.json"))
}

/// Paths produced by a successful final broadcast artifact write.
pub struct BroadcastArtifactPaths {
    pub latest_broadcast_path: PathBuf,
    pub archived_broadcast_path: PathBuf,
    pub latest_queued_path: Option<PathBuf>,
    pub archived_queued_path: Option<PathBuf>,
}

fn tx_ids_for_run(run: &TransactionRun, recorded_txs: &[RecordedTransaction]) -> Vec<String> {
    run.tx_indices
        .iter()
        .filter_map(|&idx| recorded_txs.get(idx))
        .map(|rt| rt.transaction.id.clone())
        .collect()
}

fn same_tx_ids(left: &[String], right: &[String]) -> bool {
    left.len() == right.len() && left.iter().zip(right).all(|(l, r)| l == r)
}

fn copy_existing_safe_state(
    safe_proposals: &mut [QueuedSafeProposal],
    existing: Option<&QueuedOperations>,
) {
    let Some(existing) = existing else { return };
    for proposal in safe_proposals {
        if let Some(previous) =
            existing.safe_proposals.iter().find(|p| p.safe_tx_hash == proposal.safe_tx_hash)
        {
            proposal.status = previous.status.clone();
            proposal.execution_tx_hash = previous.execution_tx_hash.clone();
        }
    }
}

fn copy_existing_governor_state(
    governor_proposals: &mut [QueuedGovernorProposal],
    existing: Option<&QueuedOperations>,
) {
    let Some(existing) = existing else { return };
    for proposal in governor_proposals {
        if let Some(previous) = existing.governor_proposals.iter().find(|p| {
            p.governor_address.eq_ignore_ascii_case(&proposal.governor_address)
                && same_tx_ids(&p.transaction_ids, &proposal.transaction_ids)
        }) {
            proposal.status = previous.status.clone();
            if !previous.proposal_id.is_empty() {
                proposal.proposal_id = previous.proposal_id.clone();
            }
            proposal.propose_tx_hash = previous.propose_tx_hash.clone();
            proposal.propose_safe_tx_hash = previous.propose_safe_tx_hash.clone();
        }
    }
}

/// Build the queued operations file from the routing plan before execution.
///
/// This captures everything that may need to be queued later in the run. New
/// entries start as `pending`, and any matching prior resume state is merged in
/// so already-queued operations are preserved across retries.
pub fn build_pending_queued_operations_from_plan(
    plan: &RoutingPlan,
    recorded_txs: &[RecordedTransaction],
    ctx: &PipelineContext,
    existing: Option<&QueuedOperations>,
) -> QueuedOperations {
    let timestamp =
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);

    let mut safe_proposals = Vec::new();
    let mut governor_proposals = Vec::new();

    for planned in &plan.actions {
        let tx_ids = tx_ids_for_run(&planned.run, recorded_txs);

        if let RoutingAction::Propose {
            safe_address,
            chain_id,
            operations,
            sender_role,
            nonce,
            ..
        } = &planned.action
        {
            let safe_tx_hash = match &planned.queued {
                Some(QueuedExecution::SafeProposal { safe_tx_hash, .. }) => *safe_tx_hash,
                _ => compute_safe_tx_hash_for_ops(operations, *safe_address, *nonce, *chain_id),
            };

            safe_proposals.push(QueuedSafeProposal {
                safe_tx_hash: format!("{:#x}", safe_tx_hash),
                safe_address: format!("{:#x}", safe_address),
                nonce: *nonce,
                chain_id: *chain_id,
                sender_role: sender_role.clone(),
                transaction_ids: tx_ids.clone(),
                status: "pending".into(),
                execution_tx_hash: None,
            });
        }

        if let Some(QueuedExecution::GovernanceProposal { governor_address, .. }) = &planned.queued
        {
            governor_proposals.push(QueuedGovernorProposal {
                proposal_id: String::new(),
                governor_address: format!("{:#x}", governor_address),
                sender_role: planned.run.sender_role.clone(),
                transaction_ids: tx_ids,
                status: "pending".into(),
                propose_tx_hash: None,
                propose_safe_tx_hash: None,
            });
        }
    }

    copy_existing_safe_state(&mut safe_proposals, existing);
    copy_existing_governor_state(&mut governor_proposals, existing);

    QueuedOperations {
        timestamp,
        chain: ctx.config.chain_id,
        commit: if ctx.git_commit.is_empty() { None } else { Some(ctx.git_commit.clone()) },
        safe_proposals,
        governor_proposals,
    }
}

// ---------------------------------------------------------------------------
// Checkpoint helper
// ---------------------------------------------------------------------------

/// Update a pre-built `ScriptSequence` in-place after a transaction is confirmed.
///
/// Sets the hash on the transaction at `tx_idx` and appends the raw receipt
/// (if available) to the sequence's receipts list.
pub fn update_sequence_checkpoint(
    sequence: &mut ScriptSequence,
    tx_idx: usize,
    receipt: &crate::script::BroadcastReceipt,
) {
    if let Some(tx_meta) = sequence.transactions.get_mut(tx_idx) {
        tx_meta.hash = Some(receipt.hash);
    }
    if let Some(ref raw) = receipt.raw_receipt {
        if let Ok(any_receipt) = serde_json::from_value(raw.clone()) {
            sequence.receipts.push(any_receipt);
        }
    }
}

/// Ensure broadcast and cache directories exist for the sequence's paths.
///
/// Must be called before the first `save_sequence_checkpoint()` so that
/// Foundry's `ScriptSequence::save()` can write to the expected locations.
pub fn ensure_broadcast_dirs(sequence: &ScriptSequence) -> Result<(), TrebError> {
    if let Some((ref broadcast_path, ref cache_path)) = sequence.paths {
        if let Some(parent) = broadcast_path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                TrebError::Forge(format!(
                    "failed to create broadcast directory {}: {e}",
                    parent.display()
                ))
            })?;
        }
        if let Some(parent) = cache_path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                TrebError::Forge(format!(
                    "failed to create cache directory {}: {e}",
                    parent.display()
                ))
            })?;
        }
    }
    Ok(())
}

/// Write a checkpoint of the sequence to `run-latest.json` (no timestamped copy).
///
/// Called after each confirmed receipt so that a crash mid-broadcast preserves
/// all prior progress. The timestamped copy is only written by the final
/// `write_broadcast_artifacts()` call.
pub fn save_sequence_checkpoint(sequence: &mut ScriptSequence) -> Result<(), TrebError> {
    let Some((broadcast_path, cache_path)) = sequence.paths.clone() else {
        return Ok(());
    };

    write_sequence_latest(&broadcast_path, sequence)?;
    write_sequence_latest(&cache_path, sequence)?;
    Ok(())
}

fn write_queued_latest(
    broadcast_path: &Path,
    queued: &QueuedOperations,
    timestamp: u128,
) -> Result<PathBuf, TrebError> {
    let queued_path = queued_path_from(broadcast_path);
    if let Some(parent) = queued_path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            TrebError::Forge(format!(
                "failed to create queued checkpoint directory {}: {e}",
                parent.display()
            ))
        })?;
    }

    let mut queued = queued.clone();
    queued.timestamp = timestamp;

    let file = fs::File::create(&queued_path).map_err(|e| {
        TrebError::Forge(format!(
            "failed to create queued checkpoint file {}: {e}",
            queued_path.display()
        ))
    })?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, &queued)
        .map_err(|e| TrebError::Forge(format!("failed to serialize queued checkpoint: {e}")))?;
    writer
        .flush()
        .map_err(|e| TrebError::Forge(format!("failed to flush queued checkpoint: {e}")))?;

    Ok(queued_path)
}

/// Write the mutable queued checkpoint alongside `run-latest.json`.
pub fn save_queued_checkpoint(
    sequence: &ScriptSequence,
    queued: &QueuedOperations,
) -> Result<(), TrebError> {
    let Some((broadcast_path, _cache_path)) = sequence.paths.clone() else {
        return Ok(());
    };
    if queued.safe_proposals.is_empty() && queued.governor_proposals.is_empty() {
        return Ok(());
    }
    let _ = write_queued_latest(&broadcast_path, queued, sequence.timestamp)?;
    Ok(())
}

pub fn safe_proposal_completed(queued: &QueuedOperations, safe_tx_hash: &str) -> bool {
    queued
        .safe_proposals
        .iter()
        .any(|p| p.safe_tx_hash == safe_tx_hash && (p.status == "queued" || p.status == "executed"))
}

pub fn mark_safe_proposal_queued(
    queued: &mut QueuedOperations,
    safe_tx_hash: &str,
) -> Result<(), TrebError> {
    if let Some(proposal) =
        queued.safe_proposals.iter_mut().find(|p| p.safe_tx_hash == safe_tx_hash)
    {
        proposal.status = "queued".into();
    }
    Ok(())
}

pub fn governor_proposal_completed(
    queued: &QueuedOperations,
    governor_address: &str,
    transaction_ids: &[String],
) -> bool {
    queued.governor_proposals.iter().any(|p| {
        p.governor_address.eq_ignore_ascii_case(governor_address)
            && same_tx_ids(&p.transaction_ids, transaction_ids)
            && (p.status == "queued" || p.status == "executed")
    })
}

pub fn mark_governor_proposal_queued(
    queued: &mut QueuedOperations,
    governor_address: &str,
    transaction_ids: &[String],
    proposal_id: Option<&str>,
    propose_tx_hash: Option<&str>,
    propose_safe_tx_hash: Option<&str>,
) -> Result<(), TrebError> {
    if let Some(proposal) = queued.governor_proposals.iter_mut().find(|p| {
        p.governor_address.eq_ignore_ascii_case(governor_address)
            && same_tx_ids(&p.transaction_ids, transaction_ids)
    }) {
        proposal.status = "queued".into();
        if let Some(id) = proposal_id.filter(|id| !id.is_empty()) {
            proposal.proposal_id = id.to_string();
        }
        if let Some(hash) = propose_tx_hash.filter(|hash| !hash.is_empty()) {
            proposal.propose_tx_hash = Some(hash.to_string());
        }
        if let Some(hash) = propose_safe_tx_hash.filter(|hash| !hash.is_empty()) {
            proposal.propose_safe_tx_hash = Some(hash.to_string());
        }
    }
    Ok(())
}

fn write_sequence_latest(path: &Path, sequence: &ScriptSequence) -> Result<(), TrebError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            TrebError::Forge(format!(
                "failed to create checkpoint directory {}: {e}",
                parent.display()
            ))
        })?;
    }

    let file = fs::File::create(path).map_err(|e| {
        TrebError::Forge(format!("failed to create checkpoint file {}: {e}", path.display()))
    })?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, sequence)
        .map_err(|e| TrebError::Forge(format!("failed to serialize checkpoint: {e}")))?;
    writer.flush().map_err(|e| TrebError::Forge(format!("failed to flush checkpoint: {e}")))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Build ScriptSequence
// ---------------------------------------------------------------------------

/// Build a pre-routing `ScriptSequence` from broadcastable transactions.
///
/// Creates a mutable checkpoint target **before** routing begins. Every
/// transaction from the script is included with `hash: None` and no receipts,
/// so the sequence can be updated in-place as each transaction is confirmed
/// during broadcast.
pub fn build_pre_routing_sequence(
    btxs: &foundry_cheatcodes::BroadcastableTransactions,
    recorded_txs: &[RecordedTransaction],
    ctx: &PipelineContext,
    broadcast_path: PathBuf,
    cache_path: PathBuf,
) -> ScriptSequence {
    let mut transactions = VecDeque::new();
    let rpc_url = ctx.config.rpc_url.as_deref().unwrap_or_default();

    for (i, btx) in btxs.iter().enumerate() {
        let tx_request = btx_to_transaction_request(btx);
        let tx_maybe_signed = foundry_common::TransactionMaybeSigned::new(tx_request.into());
        let mut tx_meta = TransactionWithMetadata::from_tx_request(tx_maybe_signed);
        tx_meta.rpc = rpc_url.to_string();

        // hash is None — not yet broadcast

        // Determine opcode (Create vs Call)
        let is_create =
            matches!(btx.transaction.to(), None | Some(alloy_primitives::TxKind::Create));
        tx_meta.opcode = if is_create { CallKind::Create } else { CallKind::Call };

        // Set contract metadata from recorded transaction
        if let Some(rt) = recorded_txs.get(i) {
            if let Some(op) = rt.transaction.operations.first() {
                if op.operation_type == "DEPLOY" {
                    tx_meta.contract_name = Some(op.target.clone());
                    if let Some(addr_val) = op.result.get("address") {
                        if let Some(addr_str) = addr_val.as_str() {
                            tx_meta.contract_address = addr_str.parse().ok();
                        }
                    }
                }
                if !op.method.is_empty() && op.method != "CREATE" {
                    tx_meta.function = Some(op.method.clone());
                }
            }
        }

        transactions.push_back(tx_meta);
    }

    let timestamp =
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);

    ScriptSequence {
        transactions,
        receipts: Vec::new(),
        libraries: Vec::new(),
        pending: Vec::new(),
        paths: Some((broadcast_path, cache_path)),
        returns: AlloyHashMap::default(),
        timestamp,
        chain: ctx.config.chain_id,
        commit: if ctx.git_commit.is_empty() { None } else { Some(ctx.git_commit.clone()) },
    }
}

/// Build the checkpoint sequence used for incremental broadcast saves.
///
/// When resuming from a prior broadcast file, start from the saved sequence so
/// already-confirmed hashes survive another mid-broadcast failure. Otherwise
/// build a fresh pre-routing sequence from the current script transactions.
pub fn build_checkpoint_sequence(
    btxs: &foundry_cheatcodes::BroadcastableTransactions,
    recorded_txs: &[RecordedTransaction],
    ctx: &PipelineContext,
    broadcast_path: PathBuf,
    cache_path: PathBuf,
    resume: Option<&ResumeState>,
) -> ScriptSequence {
    if let Some(resume) = resume.filter(|resume| resume.sequence.transactions.len() == btxs.len()) {
        let timestamp =
            SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let mut sequence = resume.sequence.clone();
        sequence.paths = Some((broadcast_path, cache_path));
        sequence.chain = ctx.config.chain_id;
        sequence.timestamp = timestamp;
        sequence.commit =
            if ctx.git_commit.is_empty() { None } else { Some(ctx.git_commit.clone()) };
        return sequence;
    }

    build_pre_routing_sequence(btxs, recorded_txs, ctx, broadcast_path, cache_path)
}

/// Build a Foundry-compatible `ScriptSequence` from routing results.
///
/// Only includes transactions that were actually broadcast on-chain (wallet
/// direct or Safe 1/1 execution). Deferred operations go to the companion file.
pub fn build_script_sequence(
    btxs: &foundry_cheatcodes::BroadcastableTransactions,
    run_results: &[(TransactionRun, RunResult)],
    recorded_txs: &[RecordedTransaction],
    ctx: &PipelineContext,
    broadcast_path: PathBuf,
    cache_path: PathBuf,
) -> ScriptSequence {
    let mut transactions = VecDeque::new();
    let mut receipts = Vec::new();
    let rpc_url = ctx.config.rpc_url.as_deref().unwrap_or_default();

    // Build a lookup from btx index → recorded transaction for metadata
    let rt_by_index: std::collections::HashMap<usize, &RecordedTransaction> = {
        let mut map = std::collections::HashMap::new();
        // Recorded transactions are in the same order as BroadcastableTransactions indices
        for (i, rt) in recorded_txs.iter().enumerate() {
            map.insert(i, rt);
        }
        map
    };

    for (run, result) in run_results {
        if let RunResult::Broadcast(run_receipts) = result {
            for (receipt_idx, &tx_idx) in run.tx_indices.iter().enumerate() {
                let Some(btx) = btxs.get(tx_idx) else { continue };
                let receipt = run_receipts.get(receipt_idx);

                // Build TransactionWithMetadata
                let tx_request = btx_to_transaction_request(btx);
                let tx_maybe_signed =
                    foundry_common::TransactionMaybeSigned::new(tx_request.into());
                let mut tx_meta = TransactionWithMetadata::from_tx_request(tx_maybe_signed);
                tx_meta.rpc = rpc_url.to_string();

                // Set hash from receipt
                if let Some(r) = receipt {
                    tx_meta.hash = Some(r.hash);
                }

                // Determine opcode (Create vs Call)
                let is_create =
                    matches!(btx.transaction.to(), None | Some(alloy_primitives::TxKind::Create));
                tx_meta.opcode = if is_create { CallKind::Create } else { CallKind::Call };

                // Set contract metadata from recorded transaction
                if let Some(rt) = rt_by_index.get(&tx_idx) {
                    if let Some(op) = rt.transaction.operations.first() {
                        if op.operation_type == "DEPLOY" {
                            tx_meta.contract_name = Some(op.target.clone());
                            if let Some(addr_val) = op.result.get("address") {
                                if let Some(addr_str) = addr_val.as_str() {
                                    tx_meta.contract_address = addr_str.parse().ok();
                                }
                            }
                        }
                        if !op.method.is_empty() && op.method != "CREATE" {
                            tx_meta.function = Some(op.method.clone());
                        }
                    }
                }

                // Set contract metadata from receipt
                if let Some(r) = receipt {
                    if tx_meta.contract_name.is_none() {
                        tx_meta.contract_name = r.contract_name.clone();
                    }
                    if tx_meta.contract_address.is_none() {
                        tx_meta.contract_address = r.contract_address;
                    }
                }

                transactions.push_back(tx_meta);

                // Add receipt if we have the raw JSON
                if let Some(r) = receipt {
                    if let Some(ref raw) = r.raw_receipt {
                        if let Ok(any_receipt) = serde_json::from_value(raw.clone()) {
                            receipts.push(any_receipt);
                        }
                    }
                }
            }
        }
    }

    let timestamp =
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);

    ScriptSequence {
        transactions,
        receipts,
        libraries: Vec::new(),
        pending: Vec::new(),
        paths: Some((broadcast_path, cache_path)),
        returns: AlloyHashMap::default(),
        timestamp,
        chain: ctx.config.chain_id,
        commit: if ctx.git_commit.is_empty() { None } else { Some(ctx.git_commit.clone()) },
    }
}

// ---------------------------------------------------------------------------
// Build QueuedOperations
// ---------------------------------------------------------------------------

/// Build the queued operations companion file from routing results.
pub fn build_queued_operations(
    run_results: &[(TransactionRun, RunResult)],
    recorded_txs: &[RecordedTransaction],
    ctx: &PipelineContext,
) -> QueuedOperations {
    let mut safe_proposals = Vec::new();
    let mut governor_proposals = Vec::new();

    let timestamp =
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);

    for (run, result) in run_results {
        match result {
            RunResult::SafeProposed { safe_tx_hash, safe_address, nonce, tx_count: _ } => {
                let tx_ids: Vec<String> = run
                    .tx_indices
                    .iter()
                    .filter_map(|&idx| recorded_txs.get(idx))
                    .map(|rt| rt.transaction.id.clone())
                    .collect();

                safe_proposals.push(QueuedSafeProposal {
                    safe_tx_hash: format!("{:#x}", safe_tx_hash),
                    safe_address: format!("{:#x}", safe_address),
                    nonce: *nonce,
                    chain_id: ctx.config.chain_id,
                    sender_role: run.sender_role.clone(),
                    transaction_ids: tx_ids,
                    status: "queued".into(),
                    execution_tx_hash: None,
                });
            }
            RunResult::GovernorProposed { proposal_id, governor_address, tx_count: _ } => {
                let tx_ids: Vec<String> = run
                    .tx_indices
                    .iter()
                    .filter_map(|&idx| recorded_txs.get(idx))
                    .map(|rt| rt.transaction.id.clone())
                    .collect();

                governor_proposals.push(QueuedGovernorProposal {
                    proposal_id: proposal_id.clone(),
                    governor_address: format!("{:#x}", governor_address),
                    sender_role: run.sender_role.clone(),
                    transaction_ids: tx_ids,
                    status: "queued".into(),
                    propose_tx_hash: None,
                    propose_safe_tx_hash: None,
                });
            }
            RunResult::Broadcast(_) => {}
        }
    }

    QueuedOperations {
        timestamp,
        chain: ctx.config.chain_id,
        commit: if ctx.git_commit.is_empty() { None } else { Some(ctx.git_commit.clone()) },
        safe_proposals,
        governor_proposals,
    }
}

/// Write broadcast files: `run-latest.json` + timestamped copy + queued.
///
/// Uses Foundry's built-in `ScriptSequence::save()` for the main file,
/// then writes the queued companion file alongside it.
pub fn write_broadcast_artifacts(
    sequence: &mut ScriptSequence,
    queued: &QueuedOperations,
) -> Result<BroadcastArtifactPaths, TrebError> {
    let Some((broadcast_path, _cache_path)) = sequence.paths.clone() else {
        return Err(TrebError::Forge("script sequence missing broadcast paths".into()));
    };

    // Ensure broadcast and cache directories exist before Foundry's save(),
    // which assumes the parent directories are already present.
    if let Some((ref broadcast_path, ref cache_path)) = sequence.paths {
        if let Some(parent) = broadcast_path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                TrebError::Forge(format!(
                    "failed to create broadcast directory {}: {e}",
                    parent.display()
                ))
            })?;
        }
        if let Some(parent) = cache_path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                TrebError::Forge(format!(
                    "failed to create cache directory {}: {e}",
                    parent.display()
                ))
            })?;
        }
    }

    // Save the main ScriptSequence using Foundry's save method
    // (writes run-latest.json + run-{timestamp}.json + cache/ sensitive copy)
    sequence
        .save(true, true)
        .map_err(|e| TrebError::Forge(format!("failed to write broadcast file: {e}")))?;
    let archived_broadcast_path = timestamped_run_path(&broadcast_path, sequence.timestamp);

    // Write queued operations companion file
    let mut latest_queued_path = None;
    let mut archived_queued_path = None;
    if !queued.safe_proposals.is_empty() || !queued.governor_proposals.is_empty() {
        let mut queued = queued.clone();
        queued.timestamp = sequence.timestamp;
        if let Some((ref broadcast_path, _)) = sequence.paths {
            let queued_file = queued_path_from(broadcast_path);
            let ts_queued = queued_path_from(&archived_broadcast_path);

            // Ensure directory exists
            if let Some(parent) = queued_file.parent() {
                fs::create_dir_all(parent).map_err(|e| {
                    TrebError::Forge(format!(
                        "failed to create queued directory {}: {e}",
                        parent.display()
                    ))
                })?;
            }

            let file = fs::File::create(&queued_file).map_err(|e| {
                TrebError::Forge(format!(
                    "failed to create queued file {}: {e}",
                    queued_file.display()
                ))
            })?;
            let mut writer = BufWriter::new(file);
            serde_json::to_writer_pretty(&mut writer, &queued).map_err(|e| {
                TrebError::Forge(format!("failed to serialize queued operations: {e}"))
            })?;
            writer
                .flush()
                .map_err(|e| TrebError::Forge(format!("failed to flush queued file: {e}")))?;
            let _ = fs::copy(&queued_file, &ts_queued);
            latest_queued_path = Some(queued_file);
            archived_queued_path = Some(ts_queued);
        }
    }

    Ok(BroadcastArtifactPaths {
        latest_broadcast_path: broadcast_path,
        archived_broadcast_path,
        latest_queued_path,
        archived_queued_path,
    })
}

// ---------------------------------------------------------------------------
// Queued file back-patching (compose merge)
// ---------------------------------------------------------------------------

/// Update a Safe proposal in an existing queued file with a new hash and nonce.
///
/// Used by compose's merge flow: after adjacent Safe proposals are merged,
/// each component's `run-latest.queued.json` must reference the merged
/// hash/nonce instead of the per-component values.
pub fn update_queued_safe_proposal(
    broadcast_path: &Path,
    old_hash: &str,
    new_hash: &str,
    new_nonce: u64,
) -> Result<(), TrebError> {
    let queued_file = queued_path_from(broadcast_path);
    if !queued_file.exists() {
        return Ok(());
    }

    let contents = fs::read_to_string(&queued_file).map_err(|e| {
        TrebError::Forge(format!("failed to read queued file {}: {e}", queued_file.display()))
    })?;

    let mut queued: QueuedOperations = serde_json::from_str(&contents).map_err(|e| {
        TrebError::Forge(format!("failed to parse queued file {}: {e}", queued_file.display()))
    })?;

    for proposal in &mut queued.safe_proposals {
        if proposal.safe_tx_hash == old_hash {
            proposal.safe_tx_hash = new_hash.to_string();
            proposal.nonce = new_nonce;
            proposal.status = "queued".into();
        }
    }

    let file = fs::File::create(&queued_file).map_err(|e| {
        TrebError::Forge(format!("failed to write queued file {}: {e}", queued_file.display()))
    })?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, &queued)
        .map_err(|e| TrebError::Forge(format!("failed to serialize queued operations: {e}")))?;
    writer.flush().map_err(|e| TrebError::Forge(format!("failed to flush queued file: {e}")))?;

    Ok(())
}

/// Load existing broadcast files for `--resume`.
///
/// Returns `None` if no broadcast file exists for this script/chain/sig combo.
///
/// Polls `eth_getTransactionReceipt` for each transaction that has a hash to
/// distinguish confirmed (on-chain receipt) from pending (no receipt yet).
/// Transactions with `hash: None` are unsent and appear in neither set.
pub async fn load_resume_state(
    project_root: &Path,
    script_path: &str,
    chain_id: u64,
    sig: &str,
    rpc_url: &str,
) -> Option<ResumeState> {
    let (_, broadcast_path, _cache_path) =
        compute_broadcast_paths(project_root, script_path, chain_id, sig);
    load_resume_state_from_path(&broadcast_path, rpc_url).await
}

/// Load resume state from an exact broadcast file path.
pub async fn load_resume_state_from_path(
    broadcast_path: &Path,
    rpc_url: &str,
) -> Option<ResumeState> {
    if !broadcast_path.exists() {
        return None;
    }

    // Load ScriptSequence
    let sequence: ScriptSequence = {
        let contents = fs::read_to_string(&broadcast_path).ok()?;
        serde_json::from_str(&contents).ok()?
    };

    // Load queued operations (optional). Fall back to the legacy deferred
    // suffix so older checkpoints can still resume.
    let queued_file = queued_path_from(&broadcast_path);
    let legacy_deferred_file = legacy_deferred_path_from(&broadcast_path);
    let queued: Option<QueuedOperations> = if queued_file.exists() {
        fs::read_to_string(&queued_file).ok().and_then(|c| serde_json::from_str(&c).ok())
    } else if legacy_deferred_file.exists() {
        fs::read_to_string(&legacy_deferred_file).ok().and_then(|c| serde_json::from_str(&c).ok())
    } else {
        None
    };

    // Poll on-chain receipts for transactions that have hashes
    let hashes_to_check: Vec<B256> =
        sequence.transactions.iter().filter_map(|tx_meta| tx_meta.hash).collect();

    let mut completed_tx_hashes = std::collections::HashSet::new();
    let mut pending_tx_hashes = std::collections::HashSet::new();

    if !hashes_to_check.is_empty() {
        match crate::provider::build_http_provider(rpc_url) {
            Ok(provider) => {
                for hash in &hashes_to_check {
                    match poll_receipt_exists(&provider, hash).await {
                        true => {
                            completed_tx_hashes.insert(*hash);
                        }
                        false => {
                            pending_tx_hashes.insert(*hash);
                        }
                    }
                }
            }
            Err(_) => {
                // Provider construction failed (e.g. malformed URL).
                // Treat all hashes as pending — connection failures mean
                // we can't confirm receipts, so assume pending.
                for hash in &hashes_to_check {
                    pending_tx_hashes.insert(*hash);
                }
            }
        }
    }

    let completed_safe_hashes: std::collections::HashSet<String> = queued
        .as_ref()
        .map(|d| {
            d.safe_proposals
                .iter()
                .filter(|p| p.status == "queued" || p.status == "executed")
                .map(|p| p.safe_tx_hash.clone())
                .collect()
        })
        .unwrap_or_default();

    let completed_gov_ids: std::collections::HashSet<String> = queued
        .as_ref()
        .map(|d| {
            d.governor_proposals
                .iter()
                .filter(|p| p.status == "queued" || p.status == "executed")
                .filter(|p| !p.proposal_id.is_empty())
                .map(|p| p.proposal_id.clone())
                .collect()
        })
        .unwrap_or_default();

    Some(ResumeState {
        sequence,
        queued,
        completed_tx_hashes,
        pending_tx_hashes,
        completed_safe_hashes,
        completed_gov_ids,
    })
}

/// Check whether an on-chain receipt exists for a transaction hash.
///
/// Returns `true` if the provider returns a receipt, `false` otherwise
/// (including RPC errors — treated as "not yet confirmed").
async fn poll_receipt_exists(provider: &impl alloy_provider::Provider, tx_hash: &B256) -> bool {
    matches!(provider.get_transaction_receipt(*tx_hash).await, Ok(Some(_)))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_broadcast_paths_basic() {
        let (dir, broadcast, cache) =
            compute_broadcast_paths(Path::new("/project"), "script/Deploy.s.sol", 42220, "run()");
        assert_eq!(dir, PathBuf::from("/project/broadcast/Deploy.s.sol/42220"));
        assert_eq!(
            broadcast,
            PathBuf::from("/project/broadcast/Deploy.s.sol/42220/run-latest.json")
        );
        assert_eq!(cache, PathBuf::from("/project/cache/Deploy.s.sol/42220/run-latest.json"));
    }

    #[test]
    fn compute_broadcast_paths_custom_sig() {
        let (_, broadcast, _) = compute_broadcast_paths(
            Path::new("/project"),
            "script/Deploy.s.sol",
            1,
            "deploy(uint256)",
        );
        assert_eq!(
            broadcast,
            PathBuf::from("/project/broadcast/Deploy.s.sol/1/deploy-latest.json")
        );
    }

    #[test]
    fn queued_path_from_broadcast() {
        let bp = PathBuf::from("/project/broadcast/Deploy.s.sol/42220/run-latest.json");
        let dp = queued_path_from(&bp);
        assert_eq!(
            dp,
            PathBuf::from("/project/broadcast/Deploy.s.sol/42220/run-latest.queued.json")
        );
    }

    #[test]
    fn relative_broadcast_path_strips_root() {
        let root = Path::new("/project");
        let bp = Path::new("/project/broadcast/Deploy.s.sol/42220/run-latest.json");
        let rel = relative_broadcast_path(root, bp);
        assert_eq!(rel, "broadcast/Deploy.s.sol/42220/run-latest.json");
    }

    #[test]
    fn timestamped_path_from_latest_replaces_latest_suffix() {
        let compose = Path::new("/project/broadcast/full.yaml/42220/compose-latest.json");
        let archived = timestamped_path_from_latest(compose, 1234567890);
        assert_eq!(
            archived,
            PathBuf::from("/project/broadcast/full.yaml/42220/compose-1234567890.json")
        );

        let queued = Path::new("/project/broadcast/Deploy.s.sol/42220/run-latest.queued.json");
        let archived_queued = timestamped_path_from_latest(queued, 1234567890);
        assert_eq!(
            archived_queued,
            PathBuf::from("/project/broadcast/Deploy.s.sol/42220/run-1234567890.queued.json")
        );
    }

    #[test]
    fn timestamped_run_path_uses_foundry_run_archive_name() {
        let latest = Path::new("/project/broadcast/Deploy.s.sol/42220/deploy-latest.json");
        let archived = timestamped_run_path(latest, 1234567890);
        assert_eq!(
            archived,
            PathBuf::from("/project/broadcast/Deploy.s.sol/42220/run-1234567890.json")
        );
    }

    #[test]
    fn queued_operations_empty_roundtrip() {
        let queued = QueuedOperations {
            timestamp: 1710600000000,
            chain: 42220,
            commit: Some("abc1234".into()),
            safe_proposals: Vec::new(),
            governor_proposals: Vec::new(),
        };
        let json = serde_json::to_string_pretty(&queued).unwrap();
        let parsed: QueuedOperations = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.chain, 42220);
        assert_eq!(parsed.commit, Some("abc1234".into()));
    }

    #[test]
    fn queued_operations_with_safe_roundtrip() {
        let queued = QueuedOperations {
            timestamp: 1710600000000,
            chain: 1,
            commit: None,
            safe_proposals: vec![QueuedSafeProposal {
                safe_tx_hash: "0xabc".into(),
                safe_address: "0x123".into(),
                nonce: 5,
                chain_id: 1,
                sender_role: "deployer".into(),
                transaction_ids: vec!["tx-1".into(), "tx-2".into()],
                status: "pending".into(),
                execution_tx_hash: None,
            }],
            governor_proposals: Vec::new(),
        };
        let json = serde_json::to_string_pretty(&queued).unwrap();
        let parsed: QueuedOperations = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.safe_proposals.len(), 1);
        assert_eq!(parsed.safe_proposals[0].nonce, 5);
        assert_eq!(parsed.safe_proposals[0].transaction_ids.len(), 2);
    }

    #[test]
    fn pending_queued_operations_include_safe_and_governor_for_safe_reduced_governor() {
        use super::super::routing::{
            GovernanceContext, GovernorAction, PlannedAction, QueuedExecution, RoutableTx,
            RoutingAction, RoutingPlan, TransactionRun,
        };
        use crate::sender::SenderCategory;
        use alloy_primitives::{Address, U256};

        let safe_address = Address::repeat_byte(0x11);
        let governor_address = Address::repeat_byte(0x22);
        let target = Address::repeat_byte(0x33);
        let recorded_txs = vec![make_recorded_tx("tx-1", None, "upgrade")];
        let plan = RoutingPlan {
            actions: vec![PlannedAction {
                run: TransactionRun {
                    sender_role: "governor".into(),
                    category: SenderCategory::Governor,
                    sender_address: governor_address,
                    tx_indices: vec![0],
                },
                action: RoutingAction::Propose {
                    safe_address,
                    chain_id: 42220,
                    operations: vec![treb_safe::MultiSendOperation {
                        operation: 0,
                        to: governor_address,
                        value: U256::ZERO,
                        data: alloy_primitives::Bytes::from(vec![0x12, 0x34]),
                    }],
                    inner_transactions: vec![RoutableTx {
                        to: target,
                        value: U256::ZERO,
                        data: vec![0xab, 0xcd],
                    }],
                    sender_role: "committee".into(),
                    nonce: 7,
                    governance: Some(GovernanceContext {
                        governor_address,
                        timelock_address: None,
                        proposal_description: String::new(),
                    }),
                },
                queued: Some(QueuedExecution::GovernanceProposal {
                    governor_address,
                    timelock_address: None,
                    actions: vec![GovernorAction {
                        target,
                        value: U256::ZERO,
                        calldata: vec![0xab, 0xcd],
                    }],
                    proposal_description: String::new(),
                }),
            }],
        };

        let queued = build_pending_queued_operations_from_plan(
            &plan,
            &recorded_txs,
            &test_pipeline_context(),
            None,
        );

        assert_eq!(queued.safe_proposals.len(), 1);
        assert_eq!(queued.governor_proposals.len(), 1);
        assert_eq!(queued.safe_proposals[0].status, "pending");
        assert_eq!(queued.governor_proposals[0].status, "pending");
        assert_eq!(queued.safe_proposals[0].transaction_ids, vec!["tx-1"]);
        assert_eq!(queued.governor_proposals[0].transaction_ids, vec!["tx-1"]);
        assert_eq!(queued.governor_proposals[0].proposal_id, "");
    }

    #[test]
    fn pending_queued_operations_preserve_existing_resume_status() {
        use super::super::routing::{
            GovernanceContext, GovernorAction, PlannedAction, QueuedExecution, RoutableTx,
            RoutingAction, RoutingPlan, TransactionRun,
        };
        use crate::sender::SenderCategory;
        use alloy_primitives::{Address, U256};

        let safe_address = Address::repeat_byte(0x44);
        let governor_address = Address::repeat_byte(0x55);
        let recorded_txs = vec![make_recorded_tx("tx-1", None, "upgrade")];
        let plan = RoutingPlan {
            actions: vec![PlannedAction {
                run: TransactionRun {
                    sender_role: "governor".into(),
                    category: SenderCategory::Governor,
                    sender_address: governor_address,
                    tx_indices: vec![0],
                },
                action: RoutingAction::Propose {
                    safe_address,
                    chain_id: 42220,
                    operations: vec![treb_safe::MultiSendOperation {
                        operation: 0,
                        to: governor_address,
                        value: U256::ZERO,
                        data: alloy_primitives::Bytes::from(vec![0x56]),
                    }],
                    inner_transactions: vec![RoutableTx {
                        to: governor_address,
                        value: U256::ZERO,
                        data: vec![0x56],
                    }],
                    sender_role: "committee".into(),
                    nonce: 9,
                    governance: Some(GovernanceContext {
                        governor_address,
                        timelock_address: None,
                        proposal_description: String::new(),
                    }),
                },
                queued: Some(QueuedExecution::GovernanceProposal {
                    governor_address,
                    timelock_address: None,
                    actions: vec![GovernorAction {
                        target: governor_address,
                        value: U256::ZERO,
                        calldata: vec![0x56],
                    }],
                    proposal_description: String::new(),
                }),
            }],
        };
        let mut existing = build_pending_queued_operations_from_plan(
            &plan,
            &recorded_txs,
            &test_pipeline_context(),
            None,
        );
        existing.safe_proposals[0].status = "queued".into();
        existing.governor_proposals[0].status = "queued".into();
        existing.governor_proposals[0].proposal_id = "0xproposal".into();
        existing.governor_proposals[0].propose_safe_tx_hash = Some("0xsafe".into());

        let rebuilt = build_pending_queued_operations_from_plan(
            &plan,
            &recorded_txs,
            &test_pipeline_context(),
            Some(&existing),
        );

        assert_eq!(rebuilt.safe_proposals[0].status, "queued");
        assert_eq!(rebuilt.governor_proposals[0].status, "queued");
        assert_eq!(rebuilt.governor_proposals[0].proposal_id, "0xproposal");
        assert_eq!(rebuilt.governor_proposals[0].propose_safe_tx_hash.as_deref(), Some("0xsafe"));
    }

    #[test]
    fn save_queued_checkpoint_writes_pending_latest_file() {
        let dir = tempfile::tempdir().unwrap();
        let queued = QueuedOperations {
            timestamp: 0,
            chain: 1,
            commit: None,
            safe_proposals: vec![QueuedSafeProposal {
                safe_tx_hash: "0xabc".into(),
                safe_address: "0x1111".into(),
                nonce: 1,
                chain_id: 1,
                sender_role: "safe".into(),
                transaction_ids: vec!["tx-1".into()],
                status: "pending".into(),
                execution_tx_hash: None,
            }],
            governor_proposals: Vec::new(),
        };
        let sequence = ScriptSequence {
            transactions: VecDeque::new(),
            receipts: Vec::new(),
            libraries: Vec::new(),
            pending: Vec::new(),
            paths: Some((
                dir.path().join("broadcast/Deploy.s.sol/1/run-latest.json"),
                dir.path().join("cache/Deploy.s.sol/1/run-latest.json"),
            )),
            returns: AlloyHashMap::default(),
            timestamp: 1234567890,
            chain: 1,
            commit: None,
        };

        save_queued_checkpoint(&sequence, &queued).unwrap();

        let queued_path = dir.path().join("broadcast/Deploy.s.sol/1/run-latest.queued.json");
        let saved: QueuedOperations =
            serde_json::from_str(&std::fs::read_to_string(queued_path).unwrap()).unwrap();
        assert_eq!(saved.safe_proposals[0].status, "pending");
        assert_eq!(saved.timestamp, 1234567890);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn load_resume_state_returns_none_for_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let result = load_resume_state(
            tmp.path(),
            "script/Deploy.s.sol",
            1,
            "run()",
            "http://localhost:8545",
        )
        .await;
        assert!(result.is_none());
    }

    // -----------------------------------------------------------------------
    // build_pre_routing_sequence tests
    // -----------------------------------------------------------------------

    use alloy_primitives::Address;
    use foundry_cheatcodes::BroadcastableTransaction;
    use foundry_common::TransactionMaybeSigned;
    use treb_core::types::{
        enums::TransactionStatus,
        transaction::{Operation, Transaction},
    };

    /// Build a synthetic broadcastable transaction for testing.
    fn make_btx(from: Address, to: Option<Address>, data: &[u8]) -> BroadcastableTransaction {
        use alloy_rpc_types::{TransactionInput, TransactionRequest};
        let mut tx_req = TransactionRequest::default().from(from);
        if let Some(to_addr) = to {
            tx_req = tx_req.to(to_addr);
        }
        if !data.is_empty() {
            tx_req.input = TransactionInput::new(alloy_primitives::Bytes::from(data.to_vec()));
        }
        BroadcastableTransaction {
            rpc: None,
            transaction: TransactionMaybeSigned::new(tx_req.into()),
        }
    }

    /// Build a minimal RecordedTransaction with an optional DEPLOY operation.
    fn make_recorded_tx(
        id: &str,
        deploy_contract: Option<(&str, &str)>,
        method: &str,
    ) -> RecordedTransaction {
        let operations = if let Some((name, addr)) = deploy_contract {
            let mut result = std::collections::HashMap::new();
            result.insert("address".into(), serde_json::json!(addr));
            vec![Operation {
                operation_type: "DEPLOY".into(),
                target: name.into(),
                method: method.into(),
                result,
            }]
        } else if !method.is_empty() {
            vec![Operation {
                operation_type: "CALL".into(),
                target: String::new(),
                method: method.into(),
                result: Default::default(),
            }]
        } else {
            Vec::new()
        };

        RecordedTransaction {
            transaction: Transaction {
                id: id.into(),
                chain_id: 1,
                hash: String::new(),
                status: TransactionStatus::Executed,
                block_number: 0,
                sender: "0xSender".into(),
                nonce: 0,
                deployments: Vec::new(),
                operations,
                safe_context: None,
                broadcast_file: None,
                environment: "production".into(),
                created_at: chrono::Utc::now(),
            },
            sender_name: None,
            sender_category: None,
            gas_used: None,
            trace: None,
        }
    }

    fn test_pipeline_context() -> PipelineContext {
        PipelineContext {
            config: super::super::PipelineConfig {
                script_path: "script/Deploy.s.sol".into(),
                chain_id: 42220,
                rpc_url: Some("http://localhost:8545".into()),
                ..Default::default()
            },
            script_path: PathBuf::from("script/Deploy.s.sol"),
            git_commit: "abc1234".into(),
            project_root: PathBuf::from("/tmp/project"),
            resolved_senders: Default::default(),
            sender_labels: Default::default(),
            sender_configs: Default::default(),
            sender_role_names: Default::default(),
        }
    }

    #[test]
    fn pre_routing_sequence_transaction_count_matches_btxs() {
        let from = Address::repeat_byte(0x01);
        let to = Address::repeat_byte(0x02);
        let mut btxs = foundry_cheatcodes::BroadcastableTransactions::default();
        btxs.push_back(make_btx(from, Some(to), &[0x01]));
        btxs.push_back(make_btx(from, Some(to), &[0x02]));
        btxs.push_back(make_btx(from, None, &[0x03]));

        let ctx = test_pipeline_context();
        let seq = build_pre_routing_sequence(
            &btxs,
            &[],
            &ctx,
            PathBuf::from("/tmp/broadcast.json"),
            PathBuf::from("/tmp/cache.json"),
        );

        assert_eq!(seq.transactions.len(), btxs.len());
    }

    #[test]
    fn pre_routing_sequence_all_hashes_none() {
        let from = Address::repeat_byte(0x01);
        let to = Address::repeat_byte(0x02);
        let mut btxs = foundry_cheatcodes::BroadcastableTransactions::default();
        btxs.push_back(make_btx(from, Some(to), &[0x01]));
        btxs.push_back(make_btx(from, None, &[0x02]));

        let ctx = test_pipeline_context();
        let seq = build_pre_routing_sequence(
            &btxs,
            &[],
            &ctx,
            PathBuf::from("/tmp/broadcast.json"),
            PathBuf::from("/tmp/cache.json"),
        );

        for tx_meta in &seq.transactions {
            assert!(tx_meta.hash.is_none(), "pre-routing hash must be None");
        }
    }

    #[test]
    fn pre_routing_sequence_receipts_and_pending_empty() {
        let from = Address::repeat_byte(0x01);
        let mut btxs = foundry_cheatcodes::BroadcastableTransactions::default();
        btxs.push_back(make_btx(from, Some(Address::repeat_byte(0x02)), &[0x01]));

        let ctx = test_pipeline_context();
        let seq = build_pre_routing_sequence(
            &btxs,
            &[],
            &ctx,
            PathBuf::from("/tmp/broadcast.json"),
            PathBuf::from("/tmp/cache.json"),
        );

        assert!(seq.receipts.is_empty(), "pre-routing receipts must be empty");
        assert!(seq.pending.is_empty(), "pre-routing pending must be empty");
    }

    #[test]
    fn pre_routing_sequence_contract_metadata_from_recorded_tx() {
        let from = Address::repeat_byte(0x01);
        let mut btxs = foundry_cheatcodes::BroadcastableTransactions::default();
        // Create transaction (deploy)
        btxs.push_back(make_btx(from, None, &[0x60, 0x80]));
        // Call transaction
        btxs.push_back(make_btx(from, Some(Address::repeat_byte(0x02)), &[0xab, 0xcd]));

        let recorded_txs = vec![
            make_recorded_tx(
                "tx-1",
                Some(("Counter", "0x0000000000000000000000000000000000001234")),
                "CREATE",
            ),
            make_recorded_tx("tx-2", None, "setNumber"),
        ];

        let ctx = test_pipeline_context();
        let seq = build_pre_routing_sequence(
            &btxs,
            &recorded_txs,
            &ctx,
            PathBuf::from("/tmp/broadcast.json"),
            PathBuf::from("/tmp/cache.json"),
        );

        // First tx: deploy → should have contract name and address
        let deploy_tx = &seq.transactions[0];
        assert_eq!(deploy_tx.contract_name.as_deref(), Some("Counter"));
        assert!(deploy_tx.contract_address.is_some());
        assert_eq!(deploy_tx.opcode, CallKind::Create);

        // Second tx: call → should have function name, no contract name
        let call_tx = &seq.transactions[1];
        assert_eq!(call_tx.function.as_deref(), Some("setNumber"));
        assert!(call_tx.contract_name.is_none());
        assert_eq!(call_tx.opcode, CallKind::Call);
    }

    #[test]
    fn pre_routing_sequence_chain_and_commit() {
        let mut btxs = foundry_cheatcodes::BroadcastableTransactions::default();
        btxs.push_back(make_btx(
            Address::repeat_byte(0x01),
            Some(Address::repeat_byte(0x02)),
            &[0x01],
        ));

        let ctx = test_pipeline_context();
        let seq = build_pre_routing_sequence(
            &btxs,
            &[],
            &ctx,
            PathBuf::from("/tmp/broadcast.json"),
            PathBuf::from("/tmp/cache.json"),
        );

        assert_eq!(seq.chain, 42220);
        assert_eq!(seq.commit.as_deref(), Some("abc1234"));
    }

    #[test]
    fn pre_routing_sequence_empty_btxs() {
        let btxs = foundry_cheatcodes::BroadcastableTransactions::default();
        let ctx = test_pipeline_context();
        let seq = build_pre_routing_sequence(
            &btxs,
            &[],
            &ctx,
            PathBuf::from("/tmp/broadcast.json"),
            PathBuf::from("/tmp/cache.json"),
        );

        assert!(seq.transactions.is_empty());
        assert!(seq.receipts.is_empty());
        assert!(seq.pending.is_empty());
    }

    #[test]
    fn pre_routing_sequence_rpc_url_set() {
        let mut btxs = foundry_cheatcodes::BroadcastableTransactions::default();
        btxs.push_back(make_btx(
            Address::repeat_byte(0x01),
            Some(Address::repeat_byte(0x02)),
            &[0x01],
        ));

        let ctx = test_pipeline_context();
        let seq = build_pre_routing_sequence(
            &btxs,
            &[],
            &ctx,
            PathBuf::from("/tmp/broadcast.json"),
            PathBuf::from("/tmp/cache.json"),
        );

        assert_eq!(seq.transactions[0].rpc, "http://localhost:8545");
    }

    #[test]
    fn save_sequence_checkpoint_writes_broadcast_and_cache_latest_files() {
        let mut btxs = foundry_cheatcodes::BroadcastableTransactions::default();
        btxs.push_back(make_btx(
            Address::repeat_byte(0x01),
            Some(Address::repeat_byte(0x02)),
            &[0x01],
        ));

        let dir = tempfile::tempdir().unwrap();
        let broadcast_path = dir.path().join("broadcast").join("run-latest.json");
        let cache_path = dir.path().join("cache").join("run-latest.json");

        let ctx = test_pipeline_context();
        let mut seq = build_pre_routing_sequence(
            &btxs,
            &[],
            &ctx,
            broadcast_path.clone(),
            cache_path.clone(),
        );
        let tx_hash = B256::repeat_byte(0xAA);
        let receipt = crate::script::BroadcastReceipt {
            hash: tx_hash,
            block_number: 1,
            gas_used: 21_000,
            status: true,
            contract_name: None,
            contract_address: None,
            raw_receipt: Some(serde_json::json!({
                "transactionHash": format!("{:#x}", tx_hash),
                "status": "0x1"
            })),
        };

        update_sequence_checkpoint(&mut seq, 0, &receipt);
        save_sequence_checkpoint(&mut seq).unwrap();

        assert!(broadcast_path.exists(), "broadcast checkpoint should exist");
        assert!(cache_path.exists(), "cache checkpoint should exist");

        let broadcast_json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&broadcast_path).unwrap()).unwrap();
        let cache_json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&cache_path).unwrap()).unwrap();

        let expected_hash = format!("{:#x}", tx_hash);
        assert_eq!(
            broadcast_json["transactions"][0]["hash"].as_str(),
            Some(expected_hash.as_str())
        );
        assert_eq!(cache_json["transactions"][0]["hash"].as_str(), Some(expected_hash.as_str()));
    }

    // -----------------------------------------------------------------------
    // load_resume_state polling tests
    // -----------------------------------------------------------------------

    /// Write a minimal ScriptSequence JSON to the Foundry broadcast path.
    ///
    /// `tx_hashes` entries: `Some(hash)` = transaction has a hash, `None` = unsent.
    fn write_sequence_fixture(
        project_root: &Path,
        script_path: &str,
        chain_id: u64,
        sig: &str,
        tx_hashes: &[Option<B256>],
    ) {
        let (broadcast_dir, broadcast_path, _) =
            compute_broadcast_paths(project_root, script_path, chain_id, sig);
        fs::create_dir_all(&broadcast_dir).unwrap();

        let from = Address::repeat_byte(0x01);
        let to = Address::repeat_byte(0x02);
        let mut transactions = VecDeque::new();
        for hash in tx_hashes {
            let btx = make_btx(from, Some(to), &[0x01]);
            let tx_request = btx_to_transaction_request(&btx);
            let tx_maybe_signed: foundry_common::TransactionMaybeSigned =
                foundry_common::TransactionMaybeSigned::new(tx_request.into());
            let mut tx_meta = TransactionWithMetadata::from_tx_request(tx_maybe_signed);
            tx_meta.hash = *hash;
            tx_meta.rpc = "http://localhost:8545".into();
            transactions.push_back(tx_meta);
        }

        let seq: ScriptSequence = ScriptSequence {
            transactions,
            receipts: Vec::new(),
            libraries: Vec::new(),
            pending: Vec::new(),
            paths: Some((broadcast_path.clone(), PathBuf::from("/tmp/cache.json"))),
            returns: AlloyHashMap::default(),
            timestamp: 0,
            chain: chain_id,
            commit: None,
        };

        let json = serde_json::to_string_pretty(&seq).unwrap();
        fs::write(&broadcast_path, json).unwrap();
    }

    /// Start a tiny async HTTP server that responds to `eth_getTransactionReceipt`.
    ///
    /// `confirmed_hashes` defines which hashes return a receipt; all others return null.
    async fn start_mock_rpc(
        confirmed_hashes: std::collections::HashSet<B256>,
    ) -> (u16, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let handle = tokio::spawn(async move {
            // Serve a limited number of requests then stop
            for _ in 0..20 {
                let Ok((mut stream, _)) = listener.accept().await else { break };
                use tokio::io::{AsyncReadExt, AsyncWriteExt};

                let mut buf = vec![0u8; 4096];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);

                // Extract JSON body after \r\n\r\n
                let body = request.split("\r\n\r\n").nth(1).unwrap_or("{}");
                let req_json: serde_json::Value = serde_json::from_str(body).unwrap_or_default();

                let hash_str = req_json
                    .get("params")
                    .and_then(|p| p.get(0))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                let is_confirmed =
                    hash_str.parse::<B256>().ok().is_some_and(|h| confirmed_hashes.contains(&h));

                let result = if is_confirmed {
                    serde_json::json!({
                        "transactionHash": hash_str,
                        "transactionIndex": "0x0",
                        "blockHash": "0x0000000000000000000000000000000000000000000000000000000000000001",
                        "blockNumber": "0x1",
                        "from": "0x0000000000000000000000000000000000000001",
                        "to": "0x0000000000000000000000000000000000000002",
                        "cumulativeGasUsed": "0x5208",
                        "gasUsed": "0x5208",
                        "effectiveGasPrice": "0x3b9aca00",
                        "status": "0x1",
                        "logs": [],
                        "logsBloom": "0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
                        "type": "0x0"
                    })
                } else {
                    serde_json::Value::Null
                };

                let resp_body = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": req_json.get("id").cloned().unwrap_or(serde_json::json!(1)),
                    "result": result,
                });
                let resp_str = resp_body.to_string();
                let http_resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    resp_str.len(),
                    resp_str,
                );
                let _ = stream.write_all(http_resp.as_bytes()).await;
            }
        });

        (port, handle)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resume_state_all_unsent() {
        // All transactions have hash: None → both completed and pending are empty
        let tmp = tempfile::tempdir().unwrap();
        write_sequence_fixture(tmp.path(), "script/Deploy.s.sol", 1, "run()", &[None, None]);

        let (port, handle) = start_mock_rpc(std::collections::HashSet::new()).await;
        let rpc_url = format!("http://127.0.0.1:{port}");

        let state = load_resume_state(tmp.path(), "script/Deploy.s.sol", 1, "run()", &rpc_url)
            .await
            .expect("should load");

        assert_eq!(state.sequence.transactions.len(), 2);
        assert!(state.completed_tx_hashes.is_empty(), "no completed hashes for unsent txs");
        assert!(state.pending_tx_hashes.is_empty(), "no pending hashes for unsent txs");

        handle.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resume_state_completed_via_rpc_poll() {
        // Hash is set and RPC returns a receipt → completed
        let hash = B256::repeat_byte(0xAA);
        let tmp = tempfile::tempdir().unwrap();
        write_sequence_fixture(tmp.path(), "script/Deploy.s.sol", 1, "run()", &[Some(hash)]);

        let mut confirmed = std::collections::HashSet::new();
        confirmed.insert(hash);
        let (port, handle) = start_mock_rpc(confirmed).await;
        let rpc_url = format!("http://127.0.0.1:{port}");

        let state = load_resume_state(tmp.path(), "script/Deploy.s.sol", 1, "run()", &rpc_url)
            .await
            .expect("should load");

        assert!(state.completed_tx_hashes.contains(&hash), "hash should be completed");
        assert!(state.pending_tx_hashes.is_empty(), "no pending hashes");

        handle.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resume_state_pending_via_rpc_poll() {
        // Hash is set but RPC returns null → pending
        let hash = B256::repeat_byte(0xBB);
        let tmp = tempfile::tempdir().unwrap();
        write_sequence_fixture(tmp.path(), "script/Deploy.s.sol", 1, "run()", &[Some(hash)]);

        // Empty confirmed set: all hashes come back as null
        let (port, handle) = start_mock_rpc(std::collections::HashSet::new()).await;
        let rpc_url = format!("http://127.0.0.1:{port}");

        let state = load_resume_state(tmp.path(), "script/Deploy.s.sol", 1, "run()", &rpc_url)
            .await
            .expect("should load");

        assert!(state.completed_tx_hashes.is_empty(), "no completed hashes");
        assert!(state.pending_tx_hashes.contains(&hash), "hash should be pending");

        handle.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resume_state_mixed_confirmed_pending_unsent() {
        let confirmed_hash = B256::repeat_byte(0x11);
        let pending_hash = B256::repeat_byte(0x22);
        let tmp = tempfile::tempdir().unwrap();
        write_sequence_fixture(
            tmp.path(),
            "script/Deploy.s.sol",
            1,
            "run()",
            &[Some(confirmed_hash), Some(pending_hash), None],
        );

        let mut confirmed = std::collections::HashSet::new();
        confirmed.insert(confirmed_hash);
        let (port, handle) = start_mock_rpc(confirmed).await;
        let rpc_url = format!("http://127.0.0.1:{port}");

        let state = load_resume_state(tmp.path(), "script/Deploy.s.sol", 1, "run()", &rpc_url)
            .await
            .expect("should load");

        assert_eq!(state.sequence.transactions.len(), 3);
        assert_eq!(state.completed_tx_hashes.len(), 1, "one confirmed");
        assert!(state.completed_tx_hashes.contains(&confirmed_hash));
        assert_eq!(state.pending_tx_hashes.len(), 1, "one pending");
        assert!(state.pending_tx_hashes.contains(&pending_hash));

        handle.abort();
    }

    #[test]
    fn update_queued_safe_proposal_patches_hash_and_nonce() {
        let dir = tempfile::tempdir().unwrap();
        let broadcast_path = dir.path().join("run-latest.json");
        let queued_path = dir.path().join("run-latest.queued.json");

        let queued = QueuedOperations {
            timestamp: 1234567890,
            chain: 1,
            commit: None,
            safe_proposals: vec![QueuedSafeProposal {
                safe_tx_hash: "0xaaaa".into(),
                safe_address: "0x1111".into(),
                nonce: 5,
                chain_id: 1,
                sender_role: "deployer".into(),
                transaction_ids: vec!["tx-1".into()],
                status: "pending".into(),
                execution_tx_hash: None,
            }],
            governor_proposals: Vec::new(),
        };
        let contents = serde_json::to_string_pretty(&queued).unwrap();
        std::fs::write(&queued_path, contents).unwrap();

        update_queued_safe_proposal(&broadcast_path, "0xaaaa", "0xbbbb", 10).unwrap();

        let updated: QueuedOperations =
            serde_json::from_str(&std::fs::read_to_string(&queued_path).unwrap()).unwrap();
        assert_eq!(updated.safe_proposals.len(), 1);
        assert_eq!(updated.safe_proposals[0].safe_tx_hash, "0xbbbb");
        assert_eq!(updated.safe_proposals[0].nonce, 10);
        // Unchanged fields
        assert_eq!(updated.safe_proposals[0].safe_address, "0x1111");
        assert_eq!(updated.safe_proposals[0].sender_role, "deployer");
    }

    #[test]
    fn update_queued_safe_proposal_no_match_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let broadcast_path = dir.path().join("run-latest.json");
        let queued_path = dir.path().join("run-latest.queued.json");

        let queued = QueuedOperations {
            timestamp: 1234567890,
            chain: 1,
            commit: None,
            safe_proposals: vec![QueuedSafeProposal {
                safe_tx_hash: "0xaaaa".into(),
                safe_address: "0x1111".into(),
                nonce: 5,
                chain_id: 1,
                sender_role: "deployer".into(),
                transaction_ids: vec!["tx-1".into()],
                status: "pending".into(),
                execution_tx_hash: None,
            }],
            governor_proposals: Vec::new(),
        };
        std::fs::write(&queued_path, serde_json::to_string_pretty(&queued).unwrap()).unwrap();

        update_queued_safe_proposal(&broadcast_path, "0xcccc", "0xbbbb", 10).unwrap();

        let updated: QueuedOperations =
            serde_json::from_str(&std::fs::read_to_string(&queued_path).unwrap()).unwrap();
        // Should be unchanged
        assert_eq!(updated.safe_proposals[0].safe_tx_hash, "0xaaaa");
        assert_eq!(updated.safe_proposals[0].nonce, 5);
    }

    #[test]
    fn update_queued_safe_proposal_missing_file_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        let broadcast_path = dir.path().join("run-latest.json");
        let result = update_queued_safe_proposal(&broadcast_path, "0xaaaa", "0xbbbb", 10);
        assert!(result.is_ok());
    }
}
