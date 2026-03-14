//! Pipeline orchestrator — drives the full deployment recording flow.
//!
//! [`RunPipeline`] sequences compilation, script execution, event decoding,
//! deployment extraction, proxy detection, hydration, duplicate detection,
//! and registry recording into a single `execute` call.

use std::{collections::HashMap, path::Path};

use alloy_primitives::{Address, B256, U256};
use alloy_signer::Signer;
use foundry_config::Config;
use foundry_evm::traces::{
    CallKind, CallTraceDecoderBuilder, TraceKind, Traces, decode_trace_arena,
    identifier::TraceIdentifiers, render_trace_arena,
};
use treb_core::{error::TrebError, types::TransactionStatus};
use treb_registry::Registry;
use treb_safe::{
    SafeServiceClient, SafeTx, compute_safe_tx_hash, sign_safe_tx, types::ProposeRequest,
};

use crate::{
    artifacts::ArtifactIndex,
    broadcast::read_latest_broadcast,
    compiler::compile_project,
    events::{
        ExtractedDeployment, GovernorProposalCreated, SafeTransactionQueued, TransactionSimulated,
        abi::SimulatedTransaction,
        decode_events,
        decoder::{ParsedEvent, TrebEvent},
        detect_proxy_relationships, extract_collisions, extract_deployments,
    },
    script::{BroadcastReceipt, ScriptConfig, broadcast_script, execute_script},
};

use super::{
    PipelineContext,
    duplicates::{DuplicateStrategy, resolve_duplicates},
    hydration::{
        hydrate_deployment, hydrate_governor_proposals, hydrate_safe_transactions,
        hydrate_transactions, populate_safe_context,
    },
    types::{PipelineResult, RecordedDeployment, RecordedTransaction},
};

/// Callback invoked between simulation and broadcast.
///
/// Receives the hydrated transactions so the caller can display a preview
/// and prompt for confirmation. Return `true` to proceed with broadcast,
/// `false` to abort (the result will be returned as a dry-run).
pub type BroadcastHook = Box<dyn FnOnce(&[RecordedTransaction]) -> bool + Send>;

/// Orchestrator for the deployment recording pipeline.
///
/// Drives the end-to-end flow: compile → execute → decode → extract →
/// hydrate → deduplicate → record. Supports dry-run mode where the
/// result is fully populated but the registry is left unchanged.
pub struct RunPipeline {
    context: PipelineContext,
    /// Optional pre-built ScriptConfig. When provided, this is used instead
    /// of building one from PipelineConfig. This allows the CLI layer to wire
    /// in all flags (broadcast, sender credentials, legacy, etc.) directly.
    script_config: Option<ScriptConfig>,
    /// Optional hook called after simulation to confirm broadcast.
    broadcast_hook: Option<BroadcastHook>,
}

impl RunPipeline {
    /// Create a new pipeline orchestrator with the given execution context.
    pub fn new(context: PipelineContext) -> Self {
        Self { context, script_config: None, broadcast_hook: None }
    }

    /// Set a pre-built ScriptConfig for this pipeline.
    ///
    /// When set, the pipeline uses this config instead of building one from
    /// `PipelineConfig`. This allows the CLI layer to wire in all flags
    /// (broadcast, sender credentials, legacy, verify, etc.) directly.
    pub fn with_script_config(mut self, config: ScriptConfig) -> Self {
        self.script_config = Some(config);
        self
    }

    /// Set a broadcast confirmation hook.
    ///
    /// When set, the hook is called after simulation with the hydrated
    /// transaction list. If it returns `false`, the pipeline skips
    /// broadcast and recording, returning a dry-run result.
    pub fn with_broadcast_hook(mut self, hook: BroadcastHook) -> Self {
        self.broadcast_hook = Some(hook);
        self
    }

    /// Execute the full deployment recording pipeline.
    ///
    /// # Steps
    ///
    /// 1. Compile the project to build an artifact index
    /// 2. Execute the forge script
    /// 3. Decode raw EVM logs into structured events
    /// 4. Extract deployments, collisions, and proxy relationships
    /// 5. Hydrate forge-domain types into core-domain types
    /// 6. For Safe sender: populate safe_context and propose to Safe Service
    /// 7. For Governor sender: hydrate governor proposals and insert into registry
    /// 8. Run duplicate detection against the registry
    /// 9. Record deployments and transactions (skipped in dry-run mode)
    ///
    /// # Errors
    ///
    /// Returns `TrebError::Forge` if compilation or script execution fails,
    /// or `TrebError::Registry` if registry operations fail.
    pub async fn execute(self, registry: &mut Registry) -> treb_core::Result<PipelineResult> {
        // 1. Compile project for artifact index
        let foundry_config = load_foundry_config(&self.context.project_root)?;
        let compilation = compile_project(&foundry_config)?;
        let artifact_index = ArtifactIndex::from_compile_output(compilation);

        // 2. Build script args and execute (simulation only)
        let script_config = match self.script_config {
            Some(config) => config,
            None => {
                // Fallback: build from PipelineConfig (backward compatibility)
                let mut config = ScriptConfig::new(&self.context.config.script_path);
                config
                    .sig(&self.context.config.script_sig)
                    .args(self.context.config.script_args.clone())
                    .chain_id(self.context.config.chain_id);
                config
            }
        };

        let script_args = script_config.into_script_args()?;

        // Clone args for potential broadcast (before simulation consumes them)
        let wants_broadcast = script_args.broadcast
            && !self.context.config.dry_run
            && !self.context.deployer_sender.as_ref().is_some_and(|s| s.is_safe())
            && !self.context.deployer_sender.as_ref().is_some_and(|s| s.is_governor());
        let broadcast_args = if wants_broadcast { Some(script_args.clone()) } else { None };

        // Force simulation mode for initial execution
        let mut sim_args = script_args;
        sim_args.broadcast = false;
        let execution = execute_script(sim_args).await?;

        // 3. Check for failed execution
        if !execution.success {
            return Err(TrebError::Forge(format!(
                "script execution failed:\n{}",
                execution.logs.join("\n")
            )));
        }

        // 4. Decode events
        let parsed_events = decode_events(&execution.raw_logs);
        let event_count = parsed_events.len();

        // 5. Extract deployments, collisions, and proxy relationships
        let extracted_deployments = extract_deployments(&parsed_events, Some(&artifact_index));
        let collisions = extract_collisions(&parsed_events);
        let proxy_relationships = detect_proxy_relationships(&parsed_events);

        // 6. Hydrate deployments
        let hydrated_deployments = extracted_deployments
            .iter()
            .map(|extracted| {
                let proxy = proxy_relationships.get(&extracted.address);
                hydrate_deployment(extracted, proxy, &self.context)
            })
            .collect::<Vec<_>>();

        // 7. Extract event types for transaction hydration
        let tx_events = extract_transaction_simulated(&parsed_events);
        let safe_tx_events = extract_safe_transaction_queued(&parsed_events);
        let governor_events = extract_governor_proposal_created(&parsed_events);

        // 8. Hydrate transactions
        let mut transactions =
            hydrate_transactions(&tx_events, &hydrated_deployments, &self.context);
        let safe_transactions = hydrate_safe_transactions(&safe_tx_events, &self.context);
        let governor_proposals = hydrate_governor_proposals(&governor_events, &self.context);
        let mut transaction_metadata = build_recorded_transaction_metadata(
            &tx_events,
            &extracted_deployments,
            &execution.traces,
            &self.context.sender_labels,
        );

        // 9. Build combined address labels for trace decoding:
        //    execution labels + senders + current deployments + existing registry entries.
        let mut labeled_addresses = execution.labeled_addresses.clone();
        for (addr, role) in &self.context.sender_labels {
            labeled_addresses.entry(*addr).or_insert_with(|| role.clone());
        }
        for dep in &extracted_deployments {
            let label = if dep.label.is_empty() {
                dep.contract_name.clone()
            } else {
                format!("{}:{}", dep.contract_name, dep.label)
            };
            labeled_addresses.insert(dep.address, label);
        }
        for dep in registry.list_deployments() {
            if let Ok(addr) = dep.address.parse::<Address>() {
                let label = if dep.label.is_empty() {
                    dep.contract_name.clone()
                } else {
                    format!("{}:{}", dep.contract_name, dep.label)
                };
                labeled_addresses.entry(addr).or_insert(label);
            }
        }

        // 10. Render traces and extract per-transaction sub-trees
        let (execution_traces, setup_traces) = render_traces_for_verbosity(
            execution.traces,
            &labeled_addresses,
            &artifact_index,
            self.context.config.verbosity,
            &mut transaction_metadata,
        )
        .await;

        // 11. Safe sender: populate safe_context and propose to Safe Service
        let is_safe_sender = self.context.deployer_sender.as_ref().is_some_and(|s| s.is_safe());
        let is_governor_sender =
            self.context.deployer_sender.as_ref().is_some_and(|s| s.is_governor());

        if is_safe_sender {
            populate_safe_context(&mut transactions, &safe_transactions);
            if !self.context.config.dry_run {
                propose_safe_transactions(&self.context, &safe_tx_events, &tx_events).await?;
            }
        }

        // 12. Build recorded transactions (needed for hook preview AND final result)
        let mut recorded_transactions: Vec<RecordedTransaction> = transactions
            .into_iter()
            .map(|tx| {
                let metadata = transaction_metadata.remove(&tx.id).unwrap_or_default();
                RecordedTransaction {
                    transaction: tx,
                    sender_name: metadata.sender_name,
                    gas_used: metadata.gas_used,
                    trace: metadata.trace,
                }
            })
            .collect();

        // 13. Broadcast hook: let the CLI confirm before broadcasting
        let broadcast_confirmed = if let Some(broadcast_args) = broadcast_args {
            if recorded_transactions.is_empty() {
                false
            } else {
                let confirmed = self
                    .broadcast_hook
                    .map_or(true, |hook| hook(&recorded_transactions));

                if confirmed {
                    // Silence foundry shell during re-execution so the user
                    // does not see a second "[⠒] Compiling..." line.
                    *foundry_common::Shell::get() = foundry_common::Shell::empty();
                    // Skip forge's own "Do you wish to continue?" prompt —
                    // the user already confirmed through our hook.
                    let mut broadcast_args = broadcast_args;
                    broadcast_args.non_interactive = true;
                    let receipts = broadcast_script(broadcast_args).await?;
                    apply_broadcast_receipts(&mut recorded_transactions, &receipts);
                    true
                } else {
                    false
                }
            }
        } else {
            false
        };

        // 14. Duplicate detection
        let resolved = resolve_duplicates(hydrated_deployments, registry, DuplicateStrategy::Skip)?;
        let skipped = resolved.skipped;

        // Effective dry_run: explicit dry-run OR broadcast was wanted but declined
        let effective_dry_run =
            self.context.config.dry_run || (wants_broadcast && !broadcast_confirmed);

        let registry_updated = !effective_dry_run
            && (!resolved.to_insert.is_empty()
                || !resolved.to_update.is_empty()
                || !recorded_transactions.is_empty()
                || !safe_transactions.is_empty()
                || (is_governor_sender && !governor_proposals.is_empty()));

        // 15. Record to registry (or build dry-run result)
        let mut recorded_deployments = Vec::new();

        if !effective_dry_run {
            for dep in resolved.to_insert {
                registry.insert_deployment(dep.clone())?;
                recorded_deployments
                    .push(RecordedDeployment { deployment: dep, safe_transaction: None });
            }
            for dep in resolved.to_update {
                registry.update_deployment(dep.clone())?;
                recorded_deployments
                    .push(RecordedDeployment { deployment: dep, safe_transaction: None });
            }
            for rt in &recorded_transactions {
                registry.insert_transaction(rt.transaction.clone())?;
            }
            for safe_tx in safe_transactions {
                registry.insert_safe_transaction(safe_tx)?;
            }
            if is_governor_sender {
                for proposal in &governor_proposals {
                    registry.insert_governor_proposal(proposal.clone())?;
                }
            }
        } else {
            for dep in resolved.to_insert.into_iter().chain(resolved.to_update) {
                recorded_deployments
                    .push(RecordedDeployment { deployment: dep, safe_transaction: None });
            }
        }

        Ok(PipelineResult {
            deployments: recorded_deployments,
            transactions: recorded_transactions,
            registry_updated,
            collisions,
            skipped,
            dry_run: effective_dry_run,
            success: true,
            gas_used: execution.gas_used,
            event_count,
            console_logs: execution.logs,
            governor_proposals,
            execution_traces,
            setup_traces,
        })
    }
}

/// Render traces into human-readable strings and extract per-transaction sub-trees.
///
/// Execution traces are always rendered.
/// Setup traces are rendered when `verbosity >= 3`.
/// Per-transaction traces are extracted for every matched transaction.
async fn render_traces_for_verbosity(
    mut traces: Traces,
    labeled_addresses: &HashMap<Address, String>,
    artifact_index: &ArtifactIndex,
    verbosity: u8,
    transaction_metadata: &mut HashMap<String, RecordedTransactionMetadata>,
) -> (Option<String>, Option<String>) {
    let contracts = artifact_index.inner();

    let mut decoder = CallTraceDecoderBuilder::new()
        .with_labels(labeled_addresses.clone())
        .with_known_contracts(contracts)
        .build();

    // Identify deployed contracts by bytecode matching
    let mut identifier = TraceIdentifiers::new().with_local(contracts);
    for (_, arena) in &traces {
        decoder.identify(&arena.arena, &mut identifier);
    }

    let mut execution_parts = Vec::new();
    let mut setup_parts = Vec::new();

    for (kind, arena) in &mut traces {
        decode_trace_arena(&mut arena.arena, &decoder).await;

        // Extract per-transaction sub-trees from execution arenas
        if matches!(kind, TraceKind::Execution) {
            extract_per_transaction_traces(&arena.arena, transaction_metadata);
        }

        let rendered = render_trace_arena(arena);
        match kind {
            TraceKind::Execution => execution_parts.push(rendered),
            TraceKind::Setup if verbosity >= 3 => setup_parts.push(rendered),
            _ => {}
        }
    }

    let execution_traces =
        (!execution_parts.is_empty()).then(|| execution_parts.join("\n"));
    let setup_traces =
        (!setup_parts.is_empty()).then(|| setup_parts.join("\n"));

    (execution_traces, setup_traces)
}

/// Load the foundry configuration from the project root.
fn load_foundry_config(project_root: &Path) -> treb_core::Result<Config> {
    Config::load_with_root(project_root)
        .map(|c| c.sanitized())
        .map_err(|e| TrebError::Forge(format!("failed to load foundry config: {e}")))
}

/// Extract `TransactionSimulated` events from parsed event list.
fn extract_transaction_simulated(events: &[ParsedEvent]) -> Vec<TransactionSimulated> {
    events
        .iter()
        .filter_map(|e| match e {
            ParsedEvent::Treb(treb) => match treb.as_ref() {
                TrebEvent::TransactionSimulated(ts) => Some(ts.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// Extract `SafeTransactionQueued` events from parsed event list.
fn extract_safe_transaction_queued(events: &[ParsedEvent]) -> Vec<SafeTransactionQueued> {
    events
        .iter()
        .filter_map(|e| match e {
            ParsedEvent::Treb(treb) => match treb.as_ref() {
                TrebEvent::SafeTransactionQueued(stq) => Some(stq.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// Extract `GovernorProposalCreated` events from parsed event list.
fn extract_governor_proposal_created(events: &[ParsedEvent]) -> Vec<GovernorProposalCreated> {
    events
        .iter()
        .filter_map(|e| match e {
            ParsedEvent::Treb(treb) => match treb.as_ref() {
                TrebEvent::GovernorProposalCreated(gpc) => Some(gpc.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

#[derive(Clone, Debug, Default)]
struct RecordedTransactionMetadata {
    sender_name: Option<String>,
    gas_used: Option<u64>,
    /// Index of the matched node in the execution trace arena.
    trace_node_idx: Option<usize>,
    /// Pre-rendered per-transaction trace sub-tree.
    trace: Option<String>,
}

#[derive(Clone, Debug)]
struct PendingExecutionTrace {
    to: Address,
    kind: CallKind,
    data: Vec<u8>,
    value: U256,
    gas_used: Option<u64>,
    matched: bool,
    /// Index of this node in the arena.
    node_idx: usize,
}

fn build_recorded_transaction_metadata(
    tx_events: &[TransactionSimulated],
    extracted_deployments: &[ExtractedDeployment],
    traces: &Traces,
    sender_labels: &HashMap<Address, String>,
) -> HashMap<String, RecordedTransactionMetadata> {
    let transaction_deployments = build_transaction_deployment_index(extracted_deployments);
    let mut pending_traces = traces
        .iter()
        .filter(|(kind, _)| matches!(kind, TraceKind::Execution))
        .flat_map(|(_, arena)| arena.nodes().iter())
        .map(|node| PendingExecutionTrace {
            to: node.trace.address,
            kind: node.trace.kind,
            data: node.trace.data.to_vec(),
            value: node.trace.value,
            gas_used: Some(node.trace.gas_used).filter(|gas| *gas > 0),
            matched: false,
            node_idx: node.idx,
        })
        .collect::<Vec<_>>();

    collect_recorded_transaction_metadata(tx_events, &transaction_deployments, &mut pending_traces, sender_labels)
}

fn build_transaction_deployment_index(
    extracted_deployments: &[ExtractedDeployment],
) -> HashMap<String, Vec<Address>> {
    let mut deployments = HashMap::new();
    for deployment in extracted_deployments {
        deployments
            .entry(format!("tx-{:#x}", deployment.transaction_id))
            .or_insert_with(Vec::new)
            .push(deployment.address);
    }
    deployments
}

fn collect_recorded_transaction_metadata(
    tx_events: &[TransactionSimulated],
    transaction_deployments: &HashMap<String, Vec<Address>>,
    pending_traces: &mut [PendingExecutionTrace],
    sender_labels: &HashMap<Address, String>,
) -> HashMap<String, RecordedTransactionMetadata> {
    tx_events
        .iter()
        .map(|event| &event.simulatedTx)
        .map(|sim_tx| {
            let tx_id = format!("tx-{:#x}", sim_tx.transactionId);
            let deployment_targets = transaction_deployments.get(&tx_id).map(Vec::as_slice);
            let matched = pending_traces
                .iter_mut()
                .find(|candidate| {
                    matches_simulated_transaction(candidate, sim_tx, deployment_targets)
                })
                .map(|candidate| {
                    candidate.matched = true;
                    (candidate.gas_used, candidate.node_idx)
                });

            // Resolve sender name: prefer config role name, fall back to senderId hex
            let sender_name = sender_labels
                .get(&sim_tx.sender)
                .cloned()
                .or_else(|| (!sim_tx.senderId.is_zero()).then(|| format!("{:#x}", sim_tx.senderId)));

            (
                tx_id,
                RecordedTransactionMetadata {
                    sender_name,
                    gas_used: matched.and_then(|(gas, _)| gas),
                    trace_node_idx: matched.map(|(_, idx)| idx),
                    trace: None,
                },
            )
        })
        .collect()
}

fn matches_simulated_transaction(
    candidate: &PendingExecutionTrace,
    sim_tx: &SimulatedTransaction,
    deployment_targets: Option<&[Address]>,
) -> bool {
    // Note: candidate.from is NOT compared to sim_tx.sender because forge
    // script traces show the script contract as the EVM caller, while
    // sim_tx.sender reflects the intended broadcast sender set via
    // vm.startBroadcast().
    if candidate.matched || candidate.value != sim_tx.transaction.value {
        return false;
    }

    if sim_tx.transaction.to == Address::ZERO {
        if !candidate.kind.is_any_create() {
            return false;
        }

        return deployment_targets.is_some_and(|targets| targets.contains(&candidate.to))
            || (!sim_tx.transaction.data.is_empty()
                && candidate.data == sim_tx.transaction.data.as_ref());
    }

    !candidate.kind.is_any_create()
        && candidate.to == sim_tx.transaction.to
        && candidate.data == sim_tx.transaction.data.as_ref()
}

/// Extract per-transaction trace sub-trees from a decoded execution arena.
///
/// For each transaction in `metadata` that has a `trace_node_idx`, renders
/// just that node and its children by cloning the arena and swapping the
/// target node into position 0 so the renderer treats it as the root.
fn extract_per_transaction_traces(
    arena: &foundry_evm::traces::CallTraceArena,
    metadata: &mut HashMap<String, RecordedTransactionMetadata>,
) {
    use foundry_evm::traces::SparsedTraceArena;

    let nodes = arena.nodes();
    if nodes.is_empty() {
        return;
    }

    for meta in metadata.values_mut() {
        let Some(target_idx) = meta.trace_node_idx else {
            continue;
        };

        // Clone the arena and swap the target node into position 0 so the
        // renderer treats it as the tree root. All internal references
        // (children, parent) use the original indices which stay valid
        // because the underlying Vec isn't resized — only positions 0 and
        // target_idx are swapped.
        let mut cloned_arena = arena.clone();
        if target_idx != 0 {
            let cloned_nodes = cloned_arena.nodes_mut();
            cloned_nodes.swap(0, target_idx);
            // Fix up the swapped nodes' idx fields
            cloned_nodes[0].idx = 0;
            cloned_nodes[target_idx].idx = target_idx;
        }

        let sparsed = SparsedTraceArena { arena: cloned_arena, ignored: Default::default() };
        let rendered = render_trace_arena(&sparsed);
        if !rendered.trim().is_empty() {
            meta.trace = Some(rendered);
        }
    }
}

// ---------------------------------------------------------------------------
// Broadcast receipt application
// ---------------------------------------------------------------------------

/// Apply broadcast receipts to recorded transactions.
///
/// Uses positional matching: `TransactionSimulated` events and
/// `BroadcastableTransactions` are collected in the same order during
/// script execution, so the i-th receipt corresponds to the i-th transaction.
fn apply_broadcast_receipts(
    transactions: &mut [RecordedTransaction],
    receipts: &[BroadcastReceipt],
) {
    for (rt, receipt) in transactions.iter_mut().zip(receipts.iter()) {
        rt.transaction.hash = format!("{:#x}", receipt.hash);
        rt.transaction.block_number = receipt.block_number;
        rt.transaction.status = if receipt.status {
            TransactionStatus::Executed
        } else {
            TransactionStatus::Failed
        };
        // Override simulated gas estimate with actual on-chain gas
        rt.gas_used = Some(receipt.gas_used);
    }
}

// ---------------------------------------------------------------------------
// Safe proposal helpers
// ---------------------------------------------------------------------------

/// Build an index of simulated transactions keyed by transaction ID.
fn build_sim_tx_index(tx_events: &[TransactionSimulated]) -> HashMap<B256, &SimulatedTransaction> {
    tx_events
        .iter()
        .map(|event| &event.simulatedTx)
        .map(|sim_tx| (sim_tx.transactionId, sim_tx))
        .collect()
}

/// Construct a [`SafeTx`] from a single simulated transaction.
///
/// Sets gas-related fields to zero and operation to Call (0), matching
/// the default Safe transaction parameters.
pub fn build_safe_tx(sim_tx: &SimulatedTransaction, nonce: u64) -> SafeTx {
    SafeTx {
        to: sim_tx.transaction.to,
        value: sim_tx.transaction.value,
        data: sim_tx.transaction.data.to_vec().into(),
        operation: 0, // Call
        safeTxGas: U256::ZERO,
        baseGas: U256::ZERO,
        gasPrice: U256::ZERO,
        gasToken: Address::ZERO,
        refundReceiver: Address::ZERO,
        nonce: U256::from(nonce),
    }
}

/// Construct a [`ProposeRequest`] from a Safe transaction's components.
pub fn build_propose_request(
    safe_tx: &SafeTx,
    safe_tx_hash: B256,
    sender_address: Address,
    signature: &[u8],
    nonce: u64,
) -> ProposeRequest {
    let data_hex = if safe_tx.data.is_empty() {
        None
    } else {
        Some(format!("0x{}", alloy_primitives::hex::encode(&safe_tx.data)))
    };

    ProposeRequest {
        to: safe_tx.to.to_checksum(None),
        value: safe_tx.value.to_string(),
        data: data_hex,
        operation: safe_tx.operation,
        safe_tx_gas: "0".to_string(),
        base_gas: "0".to_string(),
        gas_price: "0".to_string(),
        gas_token: Address::ZERO.to_checksum(None),
        refund_receiver: Address::ZERO.to_checksum(None),
        nonce,
        contract_transaction_hash: format!("{:#x}", safe_tx_hash),
        sender: sender_address.to_checksum(None),
        signature: format!("0x{}", alloy_primitives::hex::encode(signature)),
        origin: Some("treb".to_string()),
    }
}

/// Propose Safe transactions to the Safe Transaction Service.
///
/// For each `SafeTransactionQueued` event, looks up the linked simulated
/// transactions, constructs a proposal, signs it with the deployer's
/// sub-signer, and submits to the Safe Transaction Service.
async fn propose_safe_transactions(
    context: &PipelineContext,
    safe_tx_events: &[SafeTransactionQueued],
    tx_events: &[TransactionSimulated],
) -> treb_core::Result<()> {
    let deployer = context
        .deployer_sender
        .as_ref()
        .ok_or_else(|| TrebError::Safe("deployer sender not set".to_string()))?;

    let safe_address = deployer
        .safe_address()
        .ok_or_else(|| TrebError::Safe("deployer is not a Safe sender".to_string()))?;

    let signer = deployer
        .sub_signer()
        .wallet_signer()
        .ok_or_else(|| TrebError::Safe("Safe sender's sub-signer is not a wallet".to_string()))?;

    let chain_id = context.config.chain_id;

    let client = SafeServiceClient::new(chain_id).ok_or_else(|| {
        TrebError::Safe(format!(
            "chain ID {chain_id} is not supported by the Safe Transaction Service"
        ))
    })?;

    // Build index of simulated transactions by ID
    let sim_tx_index = build_sim_tx_index(tx_events);

    // Fetch current Safe nonce
    let safe_addr_str = safe_address.to_checksum(None);
    let nonce = client.get_nonce(&safe_addr_str).await?;

    for (i, event) in safe_tx_events.iter().enumerate() {
        // For single-transaction batches, construct and submit the proposal
        if event.transactionIds.len() == 1 {
            let tx_id = event.transactionIds[0];
            if let Some(sim_tx) = sim_tx_index.get(&tx_id) {
                let effective_nonce = nonce + i as u64;
                let safe_tx = build_safe_tx(sim_tx, effective_nonce);
                let hash = compute_safe_tx_hash(chain_id, safe_address, &safe_tx);
                let sig_bytes = sign_safe_tx(signer, hash).await?;

                let signer_address = signer.address();
                let propose_req = build_propose_request(
                    &safe_tx,
                    hash,
                    signer_address,
                    &sig_bytes,
                    effective_nonce,
                );

                client.propose_transaction(&safe_addr_str, &propose_req).await?;
            }
        }
        // Multi-transaction batches require MultiSend encoding — deferred to future work
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{ExtractedDeployment, abi};
    use alloy_primitives::{Bytes, address, b256, keccak256};
    use treb_core::types::enums::DeploymentMethod;

    fn sample_sim_tx(to: Address, value: U256, data: &[u8], tx_id: B256) -> SimulatedTransaction {
        SimulatedTransaction {
            transactionId: tx_id,
            senderId: keccak256(b"deployer"),
            sender: address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
            returnData: Bytes::new(),
            gasUsed: U256::ZERO,
            transaction: abi::Transaction { to, data: Bytes::from(data.to_vec()), value },
        }
    }

    fn sample_extracted_deployment(tx_id: B256, address: Address) -> ExtractedDeployment {
        ExtractedDeployment {
            address,
            deployer: address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
            transaction_id: tx_id,
            contract_name: "Counter".to_string(),
            label: "Counter".to_string(),
            strategy: DeploymentMethod::Create,
            salt: B256::ZERO,
            bytecode_hash: B256::ZERO,
            init_code_hash: B256::ZERO,
            constructor_args: Bytes::new(),
            entropy: String::new(),
            artifact_match: None,
        }
    }

    // ── build_safe_tx ───────────────────────────────────────────────────

    #[test]
    fn build_safe_tx_maps_fields_correctly() {
        let to = address!("5FbDB2315678afecb367f032d93F642f64180aa3");
        let value = U256::from(1_000_000_000_000_000_000u64); // 1 ETH
        let data = vec![0xaa, 0xbb, 0xcc];
        let tx_id = b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let sim_tx = sample_sim_tx(to, value, &data, tx_id);

        let safe_tx = build_safe_tx(&sim_tx, 42);

        assert_eq!(safe_tx.to, to);
        assert_eq!(safe_tx.value, value);
        assert_eq!(safe_tx.data.as_ref(), &data);
        assert_eq!(safe_tx.operation, 0);
        assert_eq!(safe_tx.safeTxGas, U256::ZERO);
        assert_eq!(safe_tx.baseGas, U256::ZERO);
        assert_eq!(safe_tx.gasPrice, U256::ZERO);
        assert_eq!(safe_tx.gasToken, Address::ZERO);
        assert_eq!(safe_tx.refundReceiver, Address::ZERO);
        assert_eq!(safe_tx.nonce, U256::from(42));
    }

    #[test]
    fn build_safe_tx_with_empty_data() {
        let sim_tx = sample_sim_tx(Address::ZERO, U256::ZERO, &[], B256::ZERO);
        let safe_tx = build_safe_tx(&sim_tx, 0);

        assert!(safe_tx.data.is_empty());
        assert_eq!(safe_tx.to, Address::ZERO);
        assert_eq!(safe_tx.value, U256::ZERO);
        assert_eq!(safe_tx.nonce, U256::ZERO);
    }

    // ── build_propose_request ───────────────────────────────────────────

    #[test]
    fn build_propose_request_correct_fields() {
        let to = address!("5FbDB2315678afecb367f032d93F642f64180aa3");
        let safe_tx = SafeTx {
            to,
            value: U256::from(1000u64),
            data: vec![0xab, 0xcd].into(),
            operation: 0,
            safeTxGas: U256::ZERO,
            baseGas: U256::ZERO,
            gasPrice: U256::ZERO,
            gasToken: Address::ZERO,
            refundReceiver: Address::ZERO,
            nonce: U256::from(7),
        };

        let hash = b256!("1111111111111111111111111111111111111111111111111111111111111111");
        let sender = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
        let signature = vec![0xde, 0xad, 0xbe, 0xef];

        let req = build_propose_request(&safe_tx, hash, sender, &signature, 7);

        assert_eq!(req.to, to.to_checksum(None));
        assert_eq!(req.value, "1000");
        assert_eq!(req.data, Some("0xabcd".to_string()));
        assert_eq!(req.operation, 0);
        assert_eq!(req.safe_tx_gas, "0");
        assert_eq!(req.base_gas, "0");
        assert_eq!(req.gas_price, "0");
        assert_eq!(req.gas_token, Address::ZERO.to_checksum(None));
        assert_eq!(req.refund_receiver, Address::ZERO.to_checksum(None));
        assert_eq!(req.nonce, 7);
        assert_eq!(req.contract_transaction_hash, format!("{:#x}", hash));
        assert_eq!(req.sender, sender.to_checksum(None));
        assert_eq!(req.signature, "0xdeadbeef");
        assert_eq!(req.origin, Some("treb".to_string()));
    }

    #[test]
    fn build_propose_request_empty_data_is_none() {
        let safe_tx = SafeTx {
            to: Address::ZERO,
            value: U256::ZERO,
            data: vec![].into(),
            operation: 0,
            safeTxGas: U256::ZERO,
            baseGas: U256::ZERO,
            gasPrice: U256::ZERO,
            gasToken: Address::ZERO,
            refundReceiver: Address::ZERO,
            nonce: U256::ZERO,
        };

        let req = build_propose_request(&safe_tx, B256::ZERO, Address::ZERO, &[], 0);
        assert!(req.data.is_none(), "empty data should produce None");
    }

    // ── build_sim_tx_index ──────────────────────────────────────────────

    #[test]
    fn build_sim_tx_index_creates_correct_mapping() {
        let tx_id_1 = b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let tx_id_2 = b256!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");

        let events = vec![
            TransactionSimulated {
                simulatedTx: sample_sim_tx(Address::ZERO, U256::ZERO, &[], tx_id_1),
            },
            TransactionSimulated {
                simulatedTx: sample_sim_tx(Address::ZERO, U256::ZERO, &[], tx_id_2),
            },
        ];

        let index = build_sim_tx_index(&events);
        assert_eq!(index.len(), 2);
        assert!(index.contains_key(&tx_id_1));
        assert!(index.contains_key(&tx_id_2));
        assert_eq!(index[&tx_id_1].transactionId, tx_id_1);
        assert_eq!(index[&tx_id_2].transactionId, tx_id_2);
    }

    #[test]
    fn build_sim_tx_index_empty_events() {
        let index = build_sim_tx_index(&[]);
        assert!(index.is_empty());
    }

    // ── extract_governor_proposal_created ──────────────────────────────

    #[test]
    fn extract_governor_proposal_created_filters_correctly() {
        use crate::events::decoder::{ParsedEvent, TrebEvent};

        let governor = address!("0000000000000000000000000000000000000099");
        let proposer = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
        let tx_id = b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");

        let events = vec![
            // A TransactionSimulated event (should be skipped)
            ParsedEvent::Treb(Box::new(TrebEvent::TransactionSimulated(TransactionSimulated {
                simulatedTx: sample_sim_tx(Address::ZERO, U256::ZERO, &[], B256::ZERO),
            }))),
            // A GovernorProposalCreated event (should be extracted)
            ParsedEvent::Treb(Box::new(TrebEvent::GovernorProposalCreated(
                GovernorProposalCreated {
                    proposalId: U256::from(42u64),
                    governor,
                    proposer,
                    transactionIds: vec![tx_id],
                },
            ))),
        ];

        let extracted = extract_governor_proposal_created(&events);
        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].proposalId, U256::from(42u64));
        assert_eq!(extracted[0].governor, governor);
        assert_eq!(extracted[0].proposer, proposer);
        assert_eq!(extracted[0].transactionIds.len(), 1);
        assert_eq!(extracted[0].transactionIds[0], tx_id);
    }

    #[test]
    fn extract_governor_proposal_created_empty() {
        let events: Vec<ParsedEvent> = vec![];
        let extracted = extract_governor_proposal_created(&events);
        assert!(extracted.is_empty());
    }

    #[test]
    fn build_recorded_transaction_metadata_uses_sender_id_and_gas() {
        let tx_id = b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let sender = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
        let to = address!("0000000000000000000000000000000000001000");
        let data = vec![0xde, 0xad, 0xbe, 0xef];

        let governance_id = keccak256(b"governance");
        let events = vec![TransactionSimulated {
            simulatedTx: SimulatedTransaction {
                transactionId: tx_id,
                senderId: governance_id,
                sender,
                returnData: Bytes::new(),
                gasUsed: U256::ZERO,
                transaction: abi::Transaction {
                    to,
                    data: Bytes::from(data.clone()),
                    value: U256::ZERO,
                },
            },
        }];

        let mut pending = vec![PendingExecutionTrace {

            to,
            kind: CallKind::Call,
            data,
            value: U256::ZERO,
            gas_used: Some(123_456),
            matched: false,
            node_idx: 0,
        }];

        let metadata =
            collect_recorded_transaction_metadata(&events, &HashMap::new(), &mut pending, &HashMap::new());
        let tx_meta = metadata
            .get("tx-0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
            .expect("metadata should exist");

        assert_eq!(tx_meta.sender_name.as_deref(), Some(&format!("{:#x}", governance_id)[..]));
        assert_eq!(tx_meta.gas_used, Some(123_456));
        assert_eq!(tx_meta.trace_node_idx, Some(0));
        assert!(pending[0].matched);
    }

    #[test]
    fn build_recorded_transaction_metadata_matches_create_transactions_by_deployment_address() {
        let tx_id_1 = b256!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        let tx_id_2 = b256!("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc");
        let sender = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
        let deployed_1 = address!("5FbDB2315678afecb367f032d93F642f64180aa3");
        let deployed_2 = address!("e7f1725E7734CE288F8367e1Bb143E90bb3F0512");

        let events = vec![
            TransactionSimulated {
                simulatedTx: SimulatedTransaction {
                    transactionId: tx_id_1,
                    senderId: keccak256(b"deployer"),
                    sender,
                    returnData: Bytes::new(),
                    gasUsed: U256::ZERO,
                    transaction: abi::Transaction {
                        to: Address::ZERO,
                        data: Bytes::new(),
                        value: U256::ZERO,
                    },
                },
            },
            TransactionSimulated {
                simulatedTx: SimulatedTransaction {
                    transactionId: tx_id_2,
                    senderId: keccak256(b"deployer"),
                    sender,
                    returnData: Bytes::new(),
                    gasUsed: U256::ZERO,
                    transaction: abi::Transaction {
                        to: Address::ZERO,
                        data: Bytes::new(),
                        value: U256::ZERO,
                    },
                },
            },
        ];

        let deployments = HashMap::from([
            (format!("tx-{:#x}", tx_id_1), vec![deployed_1]),
            (format!("tx-{:#x}", tx_id_2), vec![deployed_2]),
        ]);
        let mut pending = vec![
            PendingExecutionTrace {
    
                to: deployed_2,
                kind: CallKind::Create,
                data: vec![1, 2, 3, 4],
                value: U256::ZERO,
                gas_used: Some(654_321),
                matched: false,
                node_idx: 0,
            },
            PendingExecutionTrace {
    
                to: deployed_1,
                kind: CallKind::Create,
                data: vec![4, 3, 2, 1],
                value: U256::ZERO,
                gas_used: Some(111_222),
                matched: false,
                node_idx: 1,
            },
        ];

        let metadata = collect_recorded_transaction_metadata(&events, &deployments, &mut pending, &HashMap::new());
        let tx_meta_1 = metadata
            .get("tx-0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
            .expect("metadata should exist");
        let tx_meta_2 = metadata
            .get("tx-0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc")
            .expect("metadata should exist");

        let deployer_hex = format!("{:#x}", keccak256(b"deployer"));
        assert_eq!(tx_meta_1.sender_name.as_deref(), Some(deployer_hex.as_str()));
        assert_eq!(tx_meta_1.gas_used, Some(111_222));
        assert_eq!(tx_meta_2.sender_name.as_deref(), Some(deployer_hex.as_str()));
        assert_eq!(tx_meta_2.gas_used, Some(654_321));
        assert!(pending.iter().all(|candidate| candidate.matched));
    }

    #[test]
    fn build_recorded_transaction_metadata_uses_execution_trace_gas() {
        let tx_id = b256!("dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd");
        let deployed = address!("5FbDB2315678afecb367f032d93F642f64180aa3");
        let events = vec![TransactionSimulated {
            simulatedTx: sample_sim_tx(Address::ZERO, U256::ZERO, &[], tx_id),
        }];
        let deployments = vec![sample_extracted_deployment(tx_id, deployed)];
        let mut arena = foundry_evm::traces::CallTraceArena::default();
        arena.nodes_mut()[0] = foundry_evm::traces::CallTraceNode {
            parent: None,
            children: Vec::new(),
            idx: 0,
            trace: foundry_evm::traces::CallTrace {
                depth: 0,
                success: true,
                caller: address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
                address: deployed,
                maybe_precompile: None,
                selfdestruct_address: None,
                selfdestruct_refund_target: None,
                selfdestruct_transferred_value: None,
                kind: CallKind::Create,
                value: U256::ZERO,
                data: Bytes::from(vec![1, 2, 3, 4]),
                output: Bytes::new(),
                gas_used: 987_654,
                gas_limit: 1_000_000,
                status: None,
                steps: Vec::new(),
                decoded: None,
            },
            logs: Vec::new(),
            ordering: Vec::new(),
        };

        let traces = vec![(
            TraceKind::Execution,
            foundry_evm::traces::SparsedTraceArena { arena, ignored: Default::default() },
        )];

        let metadata = build_recorded_transaction_metadata(&events, &deployments, &traces, &HashMap::new());
        let tx_meta = metadata
            .get("tx-0xdddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd")
            .expect("metadata should exist");

        assert_eq!(tx_meta.gas_used, Some(987_654));
    }
}
