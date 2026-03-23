//! Pipeline orchestrator — drives the full deployment recording flow.
//!
//! [`RunPipeline`] sequences compilation, script execution, event decoding,
//! deployment extraction, proxy detection, hydration, duplicate detection,
//! and registry recording into a single `execute` call.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

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
    compiler::compile_project,
    events::{
        ExtractedDeployment, GovernorProposalCreated, SafeTransactionQueued, TransactionSimulated,
        abi::SimulatedTransaction,
        decoder::{ParsedEvent, TrebEvent},
    },
    script::{ExecutionResult, ScriptConfig},
};

use super::{
    PipelineContext,
    duplicates::{DuplicateStrategy, resolve_duplicates},
    types::{PipelineResult, RecordedDeployment, RecordedTransaction},
};

/// Callback that receives hydrated transactions and returns true to broadcast.
pub type BroadcastHook = Box<dyn FnOnce(&[RecordedTransaction]) -> bool + Send>;

/// Callback for pipeline progress updates.
pub type BroadcastProgressCallback = Box<dyn Fn(BroadcastPhase) + Send>;

/// Phases of the pipeline, reported via [`BroadcastProgressCallback`].
#[derive(Debug, Clone, Copy)]
pub enum BroadcastPhase {
    Compiling,
    Executing,
    Simulating,
    Broadcasting,
    Complete,
}

/// Orchestrator for the deployment recording pipeline.
pub struct RunPipeline {
    context: PipelineContext,
    script_config: Option<ScriptConfig>,
    broadcast_hook: Option<BroadcastHook>,
    broadcast_progress: Option<BroadcastProgressCallback>,
    resume_state: Option<super::broadcast_writer::ResumeState>,
}

impl RunPipeline {
    pub fn new(context: PipelineContext) -> Self {
        Self {
            context,
            script_config: None,
            broadcast_hook: None,
            broadcast_progress: None,
            resume_state: None,
        }
    }

    pub fn with_script_config(mut self, config: ScriptConfig) -> Self {
        self.script_config = Some(config);
        self
    }

    /// Set a broadcast confirmation hook. Called with hydrated transactions
    /// after simulation, before broadcast. Return false to skip broadcast.
    pub fn with_broadcast_hook(mut self, hook: BroadcastHook) -> Self {
        self.broadcast_hook = Some(hook);
        self
    }

    /// Set a progress callback for broadcast pipeline phases.
    pub fn with_broadcast_progress(mut self, cb: BroadcastProgressCallback) -> Self {
        self.broadcast_progress = Some(cb);
        self
    }

    /// Set resume state loaded from a previous broadcast file.
    pub fn with_resume_state(mut self, state: super::broadcast_writer::ResumeState) -> Self {
        self.resume_state = Some(state);
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
        let report = |phase: BroadcastPhase| {
            if let Some(ref cb) = self.broadcast_progress {
                cb(phase);
            }
        };

        // Derive deployer sender category from resolved_senders for broadcast gating.
        let deployer_is_safe =
            self.context.resolved_senders.get("deployer").is_some_and(|s| s.is_safe());
        let has_governor_sender = self.context.resolved_senders.values().any(|s| s.is_governor());

        // 1. Compile project for artifact index
        report(BroadcastPhase::Compiling);
        let foundry_config = load_foundry_config(&self.context.project_root)?;
        let compilation = compile_project(&foundry_config)?;
        let artifact_index = ArtifactIndex::from_compile_output(compilation);

        // 2. Build script args and execute
        let script_config = match self.script_config {
            Some(config) => config,
            None => {
                let mut config = ScriptConfig::new(&self.context.config.script_path);
                config
                    .sig(&self.context.config.script_sig)
                    .args(self.context.config.script_args.clone())
                    .chain_id(self.context.config.chain_id);
                config
            }
        };

        let script_args = script_config.into_script_args()?;
        // All sender types can broadcast — routing handles Safe/Governor.
        let wants_broadcast = script_args.broadcast && self.context.config.broadcast;

        // Run forge: preprocess → compile → link → prepare → execute
        report(BroadcastPhase::Executing);
        let preprocessed = script_args
            .preprocess()
            .await
            .map_err(|e| TrebError::Forge(format!("forge preprocessing failed: {e}")))?;
        let compiled = preprocessed
            .compile()
            .map_err(|e| TrebError::Forge(format!("forge compilation failed: {e}")))?;
        let linked = compiled
            .link()
            .await
            .map_err(|e| TrebError::Forge(format!("forge linking failed: {e}")))?;
        let prepared = linked
            .prepare_execution()
            .await
            .map_err(|e| TrebError::Forge(format!("forge execution preparation failed: {e}")))?;
        let executed = prepared
            .execute()
            .await
            .map_err(|e| TrebError::Forge(format!("forge execution failed: {e}")))?;

        // Clone the result — we need it for hydration, and the state machine
        // may consume `executed` if we broadcast.
        let script_result = executed.execution_result.clone();
        let decoded_logs = crate::console::decode_console_logs(&script_result.logs);
        let execution = ExecutionResult {
            success: script_result.success,
            logs: decoded_logs,
            raw_logs: script_result.logs,
            gas_used: script_result.gas_used,
            returned: script_result.returned,
            labeled_addresses: script_result.labeled_addresses.into_iter().collect(),
            transactions: script_result.transactions,
            traces: script_result.traces,
            broadcast_receipts: None,
        };

        // 3. Hydrate simulation results
        let sim = super::simulation::hydrate_simulation(
            execution,
            &artifact_index,
            &self.context,
            registry,
            &super::simulation::HydrationOptions {
                populate_safe_context: deployer_is_safe,
                include_addressbook_labels: true,
            },
        )
        .await?;

        let collisions = sim.collisions;
        let event_count = sim.event_count;
        let governor_proposals = sim.governor_proposals;
        let safe_transactions = sim.safe_transactions;
        let execution_traces = sim.execution_traces;
        let setup_traces = sim.setup_traces;
        let hydrated_deployments: Vec<_> =
            sim.recorded_deployments.into_iter().map(|rd| rd.deployment).collect();
        let mut recorded_transactions = sim.recorded_transactions;

        // 4. Broadcast: hook → confirm → route by sender type
        let mut proposed_results = Vec::new();
        let mut routing_safe_transactions = Vec::new();
        let mut routing_queued_executions = Vec::new();

        let broadcast_confirmed = if wants_broadcast && !recorded_transactions.is_empty() {
            let confirmed = self.broadcast_hook.is_none_or(|hook| hook(&recorded_transactions));

            if confirmed {
                if let Some(btxs) = &sim.broadcastable_transactions {
                    let rpc_url =
                        self.context.config.rpc_url.as_deref().ok_or_else(|| {
                            TrebError::Forge("RPC URL required for broadcast".into())
                        })?;

                    // Build pre-routing sequence and ensure directories exist
                    // before broadcast so checkpoints can be saved incrementally.
                    let (_, bp, cp) = super::broadcast_writer::compute_broadcast_paths(
                        &self.context.project_root,
                        &self.context.config.script_path,
                        self.context.config.chain_id,
                        &self.context.config.script_sig,
                    );
                    let mut pre_sequence = super::broadcast_writer::build_pre_routing_sequence(
                        btxs,
                        &recorded_transactions,
                        &self.context,
                        bp,
                        cp,
                    );
                    super::broadcast_writer::ensure_broadcast_dirs(&pre_sequence)?;

                    report(BroadcastPhase::Broadcasting);
                    let mut ctx = super::routing::RouteContext {
                        rpc_url,
                        chain_id: self.context.config.chain_id,
                        is_fork: self.context.config.is_fork,
                        quiet: self.context.config.quiet,
                        on_run_complete: None,
                        resolved_senders: &self.context.resolved_senders,
                        sender_labels: &self.context.sender_labels,
                        sender_configs: &self.context.sender_configs,
                        sequence: Some(&mut pre_sequence),
                        safe_nonce_offsets: std::collections::HashMap::new(),
                        defer_safe_proposals: false,
                        safe_threshold_cache: std::collections::HashMap::new(),
                    };
                    let run_results = if let Some(ref resume) = self.resume_state {
                        super::routing::route_all_with_resume(btxs, &mut ctx, resume)
                            .await?
                            .into_iter()
                            .map(|(run, result)| (run, result, None))
                            .collect()
                    } else {
                        super::routing::route_all_with_queued(btxs, &mut ctx).await?
                    };

                    let outcome = apply_routing_results_with_queued(
                        &run_results,
                        btxs,
                        &mut recorded_transactions,
                        &self.context,
                        &self.context.config.script_path,
                        self.context.config.chain_id,
                        &self.context.config.script_sig,
                        Some(pre_sequence),
                    )?;
                    proposed_results = outcome.proposed_results;
                    routing_safe_transactions = outcome.safe_transactions;
                    routing_queued_executions = outcome.queued_executions;

                    report(BroadcastPhase::Complete);
                }
                true
            } else {
                false
            }
        } else {
            false
        };

        // Merge routing-produced safe transactions with event-hydrated ones
        let all_safe_transactions: Vec<_> =
            safe_transactions.into_iter().chain(routing_safe_transactions).collect();

        // 13. Duplicate detection
        let resolved = resolve_duplicates(hydrated_deployments, registry, DuplicateStrategy::Skip)?;
        let skipped = resolved.skipped;

        let should_record = broadcast_confirmed;

        let registry_updated = should_record
            && (!resolved.to_insert.is_empty()
                || !resolved.to_update.is_empty()
                || !recorded_transactions.is_empty()
                || !all_safe_transactions.is_empty()
                || (has_governor_sender && !governor_proposals.is_empty()));

        // 14. Record to registry (or build dry-run result)
        let mut recorded_deployments = Vec::new();

        if should_record {
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
            for safe_tx in &all_safe_transactions {
                registry.insert_safe_transaction(safe_tx.clone())?;
            }
            for proposal in &governor_proposals {
                registry.insert_governor_proposal(proposal.clone())?;
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
            dry_run: !should_record,
            success: true,
            gas_used: sim.gas_used,
            event_count,
            console_logs: sim.console_logs,
            governor_proposals,
            safe_transactions: all_safe_transactions,
            proposed_results,
            queued_executions: routing_queued_executions,
            execution_traces,
            setup_traces,
        })
    }
}

// ---------------------------------------------------------------------------
// Routing → registry record builders
// ---------------------------------------------------------------------------

use super::{
    routing::{RunResult, TransactionRun},
    types::ProposedResult,
};
use treb_core::types::safe_transaction::SafeTxData;

/// Build `ProposedResult`, `SafeTransaction`, and `GovernorProposal` records from routing results.
///
/// For each `SafeProposed` or `GovernorProposed` run result, creates the
/// appropriate registry records so they can be persisted.
fn build_proposed_records_from_routing(
    run_results: &[(TransactionRun, RunResult)],
    btxs: &foundry_cheatcodes::BroadcastableTransactions,
    context: &PipelineContext,
    recorded_transactions: &[RecordedTransaction],
) -> (
    Vec<ProposedResult>,
    Vec<treb_core::types::SafeTransaction>,
    Vec<treb_core::types::GovernorProposal>,
) {
    let now = chrono::Utc::now();
    let mut proposed = Vec::new();
    let mut safe_txs = Vec::new();
    let mut gov_proposals = Vec::new();

    for (run, result) in run_results {
        match result {
            RunResult::SafeProposed { safe_tx_hash, safe_address, nonce, tx_count } => {
                // Build ProposedResult for CLI display
                proposed.push(ProposedResult {
                    sender_role: run.sender_role.clone(),
                    run_result: result.clone(),
                    tx_count: *tx_count,
                });

                // Build SafeTransaction record for registry
                let tx_ids: Vec<String> = run
                    .tx_indices
                    .iter()
                    .filter_map(|&idx| recorded_transactions.get(idx))
                    .map(|rt| rt.transaction.id.clone())
                    .collect();

                let safe_tx_data: Vec<SafeTxData> = run
                    .tx_indices
                    .iter()
                    .filter_map(|&idx| recorded_transactions.get(idx))
                    .map(|rt| {
                        let op = rt.transaction.operations.first();
                        SafeTxData {
                            to: op.map(|o| o.target.clone()).unwrap_or_default(),
                            value: "0".into(),
                            data: op.map(|o| o.method.clone()).unwrap_or_default(),
                            operation: 0,
                        }
                    })
                    .collect();

                // Determine the signer address from resolved senders
                let proposed_by = context
                    .resolved_senders
                    .get(&run.sender_role)
                    .and_then(|s| s.sub_signer().wallet_signer())
                    .map(|ws| {
                        use alloy_signer::Signer;
                        ws.address().to_checksum(None)
                    })
                    .unwrap_or_default();

                safe_txs.push(treb_core::types::SafeTransaction {
                    safe_tx_hash: format!("{:#x}", safe_tx_hash),
                    safe_address: safe_address.to_checksum(None),
                    chain_id: context.config.chain_id,
                    status: TransactionStatus::Queued,
                    nonce: *nonce,
                    transactions: safe_tx_data,
                    transaction_ids: tx_ids,
                    proposed_by,
                    proposed_at: now,
                    confirmations: Vec::new(),
                    executed_at: None,
                    fork_executed_at: None,
                    execution_tx_hash: String::new(),
                });
            }
            RunResult::GovernorProposed { proposal_id, governor_address, tx_count } => {
                proposed.push(ProposedResult {
                    sender_role: run.sender_role.clone(),
                    run_result: result.clone(),
                    tx_count: *tx_count,
                });

                // Extract actions from the broadcastable transactions
                let actions: Vec<treb_core::types::GovernorAction> = run
                    .tx_indices
                    .iter()
                    .filter_map(|&idx| btxs.get(idx))
                    .map(|btx| {
                        let to = btx
                            .transaction
                            .to()
                            .and_then(|kind| match kind {
                                alloy_primitives::TxKind::Call(addr) => {
                                    Some(format!("{:#x}", addr))
                                }
                                alloy_primitives::TxKind::Create => None,
                            })
                            .unwrap_or_default();
                        let value = format!("{}", btx.transaction.value().unwrap_or_default());
                        let calldata = btx
                            .transaction
                            .input()
                            .map(|b| format!("0x{}", alloy_primitives::hex::encode(b)))
                            .unwrap_or_default();
                        treb_core::types::GovernorAction { target: to, value, calldata }
                    })
                    .collect();

                let tx_ids: Vec<String> = run
                    .tx_indices
                    .iter()
                    .filter_map(|&idx| recorded_transactions.get(idx))
                    .map(|rt| rt.transaction.id.clone())
                    .collect();

                // Determine timelock from resolved sender
                let timelock_addr = context
                    .resolved_senders
                    .get(&run.sender_role)
                    .and_then(|s| s.timelock_address())
                    .map(|a| a.to_checksum(None))
                    .unwrap_or_default();

                let proposer_addr = context
                    .resolved_senders
                    .get(&run.sender_role)
                    .map(|s| s.sub_signer().sender_address().to_checksum(None))
                    .unwrap_or_default();

                gov_proposals.push(treb_core::types::GovernorProposal {
                    proposal_id: proposal_id.clone(),
                    governor_address: governor_address.to_checksum(None),
                    timelock_address: timelock_addr,
                    chain_id: context.config.chain_id,
                    status: treb_core::types::ProposalStatus::Pending,
                    transaction_ids: tx_ids,
                    proposed_by: proposer_addr,
                    proposed_at: now,
                    description: String::new(),
                    executed_at: None,
                    execution_tx_hash: String::new(),
                    fork_executed_at: None,
                    actions,
                });
            }
            RunResult::Broadcast(_) => {
                // Nothing to record — already handled by apply_receipts
            }
        }
    }

    (proposed, safe_txs, gov_proposals)
}

/// Update transaction statuses based on routing results.
///
/// For proposed runs (Safe/Governor), set the linked transactions to Queued
/// since they haven't been executed on-chain yet.
fn update_transaction_statuses_from_routing(
    recorded_transactions: &mut [RecordedTransaction],
    run_results: &[(TransactionRun, RunResult)],
    _btxs: &foundry_cheatcodes::BroadcastableTransactions,
) {
    for (run, result) in run_results {
        let is_proposed =
            matches!(result, RunResult::SafeProposed { .. } | RunResult::GovernorProposed { .. });
        if is_proposed {
            // Mark all transactions in this run as Queued
            for &tx_idx in &run.tx_indices {
                if let Some(rt) = recorded_transactions.get_mut(tx_idx) {
                    rt.transaction.status = TransactionStatus::Queued;
                }
            }
        }
    }
}

/// Outcome of applying routing results to recorded transactions.
///
/// Returned by [`apply_routing_results()`] so the caller can persist
/// proposed records and safe transactions to the registry.
pub struct RoutingOutcome {
    /// Proposed (non-broadcast) results for CLI display.
    pub proposed_results: Vec<ProposedResult>,
    /// Safe transactions produced by routing.
    pub safe_transactions: Vec<treb_core::types::SafeTransaction>,
    /// Governor proposals produced by routing.
    pub governor_proposals: Vec<treb_core::types::GovernorProposal>,
    /// Queued executions from routing (for inline processing by the CLI).
    pub queued_executions: Vec<super::routing::QueuedExecution>,
}

/// Apply routing results to recorded transactions: build proposed records,
/// update statuses, apply receipts, and write broadcast artifacts.
///
/// This is the shared post-routing path used by both the orchestrator and
/// compose's Phase 4 broadcast loop.
///
/// When `pre_built_sequence` is `Some`, skips `build_script_sequence()` and
/// uses the pre-built (and already checkpointed) sequence for the final
/// broadcast artifact write.
#[allow(clippy::too_many_arguments)]
pub fn apply_routing_results(
    run_results: &[(TransactionRun, RunResult)],
    btxs: &foundry_cheatcodes::BroadcastableTransactions,
    recorded_transactions: &mut [RecordedTransaction],
    context: &PipelineContext,
    script_path: &str,
    chain_id: u64,
    script_sig: &str,
    pre_built_sequence: Option<forge_script_sequence::ScriptSequence>,
) -> Result<RoutingOutcome, TrebError> {
    // Build Safe/Governor records from routing results
    let (proposed_results, safe_transactions, governor_proposals) =
        build_proposed_records_from_routing(run_results, btxs, context, recorded_transactions);

    // Apply receipts from broadcast results first, then override statuses
    // for proposed runs. Order matters: apply_receipts sets status based on
    // receipt.status (Executed/Failed), but proposed runs use placeholder
    // receipts. update_transaction_statuses_from_routing must run second
    // so Queued overrides the placeholder Executed for proposed transactions.
    let receipts = super::routing::flatten_receipts(run_results);
    super::hydration::apply_receipts(recorded_transactions, &receipts);

    // Override status to Queued for proposed (Safe/Governor) runs
    update_transaction_statuses_from_routing(recorded_transactions, run_results, btxs);

    // Write Foundry-compatible broadcast files.
    // When a pre-built sequence is provided (from incremental checkpointing),
    // reuse it instead of rebuilding from routing results.
    let (_, bp, cp) = super::broadcast_writer::compute_broadcast_paths(
        &context.project_root,
        script_path,
        chain_id,
        script_sig,
    );
    let mut sequence = if let Some(seq) = pre_built_sequence {
        seq
    } else {
        super::broadcast_writer::build_script_sequence(
            btxs,
            run_results,
            recorded_transactions,
            context,
            bp.clone(),
            cp,
        )
    };
    let deferred = super::broadcast_writer::build_deferred_operations(
        run_results,
        recorded_transactions,
        context,
    );
    super::broadcast_writer::write_broadcast_artifacts(&mut sequence, &deferred)?;

    // Set broadcastFile on recorded transactions
    let rel_path = super::broadcast_writer::relative_broadcast_path(&context.project_root, &bp);
    for rt in recorded_transactions {
        rt.transaction.broadcast_file = Some(rel_path.clone());
    }

    Ok(RoutingOutcome {
        proposed_results,
        safe_transactions,
        governor_proposals,
        queued_executions: Vec::new(),
    })
}

/// Apply routing results with queued execution items.
///
/// Same as [`apply_routing_results`] but accepts the full
/// `(TransactionRun, RunResult, Option<QueuedExecution>)` triples from
/// `route_all_with_queued` and passes the queued items through.
#[allow(clippy::too_many_arguments)]
pub fn apply_routing_results_with_queued(
    run_results_with_queued: &[(
        TransactionRun,
        RunResult,
        Option<super::routing::QueuedExecution>,
    )],
    btxs: &foundry_cheatcodes::BroadcastableTransactions,
    recorded_transactions: &mut [RecordedTransaction],
    context: &PipelineContext,
    script_path: &str,
    chain_id: u64,
    script_sig: &str,
    pre_built_sequence: Option<forge_script_sequence::ScriptSequence>,
) -> Result<RoutingOutcome, TrebError> {
    // Extract (run, result) pairs for the existing functions
    let run_results: Vec<(TransactionRun, RunResult)> = run_results_with_queued
        .iter()
        .map(|(run, result, _)| {
            (
                TransactionRun {
                    sender_role: run.sender_role.clone(),
                    category: run.category,
                    sender_address: run.sender_address,
                    tx_indices: run.tx_indices.clone(),
                },
                result.clone(),
            )
        })
        .collect();

    // Collect queued items
    let queued_executions: Vec<super::routing::QueuedExecution> =
        run_results_with_queued.iter().filter_map(|(_, _, q)| q.clone()).collect();

    let mut outcome = apply_routing_results(
        &run_results,
        btxs,
        recorded_transactions,
        context,
        script_path,
        chain_id,
        script_sig,
        pre_built_sequence,
    )?;
    outcome.queued_executions = queued_executions;
    Ok(outcome)
}

/// Render traces into human-readable strings and extract per-transaction sub-trees.
///
/// Execution traces are always rendered.
/// Setup traces are rendered when `verbosity >= 3`.
/// Per-transaction traces are extracted for every matched transaction.
pub(super) async fn render_traces_for_verbosity(
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

        // Replace long hex bytecode blobs in decoded call args with artifact
        // names before rendering — this way both full traces and per-tx
        // sub-trees get the collapsed form automatically.
        collapse_decoded_bytecode_args(&mut arena.arena, artifact_index);

        // Extract per-transaction sub-trees from execution arenas
        if matches!(kind, TraceKind::Execution) {
            extract_per_transaction_traces(&arena.arena, transaction_metadata);
        }

        let rendered = strip_internal_events(&render_trace_arena(arena));
        match kind {
            TraceKind::Execution => execution_parts.push(rendered),
            TraceKind::Setup if verbosity >= 3 => setup_parts.push(rendered),
            _ => {}
        }
    }

    let execution_traces = (!execution_parts.is_empty()).then(|| execution_parts.join("\n"));
    let setup_traces = (!setup_parts.is_empty()).then(|| setup_parts.join("\n"));

    (execution_traces, setup_traces)
}

/// Strip internal treb event lines (e.g. `emit TransactionSimulated`,
/// `emit DeploymentRecorded`) from rendered trace output.
pub(super) fn strip_internal_events(rendered: &str) -> String {
    const INTERNAL_EVENTS: &[&str] = &[
        "TransactionSimulated",
        "DeploymentRecorded",
        "ContractDeployed",
        "SafeTransactionQueued",
        "GovernorProposalCreated",
        "CollisionDetected",
        "ProxyRelationship",
    ];
    rendered
        .lines()
        .filter(|line| !INTERNAL_EVENTS.iter().any(|ev| line.contains(&format!("emit {ev}"))))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Load the foundry configuration from the project root.
pub(super) fn load_foundry_config(project_root: &Path) -> treb_core::Result<Config> {
    Config::load_with_root(project_root)
        .map(|c| c.sanitized())
        .map_err(|e| TrebError::Forge(format!("failed to load foundry config: {e}")))
}

/// Extract `TransactionSimulated` events from parsed event list.
pub(super) fn extract_transaction_simulated(events: &[ParsedEvent]) -> Vec<TransactionSimulated> {
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
pub(super) fn extract_safe_transaction_queued(
    events: &[ParsedEvent],
) -> Vec<SafeTransactionQueued> {
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
pub(super) fn extract_governor_proposal_created(
    events: &[ParsedEvent],
) -> Vec<GovernorProposalCreated> {
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
pub(super) struct RecordedTransactionMetadata {
    pub(super) sender_name: Option<String>,
    pub(super) gas_used: Option<u64>,
    /// Index of the matched node in the execution trace arena.
    pub(super) trace_node_idx: Option<usize>,
    /// Pre-rendered per-transaction trace sub-tree.
    pub(super) trace: Option<String>,
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

pub(super) fn build_recorded_transaction_metadata(
    tx_events: &[TransactionSimulated],
    extracted_deployments: &[ExtractedDeployment],
    traces: &Traces,
    sender_id_labels: &HashMap<B256, String>,
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

    collect_recorded_transaction_metadata(
        tx_events,
        &transaction_deployments,
        &mut pending_traces,
        sender_id_labels,
    )
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
    sender_id_labels: &HashMap<B256, String>,
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

            // Resolve sender name by senderId (keccak of role name) for
            // accurate identification when multiple senders share an address.
            let sender_name = sender_id_labels.get(&sim_tx.senderId).cloned().or_else(|| {
                (!sim_tx.senderId.is_zero()).then(|| format!("{:#x}", sim_tx.senderId))
            });

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

/// Build per-transaction metadata from BroadcastableTransactions (v2).
///
/// Same logic as `build_recorded_transaction_metadata` but driven by
/// `BroadcastableTransactions` instead of `TransactionSimulated` events.
pub(super) fn build_v2_recorded_transaction_metadata(
    btxs: &foundry_cheatcodes::BroadcastableTransactions,
    extracted_deployments: &[ExtractedDeployment],
    traces: &Traces,
    context: &PipelineContext,
) -> HashMap<String, RecordedTransactionMetadata> {
    let transaction_deployments = build_transaction_deployment_index(extracted_deployments);
    let mut pending_traces: Vec<PendingExecutionTrace> = traces
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
        .collect();

    let addr_to_role: HashMap<Address, String> =
        context.sender_labels.iter().map(|(addr, role)| (*addr, role.clone())).collect();

    btxs.iter()
        .enumerate()
        .map(|(idx, btx)| {
            let tx_id = super::hydration::broadcast_tx_id(&context.config.script_path, idx);
            let from_addr = btx.transaction.from().unwrap_or_default();
            let to_kind = btx.transaction.to();
            let input = btx.transaction.input().cloned().unwrap_or_default();
            let value = btx.transaction.value().unwrap_or_default();

            let deployment_targets = transaction_deployments.get(&tx_id).map(Vec::as_slice);

            let matched = pending_traces
                .iter_mut()
                .find(|candidate| {
                    if candidate.matched || candidate.value != value {
                        return false;
                    }
                    match to_kind {
                        Some(alloy_primitives::TxKind::Call(to_addr)) => {
                            !candidate.kind.is_any_create()
                                && candidate.to == to_addr
                                && candidate.data == input.as_ref()
                        }
                        Some(alloy_primitives::TxKind::Create) | None => {
                            if !candidate.kind.is_any_create() {
                                return false;
                            }
                            deployment_targets
                                .is_some_and(|targets| targets.contains(&candidate.to))
                                || (!input.is_empty() && candidate.data == input.as_ref())
                        }
                    }
                })
                .map(|candidate| {
                    candidate.matched = true;
                    (candidate.gas_used, candidate.node_idx)
                });

            let sender_name = addr_to_role.get(&from_addr).cloned();

            (
                tx_id,
                RecordedTransactionMetadata {
                    sender_name,
                    gas_used: matched.and_then(|(gas, _)| gas),
                    trace_node_idx: matched.map(|(_, idx)| idx),
                    trace: None, // filled by extract_per_transaction_traces later
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
        let rendered = strip_internal_events(&render_trace_arena(&sparsed));
        if !rendered.trim().is_empty() {
            meta.trace = Some(rendered);
        }
    }
}

// ---------------------------------------------------------------------------
// Bytecode collapse in decoded trace data
// ---------------------------------------------------------------------------

/// Minimum byte count before we collapse a hex argument.
const BYTECODE_COLLAPSE_THRESHOLD: usize = 64;

/// Walk the decoded trace arena and replace long hex bytecode arguments
/// with compact artifact-matched summaries.
///
/// Modifies `DecodedCallData.args` in-place so that both full-trace
/// rendering and per-transaction sub-tree extraction see the collapsed form.
pub(super) fn collapse_decoded_bytecode_args(
    arena: &mut foundry_evm::traces::CallTraceArena,
    artifact_index: &ArtifactIndex,
) {
    use alloy_primitives::hex;

    for node in arena.nodes_mut() {
        let Some(ref mut decoded) = node.trace.decoded else { continue };

        // For unrecognized calls, foundry leaves call_data = None and the
        // renderer falls back to showing raw trace.data. Try to match
        // the raw data as creation code and inject a DecodedCallData so
        // the renderer shows a human-readable form instead.
        if decoded.call_data.is_none() && node.trace.data.len() >= BYTECODE_COLLAPSE_THRESHOLD {
            if let Some(replacement) = try_collapse_raw_data(&node.trace.data, artifact_index) {
                decoded.call_data = Some(replacement);
            }
        }

        // Check for raw hex "selector" calls — when the signature has no
        // parentheses it means foundry couldn't decode the call and is
        // showing the first 4 bytes as a fake selector.
        if let Some(ref mut call_data) = decoded.call_data {
            if !call_data.signature.contains('(') && !call_data.args.is_empty() {
                if let Some(replacement) = try_collapse_raw_create(call_data, artifact_index) {
                    *call_data = replacement;
                    continue;
                }
            }

            // Normal case: collapse any long hex args individually
            for arg in &mut call_data.args {
                collapse_hex_arg(arg, artifact_index);
            }
        }

        // Collapse long hex in return data
        if let Some(ref mut ret) = decoded.return_data {
            collapse_hex_arg(ret, artifact_index);
        }
    }

    /// Try to match raw trace data (from an unrecognized call where
    /// `decoded.call_data` is None) against the artifact index as creation code.
    fn try_collapse_raw_data(
        data: &[u8],
        artifact_index: &ArtifactIndex,
    ) -> Option<revm_inspectors::tracing::types::DecodedCallData> {
        build_create_call_data(data, artifact_index)
    }

    /// Try to reconstruct a raw hex call (where the "selector" is bytecode)
    /// into a `create(new ContractName(...))` form by matching the full
    /// calldata against the artifact index as creation code.
    fn try_collapse_raw_create(
        call_data: &revm_inspectors::tracing::types::DecodedCallData,
        artifact_index: &ArtifactIndex,
    ) -> Option<revm_inspectors::tracing::types::DecodedCallData> {
        let sig_hex = &call_data.signature;
        let arg_hex = call_data.args.first()?;
        let arg_hex_clean = arg_hex.strip_prefix("0x").unwrap_or(arg_hex);

        if !sig_hex.bytes().all(|b| b.is_ascii_hexdigit())
            || !arg_hex_clean.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return None;
        }

        let full_hex = format!("{sig_hex}{arg_hex_clean}");
        if full_hex.len() < BYTECODE_COLLAPSE_THRESHOLD * 2 {
            return None;
        }

        let bytes = hex::decode(&full_hex).ok()?;
        build_create_call_data(&bytes, artifact_index)
    }

    /// Build a `create(new ContractName(arg1, arg2, ...))` decoded call.
    fn build_create_call_data(
        code: &[u8],
        artifact_index: &ArtifactIndex,
    ) -> Option<revm_inspectors::tracing::types::DecodedCallData> {
        let (matched, ctor_args) = artifact_index.decode_creation_code(code)?;

        let inner = if ctor_args.is_empty() {
            format!("new {}()", matched.name)
        } else {
            format!("new {}({})", matched.name, ctor_args.join(", "))
        };

        Some(revm_inspectors::tracing::types::DecodedCallData {
            signature: "create".to_string(),
            args: vec![inner],
        })
    }

    /// Replace a single string in-place if it contains a long hex blob.
    fn collapse_hex_arg(s: &mut String, artifact_index: &ArtifactIndex) {
        let hex_str = s.strip_prefix("0x").unwrap_or(s.as_str());
        if hex_str.len() < BYTECODE_COLLAPSE_THRESHOLD * 2 {
            return;
        }
        if !hex_str.bytes().all(|b| b.is_ascii_hexdigit()) {
            return;
        }

        let byte_count = hex_str.len() / 2;
        if let Ok(bytes) = hex::decode(hex_str) {
            if let Some((matched, ctor_args)) = artifact_index.decode_creation_code(&bytes) {
                if ctor_args.is_empty() {
                    *s = format!("new {}() ({byte_count} bytes)", matched.name);
                } else {
                    *s = format!(
                        "new {}({}) ({byte_count} bytes)",
                        matched.name,
                        ctor_args.join(", ")
                    );
                }
                return;
            }
        }

        // No match — truncate
        let prefix = &hex_str[..8.min(hex_str.len())];
        let suffix_start = hex_str.len().saturating_sub(8);
        let suffix = &hex_str[suffix_start..];
        *s = format!("0x{prefix}…{suffix} ({byte_count} bytes)");
    }
}

// ---------------------------------------------------------------------------
// Broadcast receipt application
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Safe proposal helpers (will be wired for Rust-side Safe proposals)
// ---------------------------------------------------------------------------

/// Build an index of simulated transactions keyed by transaction ID.
#[allow(dead_code)]
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
#[allow(dead_code)]
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
#[allow(dead_code)]
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
#[allow(dead_code)]
async fn propose_safe_transactions(
    context: &PipelineContext,
    safe_tx_events: &[SafeTransactionQueued],
    tx_events: &[TransactionSimulated],
) -> treb_core::Result<()> {
    let deployer = context
        .resolved_senders
        .get("deployer")
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

// ===========================================================================
// SessionPipeline — unified orchestrator for run (1 script) and compose (N)
// ===========================================================================

use super::types::{
    ScriptEntry, ScriptPhase, ScriptProgress, ScriptResult, SessionPhase, SessionProgressCallback,
    SessionState,
};

/// Unified pipeline that handles both single-script (`treb run`) and
/// multi-script (`treb compose`) flows.
///
/// Call [`simulate_all()`] to compile + execute all scripts, then inspect
/// the returned [`SimulatedSession`] before optionally calling
/// [`broadcast_all()`] to route transactions and write to the registry.
/// Callback fired after each routing action completes during broadcast.
pub type OnActionComplete =
    Box<dyn Fn(&super::routing::TransactionRun, &super::routing::RunResult) + Send + Sync>;

pub struct SessionPipeline {
    scripts: Vec<ScriptEntry>,
    progress: Option<SessionProgressCallback>,
    resume: bool,
    on_action_complete: Option<OnActionComplete>,
}

impl Default for SessionPipeline {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionPipeline {
    pub fn new() -> Self {
        Self { scripts: Vec::new(), progress: None, resume: false, on_action_complete: None }
    }

    /// Add a script to the execution queue.
    pub fn add_script(&mut self, entry: ScriptEntry) {
        self.scripts.push(entry);
    }

    /// Set a progress callback for pipeline phases.
    pub fn with_progress(mut self, cb: SessionProgressCallback) -> Self {
        self.progress = Some(cb);
        self
    }

    /// Enable resume mode: skip already-completed scripts/phases.
    pub fn with_resume(mut self, resume: bool) -> Self {
        self.resume = resume;
        self
    }

    /// Set a callback fired after each routing action completes during broadcast.
    ///
    /// The callback receives the transaction run and its result. Use this to
    /// print per-action status lines inline (clearing/restarting spinners, etc.)
    pub fn with_on_action_complete(mut self, cb: OnActionComplete) -> Self {
        self.on_action_complete = Some(cb);
        self
    }

    /// Simulate all scripts: compile once, execute each, hydrate results.
    ///
    /// For multi-script sessions, spawns an ephemeral Anvil fork so state
    /// flows between scripts, then restores the registry to its pre-session
    /// state. The caller inspects results and decides whether to broadcast.
    pub async fn simulate_all(
        mut self,
        registry: &mut Registry,
    ) -> Result<SimulatedSession, (Vec<ScriptResult>, String, TrebError)> {
        let report = |phase: &SessionPhase| {
            if let Some(ref cb) = self.progress {
                cb(phase.clone());
            }
        };

        let is_multi = self.scripts.len() > 1;

        // ------------------------------------------------------------------
        // Phase 1: Compile (once, shared across all scripts)
        // ------------------------------------------------------------------
        report(&SessionPhase::Compiling);
        let project_root =
            self.scripts.first().map(|e| e.context.project_root.clone()).unwrap_or_default();
        let foundry_config =
            load_foundry_config(&project_root).map_err(|e| (Vec::new(), String::new(), e))?;
        let compilation =
            compile_project(&foundry_config).map_err(|e| (Vec::new(), String::new(), e))?;
        let artifact_index = ArtifactIndex::from_compile_output(compilation);

        // ------------------------------------------------------------------
        // Multi-script: snapshot registry + spawn ephemeral Anvil
        // ------------------------------------------------------------------
        let treb_dir = project_root.join(".treb");
        let snapshot_dir = treb_dir.join("priv/snapshots/session");
        if is_multi {
            let _ = treb_registry::snapshot_registry(&treb_dir, &snapshot_dir);
        }

        let ephemeral_anvil = if is_multi {
            report(&SessionPhase::SpawningAnvil);
            let upstream_url =
                self.scripts.first().and_then(|e| e.config.rpc_url_ref().map(|s| s.to_string()));

            if let Some(ref url) = upstream_url {
                let resolved = if url.starts_with("http://") || url.starts_with("https://") {
                    url.clone()
                } else {
                    treb_config::resolve_rpc_endpoints(&project_root)
                        .ok()
                        .and_then(|eps| eps.get(url.as_str()).cloned())
                        .filter(|ep| !ep.unresolved && !ep.expanded_url.trim().is_empty())
                        .map(|ep| ep.expanded_url)
                        .unwrap_or_else(|| url.clone())
                };
                let anvil = crate::anvil::AnvilConfig::new()
                    .fork_url(&resolved)
                    .spawn()
                    .await
                    .map_err(|e| (Vec::new(), String::new(), e))?;
                Some(anvil)
            } else {
                None
            }
        } else {
            None
        };

        let ephemeral_url = ephemeral_anvil.as_ref().map(|a| a.rpc_url().to_string());

        // Override ScriptConfig RPC URLs for multi-script to point at ephemeral Anvil.
        // PipelineContext.config.rpc_url retains the original URL for broadcast.
        let _rpc_override_guard = if let Some(ref url) = ephemeral_url {
            for entry in &mut self.scripts {
                entry.config.rpc_url(url);
            }
            self.scripts
                .first()
                .and_then(|e| e.config.rpc_url_ref().map(|s| s.to_string()))
                .as_deref()
                .filter(|u| !u.starts_with("http://") && !u.starts_with("https://"))
                .and_then(|network| unsafe {
                    treb_config::override_rpc_endpoint(&project_root, network, url)
                })
        } else {
            None
        };

        // ------------------------------------------------------------------
        // Load session state for resume
        // ------------------------------------------------------------------
        let session_state = if self.resume {
            super::broadcast_writer::load_session_state(&treb_dir)
        } else {
            super::broadcast_writer::delete_session_state(&treb_dir);
            None
        };

        let script_phases: HashMap<String, ScriptPhase> = session_state
            .as_ref()
            .map(|ss| ss.scripts.iter().map(|sp| (sp.name.clone(), sp.phase.clone())).collect())
            .unwrap_or_default();

        // ------------------------------------------------------------------
        // Phase 2: Simulate (per-script loop)
        // ------------------------------------------------------------------
        let mut simulated: Vec<SimulatedScript> = Vec::new();
        let mut skipped_results: Vec<ScriptResult> = Vec::new();

        let config_hash =
            session_state.as_ref().map(|ss| ss.config_hash.clone()).unwrap_or_default();
        let mut state_scripts: Vec<ScriptProgress> = Vec::new();

        for entry in self.scripts {
            let ScriptEntry { name, context, config } = entry;
            let phase = script_phases.get(&name);

            // Resume: skip scripts that already broadcast
            if phase == Some(&ScriptPhase::Broadcast) {
                skipped_results.push(ScriptResult {
                    name: name.clone(),
                    result: PipelineResult {
                        deployments: Vec::new(),
                        transactions: Vec::new(),
                        registry_updated: false,
                        collisions: Vec::new(),
                        skipped: Vec::new(),
                        dry_run: false,
                        success: true,
                        gas_used: 0,
                        event_count: 0,
                        console_logs: Vec::new(),
                        governor_proposals: Vec::new(),
                        safe_transactions: Vec::new(),
                        proposed_results: Vec::new(),
                        queued_executions: Vec::new(),
                        execution_traces: None,
                        setup_traces: None,
                    },
                    broadcastable_transactions: None,
                });
                state_scripts.push(ScriptProgress {
                    name,
                    script_path: context.config.script_path.clone(),
                    chain_id: context.config.chain_id,
                    sig: context.config.script_sig.clone(),
                    phase: ScriptPhase::Broadcast,
                    deployments: 0,
                    transactions: 0,
                });
                continue;
            }

            report(&SessionPhase::Simulating(name.clone()));

            let deployer_is_safe =
                context.resolved_senders.get("deployer").is_some_and(|s| s.is_safe());

            let script_args = match config.into_script_args() {
                Ok(args) => args,
                Err(e) => {
                    if is_multi {
                        let _ = treb_registry::restore_registry(&snapshot_dir, &treb_dir);
                        let _ = std::fs::remove_dir_all(&snapshot_dir);
                    }
                    return Err((skipped_results, name, e));
                }
            };

            let executed = match async {
                let preprocessed = script_args
                    .preprocess()
                    .await
                    .map_err(|e| TrebError::Forge(format!("forge preprocessing failed: {e}")))?;
                let compiled = preprocessed
                    .compile()
                    .map_err(|e| TrebError::Forge(format!("forge compilation failed: {e}")))?;
                let linked = compiled
                    .link()
                    .await
                    .map_err(|e| TrebError::Forge(format!("forge linking failed: {e}")))?;
                let prepared = linked.prepare_execution().await.map_err(|e| {
                    TrebError::Forge(format!("forge execution preparation failed: {e}"))
                })?;
                prepared
                    .execute()
                    .await
                    .map_err(|e| TrebError::Forge(format!("forge execution failed: {e}")))
            }
            .await
            {
                Ok(exec) => exec,
                Err(e) => {
                    if is_multi {
                        let _ = treb_registry::restore_registry(&snapshot_dir, &treb_dir);
                        let _ = std::fs::remove_dir_all(&snapshot_dir);
                    }
                    return Err((skipped_results, name, e));
                }
            };

            let script_result = executed.execution_result.clone();
            let decoded_logs = crate::console::decode_console_logs(&script_result.logs);
            let execution = ExecutionResult {
                success: script_result.success,
                logs: decoded_logs,
                raw_logs: script_result.logs,
                gas_used: script_result.gas_used,
                returned: script_result.returned,
                labeled_addresses: script_result.labeled_addresses.into_iter().collect(),
                transactions: script_result.transactions,
                traces: script_result.traces,
                broadcast_receipts: None,
            };

            let sim = match super::simulation::hydrate_simulation(
                execution,
                &artifact_index,
                &context,
                registry,
                &super::simulation::HydrationOptions {
                    populate_safe_context: deployer_is_safe && !is_multi,
                    include_addressbook_labels: !is_multi,
                },
            )
            .await
            {
                Ok(sim) => sim,
                Err(e) => {
                    if is_multi {
                        let _ = treb_registry::restore_registry(&snapshot_dir, &treb_dir);
                        let _ = std::fs::remove_dir_all(&snapshot_dir);
                    }
                    return Err((skipped_results, name, e));
                }
            };

            let btxs = sim.broadcastable_transactions.clone();

            // Multi-script: replay on ephemeral Anvil + write intermediate deployments
            if is_multi {
                if let Some(ref b) = btxs {
                    if let Some(ref url) = ephemeral_url {
                        if let Err(e) = super::compose::replay_transactions_on_fork(url, b).await {
                            let _ = treb_registry::restore_registry(&snapshot_dir, &treb_dir);
                            let _ = std::fs::remove_dir_all(&snapshot_dir);
                            return Err((skipped_results, name, e));
                        }
                    }
                }
                for rd in &sim.recorded_deployments {
                    let _ = registry.insert_deployment(rd.deployment.clone());
                }
                for collision in &sim.collisions {
                    let dep = super::hydration::hydrate_collision(collision, &context);
                    let _ = registry.insert_deployment(dep);
                }
            }

            let pipeline_result = PipelineResult {
                deployments: sim.recorded_deployments,
                transactions: sim.recorded_transactions,
                registry_updated: false,
                collisions: sim.collisions,
                skipped: Vec::new(),
                dry_run: true,
                success: true,
                gas_used: sim.gas_used,
                event_count: sim.event_count,
                console_logs: sim.console_logs,
                governor_proposals: sim.governor_proposals,
                safe_transactions: sim.safe_transactions,
                proposed_results: Vec::new(),
                queued_executions: Vec::new(),
                execution_traces: sim.execution_traces,
                setup_traces: sim.setup_traces,
            };

            state_scripts.push(ScriptProgress {
                name: name.clone(),
                script_path: context.config.script_path.clone(),
                chain_id: context.config.chain_id,
                sig: context.config.script_sig.clone(),
                phase: ScriptPhase::Simulated,
                deployments: pipeline_result.deployments.len(),
                transactions: pipeline_result.transactions.len(),
            });

            let _ = super::broadcast_writer::save_session_state(
                &treb_dir,
                &SessionState { config_hash: config_hash.clone(), scripts: state_scripts.clone() },
            );

            simulated.push(SimulatedScript { name, result: pipeline_result, btxs, context });
        }

        // Multi-script: restore registry to pre-session state
        if is_multi {
            let _ = treb_registry::restore_registry(&snapshot_dir, &treb_dir);
            let _ = std::fs::remove_dir_all(&snapshot_dir);
        }

        report(&SessionPhase::SimulationComplete);

        Ok(SimulatedSession {
            scripts: simulated,
            skipped_results,
            resume: self.resume,
            progress: self.progress,
            on_action_complete: self.on_action_complete,
            treb_dir,
            config_hash,
            state_scripts,
        })
    }
}

/// Intermediate state between simulation and broadcast.
///
/// Returned by [`SessionPipeline::simulate_all()`]. Inspect results with
/// [`results()`], then either [`broadcast_all()`] or [`into_results()`].
pub struct SimulatedSession {
    scripts: Vec<SimulatedScript>,
    skipped_results: Vec<ScriptResult>,
    resume: bool,
    progress: Option<SessionProgressCallback>,
    on_action_complete: Option<OnActionComplete>,
    treb_dir: PathBuf,
    config_hash: String,
    state_scripts: Vec<ScriptProgress>,
}

struct SimulatedScript {
    name: String,
    result: PipelineResult,
    btxs: Option<foundry_cheatcodes::BroadcastableTransactions>,
    context: PipelineContext,
}

impl SimulatedSession {
    /// Read-only access to simulation results for display/confirmation.
    pub fn results(&self) -> impl Iterator<Item = (&str, &PipelineResult)> {
        self.scripts.iter().map(|s| (s.name.as_str(), &s.result))
    }

    /// Set a callback fired after each routing action completes during broadcast.
    pub fn set_on_action_complete(&mut self, cb: OnActionComplete) {
        self.on_action_complete = Some(cb);
    }

    /// Convert to final results without broadcasting (dry-run / user cancelled).
    pub fn into_results(self) -> Vec<ScriptResult> {
        let mut out = self.skipped_results;
        for s in self.scripts {
            out.push(ScriptResult {
                name: s.name,
                result: s.result,
                broadcastable_transactions: s.btxs,
            });
        }
        out
    }

    /// Broadcast all scripts: route transactions, write registry, produce
    /// broadcast files.
    ///
    /// On failure returns partial results, the failed script name, and the error.
    pub async fn broadcast_all(
        mut self,
        registry: &mut Registry,
    ) -> Result<Vec<ScriptResult>, (Vec<ScriptResult>, String, TrebError)> {
        let report = |phase: &SessionPhase| {
            if let Some(ref cb) = self.progress {
                cb(phase.clone());
            }
        };

        let mut results: Vec<ScriptResult> = self.skipped_results;
        // Persists across components so sequential Safe proposals get incrementing nonces
        let mut safe_nonce_offsets: std::collections::HashMap<alloy_primitives::Address, u64> =
            std::collections::HashMap::new();
        let mut safe_threshold_cache: std::collections::HashMap<alloy_primitives::Address, u64> =
            std::collections::HashMap::new();

        // Merge mode: defer Safe proposals across components and combine
        // adjacent proposals targeting the same Safe address.
        let should_merge = !self.resume && self.scripts.len() > 1;
        let mut pending_proposals: Vec<PendingSafeProposal> = Vec::new();
        let mut merge_sender_info: Option<MergeSenderInfo> = None;

        for sim_script in self.scripts {
            let SimulatedScript { name, mut result, btxs, context } = sim_script;

            let wants_broadcast = context.config.broadcast && !result.transactions.is_empty();

            if wants_broadcast {
                if let Some(ref btxs) = btxs {
                    let rpc_url = match context.config.rpc_url.as_deref() {
                        Some(url) => url,
                        None => {
                            let e = TrebError::Forge("RPC URL required for broadcast".into());
                            results.push(ScriptResult {
                                name: name.clone(),
                                result,
                                broadcastable_transactions: Some(btxs.clone()),
                            });
                            if let Some(sp) =
                                self.state_scripts.iter_mut().find(|sp| sp.name == name)
                            {
                                sp.phase = ScriptPhase::Failed { phase: "broadcast".into() };
                            }
                            let _ = super::broadcast_writer::save_session_state(
                                &self.treb_dir,
                                &SessionState {
                                    config_hash: self.config_hash.clone(),
                                    scripts: self.state_scripts.clone(),
                                },
                            );
                            return Err((results, name, e));
                        }
                    };

                    report(&SessionPhase::Broadcasting(name.clone()));
                    // Capture sender info for Phase C merge submission
                    if should_merge && merge_sender_info.is_none() {
                        let mut sender_signing = HashMap::new();
                        for (role, sender) in &context.resolved_senders {
                            if sender.is_safe() {
                                let key = crate::sender::extract_signing_key(
                                    role,
                                    sender,
                                    &context.sender_configs,
                                )
                                .map(|s| s.to_string());
                                let proposed_by = sender
                                    .sub_signer()
                                    .wallet_signer()
                                    .map(|ws| Signer::address(ws).to_checksum(None))
                                    .unwrap_or_default();
                                if let Some(key) = key {
                                    sender_signing.insert(role.clone(), (key, proposed_by));
                                }
                            }
                        }
                        merge_sender_info = Some(MergeSenderInfo {
                            sender_signing,
                            is_fork: context.config.is_fork,
                            rpc_url: context.config.rpc_url.clone(),
                        });
                    }

                    let mut route_ctx = super::routing::RouteContext {
                        rpc_url,
                        chain_id: context.config.chain_id,
                        is_fork: context.config.is_fork,
                        quiet: context.config.quiet,
                        on_run_complete: self
                            .on_action_complete
                            .as_ref()
                            .map(|cb| &**cb as &super::routing::OnRunComplete),
                        resolved_senders: &context.resolved_senders,
                        sender_labels: &context.sender_labels,
                        sender_configs: &context.sender_configs,
                        sequence: None,
                        safe_nonce_offsets: safe_nonce_offsets.clone(),
                        defer_safe_proposals: should_merge,
                        safe_threshold_cache: safe_threshold_cache.clone(),
                    };

                    let run_results = if self.resume {
                        let resume_state = super::broadcast_writer::load_resume_state(
                            &context.project_root,
                            &context.config.script_path,
                            context.config.chain_id,
                            &context.config.script_sig,
                            rpc_url,
                        )
                        .await;
                        if let Some(ref rs) = resume_state {
                            // Resume uses the old path (no queued items)
                            super::routing::route_all_with_resume(btxs, &mut route_ctx, rs)
                                .await
                                .map(|r| {
                                    r.into_iter().map(|(run, result)| (run, result, None)).collect()
                                })
                        } else {
                            super::routing::route_all_with_queued(btxs, &mut route_ctx).await
                        }
                    } else if should_merge {
                        route_with_deferred_proposals(
                            btxs,
                            &mut route_ctx,
                            &result.transactions,
                            &mut pending_proposals,
                            &name,
                            &context.project_root,
                            &context.config.script_path,
                            context.config.chain_id,
                            &context.config.script_sig,
                        )
                        .await
                    } else {
                        super::routing::route_all_with_queued(btxs, &mut route_ctx).await
                    };

                    // Carry nonce offsets and threshold cache forward for the next component
                    safe_nonce_offsets = route_ctx.safe_nonce_offsets;
                    safe_threshold_cache = route_ctx.safe_threshold_cache;

                    match run_results {
                        Ok(run_results) => {
                            let outcome = apply_routing_results_with_queued(
                                &run_results,
                                btxs,
                                &mut result.transactions,
                                &context,
                                &context.config.script_path,
                                context.config.chain_id,
                                &context.config.script_sig,
                                None,
                            );

                            match outcome {
                                Ok(outcome) => {
                                    result.proposed_results = outcome.proposed_results;
                                    result.queued_executions = outcome.queued_executions;
                                    let routing_safe_txs = outcome.safe_transactions;
                                    let routing_gov_proposals = outcome.governor_proposals;
                                    let all_safe_txs: Vec<_> = result
                                        .safe_transactions
                                        .drain(..)
                                        .chain(routing_safe_txs)
                                        .collect();
                                    result.safe_transactions = all_safe_txs;
                                    result.governor_proposals.extend(routing_gov_proposals);
                                }
                                Err(e) => {
                                    eprintln!("warning: apply_routing_results failed: {e}");
                                }
                            }

                            // Duplicate detection
                            let hydrated_deployments: Vec<_> =
                                result.deployments.drain(..).map(|rd| rd.deployment).collect();

                            let resolved = match resolve_duplicates(
                                hydrated_deployments,
                                registry,
                                DuplicateStrategy::Skip,
                            ) {
                                Ok(r) => r,
                                Err(e) => {
                                    return Err((results, name, e));
                                }
                            };
                            result.skipped = resolved.skipped;

                            // Registry writes
                            let mut recorded_deployments = Vec::new();
                            for dep in resolved.to_insert {
                                let _ = registry.insert_deployment(dep.clone());
                                recorded_deployments.push(RecordedDeployment {
                                    deployment: dep,
                                    safe_transaction: None,
                                });
                            }
                            for dep in resolved.to_update {
                                let _ = registry.update_deployment(dep.clone());
                                recorded_deployments.push(RecordedDeployment {
                                    deployment: dep,
                                    safe_transaction: None,
                                });
                            }
                            for rt in &result.transactions {
                                let _ = registry.insert_transaction(rt.transaction.clone());
                            }
                            // When merging, safe_tx registry writes are deferred to Phase C
                            if !should_merge {
                                for safe_tx in &result.safe_transactions {
                                    let _ = registry.insert_safe_transaction(safe_tx.clone());
                                }
                            }
                            for proposal in &result.governor_proposals {
                                let _ = registry.insert_governor_proposal(proposal.clone());
                            }

                            result.deployments = recorded_deployments;
                            result.registry_updated = true;
                            result.dry_run = false;
                        }
                        Err(e) => {
                            results.push(ScriptResult {
                                name: name.clone(),
                                result,
                                broadcastable_transactions: Some(btxs.clone()),
                            });
                            if let Some(sp) =
                                self.state_scripts.iter_mut().find(|sp| sp.name == name)
                            {
                                sp.phase = ScriptPhase::Failed { phase: "broadcast".into() };
                            }
                            let _ = super::broadcast_writer::save_session_state(
                                &self.treb_dir,
                                &SessionState {
                                    config_hash: self.config_hash.clone(),
                                    scripts: self.state_scripts.clone(),
                                },
                            );
                            return Err((results, name, e));
                        }
                    }
                }
            }

            // Mark script as broadcast in state
            if let Some(sp) = self.state_scripts.iter_mut().find(|sp| sp.name == name) {
                sp.phase = ScriptPhase::Broadcast;
            }
            let _ = super::broadcast_writer::save_session_state(
                &self.treb_dir,
                &SessionState {
                    config_hash: self.config_hash.clone(),
                    scripts: self.state_scripts.clone(),
                },
            );

            results.push(ScriptResult { name, result, broadcastable_transactions: btxs });
        }

        // Phase B+C: Merge adjacent Safe proposals and submit
        if !pending_proposals.is_empty() {
            let merged = merge_adjacent_safe_proposals(pending_proposals);
            if let Some(ref sender_info) = merge_sender_info {
                match submit_merged_proposals(&merged, registry, sender_info).await {
                    Ok(submitted) => {
                        // Display merged results via the on_action_complete callback
                        for sub in &submitted {
                            if let Some(ref cb) = self.on_action_complete {
                                let run = TransactionRun {
                                    sender_role: sub.sender_role.clone(),
                                    category: crate::sender::SenderCategory::Safe,
                                    sender_address: Address::ZERO,
                                    tx_indices: (0..sub.tx_count).collect(),
                                };
                                let run_result = RunResult::SafeProposed {
                                    safe_tx_hash: sub.safe_tx_hash,
                                    safe_address: Address::ZERO,
                                    nonce: sub.nonce,
                                    tx_count: sub.tx_count,
                                };
                                cb(&run, &run_result);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("warning: failed to submit merged Safe proposals: {e}");
                    }
                }
            }
        }

        report(&SessionPhase::BroadcastComplete);
        super::broadcast_writer::delete_session_state(&self.treb_dir);

        Ok(results)
    }
}

// ---------------------------------------------------------------------------
// Compose Safe proposal merging
// ---------------------------------------------------------------------------

/// Collected from per-component routing, deferred for merge.
struct PendingSafeProposal {
    safe_address: Address,
    chain_id: u64,
    operations: Vec<treb_safe::MultiSendOperation>,
    transaction_ids: Vec<String>,
    safe_tx_data: Vec<SafeTxData>,
    sender_role: String,
    component_name: String,
    original_safe_tx_hash: B256,
    broadcast_path: PathBuf,
}

/// Result of merging adjacent PendingSafeProposals.
struct MergedSafeProposal {
    safe_address: Address,
    chain_id: u64,
    operations: Vec<treb_safe::MultiSendOperation>,
    transaction_ids: Vec<String>,
    safe_tx_data: Vec<SafeTxData>,
    sender_role: String,
    component_names: Vec<String>,
    original_safe_tx_hashes: Vec<B256>,
    broadcast_paths: Vec<PathBuf>,
}

/// Sender info captured from the first component for Phase C signing.
///
/// We extract the signing key and proposed_by address at capture time
/// because `ResolvedSender` does not implement Clone.
struct MergeSenderInfo {
    /// sender_role → (private_key_hex, proposed_by_checksum)
    sender_signing: HashMap<String, (String, String)>,
    is_fork: bool,
    rpc_url: Option<String>,
}

/// Merge adjacent Safe proposals that target the same Safe address.
///
/// Walks `pending` linearly. If the current proposal targets the same
/// Safe address as the previous merged entry, the operations, tx IDs,
/// and metadata are combined. Non-adjacent proposals to the same Safe
/// (e.g. A=Safe1, B=Safe2, C=Safe1) do NOT merge — only consecutive
/// same-address proposals combine.
fn merge_adjacent_safe_proposals(pending: Vec<PendingSafeProposal>) -> Vec<MergedSafeProposal> {
    let mut merged: Vec<MergedSafeProposal> = Vec::new();

    for p in pending {
        let should_extend = merged.last().is_some_and(|last| last.safe_address == p.safe_address);

        if should_extend {
            let last = merged.last_mut().unwrap();
            last.operations.extend(p.operations);
            last.transaction_ids.extend(p.transaction_ids);
            last.safe_tx_data.extend(p.safe_tx_data);
            last.component_names.push(p.component_name);
            last.original_safe_tx_hashes.push(p.original_safe_tx_hash);
            last.broadcast_paths.push(p.broadcast_path);
        } else {
            merged.push(MergedSafeProposal {
                safe_address: p.safe_address,
                chain_id: p.chain_id,
                operations: p.operations,
                transaction_ids: p.transaction_ids,
                safe_tx_data: p.safe_tx_data,
                sender_role: p.sender_role,
                component_names: vec![p.component_name],
                original_safe_tx_hashes: vec![p.original_safe_tx_hash],
                broadcast_paths: vec![p.broadcast_path],
            });
        }
    }

    merged
}

/// Route a component's transactions with deferred Safe proposals.
///
/// Uses `reduce_queue` + selective execution: Exec actions are executed
/// normally, while Propose actions are collected into `pending` without
/// submitting to the Safe Transaction Service.
#[allow(clippy::too_many_arguments)]
async fn route_with_deferred_proposals(
    btxs: &foundry_cheatcodes::BroadcastableTransactions,
    route_ctx: &mut super::routing::RouteContext<'_>,
    recorded_txs: &[RecordedTransaction],
    pending: &mut Vec<PendingSafeProposal>,
    component_name: &str,
    project_root: &Path,
    script_path: &str,
    chain_id: u64,
    script_sig: &str,
) -> Result<Vec<(TransactionRun, RunResult, Option<super::routing::QueuedExecution>)>, TrebError> {
    let plan = super::routing::reduce_queue(btxs, route_ctx).await?;
    let mut results = Vec::new();

    for planned in &plan.actions {
        match &planned.action {
            super::routing::RoutingAction::Propose {
                safe_address,
                chain_id: proposal_chain_id,
                operations,
                inner_transactions,
                sender_role,
                nonce,
                ..
            } => {
                let safe_tx_hash = super::routing::compute_safe_tx_hash_for_ops(
                    operations,
                    *safe_address,
                    *nonce,
                    *proposal_chain_id,
                );

                let tx_ids: Vec<String> = planned
                    .run
                    .tx_indices
                    .iter()
                    .filter_map(|&i| recorded_txs.get(i))
                    .map(|rt| rt.transaction.id.clone())
                    .collect();
                let safe_tx_data_items: Vec<SafeTxData> = planned
                    .run
                    .tx_indices
                    .iter()
                    .filter_map(|&i| recorded_txs.get(i))
                    .map(|rt| {
                        let op = rt.transaction.operations.first();
                        SafeTxData {
                            to: op.map(|o| o.target.clone()).unwrap_or_default(),
                            value: "0".into(),
                            data: op.map(|o| o.method.clone()).unwrap_or_default(),
                            operation: 0,
                        }
                    })
                    .collect();

                let (_, bp, _) = super::broadcast_writer::compute_broadcast_paths(
                    project_root,
                    script_path,
                    chain_id,
                    script_sig,
                );

                pending.push(PendingSafeProposal {
                    safe_address: *safe_address,
                    chain_id: *proposal_chain_id,
                    operations: operations.clone(),
                    transaction_ids: tx_ids,
                    safe_tx_data: safe_tx_data_items,
                    sender_role: sender_role.clone(),
                    component_name: component_name.to_string(),
                    original_safe_tx_hash: safe_tx_hash,
                    broadcast_path: bp,
                });

                let run = TransactionRun {
                    sender_role: planned.run.sender_role.clone(),
                    category: planned.run.category,
                    sender_address: planned.run.sender_address,
                    tx_indices: planned.run.tx_indices.clone(),
                };
                let run_result = RunResult::SafeProposed {
                    safe_tx_hash,
                    safe_address: *safe_address,
                    nonce: *nonce,
                    tx_count: inner_transactions.len(),
                };

                // Don't fire on_run_complete — these are intermediate results
                // that will be replaced by the merged proposal in Phase C.
                results.push((run, run_result, planned.queued.clone()));
            }
            _ => {
                let (run, run_result, queued) =
                    super::routing::execute_single_action(planned, route_ctx, Some(btxs)).await?;
                if let Some(cb) = &route_ctx.on_run_complete {
                    cb(&run, &run_result);
                }
                results.push((run, run_result, queued));
            }
        }
    }

    Ok(results)
}

/// Sign and submit a merged Safe proposal to the Safe Transaction Service.
async fn submit_merged_to_safe_service(
    merged: &MergedSafeProposal,
    nonce: u64,
    safe_tx_hash: B256,
    signing_key_hex: &str,
) -> Result<(), TrebError> {
    let operations = &merged.operations;
    let (to, data, operation) = if operations.len() == 1 {
        let op = &operations[0];
        (op.to, op.data.clone(), 0u8)
    } else {
        let multi_send_data = treb_safe::encode_multi_send_call(operations);
        (treb_safe::MULTI_SEND_ADDRESS, multi_send_data, 1u8)
    };

    let key_bytes: B256 =
        signing_key_hex.parse().map_err(|e| TrebError::Safe(format!("invalid signer key: {e}")))?;
    let wallet_signer = foundry_wallets::WalletSigner::from_private_key(&key_bytes)
        .map_err(|e| TrebError::Safe(format!("failed to create signer: {e}")))?;
    let signature = sign_safe_tx(&wallet_signer, safe_tx_hash).await?;

    let signer_addr = Signer::address(&wallet_signer);
    let request = ProposeRequest {
        to: format!("{}", to),
        value: "0".into(),
        data: Some(format!("0x{}", alloy_primitives::hex::encode(&data))),
        operation,
        safe_tx_gas: "0".into(),
        base_gas: "0".into(),
        gas_price: "0".into(),
        gas_token: format!("{}", Address::ZERO),
        refund_receiver: format!("{}", Address::ZERO),
        nonce,
        contract_transaction_hash: format!("{:#x}", safe_tx_hash),
        sender: format!("{}", signer_addr),
        signature: format!("0x{}", alloy_primitives::hex::encode(&signature)),
        origin: Some("treb".into()),
    };

    let safe_client = SafeServiceClient::new(merged.chain_id).ok_or_else(|| {
        TrebError::Safe(format!(
            "Safe Transaction Service not available for chain {}",
            merged.chain_id,
        ))
    })?;
    safe_client.propose_transaction(&format!("{}", merged.safe_address), &request).await?;

    Ok(())
}

/// Info about a submitted merged proposal, for display.
struct SubmittedMergedProposal {
    sender_role: String,
    safe_tx_hash: B256,
    nonce: u64,
    tx_count: usize,
}

/// Submit merged Safe proposals: assign nonces, compute hashes, submit
/// to Safe TX Service (live mode), write registry records, and back-patch
/// per-component deferred files.
async fn submit_merged_proposals(
    merged: &[MergedSafeProposal],
    registry: &mut Registry,
    sender_info: &MergeSenderInfo,
) -> Result<Vec<SubmittedMergedProposal>, TrebError> {
    if merged.is_empty() {
        return Ok(Vec::new());
    }

    // Pre-query base nonces for each unique Safe address
    let mut base_nonces: HashMap<Address, u64> = HashMap::new();
    for proposal in merged {
        if base_nonces.contains_key(&proposal.safe_address) {
            continue;
        }
        let nonce = if sender_info.is_fork {
            let rpc = sender_info.rpc_url.as_deref().ok_or_else(|| {
                TrebError::Forge("RPC URL required for fork Safe nonce query".into())
            })?;
            let provider = crate::provider::build_http_provider(rpc)?;
            super::fork_routing::query_safe_nonce(&provider, proposal.safe_address).await?
        } else {
            let client = SafeServiceClient::new(proposal.chain_id).ok_or_else(|| {
                TrebError::Safe(format!(
                    "Safe Transaction Service not available for chain {}",
                    proposal.chain_id,
                ))
            })?;
            client.get_next_nonce(&format!("{}", proposal.safe_address)).await?
        };
        base_nonces.insert(proposal.safe_address, nonce);
    }

    let mut nonce_offsets: HashMap<Address, u64> = HashMap::new();
    let now = chrono::Utc::now();
    let mut submitted = Vec::new();

    for proposal in merged {
        let base = base_nonces[&proposal.safe_address];
        let offset = nonce_offsets.entry(proposal.safe_address).or_insert(0);
        let nonce = base + *offset;
        *offset += 1;

        // Compute merged hash
        let safe_tx_hash = super::routing::compute_safe_tx_hash_for_ops(
            &proposal.operations,
            proposal.safe_address,
            nonce,
            proposal.chain_id,
        );

        // Look up signing key and proposed_by for this sender role
        let (signing_key, proposed_by) =
            sender_info.sender_signing.get(&proposal.sender_role).cloned().unwrap_or_default();

        // Submit to Safe TX Service (live mode only)
        if !sender_info.is_fork && !signing_key.is_empty() {
            submit_merged_to_safe_service(proposal, nonce, safe_tx_hash, &signing_key).await?;
        }

        // Write merged SafeTransaction to registry
        let safe_transaction = treb_core::types::SafeTransaction {
            safe_tx_hash: format!("{:#x}", safe_tx_hash),
            safe_address: proposal.safe_address.to_checksum(None),
            chain_id: proposal.chain_id,
            status: TransactionStatus::Queued,
            nonce,
            transactions: proposal.safe_tx_data.clone(),
            transaction_ids: proposal.transaction_ids.clone(),
            proposed_by,
            proposed_at: now,
            confirmations: Vec::new(),
            executed_at: None,
            fork_executed_at: None,
            execution_tx_hash: String::new(),
        };
        let _ = registry.insert_safe_transaction(safe_transaction);

        // Back-patch each component's deferred file with the merged hash/nonce
        let new_hash_str = format!("{:#x}", safe_tx_hash);
        for (orig_hash, bp) in
            proposal.original_safe_tx_hashes.iter().zip(&proposal.broadcast_paths)
        {
            let old_hash_str = format!("{:#x}", orig_hash);
            let _ = super::broadcast_writer::update_deferred_safe_proposal(
                bp,
                &old_hash_str,
                &new_hash_str,
                nonce,
            );
        }

        submitted.push(SubmittedMergedProposal {
            sender_role: proposal.sender_role.clone(),
            safe_tx_hash,
            nonce,
            tx_count: proposal.transaction_ids.len(),
        });
    }

    Ok(submitted)
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

        let metadata = collect_recorded_transaction_metadata(
            &events,
            &HashMap::new(),
            &mut pending,
            &HashMap::new(),
        );
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

        let metadata = collect_recorded_transaction_metadata(
            &events,
            &deployments,
            &mut pending,
            &HashMap::new(),
        );
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

        let metadata =
            build_recorded_transaction_metadata(&events, &deployments, &traces, &HashMap::new());
        let tx_meta = metadata
            .get("tx-0xdddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd")
            .expect("metadata should exist");

        assert_eq!(tx_meta.gas_used, Some(987_654));
    }

    // ── merge_adjacent_safe_proposals tests ────────────────────────────

    fn make_pending(safe_addr: Address, component: &str, tx_id: &str) -> PendingSafeProposal {
        PendingSafeProposal {
            safe_address: safe_addr,
            chain_id: 1,
            operations: vec![treb_safe::MultiSendOperation {
                operation: 0,
                to: Address::ZERO,
                value: U256::ZERO,
                data: alloy_primitives::Bytes::new(),
            }],
            transaction_ids: vec![tx_id.to_string()],
            safe_tx_data: vec![treb_core::types::safe_transaction::SafeTxData {
                to: String::new(),
                value: "0".into(),
                data: String::new(),
                operation: 0,
            }],
            sender_role: "deployer".into(),
            component_name: component.into(),
            original_safe_tx_hash: B256::ZERO,
            broadcast_path: PathBuf::from(format!("broadcast/{component}/1/run-latest.json")),
        }
    }

    #[test]
    fn merge_three_same_safe_into_one() {
        let safe1 = address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let pending = vec![
            make_pending(safe1, "a", "tx-1"),
            make_pending(safe1, "b", "tx-2"),
            make_pending(safe1, "c", "tx-3"),
        ];
        let merged = merge_adjacent_safe_proposals(pending);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].operations.len(), 3);
        assert_eq!(merged[0].transaction_ids, vec!["tx-1", "tx-2", "tx-3"]);
        assert_eq!(merged[0].component_names, vec!["a", "b", "c"]);
        assert_eq!(merged[0].broadcast_paths.len(), 3);
    }

    #[test]
    fn merge_mixed_addresses_groups_correctly() {
        let safe1 = address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let safe2 = address!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        let pending = vec![
            make_pending(safe1, "a", "tx-1"),
            make_pending(safe2, "b", "tx-2"),
            make_pending(safe1, "c", "tx-3"),
        ];
        let merged = merge_adjacent_safe_proposals(pending);
        assert_eq!(merged.len(), 3, "non-adjacent same-Safe should not merge");
        assert_eq!(merged[0].safe_address, safe1);
        assert_eq!(merged[1].safe_address, safe2);
        assert_eq!(merged[2].safe_address, safe1);
    }

    #[test]
    fn merge_adjacent_same_safe_with_different_between() {
        let safe1 = address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let safe2 = address!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        let pending = vec![
            make_pending(safe1, "a", "tx-1"),
            make_pending(safe1, "b", "tx-2"),
            make_pending(safe2, "c", "tx-3"),
            make_pending(safe1, "d", "tx-4"),
            make_pending(safe1, "e", "tx-5"),
        ];
        let merged = merge_adjacent_safe_proposals(pending);
        assert_eq!(merged.len(), 3);
        // Group 1: a+b → safe1
        assert_eq!(merged[0].component_names, vec!["a", "b"]);
        assert_eq!(merged[0].operations.len(), 2);
        // Group 2: c → safe2
        assert_eq!(merged[1].component_names, vec!["c"]);
        // Group 3: d+e → safe1
        assert_eq!(merged[2].component_names, vec!["d", "e"]);
        assert_eq!(merged[2].operations.len(), 2);
    }

    #[test]
    fn merge_single_proposal_passthrough() {
        let safe1 = address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let pending = vec![make_pending(safe1, "a", "tx-1")];
        let merged = merge_adjacent_safe_proposals(pending);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].component_names, vec!["a"]);
        assert_eq!(merged[0].operations.len(), 1);
    }

    #[test]
    fn merge_empty_input() {
        let merged = merge_adjacent_safe_proposals(Vec::new());
        assert!(merged.is_empty());
    }
}
