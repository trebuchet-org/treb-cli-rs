//! Deployment hydration — converting forge-domain types into core domain types.
//!
//! The hydration step bridges the gap between [`ExtractedDeployment`] (alloy-primitive
//! types from on-chain events) and the core [`Deployment`] model (string-based,
//! registry-compatible).

use std::collections::HashMap;

use alloy_primitives::B256;
use chrono::Utc;

use treb_core::types::deployment::{
    ArtifactInfo, Deployment, DeploymentStrategy, ProxyInfo, VerificationInfo,
};
use treb_core::types::enums::{DeploymentType, TransactionStatus, VerificationStatus};
use treb_core::types::safe_transaction::SafeTransaction;
use treb_core::types::transaction::Transaction;

use crate::events::abi::{SafeTransactionQueued, TransactionSimulated};
use crate::events::deployments::ExtractedDeployment;
use crate::events::proxy::{ProxyRelationship, ProxyType};

use super::PipelineContext;

/// Generate a deployment ID in the format `{namespace}/{chainId}/{contractName}:{label}`.
pub fn generate_deployment_id(
    namespace: &str,
    chain_id: u64,
    contract_name: &str,
    label: &str,
) -> String {
    format!("{namespace}/{chain_id}/{contract_name}:{label}")
}

/// Convert a [`B256`] value to a `0x`-prefixed lowercase hex string.
///
/// Returns an empty string when all bytes are zero.
fn b256_to_hex(value: B256) -> String {
    if value == B256::ZERO {
        String::new()
    } else {
        format!("{:#x}", value)
    }
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

    let id = generate_deployment_id(namespace, chain_id, &extracted.contract_name, &extracted.label);

    let deployment_type = if proxy.is_some() {
        DeploymentType::Proxy
    } else {
        DeploymentType::Singleton
    };

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
            implementation: p
                .implementation
                .map(|addr| addr.to_checksum(None))
                .unwrap_or_default(),
            admin: p
                .admin
                .map(|addr| addr.to_checksum(None))
                .unwrap_or_default(),
            history: Vec::new(),
        }
    });

    let artifact = match &extracted.artifact_match {
        Some(am) => ArtifactInfo {
            path: am.artifact_id.path.to_string_lossy().to_string(),
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
        transaction_id: format!("{:#x}", extracted.transaction_id),
        deployment_strategy,
        proxy_info,
        artifact,
        verification,
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
        .flat_map(|event| &event.transactions)
        .map(|sim_tx| {
            let tx_id_hex = format!("{:#x}", sim_tx.transactionId);

            // Find deployment IDs that belong to this transaction.
            let linked_deployment_ids: Vec<String> = hydrated_deployments
                .iter()
                .filter(|d| d.transaction_id == tx_id_hex)
                .map(|d| d.id.clone())
                .collect();

            Transaction {
                id: tx_id_hex,
                chain_id: context.config.chain_id,
                hash: String::new(),
                status: TransactionStatus::Simulated,
                block_number: 0,
                sender: sim_tx.sender.to_checksum(None),
                nonce: 0,
                deployments: linked_deployment_ids,
                operations: Vec::new(),
                safe_context: None,
                environment: context.config.namespace.clone(),
                created_at: now,
            }
        })
        .collect()
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
            let tx_ids: Vec<String> = event
                .transactionIds
                .iter()
                .map(|id| format!("{:#x}", id))
                .collect();

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

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{address, b256, Bytes};
    use std::path::PathBuf;
    use treb_core::types::enums::DeploymentMethod;

    use crate::events::proxy::ProxyRelationship;
    use crate::pipeline::PipelineConfig;

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
            salt: b256!(
                "0000000000000000000000000000000000000000000000000000000000000001"
            ),
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
        assert_eq!(
            deployment.address,
            "0x5FbDB2315678afecb367f032d93F642f64180aa3"
        );

        // Type is Singleton (no proxy)
        assert_eq!(deployment.deployment_type, DeploymentType::Singleton);

        // Transaction ID is 0x-prefixed hex
        assert_eq!(
            deployment.transaction_id,
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
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
        assert_eq!(
            proxy_info.implementation,
            "0x9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0"
        );
        assert_eq!(
            proxy_info.admin,
            "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
        );
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
        };
        extracted.artifact_match = Some(artifact_match);

        let deployment = hydrate_deployment(&extracted, None, &ctx);
        assert_eq!(deployment.artifact.path, "out/Counter.sol/Counter.json");
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
        assert_eq!(
            hex,
            "0x0000000000000000000000000000000000000000000000000000000000000001"
        );
    }

    // -----------------------------------------------------------------------
    // Transaction hydration tests
    // -----------------------------------------------------------------------

    use crate::events::abi::SimulatedTransaction;
    use alloy_primitives::U256;

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
        let tx_id_hex = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

        let events = vec![TransactionSimulated {
            transactions: vec![SimulatedTransaction {
                transactionId: tx_id,
                senderId: "deployer".to_string(),
                sender: address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
                returnData: Bytes::new(),
                transaction: crate::events::abi::Transaction {
                    to: address!("5FbDB2315678afecb367f032d93F642f64180aa3"),
                    data: Bytes::new(),
                    value: U256::ZERO,
                },
            }],
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

        // Both deployments should be linked
        assert_eq!(tx.deployments.len(), 2);
        assert!(tx.deployments.contains(&"production/1/Counter:v1".to_string()));
        assert!(tx.deployments.contains(&"production/1/Token:v1".to_string()));
    }

    #[test]
    fn hydrate_safe_transaction_queued() {
        let ctx = test_context();
        let safe_tx_hash =
            b256!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        let tx_id_1 =
            b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let tx_id_2 =
            b256!("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc");

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
        assert_eq!(
            stx.safe_address,
            "0x5FbDB2315678afecb367f032d93F642f64180aa3"
        );
        assert_eq!(stx.chain_id, 1);
        assert_eq!(stx.status, TransactionStatus::Queued);
        assert_eq!(stx.nonce, 0);
        assert!(stx.transactions.is_empty());
        assert_eq!(
            stx.proposed_by,
            "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
        );
        assert!(stx.confirmations.is_empty());
        assert!(stx.executed_at.is_none());
        assert!(stx.execution_tx_hash.is_empty());

        // Transaction IDs should be hex strings
        assert_eq!(stx.transaction_ids.len(), 2);
        assert_eq!(
            stx.transaction_ids[0],
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(
            stx.transaction_ids[1],
            "0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
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
}
