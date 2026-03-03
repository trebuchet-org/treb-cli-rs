//! Pipeline orchestrator — drives the full deployment recording flow.
//!
//! [`RunPipeline`] sequences compilation, script execution, event decoding,
//! deployment extraction, proxy detection, hydration, duplicate detection,
//! and registry recording into a single `execute` call.

use std::path::Path;

use foundry_config::Config;
use treb_core::error::TrebError;
use treb_registry::Registry;

use crate::artifacts::ArtifactIndex;
use crate::compiler::compile_project;
use crate::events::decoder::{ParsedEvent, TrebEvent};
use crate::events::{
    decode_events, detect_proxy_relationships, extract_collisions, extract_deployments,
    TransactionSimulated, SafeTransactionQueued,
};
use crate::script::{execute_script, ScriptConfig};

use super::duplicates::{resolve_duplicates, DuplicateStrategy};
use super::hydration::{hydrate_deployment, hydrate_safe_transactions, hydrate_transactions};
use super::types::{PipelineResult, RecordedDeployment, RecordedTransaction};
use super::PipelineContext;

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
}

impl RunPipeline {
    /// Create a new pipeline orchestrator with the given execution context.
    pub fn new(context: PipelineContext) -> Self {
        Self {
            context,
            script_config: None,
        }
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

    /// Execute the full deployment recording pipeline.
    ///
    /// # Steps
    ///
    /// 1. Compile the project to build an artifact index
    /// 2. Execute the forge script
    /// 3. Decode raw EVM logs into structured events
    /// 4. Extract deployments, collisions, and proxy relationships
    /// 5. Hydrate forge-domain types into core-domain types
    /// 6. Run duplicate detection against the registry
    /// 7. Record deployments and transactions (skipped in dry-run mode)
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

        // 2. Build script args and execute
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
        let execution = execute_script(script_args).await?;

        // 3. Check for failed execution
        if !execution.success {
            return Err(TrebError::Forge(format!(
                "script execution failed:\n{}",
                execution.logs.join("\n")
            )));
        }

        // 4. Decode events
        let parsed_events = decode_events(&execution.raw_logs);

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

        // 8. Hydrate transactions
        let transactions =
            hydrate_transactions(&tx_events, &hydrated_deployments, &self.context);
        let safe_transactions =
            hydrate_safe_transactions(&safe_tx_events, &self.context);

        // 9. Duplicate detection
        let resolved = resolve_duplicates(hydrated_deployments, registry, DuplicateStrategy::Skip)?;
        let skipped = resolved.skipped;

        // 10. Record to registry (or build dry-run result)
        let mut recorded_deployments = Vec::new();
        let mut recorded_transactions = Vec::new();

        if !self.context.config.dry_run {
            // Insert new deployments
            for dep in resolved.to_insert {
                registry.insert_deployment(dep.clone())?;
                recorded_deployments.push(RecordedDeployment {
                    deployment: dep,
                    safe_transaction: None,
                });
            }

            // Update existing deployments
            for dep in resolved.to_update {
                registry.update_deployment(dep.clone())?;
                recorded_deployments.push(RecordedDeployment {
                    deployment: dep,
                    safe_transaction: None,
                });
            }

            // Insert transactions
            for tx in transactions {
                registry.insert_transaction(tx.clone())?;
                recorded_transactions.push(RecordedTransaction { transaction: tx });
            }

            // Insert safe transactions
            for safe_tx in safe_transactions {
                registry.insert_safe_transaction(safe_tx)?;
            }
        } else {
            // Dry-run: populate result without writing to registry
            for dep in resolved.to_insert.into_iter().chain(resolved.to_update) {
                recorded_deployments.push(RecordedDeployment {
                    deployment: dep,
                    safe_transaction: None,
                });
            }
            for tx in transactions {
                recorded_transactions.push(RecordedTransaction { transaction: tx });
            }
        }

        Ok(PipelineResult {
            deployments: recorded_deployments,
            transactions: recorded_transactions,
            collisions,
            skipped,
            dry_run: self.context.config.dry_run,
            success: true,
            console_logs: execution.logs,
        })
    }
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
