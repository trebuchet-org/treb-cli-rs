//! Deployment hydration — converting forge-domain types into core domain types.
//!
//! The hydration step bridges the gap between [`ExtractedDeployment`] (alloy-primitive
//! types from on-chain events) and the core [`Deployment`] model (string-based,
//! registry-compatible).

use std::collections::HashMap;

use alloy_primitives::{Address, B256, hex};
use chrono::Utc;

pub use treb_core::types::ids::generate_deployment_id;

/// Generate a unique transaction ID for a broadcastable transaction.
///
/// Incorporates the script path so IDs stay unique when compose runs
/// multiple scripts through the pipeline sequentially.
pub fn broadcast_tx_id(script_path: &str, index: usize) -> String {
    let id_input = format!("{}:{}", script_path, index);
    let id_hash = alloy_primitives::keccak256(id_input.as_bytes());
    format!("tx-{:#x}", id_hash)
}
use treb_core::types::{
    deployment::{ArtifactInfo, Deployment, DeploymentStrategy, ProxyInfo, VerificationInfo},
    enums::{DeploymentType, ProposalStatus, TransactionStatus, VerificationStatus},
    governor_proposal::GovernorProposal,
    safe_transaction::SafeTransaction,
    transaction::{Operation, Transaction},
};

use crate::events::{
    abi::{
        GovernorProposalCreated, SafeTransactionQueued, SimulatedTransaction, TransactionSimulated,
    },
    deployments::ExtractedDeployment,
    proxy::{ProxyRelationship, ProxyType},
};

use super::PipelineContext;

/// Convert a [`B256`] value to a `0x`-prefixed lowercase hex string.
///
/// Returns an empty string when all bytes are zero.
fn b256_to_hex(value: B256) -> String {
    if value == B256::ZERO { String::new() } else { format!("{:#x}", value) }
}

/// Convert a forge-domain [`ExtractedDeployment`] into a core-domain [`Deployment`].
///
/// The `proxy` parameter, when provided, sets `deployment_type` to [`DeploymentType::Proxy`]
/// and populates [`ProxyInfo`]. The `context` supplies namespace, chain ID, script path,
/// and git commit for the artifact metadata.
pub fn hydrate_deployment(
    extracted: &ExtractedDeployment,
    proxy: Option<&ProxyRelationship>,
    context: &PipelineContext,
) -> Deployment {
    let now = Utc::now();
    let namespace = &context.config.namespace;
    let chain_id = context.config.chain_id;

    let id =
        generate_deployment_id(namespace, chain_id, &extracted.contract_name, &extracted.label);

    let deployment_type =
        if proxy.is_some() { DeploymentType::Proxy } else { DeploymentType::Singleton };

    let deployment_strategy = DeploymentStrategy {
        method: extracted.strategy.clone(),
        salt: b256_to_hex(extracted.salt),
        init_code_hash: b256_to_hex(extracted.init_code_hash),
        factory: String::new(),
        constructor_args: if extracted.constructor_args.is_empty() {
            String::new()
        } else {
            format!("{}", extracted.constructor_args)
        },
        entropy: extracted.entropy.clone(),
    };

    let proxy_info = proxy.map(|p| {
        let proxy_type_str = match p.proxy_type {
            ProxyType::Transparent => "Transparent",
            ProxyType::UUPS => "UUPS",
            ProxyType::Beacon => "Beacon",
            ProxyType::Minimal => "Minimal",
        };
        ProxyInfo {
            proxy_type: proxy_type_str.to_string(),
            implementation: p.implementation.map(|addr| addr.to_checksum(None)).unwrap_or_default(),
            admin: p.admin.map(|addr| addr.to_checksum(None)).unwrap_or_default(),
            history: Vec::new(),
        }
    });

    let artifact = match &extracted.artifact_match {
        Some(am) => ArtifactInfo {
            path: am.artifact_id.source.to_string_lossy().to_string(),
            compiler_version: String::new(),
            bytecode_hash: b256_to_hex(extracted.bytecode_hash),
            script_path: context.config.script_path.clone(),
            git_commit: context.git_commit.clone(),
        },
        None => ArtifactInfo {
            path: String::new(),
            compiler_version: String::new(),
            bytecode_hash: b256_to_hex(extracted.bytecode_hash),
            script_path: context.config.script_path.clone(),
            git_commit: context.git_commit.clone(),
        },
    };

    let verification = VerificationInfo {
        status: VerificationStatus::Unverified,
        etherscan_url: String::new(),
        verified_at: None,
        reason: String::new(),
        verifiers: HashMap::new(),
    };

    Deployment {
        id,
        namespace: namespace.clone(),
        chain_id,
        contract_name: extracted.contract_name.clone(),
        label: extracted.label.clone(),
        address: extracted.address.to_checksum(None),
        deployment_type,
        transaction_id: format!("tx-{:#x}", extracted.transaction_id),
        deployment_strategy,
        proxy_info,
        artifact,
        verification,
        tags: None,
        created_at: now,
        updated_at: now,
    }
}

/// Convert an [`ExtractedCollision`] into a minimal [`Deployment`] for registry lookups.
///
/// Used by compose to register collisions (contracts that already exist at the
/// predicted address) so that `lookup()` finds them in subsequent steps.
pub fn hydrate_collision(
    collision: &crate::events::ExtractedCollision,
    context: &PipelineContext,
) -> Deployment {
    let now = Utc::now();
    let namespace = &context.config.namespace;
    let chain_id = context.config.chain_id;
    let id = generate_deployment_id(namespace, chain_id, &collision.contract_name, &collision.label);

    Deployment {
        id,
        namespace: namespace.clone(),
        chain_id,
        contract_name: collision.contract_name.clone(),
        label: collision.label.clone(),
        address: collision.existing_address.to_checksum(None),
        deployment_type: DeploymentType::Singleton,
        transaction_id: String::new(),
        deployment_strategy: DeploymentStrategy {
            method: collision.strategy.clone(),
            salt: b256_to_hex(collision.salt),
            init_code_hash: b256_to_hex(collision.init_code_hash),
            factory: String::new(),
            constructor_args: String::new(),
            entropy: collision.entropy.clone(),
        },
        proxy_info: None,
        artifact: ArtifactInfo {
            path: String::new(),
            compiler_version: String::new(),
            bytecode_hash: b256_to_hex(collision.bytecode_hash),
            script_path: context.config.script_path.clone(),
            git_commit: context.git_commit.clone(),
        },
        verification: VerificationInfo {
            status: VerificationStatus::Unverified,
            etherscan_url: String::new(),
            verified_at: None,
            reason: String::new(),
            verifiers: HashMap::new(),
        },
        tags: None,
        created_at: now,
        updated_at: now,
    }
}

/// Convert [`TransactionSimulated`] events into core-domain [`Transaction`] records.
///
/// Each `SimulatedTransaction` within the event produces one `Transaction`.
/// The `deployments` field is populated by matching `transaction_id` against
/// the already-hydrated `Deployment` list.
pub fn hydrate_transactions(
    events: &[TransactionSimulated],
    hydrated_deployments: &[Deployment],
    context: &PipelineContext,
) -> Vec<Transaction> {
    let now = Utc::now();

    events
        .iter()
        .map(|event| &event.simulatedTx)
        .map(|sim_tx| {
            let tx_id_hex = format!("tx-{:#x}", sim_tx.transactionId);

            // Find deployment IDs that belong to this transaction.
            let linked_deployments: Vec<&Deployment> =
                hydrated_deployments.iter().filter(|d| d.transaction_id == tx_id_hex).collect();
            let linked_deployment_ids: Vec<String> =
                linked_deployments.iter().map(|d| d.id.clone()).collect();

            Transaction {
                id: tx_id_hex,
                chain_id: context.config.chain_id,
                hash: String::new(),
                status: TransactionStatus::Simulated,
                block_number: 0,
                sender: sim_tx.sender.to_checksum(None),
                nonce: 0,
                deployments: linked_deployment_ids,
                operations: hydrate_transaction_operations(sim_tx, &linked_deployments),
                safe_context: None,
                environment: context.config.namespace.clone(),
                created_at: now,
            }
        })
        .collect()
}

fn hydrate_transaction_operations(
    sim_tx: &SimulatedTransaction,
    linked_deployments: &[&Deployment],
) -> Vec<Operation> {
    if !linked_deployments.is_empty() {
        return linked_deployments
            .iter()
            .map(|deployment| Operation {
                operation_type: "DEPLOY".to_string(),
                target: deployment.address.clone(),
                method: deployment.deployment_strategy.method.to_string(),
                result: {
                    let mut result = HashMap::new();
                    result.insert(
                        "address".to_string(),
                        serde_json::Value::String(deployment.address.clone()),
                    );
                    result
                },
            })
            .collect();
    }

    let target = if sim_tx.transaction.to == Address::ZERO {
        "contract creation".to_string()
    } else {
        sim_tx.transaction.to.to_checksum(None)
    };

    vec![Operation {
        operation_type: if sim_tx.transaction.to == Address::ZERO {
            "DEPLOY".to_string()
        } else {
            "CALL".to_string()
        },
        target,
        method: selector_hex(&sim_tx.transaction.data),
        result: HashMap::new(),
    }]
}

fn selector_hex(data: &[u8]) -> String {
    if data.len() < 4 { String::new() } else { format!("0x{}", hex::encode(&data[..4])) }
}

/// Convert [`SafeTransactionQueued`] events into core-domain [`SafeTransaction`] stubs.
///
/// Each event produces a `SafeTransaction` with `Queued` status. The
/// `transaction_ids` field contains the hex strings of all linked transaction IDs.
pub fn hydrate_safe_transactions(
    events: &[SafeTransactionQueued],
    context: &PipelineContext,
) -> Vec<SafeTransaction> {
    let now = Utc::now();

    events
        .iter()
        .map(|event| {
            let tx_ids: Vec<String> =
                event.transactionIds.iter().map(|id| format!("tx-{:#x}", id)).collect();

            SafeTransaction {
                safe_tx_hash: format!("{:#x}", event.safeTxHash),
                safe_address: event.safe.to_checksum(None),
                chain_id: context.config.chain_id,
                status: TransactionStatus::Queued,
                nonce: 0,
                transactions: Vec::new(),
                transaction_ids: tx_ids,
                proposed_by: event.proposer.to_checksum(None),
                proposed_at: now,
                confirmations: Vec::new(),
                executed_at: None,
                execution_tx_hash: String::new(),
            }
        })
        .collect()
}

/// Populate `safe_context` on transactions that belong to a Safe batch.
///
/// For each `SafeTransaction`, this sets `safe_context` on all `Transaction`
/// records whose IDs appear in the Safe transaction's `transaction_ids`.
/// The `batch_index` is the position of the transaction within the Safe batch.
pub fn populate_safe_context(
    transactions: &mut [Transaction],
    safe_transactions: &[SafeTransaction],
) {
    for safe_tx in safe_transactions {
        for (batch_index, tx_id) in safe_tx.transaction_ids.iter().enumerate() {
            if let Some(tx) = transactions.iter_mut().find(|t| t.id == *tx_id) {
                tx.safe_context = Some(treb_core::types::transaction::SafeContext {
                    safe_address: safe_tx.safe_address.clone(),
                    safe_tx_hash: safe_tx.safe_tx_hash.clone(),
                    batch_index: batch_index as i64,
                    proposer_address: safe_tx.proposed_by.clone(),
                });
            }
        }
    }
}

/// Convert [`GovernorProposalCreated`] events into core-domain [`GovernorProposal`] records.
///
/// Each event produces a `GovernorProposal` with `Pending` status. The
/// `proposal_id` is the hex representation of the event's `proposalId`,
/// `governor_address` and `proposed_by` come from the event's indexed fields,
/// and `transaction_ids` contains the hex strings of all linked transaction IDs.
///
/// The `timelock_address` is populated from the deployer sender if available.
pub fn hydrate_governor_proposals(
    events: &[GovernorProposalCreated],
    context: &PipelineContext,
) -> Vec<GovernorProposal> {
    let now = Utc::now();

    // Extract timelock address from the deployer sender if it is a Governor.
    let timelock_str = context
        .resolved_senders
        .get("deployer")
        .and_then(|s| s.timelock_address())
        .map(|addr| addr.to_checksum(None))
        .unwrap_or_default();

    events
        .iter()
        .map(|event| {
            let tx_ids: Vec<String> =
                event.transactionIds.iter().map(|id| format!("tx-{:#x}", id)).collect();

            GovernorProposal {
                proposal_id: format!("{}", event.proposalId),
                governor_address: event.governor.to_checksum(None),
                timelock_address: timelock_str.clone(),
                chain_id: context.config.chain_id,
                status: ProposalStatus::Pending,
                transaction_ids: tx_ids,
                proposed_by: event.proposer.to_checksum(None),
                proposed_at: now,
                description: String::new(),
                executed_at: None,
                execution_tx_hash: String::new(),
            }
        })
        .collect()
}

/// Convert [`BroadcastableTransactions`] into core-domain [`Transaction`] records.
///
/// In v2, scripts use `vm.broadcast()` instead of emitting `TransactionSimulated`
/// events. This function builds `Transaction` records directly from the collected
/// broadcastable transactions, using the `from` address to resolve sender names
/// via the address-to-role reverse map.
pub fn hydrate_transactions_from_broadcast(
    broadcastable_txs: &foundry_cheatcodes::BroadcastableTransactions,
    hydrated_deployments: &[Deployment],
    context: &PipelineContext,
) -> Vec<Transaction> {
    let now = Utc::now();

    broadcastable_txs
        .iter()
        .enumerate()
        .map(|(idx, btx)| {
            let from_addr = btx.transaction.from().unwrap_or_default();
            let to_addr = btx.transaction.to().and_then(|kind| match kind {
                alloy_primitives::TxKind::Call(addr) => Some(addr),
                alloy_primitives::TxKind::Create => None,
            });
            let input = btx.transaction.input().cloned().unwrap_or_default();

            let tx_id = broadcast_tx_id(&context.config.script_path, idx);

            // Find deployments linked to this transaction by index.
            // v2 ContractDeployed events use sequential transactionId = bytes32(uint256(i)).
            let linked_deployment_ids: Vec<String> = hydrated_deployments
                .iter()
                .filter(|d| d.transaction_id == tx_id)
                .map(|d| d.id.clone())
                .collect();

            let is_create = to_addr.is_none();

            let operations = if !linked_deployment_ids.is_empty() {
                hydrated_deployments
                    .iter()
                    .filter(|d| d.transaction_id == tx_id)
                    .map(|deployment| Operation {
                        operation_type: "DEPLOY".to_string(),
                        target: deployment.address.clone(),
                        method: deployment.deployment_strategy.method.to_string(),
                        result: {
                            let mut result = HashMap::new();
                            result.insert(
                                "address".to_string(),
                                serde_json::Value::String(deployment.address.clone()),
                            );
                            result
                        },
                    })
                    .collect()
            } else {
                let target = if is_create {
                    "contract creation".to_string()
                } else {
                    to_addr.unwrap().to_checksum(None)
                };
                vec![Operation {
                    operation_type: if is_create { "DEPLOY" } else { "CALL" }.to_string(),
                    target,
                    method: selector_hex(&input),
                    result: HashMap::new(),
                }]
            };

            Transaction {
                id: tx_id,
                chain_id: context.config.chain_id,
                hash: String::new(),
                status: TransactionStatus::Simulated,
                block_number: 0,
                sender: from_addr.to_checksum(None),
                nonce: btx.transaction.nonce().unwrap_or(0),
                deployments: linked_deployment_ids,
                operations,
                safe_context: None,
                environment: context.config.namespace.clone(),
                created_at: now,
            }
        })
        .collect()
}

/// Metadata for a v2 recorded transaction, built from BroadcastableTransactions.
pub struct V2TransactionMetadata {
    /// Sender role name resolved from the `from` address.
    pub sender_name: Option<String>,
    /// Gas used (from broadcastable transaction gas estimate).
    pub gas_used: Option<u64>,
}

/// Build metadata for v2 transactions from BroadcastableTransactions.
pub fn build_v2_transaction_metadata(
    broadcastable_txs: &foundry_cheatcodes::BroadcastableTransactions,
    context: &PipelineContext,
) -> HashMap<String, V2TransactionMetadata> {
    let addr_to_role: HashMap<Address, String> = context
        .sender_labels
        .iter()
        .map(|(addr, role)| (*addr, role.clone()))
        .collect();

    broadcastable_txs
        .iter()
        .enumerate()
        .map(|(idx, btx)| {
            let from_addr = btx.transaction.from().unwrap_or_default();
            let tx_id = broadcast_tx_id(&context.config.script_path, idx);
            let sender_name = addr_to_role.get(&from_addr).cloned();
            let gas_used = btx.transaction.gas().map(|g| g as u64);
            (tx_id, V2TransactionMetadata { sender_name, gas_used })
        })
        .collect()
}

/// Apply broadcast receipts to already-hydrated recorded transactions.
///
/// Updates each transaction's hash, block number, status, and gas based on
/// the corresponding receipt. Receipts and transactions must be in the same
/// order (aligned by index).
pub fn apply_receipts(
    transactions: &mut [super::types::RecordedTransaction],
    receipts: &[crate::script::BroadcastReceipt],
) {
    use treb_core::types::enums::TransactionStatus;

    for (rt, receipt) in transactions.iter_mut().zip(receipts.iter()) {
        rt.transaction.hash = format!("{:#x}", receipt.hash);
        rt.transaction.block_number = receipt.block_number;
        rt.transaction.status = if receipt.status {
            TransactionStatus::Executed
        } else {
            TransactionStatus::Failed
        };
        if receipt.gas_used > 0 {
            rt.gas_used = Some(receipt.gas_used);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Bytes, address, b256};
    use std::path::PathBuf;
    use treb_core::types::enums::DeploymentMethod;

    use crate::{events::proxy::ProxyRelationship, pipeline::PipelineConfig};

    fn test_context() -> PipelineContext {
        PipelineContext {
            config: PipelineConfig {
                script_path: "script/Deploy.s.sol".to_string(),
                namespace: "production".to_string(),
                chain_id: 1,
                ..PipelineConfig::default()
            },
            script_path: PathBuf::from("script/Deploy.s.sol"),
            git_commit: "abc1234".to_string(),
            project_root: PathBuf::from("/tmp/project"),
            resolved_senders: Default::default(),
            sender_labels: Default::default(),
            sender_role_names: Default::default(),
            sender_configs: std::collections::HashMap::new(),
        }
    }

    fn create_deployment() -> ExtractedDeployment {
        ExtractedDeployment {
            address: address!("5FbDB2315678afecb367f032d93F642f64180aa3"),
            deployer: address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
            transaction_id: b256!(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            ),
            contract_name: "Counter".to_string(),
            label: "counter-v1".to_string(),
            strategy: DeploymentMethod::Create,
            salt: B256::ZERO,
            bytecode_hash: b256!(
                "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
            ),
            init_code_hash: B256::ZERO,
            constructor_args: Bytes::from(vec![0x00, 0x01, 0x02]),
            entropy: String::new(),
            artifact_match: None,
        }
    }

    fn create2_deployment() -> ExtractedDeployment {
        ExtractedDeployment {
            address: address!("e7f1725E7734CE288F8367e1Bb143E90bb3F0512"),
            deployer: address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
            transaction_id: b256!(
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            ),
            contract_name: "Token".to_string(),
            label: "token-v1".to_string(),
            strategy: DeploymentMethod::Create2,
            salt: b256!("0000000000000000000000000000000000000000000000000000000000000001"),
            bytecode_hash: b256!(
                "abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd"
            ),
            init_code_hash: b256!(
                "1111111111111111111111111111111111111111111111111111111111111111"
            ),
            constructor_args: Bytes::new(),
            entropy: "my-entropy".to_string(),
            artifact_match: None,
        }
    }

    #[test]
    fn generate_deployment_id_correct_format() {
        let id = generate_deployment_id("production", 1, "Counter", "v1");
        assert_eq!(id, "production/1/Counter:v1");

        let id = generate_deployment_id("staging", 11155111, "Token", "token-main");
        assert_eq!(id, "staging/11155111/Token:token-main");
    }

    #[test]
    fn hydrate_create_deployment_all_fields() {
        let ctx = test_context();
        let extracted = create_deployment();
        let deployment = hydrate_deployment(&extracted, None, &ctx);

        // ID format
        assert_eq!(deployment.id, "production/1/Counter:counter-v1");
        assert_eq!(deployment.namespace, "production");
        assert_eq!(deployment.chain_id, 1);
        assert_eq!(deployment.contract_name, "Counter");
        assert_eq!(deployment.label, "counter-v1");

        // Address is checksummed
        assert_eq!(deployment.address, "0x5FbDB2315678afecb367f032d93F642f64180aa3");

        // Type is Singleton (no proxy)
        assert_eq!(deployment.deployment_type, DeploymentType::Singleton);

        // Transaction ID is tx-0x-prefixed hex
        assert_eq!(
            deployment.transaction_id,
            "tx-0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );

        // Strategy fields
        assert_eq!(deployment.deployment_strategy.method, DeploymentMethod::Create);
        assert!(deployment.deployment_strategy.salt.is_empty(), "zero salt should be empty");
        assert!(
            deployment.deployment_strategy.init_code_hash.is_empty(),
            "zero init_code_hash should be empty"
        );
        assert!(!deployment.deployment_strategy.constructor_args.is_empty());

        // Bytecode hash in artifact
        assert_eq!(
            deployment.artifact.bytecode_hash,
            "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
        );

        // Artifact fields from context
        assert_eq!(deployment.artifact.script_path, "script/Deploy.s.sol");
        assert_eq!(deployment.artifact.git_commit, "abc1234");

        // No proxy info
        assert!(deployment.proxy_info.is_none());

        // Verification defaults to Unverified
        assert_eq!(deployment.verification.status, VerificationStatus::Unverified);
        assert!(deployment.verification.etherscan_url.is_empty());
        assert!(deployment.verification.verified_at.is_none());

        // Tags are None
        assert!(deployment.tags.is_none());
    }

    #[test]
    fn hydrate_create2_deployment_with_salt() {
        let ctx = test_context();
        let extracted = create2_deployment();
        let deployment = hydrate_deployment(&extracted, None, &ctx);

        assert_eq!(deployment.id, "production/1/Token:token-v1");
        assert_eq!(deployment.deployment_strategy.method, DeploymentMethod::Create2);

        // Salt is non-zero, should be 0x-prefixed hex
        assert_eq!(
            deployment.deployment_strategy.salt,
            "0x0000000000000000000000000000000000000000000000000000000000000001"
        );

        // Init code hash is non-zero
        assert_eq!(
            deployment.deployment_strategy.init_code_hash,
            "0x1111111111111111111111111111111111111111111111111111111111111111"
        );

        // Constructor args empty
        assert!(deployment.deployment_strategy.constructor_args.is_empty());

        // Entropy
        assert_eq!(deployment.deployment_strategy.entropy, "my-entropy");
    }

    #[test]
    fn hydrate_proxy_deployment_transparent() {
        let ctx = test_context();
        let extracted = create_deployment();
        let proxy = ProxyRelationship {
            proxy_address: address!("5FbDB2315678afecb367f032d93F642f64180aa3"),
            proxy_type: ProxyType::Transparent,
            implementation: Some(address!("9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0")),
            admin: Some(address!("70997970C51812dc3A010C7d01b50e0d17dc79C8")),
            beacon: None,
        };

        let deployment = hydrate_deployment(&extracted, Some(&proxy), &ctx);

        // Type is Proxy
        assert_eq!(deployment.deployment_type, DeploymentType::Proxy);

        // Proxy info populated
        let proxy_info = deployment.proxy_info.as_ref().expect("proxy_info should be Some");
        assert_eq!(proxy_info.proxy_type, "Transparent");
        assert_eq!(proxy_info.implementation, "0x9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0");
        assert_eq!(proxy_info.admin, "0x70997970C51812dc3A010C7d01b50e0d17dc79C8");
        assert!(proxy_info.history.is_empty());
    }

    #[test]
    fn hydrate_deployment_without_artifact_match() {
        let ctx = test_context();
        let extracted = create_deployment();
        let deployment = hydrate_deployment(&extracted, None, &ctx);

        // Path and compiler_version should be empty when no artifact match
        assert!(deployment.artifact.path.is_empty());
        assert!(deployment.artifact.compiler_version.is_empty());

        // But bytecode_hash, script_path, git_commit still populated
        assert!(!deployment.artifact.bytecode_hash.is_empty());
        assert_eq!(deployment.artifact.script_path, "script/Deploy.s.sol");
        assert_eq!(deployment.artifact.git_commit, "abc1234");
    }

    #[test]
    fn hydrate_deployment_with_artifact_match() {
        let ctx = test_context();
        let mut extracted = create_deployment();

        // Create an ArtifactMatch with a known path
        use foundry_compilers::ArtifactId;
        let artifact_match = crate::artifacts::ArtifactMatch {
            artifact_id: ArtifactId {
                path: PathBuf::from("out/Counter.sol/Counter.json"),
                name: "Counter".to_string(),
                source: PathBuf::from("src/Counter.sol"),
                version: foundry_config::semver::Version::new(0, 8, 24),
                build_id: String::new(),
                profile: "default".to_string(),
            },
            name: "Counter".to_string(),
            abi: alloy_json_abi::JsonAbi::new(),
            has_bytecode: true,
            has_deployed_bytecode: true,
            is_library: false,
        };
        extracted.artifact_match = Some(artifact_match);

        let deployment = hydrate_deployment(&extracted, None, &ctx);
        assert_eq!(deployment.artifact.path, "src/Counter.sol");
    }

    #[test]
    fn b256_zero_produces_empty_string() {
        assert!(b256_to_hex(B256::ZERO).is_empty());
    }

    #[test]
    fn b256_nonzero_produces_0x_prefixed_hex() {
        let value = b256!("0000000000000000000000000000000000000000000000000000000000000001");
        let hex = b256_to_hex(value);
        assert!(hex.starts_with("0x"), "should start with 0x prefix");
        assert_eq!(hex, "0x0000000000000000000000000000000000000000000000000000000000000001");
    }

    // -----------------------------------------------------------------------
    // Transaction hydration tests
    // -----------------------------------------------------------------------

    use crate::events::abi::SimulatedTransaction;
    use alloy_primitives::{U256, keccak256};

    /// Helper: build a hydrated Deployment with specific id and transaction_id.
    fn mock_deployment(id: &str, transaction_id: &str) -> Deployment {
        Deployment {
            id: id.to_string(),
            namespace: "production".to_string(),
            chain_id: 1,
            contract_name: "Counter".to_string(),
            label: "v1".to_string(),
            address: "0x5FbDB2315678afecb367f032d93F642f64180aa3".to_string(),
            deployment_type: DeploymentType::Singleton,
            transaction_id: transaction_id.to_string(),
            deployment_strategy: DeploymentStrategy {
                method: DeploymentMethod::Create,
                salt: String::new(),
                init_code_hash: String::new(),
                factory: String::new(),
                constructor_args: String::new(),
                entropy: String::new(),
            },
            proxy_info: None,
            artifact: ArtifactInfo {
                path: String::new(),
                compiler_version: String::new(),
                bytecode_hash: String::new(),
                script_path: String::new(),
                git_commit: String::new(),
            },
            verification: VerificationInfo {
                status: VerificationStatus::Unverified,
                etherscan_url: String::new(),
                verified_at: None,
                reason: String::new(),
                verifiers: HashMap::new(),
            },
            tags: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn hydrate_single_transaction_with_two_deployments() {
        let ctx = test_context();
        let tx_id = b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let tx_id_hex = "tx-0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

        let events = vec![TransactionSimulated {
            simulatedTx: SimulatedTransaction {
                transactionId: tx_id,
                senderId: keccak256(b"deployer"),
                sender: address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
                returnData: Bytes::new(),
                gasUsed: U256::ZERO,
                transaction: crate::events::abi::Transaction {
                    to: address!("5FbDB2315678afecb367f032d93F642f64180aa3"),
                    data: Bytes::new(),
                    value: U256::ZERO,
                },
            },
        }];

        let deployments = vec![
            mock_deployment("production/1/Counter:v1", tx_id_hex),
            mock_deployment("production/1/Token:v1", tx_id_hex),
        ];

        let transactions = hydrate_transactions(&events, &deployments, &ctx);
        assert_eq!(transactions.len(), 1);

        let tx = &transactions[0];
        assert_eq!(tx.id, tx_id_hex);
        assert_eq!(tx.chain_id, 1);
        assert_eq!(tx.status, TransactionStatus::Simulated);
        assert_eq!(tx.sender, "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
        assert_eq!(tx.environment, "production");
        assert!(tx.hash.is_empty());
        assert_eq!(tx.block_number, 0);
        assert_eq!(tx.nonce, 0);
        assert!(tx.safe_context.is_none());
        assert_eq!(tx.operations.len(), 2);
        assert_eq!(tx.operations[0].operation_type, "DEPLOY");
        assert_eq!(tx.operations[0].method, "CREATE");

        // Both deployments should be linked
        assert_eq!(tx.deployments.len(), 2);
        assert!(tx.deployments.contains(&"production/1/Counter:v1".to_string()));
        assert!(tx.deployments.contains(&"production/1/Token:v1".to_string()));
    }

    #[test]
    fn hydrate_call_transaction_adds_call_operation() {
        let ctx = test_context();
        let tx_id = b256!("dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd");

        let events = vec![TransactionSimulated {
            simulatedTx: SimulatedTransaction {
                transactionId: tx_id,
                senderId: keccak256(b"governance"),
                sender: address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
                returnData: Bytes::new(),
                gasUsed: U256::ZERO,
                transaction: crate::events::abi::Transaction {
                    to: address!("0000000000000000000000000000000000001000"),
                    data: Bytes::from(vec![0xde, 0xad, 0xbe, 0xef, 0x01]),
                    value: U256::ZERO,
                },
            },
        }];

        let transactions = hydrate_transactions(&events, &[], &ctx);
        let tx = &transactions[0];

        assert_eq!(tx.operations.len(), 1);
        assert_eq!(tx.operations[0].operation_type, "CALL");
        assert_eq!(tx.operations[0].target, "0x0000000000000000000000000000000000001000");
        assert_eq!(tx.operations[0].method, "0xdeadbeef");
    }

    #[test]
    fn hydrate_safe_transaction_queued() {
        let ctx = test_context();
        let safe_tx_hash =
            b256!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        let tx_id_1 = b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let tx_id_2 = b256!("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc");

        let events = vec![SafeTransactionQueued {
            safeTxHash: safe_tx_hash,
            safe: address!("5FbDB2315678afecb367f032d93F642f64180aa3"),
            proposer: address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
            transactionIds: vec![tx_id_1, tx_id_2],
        }];

        let safe_txs = hydrate_safe_transactions(&events, &ctx);
        assert_eq!(safe_txs.len(), 1);

        let stx = &safe_txs[0];
        assert_eq!(
            stx.safe_tx_hash,
            "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        );
        assert_eq!(stx.safe_address, "0x5FbDB2315678afecb367f032d93F642f64180aa3");
        assert_eq!(stx.chain_id, 1);
        assert_eq!(stx.status, TransactionStatus::Queued);
        assert_eq!(stx.nonce, 0);
        assert!(stx.transactions.is_empty());
        assert_eq!(stx.proposed_by, "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
        assert!(stx.confirmations.is_empty());
        assert!(stx.executed_at.is_none());
        assert!(stx.execution_tx_hash.is_empty());

        // Transaction IDs should be tx-0x-prefixed hex strings
        assert_eq!(stx.transaction_ids.len(), 2);
        assert_eq!(
            stx.transaction_ids[0],
            "tx-0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(
            stx.transaction_ids[1],
            "tx-0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
        );
    }

    #[test]
    fn hydrate_empty_events_produce_empty_results() {
        let ctx = test_context();

        let transactions = hydrate_transactions(&[], &[], &ctx);
        assert!(transactions.is_empty());

        let safe_txs = hydrate_safe_transactions(&[], &ctx);
        assert!(safe_txs.is_empty());
    }

    // -----------------------------------------------------------------------
    // populate_safe_context tests
    // -----------------------------------------------------------------------

    #[test]
    fn populate_safe_context_links_transactions_to_safe_batch() {
        let ctx = test_context();
        let tx_id_1 = b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let tx_id_2 = b256!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");

        // Create two simulated transactions (one event per transaction)
        let events = vec![
            TransactionSimulated {
                simulatedTx: SimulatedTransaction {
                    transactionId: tx_id_1,
                    senderId: keccak256(b"deployer"),
                    sender: address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
                    returnData: Bytes::new(),
                    gasUsed: U256::ZERO,
                    transaction: crate::events::abi::Transaction {
                        to: address!("5FbDB2315678afecb367f032d93F642f64180aa3"),
                        data: Bytes::new(),
                        value: U256::ZERO,
                    },
                },
            },
            TransactionSimulated {
                simulatedTx: SimulatedTransaction {
                    transactionId: tx_id_2,
                    senderId: keccak256(b"deployer"),
                    sender: address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
                    returnData: Bytes::new(),
                    gasUsed: U256::ZERO,
                    transaction: crate::events::abi::Transaction {
                        to: address!("e7f1725E7734CE288F8367e1Bb143E90bb3F0512"),
                        data: Bytes::new(),
                        value: U256::ZERO,
                    },
                },
            },
        ];

        let mut transactions = hydrate_transactions(&events, &[], &ctx);
        assert_eq!(transactions.len(), 2);
        // Both should have no safe_context initially
        assert!(transactions[0].safe_context.is_none());
        assert!(transactions[1].safe_context.is_none());

        // Create a SafeTransaction that groups both transactions
        let safe_txs = vec![SafeTransaction {
            safe_tx_hash: "0xsafehash".to_string(),
            safe_address: "0x0000000000000000000000000000000000000042".to_string(),
            chain_id: 1,
            status: TransactionStatus::Queued,
            nonce: 0,
            transactions: Vec::new(),
            transaction_ids: vec![format!("tx-{:#x}", tx_id_1), format!("tx-{:#x}", tx_id_2)],
            proposed_by: "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".to_string(),
            proposed_at: chrono::Utc::now(),
            confirmations: Vec::new(),
            executed_at: None,
            execution_tx_hash: String::new(),
        }];

        // Populate safe_context
        populate_safe_context(&mut transactions, &safe_txs);

        // Transaction 1 should have batch_index 0
        let ctx1 = transactions[0].safe_context.as_ref().expect("should have safe_context");
        assert_eq!(ctx1.safe_address, "0x0000000000000000000000000000000000000042");
        assert_eq!(ctx1.safe_tx_hash, "0xsafehash");
        assert_eq!(ctx1.batch_index, 0);
        assert_eq!(ctx1.proposer_address, "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");

        // Transaction 2 should have batch_index 1
        let ctx2 = transactions[1].safe_context.as_ref().expect("should have safe_context");
        assert_eq!(ctx2.batch_index, 1);
        assert_eq!(ctx2.safe_tx_hash, "0xsafehash");
    }

    #[test]
    fn populate_safe_context_skips_unlinked_transactions() {
        let ctx = test_context();
        let tx_id = b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");

        let events = vec![TransactionSimulated {
            simulatedTx: SimulatedTransaction {
                transactionId: tx_id,
                senderId: keccak256(b"deployer"),
                sender: address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
                returnData: Bytes::new(),
                gasUsed: U256::ZERO,
                transaction: crate::events::abi::Transaction {
                    to: address!("5FbDB2315678afecb367f032d93F642f64180aa3"),
                    data: Bytes::new(),
                    value: U256::ZERO,
                },
            },
        }];

        let mut transactions = hydrate_transactions(&events, &[], &ctx);

        // SafeTransaction references a different transaction ID
        let safe_txs = vec![SafeTransaction {
            safe_tx_hash: "0xsafehash".to_string(),
            safe_address: "0x0000000000000000000000000000000000000042".to_string(),
            chain_id: 1,
            status: TransactionStatus::Queued,
            nonce: 0,
            transactions: Vec::new(),
            transaction_ids: vec!["0xdeadbeef".to_string()],
            proposed_by: "0xProposer".to_string(),
            proposed_at: chrono::Utc::now(),
            confirmations: Vec::new(),
            executed_at: None,
            execution_tx_hash: String::new(),
        }];

        populate_safe_context(&mut transactions, &safe_txs);

        // Transaction should NOT have safe_context since its ID doesn't match
        assert!(transactions[0].safe_context.is_none());
    }

    #[test]
    fn populate_safe_context_no_safe_transactions() {
        let ctx = test_context();
        let tx_id = b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");

        let events = vec![TransactionSimulated {
            simulatedTx: SimulatedTransaction {
                transactionId: tx_id,
                senderId: keccak256(b"deployer"),
                sender: address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
                returnData: Bytes::new(),
                gasUsed: U256::ZERO,
                transaction: crate::events::abi::Transaction {
                    to: address!("5FbDB2315678afecb367f032d93F642f64180aa3"),
                    data: Bytes::new(),
                    value: U256::ZERO,
                },
            },
        }];

        let mut transactions = hydrate_transactions(&events, &[], &ctx);
        populate_safe_context(&mut transactions, &[]);

        // No safe_context should be populated
        assert!(transactions[0].safe_context.is_none());
    }

    // -----------------------------------------------------------------------
    // hydrate_governor_proposals tests
    // -----------------------------------------------------------------------

    use crate::sender::{ResolvedSender, in_memory_signer};

    #[test]
    fn hydrate_governor_proposal_maps_fields_correctly() {
        let ctx = test_context();
        let proposal_id = U256::from(12345u64);
        let governor = address!("0000000000000000000000000000000000000099");
        let proposer = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
        let tx_id_1 = b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let tx_id_2 = b256!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");

        let events = vec![GovernorProposalCreated {
            proposalId: proposal_id,
            governor,
            proposer,
            transactionIds: vec![tx_id_1, tx_id_2],
        }];

        let proposals = hydrate_governor_proposals(&events, &ctx);
        assert_eq!(proposals.len(), 1);

        let p = &proposals[0];
        assert_eq!(p.proposal_id, format!("{}", proposal_id));
        assert_eq!(p.governor_address, governor.to_checksum(None));
        assert_eq!(p.chain_id, 1);
        assert_eq!(p.status, ProposalStatus::Pending);
        assert_eq!(p.proposed_by, proposer.to_checksum(None));
        assert!(p.description.is_empty());
        assert!(p.executed_at.is_none());
        assert!(p.execution_tx_hash.is_empty());

        // Transaction IDs should be tx-0x-prefixed hex strings
        assert_eq!(p.transaction_ids.len(), 2);
        assert_eq!(
            p.transaction_ids[0],
            "tx-0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(
            p.transaction_ids[1],
            "tx-0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        );
    }

    #[test]
    fn hydrate_governor_proposal_with_timelock_from_sender() {
        let timelock_addr = address!("0000000000000000000000000000000000000088");
        let mut ctx = test_context();
        ctx.resolved_senders.insert("deployer".to_string(), ResolvedSender::Governor {
            governor_address: address!("0000000000000000000000000000000000000099"),
            timelock_address: Some(timelock_addr),
            proposer: Box::new(ResolvedSender::Wallet(in_memory_signer(0).unwrap())),
        });

        let events = vec![GovernorProposalCreated {
            proposalId: U256::from(1u64),
            governor: address!("0000000000000000000000000000000000000099"),
            proposer: address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
            transactionIds: vec![b256!(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            )],
        }];

        let proposals = hydrate_governor_proposals(&events, &ctx);
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].timelock_address, timelock_addr.to_checksum(None));
    }

    #[test]
    fn hydrate_governor_proposal_without_timelock() {
        let ctx = test_context();

        let events = vec![GovernorProposalCreated {
            proposalId: U256::from(1u64),
            governor: address!("0000000000000000000000000000000000000099"),
            proposer: address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
            transactionIds: vec![],
        }];

        let proposals = hydrate_governor_proposals(&events, &ctx);
        assert_eq!(proposals.len(), 1);
        assert!(proposals[0].timelock_address.is_empty());
    }

    #[test]
    fn hydrate_governor_proposals_empty_events() {
        let ctx = test_context();
        let proposals = hydrate_governor_proposals(&[], &ctx);
        assert!(proposals.is_empty());
    }

    #[test]
    fn hydrate_multiple_governor_proposals() {
        let ctx = test_context();

        let events = vec![
            GovernorProposalCreated {
                proposalId: U256::from(100u64),
                governor: address!("0000000000000000000000000000000000000099"),
                proposer: address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
                transactionIds: vec![b256!(
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                )],
            },
            GovernorProposalCreated {
                proposalId: U256::from(200u64),
                governor: address!("0000000000000000000000000000000000000099"),
                proposer: address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
                transactionIds: vec![b256!(
                    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                )],
            },
        ];

        let proposals = hydrate_governor_proposals(&events, &ctx);
        assert_eq!(proposals.len(), 2);
        assert_eq!(proposals[0].proposal_id, format!("{}", U256::from(100u64)));
        assert_eq!(proposals[1].proposal_id, format!("{}", U256::from(200u64)));
    }
}
