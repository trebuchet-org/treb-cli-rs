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
    compiler::compile_project,
    events::{
        ExtractedDeployment, GovernorProposalCreated, SafeTransactionQueued, TransactionSimulated,
        abi::SimulatedTransaction,
        decode_events,
        decoder::{ParsedEvent, TrebEvent},
        detect_proxy_relationships, extract_collisions, extract_deployments,
    },
    script::{BroadcastReceipt, ExecutionResult, ScriptConfig},
};

use super::{
    PipelineContext,
    duplicates::{DuplicateStrategy, resolve_duplicates},
    hydration::{
        build_v2_transaction_metadata, hydrate_deployment, hydrate_governor_proposals,
        hydrate_safe_transactions, hydrate_transactions, hydrate_transactions_from_broadcast,
        populate_safe_context,
    },
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
}

impl RunPipeline {
    pub fn new(context: PipelineContext) -> Self {
        Self { context, script_config: None, broadcast_hook: None, broadcast_progress: None }
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
        let deployer_is_safe = self.context.resolved_senders.get("deployer").is_some_and(|s| s.is_safe());
        let deployer_is_governor = self.context.resolved_senders.get("deployer").is_some_and(|s| s.is_governor());

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
        // Wallet-only senders can broadcast directly through forge.
        // Safe/Governor senders route through Rust after execution.
        let wants_broadcast = script_args.broadcast
            && self.context.config.broadcast
            && !deployer_is_safe
            && !deployer_is_governor;

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
        let mut execution = ExecutionResult {
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

        // 3. Check for failed execution
        if !execution.success {
            let mut err_parts = Vec::new();
            if !execution.logs.is_empty() {
                err_parts.push(execution.logs.join("\n"));
            }

            let contracts = artifact_index.inner();
            let mut decoder = CallTraceDecoderBuilder::new()
                .with_known_contracts(contracts)
                .build();
            let mut identifier = TraceIdentifiers::new().with_local(contracts);
            for (_, arena) in &execution.traces {
                decoder.identify(&arena.arena, &mut identifier);
            }
            for (_, arena) in &mut execution.traces {
                decode_trace_arena(&mut arena.arena, &decoder).await;
                collapse_decoded_bytecode_args(&mut arena.arena, &artifact_index);
                let rendered = strip_internal_events(&render_trace_arena(arena));
                if !rendered.trim().is_empty() {
                    err_parts.push(rendered);
                }
            }

            let detail = if err_parts.is_empty() {
                "script reverted without output".to_string()
            } else {
                err_parts.join("\n")
            };
            return Err(TrebError::Forge(format!("script execution failed:\n{detail}")));
        }

        // 4. Decode events (ContractDeployed, collisions, proxy relationships)
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

        // 7. Hydrate transactions from BroadcastableTransactions (v2 model).
        //    Scripts use vm.broadcast() — forge collects BroadcastableTransactions
        //    which we hydrate directly instead of relying on TransactionSimulated events.
        let broadcastable_txs = execution.transactions.as_ref();
        let transactions = if let Some(btxs) = broadcastable_txs {
            hydrate_transactions_from_broadcast(btxs, &hydrated_deployments, &self.context)
        } else {
            // Fallback to TransactionSimulated events for backward compatibility
            let tx_events = extract_transaction_simulated(&parsed_events);
            hydrate_transactions(&tx_events, &hydrated_deployments, &self.context)
        };

        // Build v2 transaction metadata from broadcastable transactions.
        let mut v2_metadata = if let Some(btxs) = broadcastable_txs {
            build_v2_transaction_metadata(btxs, &self.context)
        } else {
            // Fallback to v1 metadata from TransactionSimulated events
            let tx_events = extract_transaction_simulated(&parsed_events);
            let sender_id_labels: HashMap<B256, String> = self
                .context
                .sender_role_names
                .iter()
                .map(|role| (alloy_primitives::keccak256(role.as_bytes()), role.clone()))
                .collect();
            let v1_meta = build_recorded_transaction_metadata(
                &tx_events,
                &extracted_deployments,
                &execution.traces,
                &sender_id_labels,
            );
            // Convert v1 metadata to v2 format
            v1_meta.into_iter().map(|(k, v)| {
                (k, super::hydration::V2TransactionMetadata {
                    sender_name: v.sender_name,
                    gas_used: v.gas_used,
                })
            }).collect()
        };

        // Also hydrate governor proposals from events (still emitted in v2).
        let governor_events = extract_governor_proposal_created(&parsed_events);
        let governor_proposals = hydrate_governor_proposals(&governor_events, &self.context);

        // 8. Build combined address labels for trace decoding
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
        let chain_id_str = self.context.config.chain_id.to_string();
        let mut addressbook = treb_registry::AddressbookStore::new(&self.context.project_root.join(".treb"));
        if addressbook.load().is_ok() {
            for (name, address) in addressbook.list_entries(&chain_id_str) {
                if let Ok(addr) = address.parse::<Address>() {
                    labeled_addresses.entry(addr).or_insert(name);
                }
            }
        }

        // 9. Render traces (v2 doesn't use per-tx trace matching from TransactionSimulated)
        let mut empty_v1_metadata = HashMap::new();
        let (execution_traces, setup_traces) = render_traces_for_verbosity(
            execution.traces,
            &labeled_addresses,
            &artifact_index,
            self.context.config.verbosity,
            &mut empty_v1_metadata,
        )
        .await;

        // 10. Safe/Governor proposal routing (currently no-op — Rust-side
        //     proposal submission will be wired in a follow-up).
        let safe_tx_events = extract_safe_transaction_queued(&parsed_events);
        let safe_transactions = hydrate_safe_transactions(&safe_tx_events, &self.context);
        let mut transactions = transactions;
        if deployer_is_safe {
            populate_safe_context(&mut transactions, &safe_transactions);
        }

        // 11. Build recorded transactions
        let mut recorded_transactions: Vec<RecordedTransaction> = transactions
            .into_iter()
            .map(|tx| {
                let metadata = v2_metadata.remove(&tx.id);
                RecordedTransaction {
                    transaction: tx,
                    sender_name: metadata.as_ref().and_then(|m| m.sender_name.clone()),
                    gas_used: metadata.as_ref().and_then(|m| m.gas_used),
                    trace: None,
                }
            })
            .collect();

        // 12. Broadcast: hook → confirm → broadcast
        let broadcast_confirmed = if wants_broadcast && !recorded_transactions.is_empty() {
            let confirmed = self
                .broadcast_hook
                .map_or(true, |hook| hook(&recorded_transactions));

            if confirmed {
                report(BroadcastPhase::Simulating);
                let pre_sim = executed
                    .prepare_simulation()
                    .await
                    .map_err(|e| TrebError::Forge(format!("forge simulation preparation failed: {e}")))?;
                let filled = pre_sim
                    .fill_metadata()
                    .await
                    .map_err(|e| TrebError::Forge(format!("forge metadata fill failed: {e}")))?;
                let bundled = filled
                    .bundle()
                    .await
                    .map_err(|e| TrebError::Forge(format!("forge bundling failed: {e}")))?;
                let bundled = bundled
                    .wait_for_pending()
                    .await
                    .map_err(|e| TrebError::Forge(format!("forge pending transaction wait failed: {e}")))?;

                report(BroadcastPhase::Broadcasting);
                let broadcasted = bundled
                    .broadcast()
                    .await
                    .map_err(|e| TrebError::Forge(format!("forge broadcast failed: {e}")))?;
                report(BroadcastPhase::Complete);

                let mut receipts = Vec::new();
                for seq in broadcasted.sequence.sequences() {
                    for (tx_meta, receipt) in seq.transactions.iter().zip(seq.receipts.iter()) {
                        receipts.push(crate::script::BroadcastReceipt {
                            hash: receipt.transaction_hash,
                            block_number: receipt.block_number.unwrap_or_default(),
                            gas_used: receipt.gas_used,
                            status: receipt.inner.inner.inner.receipt.status.coerce_status(),
                            contract_name: tx_meta.contract_name.clone().filter(|s| !s.is_empty()),
                            contract_address: receipt.contract_address,
                        });
                    }
                }
                apply_broadcast_receipts(&mut recorded_transactions, &receipts);
                true
            } else {
                false
            }
        } else {
            false
        };

        // 13. Duplicate detection
        let resolved = resolve_duplicates(hydrated_deployments, registry, DuplicateStrategy::Skip)?;
        let skipped = resolved.skipped;

        let should_record = broadcast_confirmed;

        let registry_updated = should_record
            && (!resolved.to_insert.is_empty()
                || !resolved.to_update.is_empty()
                || !recorded_transactions.is_empty()
                || !safe_transactions.is_empty()
                || (deployer_is_governor && !governor_proposals.is_empty()));

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
            for safe_tx in safe_transactions {
                registry.insert_safe_transaction(safe_tx)?;
            }
            if deployer_is_governor {
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
            dry_run: !should_record,
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

    let execution_traces =
        (!execution_parts.is_empty()).then(|| execution_parts.join("\n"));
    let setup_traces =
        (!setup_parts.is_empty()).then(|| setup_parts.join("\n"));

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
        .filter(|line| {
            !INTERNAL_EVENTS.iter().any(|ev| line.contains(&format!("emit {ev}")))
        })
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
pub(super) fn extract_safe_transaction_queued(events: &[ParsedEvent]) -> Vec<SafeTransactionQueued> {
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
pub(super) fn extract_governor_proposal_created(events: &[ParsedEvent]) -> Vec<GovernorProposalCreated> {
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

    collect_recorded_transaction_metadata(tx_events, &transaction_deployments, &mut pending_traces, sender_id_labels)
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
            let sender_name = sender_id_labels
                .get(&sim_tx.senderId)
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
            if let Some(replacement) =
                try_collapse_raw_data(&node.trace.data, artifact_index)
            {
                decoded.call_data = Some(replacement);
            }
        }

        // Check for raw hex "selector" calls — when the signature has no
        // parentheses it means foundry couldn't decode the call and is
        // showing the first 4 bytes as a fake selector.
        if let Some(ref mut call_data) = decoded.call_data {
            if !call_data.signature.contains('(') && !call_data.args.is_empty() {
                if let Some(replacement) =
                    try_collapse_raw_create(call_data, artifact_index)
                {
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
