//! Shared simulation hydration — converts raw execution results into hydrated
//! pipeline types used by both the orchestrator and compose pipelines.

use std::collections::HashMap;

use alloy_primitives::Address;
use foundry_evm::traces::{CallTraceDecoderBuilder, identifier::TraceIdentifiers};
use treb_core::{error::TrebError, types::GovernorProposal};
use treb_registry::Registry;

use crate::{
    artifacts::ArtifactIndex,
    events::{
        ExtractedCollision, decode_events, detect_proxy_relationships, extract_collisions,
        extract_deployments,
    },
    foundry_compat::BroadcastableTransactions,
    script::ExecutionResult,
};

use super::{
    PipelineContext,
    hydration::{
        hydrate_deployment, hydrate_governor_proposals, hydrate_safe_transactions,
        hydrate_transactions, hydrate_transactions_from_broadcast, populate_safe_context,
    },
    orchestrator::{
        build_v2_recorded_transaction_metadata, collapse_decoded_bytecode_args,
        extract_governor_proposal_created, extract_safe_transaction_queued,
        extract_transaction_simulated, render_traces_for_verbosity, strip_internal_events,
    },
    types::{RecordedDeployment, RecordedTransaction},
};

/// Options controlling optional hydration steps.
pub struct HydrationOptions {
    /// When true, populates safe_context on transactions (orchestrator sets
    /// this when deployer is a Safe sender).
    pub populate_safe_context: bool,
    /// When true, includes addressbook entries in trace labels.
    pub include_addressbook_labels: bool,
}

/// Output of `hydrate_simulation()` — everything downstream needs to build
/// a `PipelineResult` or `ComponentSimulation`.
pub struct SimulationOutput {
    pub recorded_deployments: Vec<RecordedDeployment>,
    pub recorded_transactions: Vec<RecordedTransaction>,
    pub collisions: Vec<ExtractedCollision>,
    pub governor_proposals: Vec<GovernorProposal>,
    pub safe_transactions: Vec<treb_core::types::SafeTransaction>,
    pub gas_used: u64,
    pub event_count: usize,
    pub console_logs: Vec<String>,
    pub execution_traces: Option<String>,
    pub setup_traces: Option<String>,
    pub broadcastable_transactions: Option<BroadcastableTransactions>,
}

/// Hydrate a raw `ExecutionResult` into structured pipeline types.
///
/// This is the shared simulation path used by both `RunPipeline::execute()`
/// and `ComposePipeline::simulate_one()`. It handles:
///
/// 1. Error check with trace rendering on failure
/// 2. Event decoding → deployment/collision/proxy extraction
/// 3. Deployment + transaction hydration (v2 broadcast or v1 event fallback)
/// 4. Governor proposal + safe transaction hydration
/// 5. Address label assembly (senders + deployments + registry + optionally addressbook)
/// 6. Conditional `populate_safe_context`
/// 7. Trace rendering + per-transaction metadata
/// 8. `RecordedTransaction` assembly
pub async fn hydrate_simulation(
    mut execution: ExecutionResult,
    artifact_index: &ArtifactIndex,
    context: &PipelineContext,
    registry: &Registry,
    options: &HydrationOptions,
) -> Result<SimulationOutput, TrebError> {
    // 1. Check for failed execution
    if !execution.success {
        let mut err_parts = Vec::new();
        if !execution.logs.is_empty() {
            err_parts.push(execution.logs.join("\n"));
        }
        let contracts = artifact_index.inner();
        let mut decoder = CallTraceDecoderBuilder::new().with_known_contracts(contracts).build();
        let mut identifier = TraceIdentifiers::new().with_local(contracts);
        for (_, arena) in &execution.traces {
            decoder.identify(&arena.arena, &mut identifier);
        }
        for (_, arena) in &mut execution.traces {
            foundry_evm::traces::decode_trace_arena(&mut arena.arena, &decoder).await;
            collapse_decoded_bytecode_args(&mut arena.arena, artifact_index);
            let rendered = strip_internal_events(&foundry_evm::traces::render_trace_arena(arena));
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

    // 2. Decode events, extract deployments/collisions/proxies
    let parsed_events = decode_events(&execution.raw_logs);
    let event_count = parsed_events.len();
    let extracted_deployments = extract_deployments(&parsed_events, Some(artifact_index));
    let collisions = extract_collisions(&parsed_events, Some(artifact_index));
    let proxy_relationships = detect_proxy_relationships(&parsed_events);

    // 3. Hydrate deployments
    let hydrated_deployments: Vec<_> = extracted_deployments
        .iter()
        .map(|extracted| {
            let proxy = proxy_relationships.get(&extracted.address);
            hydrate_deployment(extracted, proxy, context)
        })
        .collect();

    // 4. Hydrate transactions (v2 broadcast or v1 event fallback)
    let broadcastable_txs = execution.transactions.as_ref();
    let mut transactions = if let Some(btxs) = broadcastable_txs {
        hydrate_transactions_from_broadcast(btxs, &hydrated_deployments, context)
    } else {
        let tx_events = extract_transaction_simulated(&parsed_events);
        hydrate_transactions(&tx_events, &hydrated_deployments, context)
    };

    // 5. Build per-transaction metadata with trace matching
    let mut transaction_metadata = if let Some(btxs) = broadcastable_txs {
        build_v2_recorded_transaction_metadata(
            btxs,
            &extracted_deployments,
            &execution.traces,
            context,
        )
    } else {
        let tx_events = extract_transaction_simulated(&parsed_events);
        let sender_id_labels: HashMap<alloy_primitives::B256, String> = context
            .sender_role_names
            .iter()
            .map(|role| (alloy_primitives::keccak256(role.as_bytes()), role.clone()))
            .collect();
        super::orchestrator::build_recorded_transaction_metadata(
            &tx_events,
            &extracted_deployments,
            &execution.traces,
            &sender_id_labels,
        )
    };

    // 6. Hydrate governor proposals and safe transactions from events
    let governor_events = extract_governor_proposal_created(&parsed_events);
    let governor_broadcasts = super::orchestrator::extract_governor_broadcasts(&parsed_events);
    let governor_proposals =
        hydrate_governor_proposals(&governor_events, &governor_broadcasts, context);

    let safe_tx_events = extract_safe_transaction_queued(&parsed_events);
    let safe_transactions = hydrate_safe_transactions(&safe_tx_events, context);

    // 7. Conditionally populate safe context
    if options.populate_safe_context {
        populate_safe_context(&mut transactions, &safe_transactions);
    }

    // 8. Build address labels for trace decoding
    let mut labeled_addresses = execution.labeled_addresses.clone();
    for (addr, role) in &context.sender_labels {
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
    if options.include_addressbook_labels {
        let chain_id_str = context.config.chain_id.to_string();
        let mut addressbook =
            treb_registry::AddressbookStore::new(&context.project_root.join(".treb"));
        if addressbook.load().is_ok() {
            for (name, address) in addressbook.list_entries(&chain_id_str) {
                if let Ok(addr) = address.parse::<Address>() {
                    labeled_addresses.entry(addr).or_insert(name);
                }
            }
        }
    }

    // 9. Render traces and extract per-transaction sub-trees
    let (execution_traces, setup_traces) = render_traces_for_verbosity(
        execution.traces,
        &labeled_addresses,
        artifact_index,
        context.config.verbosity,
        &mut transaction_metadata,
    )
    .await;

    // 10. Build recorded transactions
    let recorded_transactions: Vec<RecordedTransaction> = transactions
        .into_iter()
        .map(|tx| {
            let metadata = transaction_metadata.remove(&tx.id).unwrap_or_default();
            let sender_category = metadata
                .sender_name
                .as_ref()
                .and_then(|name| context.resolved_senders.get(name))
                .map(|s| s.category());
            RecordedTransaction {
                transaction: tx,
                sender_name: metadata.sender_name,
                sender_category,
                gas_used: metadata.gas_used,
                trace: metadata.trace,
            }
        })
        .collect();

    // 11. Build recorded deployments
    let recorded_deployments: Vec<RecordedDeployment> = hydrated_deployments
        .into_iter()
        .map(|dep| RecordedDeployment { deployment: dep, safe_transaction: None })
        .collect();

    Ok(SimulationOutput {
        recorded_deployments,
        recorded_transactions,
        collisions,
        governor_proposals,
        safe_transactions,
        gas_used: execution.gas_used,
        event_count,
        console_logs: execution.logs,
        execution_traces,
        setup_traces,
        broadcastable_transactions: execution.transactions.clone(),
    })
}
