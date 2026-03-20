//! Compose pipeline — multi-script execution with shared EVM state.
//!
//! Executes multiple forge scripts sequentially against a shared in-memory
//! EVM backend, so contracts deployed by script N are visible to script N+1.
//! After all scripts simulate, the caller can broadcast all collected
//! transactions in order.

use treb_core::error::TrebError;
use treb_registry::Registry;

use crate::{
    artifacts::ArtifactIndex,
    compiler::compile_project,
    script::{ExecutionResult, ScriptConfig},
};

use super::{
    PipelineContext,
    types::PipelineResult,
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
    /// Raw broadcastable transactions from the script execution.
    /// Used to replay state on the shared Anvil fork between compose steps.
    pub result_transactions: Option<foundry_cheatcodes::BroadcastableTransactions>,
}

/// Compose orchestrator — executes multiple scripts with shared EVM state.
pub struct ComposePipeline {
    /// Scripts to execute, in order. Each entry is (name, context, config).
    scripts: Vec<(String, PipelineContext, ScriptConfig)>,
    progress: Option<ComposeProgressCallback>,
}

impl Default for ComposePipeline {
    fn default() -> Self {
        Self::new()
    }
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

        let executed = prepared
            .execute()
            .await
            .map_err(|e| TrebError::Forge(format!("forge execution failed: {e}")))?;

        // Clone execution result for hydration
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

        // Hydrate simulation results using shared path
        let sim = super::simulation::hydrate_simulation(
            execution,
            artifact_index,
            context,
            registry,
            &super::simulation::HydrationOptions {
                populate_safe_context: false,
                include_addressbook_labels: false,
            },
        )
        .await?;

        let result_transactions = sim.broadcastable_transactions.clone();

        let result = PipelineResult {
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

        Ok(ComponentSimulation {
            name: name.to_string(),
            result,
            result_transactions,
        })
    }
}

/// Replay broadcastable transactions on an Anvil fork so subsequent scripts
/// see the state changes (deployed contracts, storage writes, balance changes).
///
/// Uses `anvil_impersonateAccount` + `eth_sendTransaction` for each tx.
/// Anvil mines each tx immediately, persisting the state for the next script.
pub async fn replay_transactions_on_fork(
    rpc_url: &str,
    txs: &foundry_cheatcodes::BroadcastableTransactions,
) -> Result<(), TrebError> {
    use alloy_primitives::U256;
    use alloy_provider::Provider;
    use alloy_rpc_types::{TransactionInput, TransactionRequest};
    use super::fork_routing::{anvil_impersonate, anvil_stop_impersonating};

    let provider = crate::provider::build_http_provider(rpc_url)?;

    // Collect unique senders and fund them on the fork
    let mut funded = std::collections::HashSet::new();
    for btx in txs.iter() {
        let from = btx.transaction.from().unwrap_or_default();
        if funded.insert(from) {
            // 100 ETH — enough for any gas cost on the fork
            provider
                .raw_request::<_, serde_json::Value>(
                    "anvil_setBalance".into(),
                    (from, U256::from(100_000_000_000_000_000_000u128)),
                )
                .await
                .map_err(|e| TrebError::Forge(format!("compose replay: fund sender failed: {e}")))?;
        }
    }

    for (i, btx) in txs.iter().enumerate() {
        let from = btx.transaction.from().unwrap_or_default();

        // Impersonate the sender so Anvil accepts the tx without a signature
        anvil_impersonate(&provider, from).await.map_err(|e| {
            TrebError::Forge(format!("compose replay: impersonate failed: {e}"))
        })?;

        // Build the transaction request
        let mut tx = TransactionRequest::default().from(from);
        tx.gas = Some(30_000_000);

        if let Some(to) = btx.transaction.to() {
            match to {
                alloy_primitives::TxKind::Call(addr) => {
                    tx = tx.to(addr);
                }
                alloy_primitives::TxKind::Create => {}
            }
        }

        if let Some(input) = btx.transaction.input() {
            if !input.is_empty() {
                tx = tx.input(TransactionInput::new(input.clone()));
            }
        }

        let value = btx.transaction.value().unwrap_or_default();
        if !value.is_zero() {
            tx = tx.value(value);
        }

        // Send the transaction and wait for receipt
        let pending = provider.send_transaction(tx).await.map_err(|e| {
            TrebError::Forge(format!(
                "compose replay: tx {} from {:#x} failed: {e}",
                i, from
            ))
        })?;

        pending.get_receipt().await.map_err(|e| {
            TrebError::Forge(format!(
                "compose replay: tx {} from {:#x} receipt failed: {e}",
                i, from
            ))
        })?;

        // Stop impersonating
        anvil_stop_impersonating(&provider, from).await;
    }

    Ok(())
}

