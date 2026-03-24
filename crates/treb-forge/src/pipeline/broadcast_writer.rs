//! Broadcast file writer — Foundry-compatible `ScriptSequence` construction and persistence.
//!
//! After routing, this module constructs `run-latest.json` in Foundry's exact
//! `ScriptSequence` format for on-chain transactions, and a companion
//! `run-latest.deferred.json` for pending Safe/Governor operations.
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

use alloy_network::Ethereum;
use alloy_primitives::B256;
use forge_script_sequence::{ScriptSequence, TransactionWithMetadata, sig_to_file_name};
use foundry_evm::traces::CallKind;
use serde::{Deserialize, Serialize};
use treb_core::error::TrebError;

use super::{
    PipelineContext,
    routing::{RunResult, TransactionRun},
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

    if let Some(to) = btx.transaction.to() {
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
// Deferred operations (treb extension)
// ---------------------------------------------------------------------------

/// Pending operations that haven't hit the chain yet.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeferredOperations {
    pub timestamp: u128,
    pub chain: u64,
    pub commit: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub safe_proposals: Vec<DeferredSafeProposal>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub governor_proposals: Vec<DeferredGovernorProposal>,
}

/// A Safe proposal awaiting multi-sig execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeferredSafeProposal {
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
pub struct DeferredGovernorProposal {
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

// ---------------------------------------------------------------------------
// Resume state
// ---------------------------------------------------------------------------

/// State loaded from existing broadcast files for `--resume`.
pub struct ResumeState {
    pub sequence: ScriptSequence<Ethereum>,
    pub deferred: Option<DeferredOperations>,
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

/// Compute the deferred file path from the broadcast path.
fn deferred_path_from(broadcast_path: &Path) -> PathBuf {
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

// ---------------------------------------------------------------------------
// Checkpoint helper
// ---------------------------------------------------------------------------

/// Update a pre-built `ScriptSequence` in-place after a transaction is confirmed.
///
/// Sets the hash on the transaction at `tx_idx` and appends the raw receipt
/// (if available) to the sequence's receipts list.
pub fn update_sequence_checkpoint(
    sequence: &mut ScriptSequence<Ethereum>,
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
pub fn ensure_broadcast_dirs(sequence: &ScriptSequence<Ethereum>) -> Result<(), TrebError> {
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
pub fn save_sequence_checkpoint(sequence: &mut ScriptSequence<Ethereum>) -> Result<(), TrebError> {
    sequence
        .save(true, false)
        .map_err(|e| TrebError::Forge(format!("failed to write checkpoint: {e}")))
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
    btxs: &foundry_cheatcodes::BroadcastableTransactions<Ethereum>,
    recorded_txs: &[RecordedTransaction],
    ctx: &PipelineContext,
    broadcast_path: PathBuf,
    cache_path: PathBuf,
) -> ScriptSequence<Ethereum> {
    let mut transactions = VecDeque::new();
    let rpc_url = ctx.config.rpc_url.as_deref().unwrap_or_default();

    for (i, btx) in btxs.iter().enumerate() {
        let tx_request = btx_to_transaction_request(btx);
        let tx_maybe_signed = foundry_common::TransactionMaybeSigned::new(tx_request);
        let mut tx_meta = TransactionWithMetadata::from_tx_request(tx_maybe_signed);
        tx_meta.rpc = rpc_url.to_string();

        // hash is None — not yet broadcast

        // Determine opcode (Create vs Call)
        let is_create = btx.transaction.to().is_none();
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

/// Build a Foundry-compatible `ScriptSequence` from routing results.
///
/// Only includes transactions that were actually broadcast on-chain (wallet
/// direct or Safe 1/1 execution). Deferred operations go to the companion file.
pub fn build_script_sequence(
    btxs: &foundry_cheatcodes::BroadcastableTransactions<Ethereum>,
    run_results: &[(TransactionRun, RunResult)],
    recorded_txs: &[RecordedTransaction],
    ctx: &PipelineContext,
    broadcast_path: PathBuf,
    cache_path: PathBuf,
) -> ScriptSequence<Ethereum> {
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
                let tx_maybe_signed = foundry_common::TransactionMaybeSigned::new(tx_request);
                let mut tx_meta = TransactionWithMetadata::from_tx_request(tx_maybe_signed);
                tx_meta.rpc = rpc_url.to_string();

                // Set hash from receipt
                if let Some(r) = receipt {
                    tx_meta.hash = Some(r.hash);
                }

                // Determine opcode (Create vs Call)
                let is_create = btx.transaction.to().is_none();
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
// Build DeferredOperations
// ---------------------------------------------------------------------------

/// Build the deferred operations companion file from routing results.
pub fn build_deferred_operations(
    run_results: &[(TransactionRun, RunResult)],
    recorded_txs: &[RecordedTransaction],
    ctx: &PipelineContext,
) -> DeferredOperations {
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

                safe_proposals.push(DeferredSafeProposal {
                    safe_tx_hash: format!("{:#x}", safe_tx_hash),
                    safe_address: format!("{:#x}", safe_address),
                    nonce: *nonce,
                    chain_id: ctx.config.chain_id,
                    sender_role: run.sender_role.clone(),
                    transaction_ids: tx_ids,
                    status: "proposed".into(),
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

                governor_proposals.push(DeferredGovernorProposal {
                    proposal_id: proposal_id.clone(),
                    governor_address: format!("{:#x}", governor_address),
                    sender_role: run.sender_role.clone(),
                    transaction_ids: tx_ids,
                    status: "proposed".into(),
                    propose_tx_hash: None,
                    propose_safe_tx_hash: None,
                });
            }
            RunResult::Broadcast(_) => {}
        }
    }

    DeferredOperations {
        timestamp,
        chain: ctx.config.chain_id,
        commit: if ctx.git_commit.is_empty() { None } else { Some(ctx.git_commit.clone()) },
        safe_proposals,
        governor_proposals,
    }
}

// ---------------------------------------------------------------------------
// Write broadcast artifacts
// ---------------------------------------------------------------------------

/// Write broadcast files: `run-latest.json` + timestamped copy + deferred.
///
/// Uses Foundry's built-in `ScriptSequence::save()` for the main file,
/// then writes the deferred companion file alongside it.
pub fn write_broadcast_artifacts(
    sequence: &mut ScriptSequence<Ethereum>,
    deferred: &DeferredOperations,
) -> Result<(), TrebError> {
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

    // Write deferred operations companion file
    if !deferred.safe_proposals.is_empty() || !deferred.governor_proposals.is_empty() {
        if let Some((ref broadcast_path, _)) = sequence.paths {
            let deferred_file = deferred_path_from(broadcast_path);

            // Ensure directory exists
            if let Some(parent) = deferred_file.parent() {
                fs::create_dir_all(parent).map_err(|e| {
                    TrebError::Forge(format!(
                        "failed to create deferred directory {}: {e}",
                        parent.display()
                    ))
                })?;
            }

            let file = fs::File::create(&deferred_file).map_err(|e| {
                TrebError::Forge(format!(
                    "failed to create deferred file {}: {e}",
                    deferred_file.display()
                ))
            })?;
            let mut writer = BufWriter::new(file);
            serde_json::to_writer_pretty(&mut writer, deferred).map_err(|e| {
                TrebError::Forge(format!("failed to serialize deferred operations: {e}"))
            })?;
            writer
                .flush()
                .map_err(|e| TrebError::Forge(format!("failed to flush deferred file: {e}")))?;

            // Timestamped copy of deferred file
            let ts_deferred = broadcast_path.with_file_name(format!(
                "{}.deferred.json",
                broadcast_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("run")
                    .replace("-latest", &format!("-{}", deferred.timestamp))
            ));
            let _ = fs::copy(&deferred_file, &ts_deferred);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Deferred file back-patching (compose merge)
// ---------------------------------------------------------------------------

/// Update a Safe proposal in an existing deferred file with a new hash and nonce.
///
/// Used by compose's merge flow: after adjacent Safe proposals are merged,
/// each component's `run-latest.deferred.json` must reference the merged
/// hash/nonce instead of the per-component values.
pub fn update_deferred_safe_proposal(
    broadcast_path: &Path,
    old_hash: &str,
    new_hash: &str,
    new_nonce: u64,
) -> Result<(), TrebError> {
    let deferred_file = deferred_path_from(broadcast_path);
    if !deferred_file.exists() {
        return Ok(());
    }

    let contents = fs::read_to_string(&deferred_file).map_err(|e| {
        TrebError::Forge(format!("failed to read deferred file {}: {e}", deferred_file.display()))
    })?;

    let mut deferred: DeferredOperations = serde_json::from_str(&contents).map_err(|e| {
        TrebError::Forge(format!("failed to parse deferred file {}: {e}", deferred_file.display()))
    })?;

    for proposal in &mut deferred.safe_proposals {
        if proposal.safe_tx_hash == old_hash {
            proposal.safe_tx_hash = new_hash.to_string();
            proposal.nonce = new_nonce;
        }
    }

    let file = fs::File::create(&deferred_file).map_err(|e| {
        TrebError::Forge(format!("failed to write deferred file {}: {e}", deferred_file.display()))
    })?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, &deferred)
        .map_err(|e| TrebError::Forge(format!("failed to serialize deferred operations: {e}")))?;
    writer.flush().map_err(|e| TrebError::Forge(format!("failed to flush deferred file: {e}")))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Load resume state
// ---------------------------------------------------------------------------

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

    if !broadcast_path.exists() {
        return None;
    }

    // Load ScriptSequence
    let sequence: ScriptSequence<Ethereum> = {
        let contents = fs::read_to_string(&broadcast_path).ok()?;
        serde_json::from_str(&contents).ok()?
    };

    // Load deferred operations (optional)
    let deferred_file = deferred_path_from(&broadcast_path);
    let deferred: Option<DeferredOperations> = if deferred_file.exists() {
        fs::read_to_string(&deferred_file).ok().and_then(|c| serde_json::from_str(&c).ok())
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

    let completed_safe_hashes: std::collections::HashSet<String> = deferred
        .as_ref()
        .map(|d| d.safe_proposals.iter().map(|p| p.safe_tx_hash.clone()).collect())
        .unwrap_or_default();

    let completed_gov_ids: std::collections::HashSet<String> = deferred
        .as_ref()
        .map(|d| d.governor_proposals.iter().map(|p| p.proposal_id.clone()).collect())
        .unwrap_or_default();

    Some(ResumeState {
        sequence,
        deferred,
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
// Session state persistence
// ---------------------------------------------------------------------------

use super::types::SessionState;

const SESSION_STATE_FILE: &str = "session-state.json";

/// Load the session state file from `.treb/session-state.json`.
///
/// Returns `None` if the file does not exist.
pub fn load_session_state(treb_dir: &Path) -> Option<SessionState> {
    let path = treb_dir.join(SESSION_STATE_FILE);
    if !path.exists() {
        return None;
    }
    let contents = fs::read_to_string(&path).ok()?;
    serde_json::from_str(&contents).ok()
}

/// Save the session state file to `.treb/session-state.json`.
pub fn save_session_state(treb_dir: &Path, state: &SessionState) -> Result<(), TrebError> {
    let path = treb_dir.join(SESSION_STATE_FILE);
    let contents = serde_json::to_string_pretty(state)
        .map_err(|e| TrebError::Forge(format!("failed to serialize session state: {e}")))?;
    fs::write(&path, contents).map_err(|e| {
        TrebError::Forge(format!("failed to write session state file {}: {e}", path.display()))
    })
}

/// Delete the session state file if it exists.
pub fn delete_session_state(treb_dir: &Path) {
    let path = treb_dir.join(SESSION_STATE_FILE);
    if path.exists() {
        let _ = fs::remove_file(&path);
    }
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
    fn deferred_path_from_broadcast() {
        let bp = PathBuf::from("/project/broadcast/Deploy.s.sol/42220/run-latest.json");
        let dp = deferred_path_from(&bp);
        assert_eq!(
            dp,
            PathBuf::from("/project/broadcast/Deploy.s.sol/42220/run-latest.deferred.json")
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
    fn deferred_operations_empty_roundtrip() {
        let deferred = DeferredOperations {
            timestamp: 1710600000000,
            chain: 42220,
            commit: Some("abc1234".into()),
            safe_proposals: Vec::new(),
            governor_proposals: Vec::new(),
        };
        let json = serde_json::to_string_pretty(&deferred).unwrap();
        let parsed: DeferredOperations = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.chain, 42220);
        assert_eq!(parsed.commit, Some("abc1234".into()));
    }

    #[test]
    fn deferred_operations_with_safe_roundtrip() {
        let deferred = DeferredOperations {
            timestamp: 1710600000000,
            chain: 1,
            commit: None,
            safe_proposals: vec![DeferredSafeProposal {
                safe_tx_hash: "0xabc".into(),
                safe_address: "0x123".into(),
                nonce: 5,
                chain_id: 1,
                sender_role: "deployer".into(),
                transaction_ids: vec!["tx-1".into(), "tx-2".into()],
                status: "proposed".into(),
                execution_tx_hash: None,
            }],
            governor_proposals: Vec::new(),
        };
        let json = serde_json::to_string_pretty(&deferred).unwrap();
        let parsed: DeferredOperations = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.safe_proposals.len(), 1);
        assert_eq!(parsed.safe_proposals[0].nonce, 5);
        assert_eq!(parsed.safe_proposals[0].transaction_ids.len(), 2);
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
        BroadcastableTransaction { rpc: None, transaction: TransactionMaybeSigned::new(tx_req) }
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
            let tx_maybe_signed: foundry_common::TransactionMaybeSigned<Ethereum> =
                foundry_common::TransactionMaybeSigned::new(tx_request);
            let mut tx_meta = TransactionWithMetadata::from_tx_request(tx_maybe_signed);
            tx_meta.hash = *hash;
            tx_meta.rpc = "http://localhost:8545".into();
            transactions.push_back(tx_meta);
        }

        let seq: ScriptSequence<Ethereum> = ScriptSequence {
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
    fn update_deferred_safe_proposal_patches_hash_and_nonce() {
        let dir = tempfile::tempdir().unwrap();
        let broadcast_path = dir.path().join("run-latest.json");
        let deferred_path = dir.path().join("run-latest.deferred.json");

        // Write a deferred file with one safe proposal
        let deferred = DeferredOperations {
            timestamp: 1234567890,
            chain: 1,
            commit: None,
            safe_proposals: vec![DeferredSafeProposal {
                safe_tx_hash: "0xaaaa".into(),
                safe_address: "0x1111".into(),
                nonce: 5,
                chain_id: 1,
                sender_role: "deployer".into(),
                transaction_ids: vec!["tx-1".into()],
                status: "proposed".into(),
                execution_tx_hash: None,
            }],
            governor_proposals: Vec::new(),
        };
        let contents = serde_json::to_string_pretty(&deferred).unwrap();
        std::fs::write(&deferred_path, contents).unwrap();

        // Patch it
        update_deferred_safe_proposal(&broadcast_path, "0xaaaa", "0xbbbb", 10).unwrap();

        // Read back and verify
        let updated: DeferredOperations =
            serde_json::from_str(&std::fs::read_to_string(&deferred_path).unwrap()).unwrap();
        assert_eq!(updated.safe_proposals.len(), 1);
        assert_eq!(updated.safe_proposals[0].safe_tx_hash, "0xbbbb");
        assert_eq!(updated.safe_proposals[0].nonce, 10);
        // Unchanged fields
        assert_eq!(updated.safe_proposals[0].safe_address, "0x1111");
        assert_eq!(updated.safe_proposals[0].sender_role, "deployer");
    }

    #[test]
    fn update_deferred_safe_proposal_no_match_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let broadcast_path = dir.path().join("run-latest.json");
        let deferred_path = dir.path().join("run-latest.deferred.json");

        let deferred = DeferredOperations {
            timestamp: 1234567890,
            chain: 1,
            commit: None,
            safe_proposals: vec![DeferredSafeProposal {
                safe_tx_hash: "0xaaaa".into(),
                safe_address: "0x1111".into(),
                nonce: 5,
                chain_id: 1,
                sender_role: "deployer".into(),
                transaction_ids: vec!["tx-1".into()],
                status: "proposed".into(),
                execution_tx_hash: None,
            }],
            governor_proposals: Vec::new(),
        };
        std::fs::write(&deferred_path, serde_json::to_string_pretty(&deferred).unwrap()).unwrap();

        // Try to patch with non-matching hash
        update_deferred_safe_proposal(&broadcast_path, "0xcccc", "0xbbbb", 10).unwrap();

        let updated: DeferredOperations =
            serde_json::from_str(&std::fs::read_to_string(&deferred_path).unwrap()).unwrap();
        // Should be unchanged
        assert_eq!(updated.safe_proposals[0].safe_tx_hash, "0xaaaa");
        assert_eq!(updated.safe_proposals[0].nonce, 5);
    }

    #[test]
    fn update_deferred_safe_proposal_missing_file_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        let broadcast_path = dir.path().join("run-latest.json");
        // No deferred file exists — should return Ok without error
        let result = update_deferred_safe_proposal(&broadcast_path, "0xaaaa", "0xbbbb", 10);
        assert!(result.is_ok());
    }
}
