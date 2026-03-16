//! Compose pipeline — multi-script execution with shared EVM state.
//!
//! Executes multiple forge scripts sequentially against a shared in-memory
//! EVM backend, so contracts deployed by script N are visible to script N+1.
//! After all scripts simulate, the caller can broadcast all collected
//! transactions in order.

use std::collections::HashMap;

use alloy_primitives::{Address, B256};
use foundry_evm::{
    traces::{
        CallTraceDecoderBuilder, decode_trace_arena, identifier::TraceIdentifiers,
        render_trace_arena,
    },
};
use treb_core::error::TrebError;
use treb_registry::Registry;

use crate::{
    artifacts::ArtifactIndex,
    compiler::compile_project,
    events::{decode_events, detect_proxy_relationships, extract_collisions, extract_deployments},
    script::{BroadcastReceipt, ExecutionResult, ScriptConfig},
};

use super::{
    PipelineContext,
    hydration::{
        hydrate_deployment, hydrate_governor_proposals, hydrate_safe_transactions,
        hydrate_transactions,
    },
    orchestrator::{
        build_recorded_transaction_metadata, collapse_decoded_bytecode_args,
        extract_governor_proposal_created, extract_safe_transaction_queued,
        extract_transaction_simulated, render_traces_for_verbosity, strip_internal_events,
    },
    types::{PipelineResult, RecordedDeployment, RecordedTransaction},
};

/// Progress callback for compose pipeline phases.
pub type ComposeProgressCallback = Box<dyn Fn(ComposePhase) + Send>;

/// Phases reported by the compose pipeline.
#[derive(Debug, Clone)]
pub enum ComposePhase {
    /// Compiling the project (once, shared across all scripts).
    Compiling,
    /// Executing a specific component script.
    Executing(String),
    /// All simulations complete.
    SimulationComplete,
    /// Broadcasting a specific component.
    Broadcasting(String),
    /// All broadcasts complete.
    BroadcastComplete,
}

/// Result of simulating a single component in the compose pipeline.
pub struct ComponentSimulation {
    /// The component name.
    pub name: String,
    /// Hydrated pipeline result (deployments, transactions, traces, etc.).
    pub result: PipelineResult,
    /// Forge's executed state, stored for later broadcast.
    /// Opaque — use [`broadcast_component`] to consume this.
    pub executed_state: Option<ExecutedStateHolder>,
    /// Raw broadcastable transactions from the script execution.
    /// Used to replay state on the shared Anvil fork between compose steps.
    pub result_transactions: Option<foundry_cheatcodes::BroadcastableTransactions>,
    /// The fork URL this component executed against.
    pub fork_url: Option<String>,
}

/// Opaque holder for forge's ExecutedState (private module type).
/// We store it as a trait object since we can't name the concrete type.
pub struct ExecutedStateHolder {
    /// The broadcast continuation: prepare_simulation → fill_metadata → bundle → broadcast.
    /// Called with `broadcast()` to continue the forge state machine.
    broadcast_fn: Option<BroadcastFn>,
}

type BroadcastFn =
    Box<dyn FnOnce() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<BroadcastReceipt>, TrebError>> + Send>> + Send>;

/// Compose orchestrator — executes multiple scripts with shared EVM state.
pub struct ComposePipeline {
    /// Scripts to execute, in order. Each entry is (name, context, config).
    scripts: Vec<(String, PipelineContext, ScriptConfig)>,
    progress: Option<ComposeProgressCallback>,
}

impl ComposePipeline {
    pub fn new() -> Self {
        Self { scripts: Vec::new(), progress: None }
    }

    /// Add a script to the execution queue.
    pub fn add_script(
        &mut self,
        name: impl Into<String>,
        context: PipelineContext,
        config: ScriptConfig,
    ) {
        self.scripts.push((name.into(), context, config));
    }

    /// Set a progress callback.
    pub fn with_progress(mut self, cb: ComposeProgressCallback) -> Self {
        self.progress = Some(cb);
        self
    }

    /// Simulate all scripts sequentially with shared EVM state.
    ///
    /// Spawns an ephemeral Anvil fork so state changes from each script are
    /// visible to subsequent scripts without mutating the upstream fork.
    /// After all scripts simulate, the ephemeral Anvil is killed.
    pub async fn simulate_all(
        mut self,
        registry: &mut Registry,
    ) -> Result<Vec<ComponentSimulation>, (Vec<ComponentSimulation>, String, TrebError)> {
        let report = |phase: ComposePhase| {
            if let Some(ref cb) = self.progress {
                cb(phase);
            }
        };

        // 1. Compile once (shared across all scripts)
        report(ComposePhase::Compiling);
        let project_root = self
            .scripts
            .first()
            .map(|(_, ctx, _)| ctx.project_root.clone())
            .unwrap_or_default();
        let foundry_config = super::orchestrator::load_foundry_config(&project_root)
            .map_err(|e| (Vec::new(), String::new(), e))?;
        let compilation = compile_project(&foundry_config)
            .map_err(|e| (Vec::new(), String::new(), e))?;
        let artifact_index = ArtifactIndex::from_compile_output(compilation);

        // Snapshot the registry so we can write intermediate deployments
        // for Solidity lookup() between scripts, then restore if we don't
        // end up broadcasting.
        let registry_dir = project_root.join(".treb");
        let snapshot_dir = registry_dir.join("priv/snapshots/compose");
        let _ = treb_registry::snapshot_registry(&registry_dir, &snapshot_dir);

        // 2. Spawn an ephemeral Anvil fork for compose simulation.
        // All scripts fork from this instance so state flows between steps
        // without mutating the upstream fork.
        let upstream_url = self
            .scripts
            .first()
            .and_then(|(_, _, sc)| sc.rpc_url_ref().map(|s| s.to_string()));

        let ephemeral_anvil = if let Some(ref url) = upstream_url {
            // If the URL is a network name (not http://...), resolve it to
            // an actual RPC endpoint so Anvil can fork from it.
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
        };

        let ephemeral_url = ephemeral_anvil.as_ref().map(|a| a.rpc_url().to_string());

        // Override each script's fork URL to point at the ephemeral Anvil.
        // Also override the foundry RPC endpoint env var so that
        // vm.createFork("network") inside Solidity resolves to the
        // ephemeral Anvil rather than the original upstream RPC.
        // The guard restores the env var when compose finishes.
        let _rpc_override_guard = if let Some(ref url) = ephemeral_url {
            for (_, _, sc) in &mut self.scripts {
                sc.rpc_url(url);
            }
            upstream_url.as_deref()
                .filter(|u| !u.starts_with("http://") && !u.starts_with("https://"))
                .and_then(|network| unsafe {
                    treb_config::override_rpc_endpoint(&project_root, network, url)
                })
        } else {
            None
        };

        let mut results: Vec<ComponentSimulation> = Vec::new();

        for (name, context, script_config) in self.scripts {
            report(ComposePhase::Executing(name.clone()));

            match Self::simulate_one(
                &name,
                &context,
                script_config,
                &artifact_index,
                registry,
            )
            .await
            {
                Ok(sim) => {
                    // Replay broadcastable transactions on the ephemeral Anvil
                    // so the next script sees this script's state changes.
                    if let Some(ref btxs) = sim.result_transactions {
                        if let Some(ref url) = ephemeral_url {
                            if let Err(e) = replay_transactions_on_fork(url, btxs).await {
                                let _ = treb_registry::restore_registry(&snapshot_dir, &registry_dir);
                                let _ = std::fs::remove_dir_all(&snapshot_dir);
                                return Err((results, name, e));
                            }
                        }
                    }

                    // Write deployments AND collisions to registry between steps
                    // so the next script's Solidity lookup() finds them via
                    // vm.readFile("registry.json").
                    for rd in &sim.result.deployments {
                        let _ = registry.insert_deployment(rd.deployment.clone());
                    }
                    // Collisions are contracts that already exist at the predicted
                    // address — still register them so lookup() works between steps.
                    for collision in &sim.result.collisions {
                        let dep = super::hydration::hydrate_collision(collision, &context);
                        let _ = registry.insert_deployment(dep);
                    }
                    results.push(sim);
                }
                Err(e) => {
                    let _ = treb_registry::restore_registry(&snapshot_dir, &registry_dir);
                    let _ = std::fs::remove_dir_all(&snapshot_dir);
                    // Ephemeral Anvil is dropped automatically
                    return Err((results, name, e));
                }
            }
        }

        // Restore registry to pre-compose state — the caller decides
        // whether to commit after broadcast.
        let _ = treb_registry::restore_registry(&snapshot_dir, &registry_dir);
        let _ = std::fs::remove_dir_all(&snapshot_dir);

        // Ephemeral Anvil is dropped here (killed automatically)
        report(ComposePhase::SimulationComplete);
        Ok(results)
    }

    async fn simulate_one(
        name: &str,
        context: &PipelineContext,
        script_config: ScriptConfig,
        artifact_index: &ArtifactIndex,
        registry: &mut Registry,
    ) -> Result<ComponentSimulation, TrebError> {
        let script_args = script_config.into_script_args()?;
        let wants_broadcast = script_args.broadcast && context.config.broadcast;

        // Run forge: preprocess → compile → link → execute
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

        let fork_url = prepared.script_config.evm_opts.fork_url.clone();

        let executed = prepared
            .execute()
            .await
            .map_err(|e| TrebError::Forge(format!("forge execution failed: {e}")))?;

        // Clone execution result for hydration
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

        // Check for failed execution
        if !execution.success {
            let mut err_parts = Vec::new();
            if !execution.logs.is_empty() {
                err_parts.push(execution.logs.join("\n"));
            }
            let contracts = artifact_index.inner();
            let mut decoder =
                CallTraceDecoderBuilder::new().with_known_contracts(contracts).build();
            let mut identifier = TraceIdentifiers::new().with_local(contracts);
            for (_, arena) in &execution.traces {
                decoder.identify(&arena.arena, &mut identifier);
            }
            for (_, arena) in &mut execution.traces {
                decode_trace_arena(&mut arena.arena, &decoder).await;
                collapse_decoded_bytecode_args(&mut arena.arena, artifact_index);
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

        // Hydrate: decode events → extract deployments → hydrate transactions
        let parsed_events = decode_events(&execution.raw_logs);
        let event_count = parsed_events.len();
        let extracted_deployments = extract_deployments(&parsed_events, Some(artifact_index));
        let collisions = extract_collisions(&parsed_events);
        let proxy_relationships = detect_proxy_relationships(&parsed_events);

        let hydrated_deployments = extracted_deployments
            .iter()
            .map(|extracted| {
                let proxy = proxy_relationships.get(&extracted.address);
                hydrate_deployment(extracted, proxy, context)
            })
            .collect::<Vec<_>>();

        let tx_events = extract_transaction_simulated(&parsed_events);
        let safe_tx_events = extract_safe_transaction_queued(&parsed_events);
        let governor_events = extract_governor_proposal_created(&parsed_events);

        let transactions = hydrate_transactions(&tx_events, &hydrated_deployments, context);
        let safe_transactions = hydrate_safe_transactions(&safe_tx_events, context);
        let governor_proposals = hydrate_governor_proposals(&governor_events, context);

        let sender_id_labels: HashMap<B256, String> = context
            .sender_role_names
            .iter()
            .map(|role| (alloy_primitives::keccak256(role.as_bytes()), role.clone()))
            .collect();

        let mut transaction_metadata = build_recorded_transaction_metadata(
            &tx_events,
            &extracted_deployments,
            &execution.traces,
            &sender_id_labels,
        );

        // Build address labels for trace decoding
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

        // Render traces
        let (execution_traces, setup_traces) = render_traces_for_verbosity(
            execution.traces,
            &labeled_addresses,
            artifact_index,
            context.config.verbosity,
            &mut transaction_metadata,
        )
        .await;

        // Build recorded transactions
        let recorded_transactions: Vec<RecordedTransaction> = transactions
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

        // Build recorded deployments (no registry writes in simulate phase)
        let recorded_deployments: Vec<RecordedDeployment> = hydrated_deployments
            .into_iter()
            .map(|dep| RecordedDeployment { deployment: dep, safe_transaction: None })
            .collect();

        // Store the executed state for later broadcast
        let broadcast_fn: Option<BroadcastFn> = if wants_broadcast {
            Some(Box::new(move || {
                Box::pin(async move {
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
                    let broadcasted = bundled
                        .broadcast()
                        .await
                        .map_err(|e| TrebError::Forge(format!("forge broadcast failed: {e}")))?;

                    let mut receipts = Vec::new();
                    for seq in broadcasted.sequence.sequences() {
                        for (tx_meta, receipt) in seq.transactions.iter().zip(seq.receipts.iter()) {
                            receipts.push(BroadcastReceipt {
                                hash: receipt.transaction_hash,
                                block_number: receipt.block_number.unwrap_or_default(),
                                gas_used: receipt.gas_used,
                                status: receipt.inner.inner.inner.receipt.status.coerce_status(),
                                contract_name: tx_meta.contract_name.clone().filter(|s| !s.is_empty()),
                                contract_address: receipt.contract_address,
                            });
                        }
                    }
                    Ok(receipts)
                })
            }))
        } else {
            None
        };

        let result = PipelineResult {
            deployments: recorded_deployments,
            transactions: recorded_transactions,
            registry_updated: false,
            collisions,
            skipped: Vec::new(),
            dry_run: !wants_broadcast,
            success: true,
            gas_used: execution.gas_used,
            event_count,
            console_logs: execution.logs,
            governor_proposals,
            execution_traces,
            setup_traces,
        };

        // Capture broadcastable transactions for replaying on the shared fork
        let result_transactions = execution.transactions.clone();

        Ok(ComponentSimulation {
            name: name.to_string(),
            result,
            executed_state: broadcast_fn.map(|f| ExecutedStateHolder { broadcast_fn: Some(f) }),
            result_transactions,
            fork_url,
        })
    }
}

/// Broadcast a component's stored transactions.
///
/// Consumes the stored forge state and continues the state machine
/// through `prepare_simulation → fill_metadata → bundle → broadcast`.
pub async fn broadcast_component(
    sim: &mut ComponentSimulation,
) -> Result<Vec<BroadcastReceipt>, TrebError> {
    let holder = sim
        .executed_state
        .as_mut()
        .ok_or_else(|| TrebError::Forge("no broadcast state stored (dry-run?)".into()))?;
    let broadcast_fn = holder
        .broadcast_fn
        .take()
        .ok_or_else(|| TrebError::Forge("broadcast already consumed".into()))?;
    broadcast_fn().await
}

/// Replay broadcastable transactions on an Anvil fork so subsequent scripts
/// see the state changes (deployed contracts, storage writes, balance changes).
///
/// Uses `anvil_impersonateAccount` + `eth_sendTransaction` for each tx.
/// Anvil mines each tx immediately, persisting the state for the next script.
async fn replay_transactions_on_fork(
    rpc_url: &str,
    txs: &foundry_cheatcodes::BroadcastableTransactions,
) -> Result<(), TrebError> {
    let client = reqwest::Client::new();

    for (i, btx) in txs.iter().enumerate() {
        let from = btx.transaction.from().unwrap_or_default();

        // Impersonate the sender so Anvil accepts the tx without a signature
        let impersonate = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "anvil_impersonateAccount",
            "params": [format!("{:#x}", from)],
            "id": i * 2 + 1,
        });
        client
            .post(rpc_url)
            .json(&impersonate)
            .send()
            .await
            .map_err(|e| TrebError::Forge(format!("compose replay: impersonate failed: {e}")))?;

        // Build the eth_sendTransaction request
        let mut tx_obj = serde_json::Map::new();
        tx_obj.insert("from".into(), serde_json::json!(format!("{:#x}", from)));

        if let Some(to) = btx.transaction.to() {
            match to {
                alloy_primitives::TxKind::Call(addr) => {
                    tx_obj.insert("to".into(), serde_json::json!(format!("{:#x}", addr)));
                }
                alloy_primitives::TxKind::Create => {}
            }
        }

        if let Some(input) = btx.transaction.input() {
            if !input.is_empty() {
                tx_obj.insert(
                    "data".into(),
                    serde_json::json!(format!("0x{}", alloy_primitives::hex::encode(input))),
                );
            }
        }

        let value = btx.transaction.value().unwrap_or_default();
        if !value.is_zero() {
            tx_obj.insert("value".into(), serde_json::json!(format!("{:#x}", value)));
        }

        // Set a high gas limit to avoid estimation issues
        tx_obj.insert("gas".into(), serde_json::json!("0x1c9c380")); // 30M

        let send_tx = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_sendTransaction",
            "params": [tx_obj],
            "id": i * 2 + 2,
        });

        let resp: serde_json::Value = client
            .post(rpc_url)
            .json(&send_tx)
            .send()
            .await
            .map_err(|e| TrebError::Forge(format!("compose replay: send tx failed: {e}")))?
            .json()
            .await
            .map_err(|e| TrebError::Forge(format!("compose replay: parse response failed: {e}")))?;

        if let Some(err) = resp.get("error") {
            return Err(TrebError::Forge(format!(
                "compose replay: tx {} from {:#x} failed: {}",
                i,
                from,
                err
            )));
        }

        // Stop impersonating
        let stop = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "anvil_stopImpersonatingAccount",
            "params": [format!("{:#x}", from)],
            "id": i * 2 + 3,
        });
        let _ = client.post(rpc_url).json(&stop).send().await;
    }

    Ok(())
}

