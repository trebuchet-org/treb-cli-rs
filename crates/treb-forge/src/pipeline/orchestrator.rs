//! Pipeline orchestrator — drives the full deployment recording flow.
//!
//! [`RunPipeline`] sequences compilation, script execution, event decoding,
//! deployment extraction, proxy detection, hydration, duplicate detection,
//! and registry recording into a single `execute` call.

use std::collections::HashMap;
use std::path::Path;

use alloy_primitives::{Address, B256, U256};
use alloy_signer::Signer;
use foundry_config::Config;
use treb_core::error::TrebError;
use treb_registry::Registry;
use treb_safe::types::ProposeRequest;
use treb_safe::{SafeServiceClient, SafeTx, compute_safe_tx_hash, sign_safe_tx};

use crate::artifacts::ArtifactIndex;
use crate::compiler::compile_project;
use crate::events::abi::SimulatedTransaction;
use crate::events::decoder::{ParsedEvent, TrebEvent};
use crate::events::{
    SafeTransactionQueued, TransactionSimulated, decode_events, detect_proxy_relationships,
    extract_collisions, extract_deployments,
};
use crate::script::{ScriptConfig, execute_script};

use super::duplicates::{DuplicateStrategy, resolve_duplicates};
use super::hydration::{
    hydrate_deployment, hydrate_safe_transactions, hydrate_transactions, populate_safe_context,
};
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
    /// 6. For Safe sender: populate safe_context and propose to Safe Service
    /// 7. Run duplicate detection against the registry
    /// 8. Record deployments and transactions (skipped in dry-run mode)
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
        let mut transactions =
            hydrate_transactions(&tx_events, &hydrated_deployments, &self.context);
        let safe_transactions = hydrate_safe_transactions(&safe_tx_events, &self.context);

        // 9. Safe sender: populate safe_context and propose to Safe Service
        let is_safe_sender = self
            .context
            .deployer_sender
            .as_ref()
            .map_or(false, |s| s.is_safe());

        if is_safe_sender {
            // Populate safe_context on Transaction records linked to Safe batches
            populate_safe_context(&mut transactions, &safe_transactions);

            // Propose to Safe Transaction Service (skip in dry-run)
            if !self.context.config.dry_run {
                propose_safe_transactions(
                    &self.context,
                    &safe_tx_events,
                    &tx_events,
                )
                .await?;
            }
        }

        // 10. Duplicate detection
        let resolved =
            resolve_duplicates(hydrated_deployments, registry, DuplicateStrategy::Skip)?;
        let skipped = resolved.skipped;

        // 11. Record to registry (or build dry-run result)
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

            // Insert transactions (with safe_context populated for Safe sender)
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

// ---------------------------------------------------------------------------
// Safe proposal helpers
// ---------------------------------------------------------------------------

/// Build an index of simulated transactions keyed by transaction ID.
fn build_sim_tx_index(tx_events: &[TransactionSimulated]) -> HashMap<B256, &SimulatedTransaction> {
    tx_events
        .iter()
        .flat_map(|event| &event.transactions)
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

    let safe_address = deployer.safe_address().ok_or_else(|| {
        TrebError::Safe("deployer is not a Safe sender".to_string())
    })?;

    let signer = deployer
        .sub_signer()
        .wallet_signer()
        .ok_or_else(|| {
            TrebError::Safe("Safe sender's sub-signer is not a wallet".to_string())
        })?;

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

                client
                    .propose_transaction(&safe_addr_str, &propose_req)
                    .await?;
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
    use alloy_primitives::{Bytes, address, b256};
    use crate::events::abi;

    fn sample_sim_tx(to: Address, value: U256, data: &[u8], tx_id: B256) -> SimulatedTransaction {
        SimulatedTransaction {
            transactionId: tx_id,
            senderId: "deployer".to_string(),
            sender: address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
            returnData: Bytes::new(),
            transaction: abi::Transaction {
                to,
                data: Bytes::from(data.to_vec()),
                value,
            },
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

        let events = vec![TransactionSimulated {
            transactions: vec![
                sample_sim_tx(Address::ZERO, U256::ZERO, &[], tx_id_1),
                sample_sim_tx(Address::ZERO, U256::ZERO, &[], tx_id_2),
            ],
        }];

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
}
