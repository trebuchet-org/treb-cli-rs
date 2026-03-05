//! Integration tests for the deployment recording pipeline.
//!
//! These tests exercise the full hydration → duplicate check → registry write
//! flow with synthetic data, verifying that the pipeline produces correct
//! core domain records without requiring real forge compilation.

use std::path::PathBuf;

use alloy_primitives::{Address, B256, Bytes, address, b256};
use tempfile::TempDir;
use treb_core::types::enums::{
    DeploymentMethod, DeploymentType, TransactionStatus, VerificationStatus,
};
use treb_forge::{
    events::{
        abi::{SafeTransactionQueued, SimulatedTransaction, TransactionSimulated},
        deployments::{ExtractedCollision, ExtractedDeployment},
        proxy::{ProxyRelationship, ProxyType},
    },
    pipeline::{
        DuplicateStrategy, PipelineConfig, PipelineContext, PipelineResult, RecordedDeployment,
        RecordedTransaction, SkippedDeployment, check_duplicate, hydrate_deployment,
        hydrate_safe_transactions, hydrate_transactions, resolve_duplicates,
    },
};
use treb_registry::Registry;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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
        deployer_sender: None,
    }
}

fn init_registry() -> (TempDir, Registry) {
    let dir = TempDir::new().unwrap();
    let registry = Registry::init(dir.path()).unwrap();
    (dir, registry)
}

fn make_extracted_create(
    name: &str,
    label: &str,
    addr: Address,
    tx_id: B256,
) -> ExtractedDeployment {
    ExtractedDeployment {
        address: addr,
        deployer: address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
        transaction_id: tx_id,
        contract_name: name.to_string(),
        label: label.to_string(),
        strategy: DeploymentMethod::Create,
        salt: B256::ZERO,
        bytecode_hash: b256!("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"),
        init_code_hash: B256::ZERO,
        constructor_args: Bytes::from(vec![0x00, 0x01]),
        entropy: String::new(),
        artifact_match: None,
    }
}

fn make_extracted_create2(
    name: &str,
    label: &str,
    addr: Address,
    tx_id: B256,
) -> ExtractedDeployment {
    ExtractedDeployment {
        address: addr,
        deployer: address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
        transaction_id: tx_id,
        contract_name: name.to_string(),
        label: label.to_string(),
        strategy: DeploymentMethod::Create2,
        salt: b256!("0000000000000000000000000000000000000000000000000000000000000042"),
        bytecode_hash: b256!("abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd"),
        init_code_hash: b256!("1111111111111111111111111111111111111111111111111111111111111111"),
        constructor_args: Bytes::new(),
        entropy: "entropy-value".to_string(),
        artifact_match: None,
    }
}

/// Build a full PipelineResult from hydrated + resolved data, simulating
/// what the orchestrator does after duplicate resolution.
fn build_pipeline_result(
    recorded_deployments: Vec<RecordedDeployment>,
    recorded_transactions: Vec<RecordedTransaction>,
    collisions: Vec<ExtractedCollision>,
    skipped: Vec<SkippedDeployment>,
    dry_run: bool,
) -> PipelineResult {
    PipelineResult {
        deployments: recorded_deployments,
        transactions: recorded_transactions,
        collisions,
        skipped,
        dry_run,
        success: true,
        gas_used: 0,
        event_count: 0,
        console_logs: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Test 1: Single deployment hydration + recording
// ---------------------------------------------------------------------------

#[test]
fn single_deployment_hydration_and_recording() {
    let ctx = test_context();
    let (_dir, mut registry) = init_registry();

    // Hydrate
    let extracted = make_extracted_create(
        "Counter",
        "counter-v1",
        address!("5FbDB2315678afecb367f032d93F642f64180aa3"),
        b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    );
    let deployment = hydrate_deployment(&extracted, None, &ctx);

    // Verify ID format
    assert_eq!(deployment.id, "production/1/Counter:counter-v1");
    assert_eq!(deployment.namespace, "production");
    assert_eq!(deployment.chain_id, 1);

    // Verify checksummed address
    assert_eq!(deployment.address, "0x5FbDB2315678afecb367f032d93F642f64180aa3");

    // Verify strategy
    assert_eq!(deployment.deployment_strategy.method, DeploymentMethod::Create);
    assert!(deployment.deployment_strategy.salt.is_empty());

    // Verify artifact info
    assert_eq!(deployment.artifact.script_path, "script/Deploy.s.sol");
    assert_eq!(deployment.artifact.git_commit, "abc1234");
    assert_eq!(
        deployment.artifact.bytecode_hash,
        "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
    );

    // Verify deployment_type is Singleton
    assert_eq!(deployment.deployment_type, DeploymentType::Singleton);

    // Duplicate check — no conflicts
    let resolved =
        resolve_duplicates(vec![deployment.clone()], &registry, DuplicateStrategy::Skip).unwrap();
    assert_eq!(resolved.to_insert.len(), 1);
    assert!(resolved.skipped.is_empty());

    // Record to registry
    registry.insert_deployment(deployment.clone()).unwrap();

    // Verify registry state
    let stored = registry.get_deployment("production/1/Counter:counter-v1").unwrap();
    assert_eq!(stored.address, "0x5FbDB2315678afecb367f032d93F642f64180aa3");
    assert_eq!(stored.contract_name, "Counter");
    assert_eq!(stored.deployment_type, DeploymentType::Singleton);
    assert_eq!(stored.verification.status, VerificationStatus::Unverified);
    assert_eq!(registry.deployment_count(), 1);
}

// ---------------------------------------------------------------------------
// Test 2: Proxy deployment
// ---------------------------------------------------------------------------

#[test]
fn proxy_deployment_hydration_and_recording() {
    let ctx = test_context();
    let (_dir, mut registry) = init_registry();

    let extracted = make_extracted_create(
        "TransparentProxy",
        "proxy-v1",
        address!("5FbDB2315678afecb367f032d93F642f64180aa3"),
        b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    );

    let proxy = ProxyRelationship {
        proxy_address: address!("5FbDB2315678afecb367f032d93F642f64180aa3"),
        proxy_type: ProxyType::Transparent,
        implementation: Some(address!("9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0")),
        admin: Some(address!("70997970C51812dc3A010C7d01b50e0d17dc79C8")),
        beacon: None,
    };

    let deployment = hydrate_deployment(&extracted, Some(&proxy), &ctx);

    // Verify deployment_type is Proxy
    assert_eq!(deployment.deployment_type, DeploymentType::Proxy);

    // Verify proxy_info fields
    let proxy_info = deployment.proxy_info.as_ref().expect("should have proxy_info");
    assert_eq!(proxy_info.proxy_type, "Transparent");
    assert_eq!(proxy_info.implementation, "0x9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0");
    assert_eq!(proxy_info.admin, "0x70997970C51812dc3A010C7d01b50e0d17dc79C8");
    assert!(proxy_info.history.is_empty());

    // Record to registry and verify
    let resolved =
        resolve_duplicates(vec![deployment.clone()], &registry, DuplicateStrategy::Skip).unwrap();
    assert_eq!(resolved.to_insert.len(), 1);

    registry.insert_deployment(deployment).unwrap();

    let stored = registry.get_deployment("production/1/TransparentProxy:proxy-v1").unwrap();
    assert_eq!(stored.deployment_type, DeploymentType::Proxy);
    assert!(stored.proxy_info.is_some());
}

// ---------------------------------------------------------------------------
// Test 3: Transaction linking
// ---------------------------------------------------------------------------

#[test]
fn transaction_linking_deployments_to_transactions() {
    let ctx = test_context();
    let (_dir, mut registry) = init_registry();

    let tx_id = b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    let tx_id_hex = "tx-0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    // Create two deployments sharing the same transaction
    let extracted1 = make_extracted_create(
        "Counter",
        "counter-v1",
        address!("5FbDB2315678afecb367f032d93F642f64180aa3"),
        tx_id,
    );
    let extracted2 = make_extracted_create(
        "Token",
        "token-v1",
        address!("e7f1725E7734CE288F8367e1Bb143E90bb3F0512"),
        tx_id,
    );

    let dep1 = hydrate_deployment(&extracted1, None, &ctx);
    let dep2 = hydrate_deployment(&extracted2, None, &ctx);
    let hydrated_deployments = vec![dep1.clone(), dep2.clone()];

    // Hydrate transaction
    let events = vec![TransactionSimulated {
        transactions: vec![SimulatedTransaction {
            transactionId: tx_id,
            senderId: "deployer".to_string(),
            sender: address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
            returnData: Bytes::new(),
            transaction: treb_forge::events::abi::Transaction {
                to: address!("5FbDB2315678afecb367f032d93F642f64180aa3"),
                data: Bytes::new(),
                value: alloy_primitives::U256::ZERO,
            },
        }],
    }];

    let transactions = hydrate_transactions(&events, &hydrated_deployments, &ctx);
    assert_eq!(transactions.len(), 1);

    let tx = &transactions[0];
    assert_eq!(tx.id, tx_id_hex);
    assert_eq!(tx.status, TransactionStatus::Simulated);
    assert_eq!(tx.sender, "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
    assert_eq!(tx.environment, "production");

    // Both deployments should be linked
    assert_eq!(tx.deployments.len(), 2);
    assert!(tx.deployments.contains(&"production/1/Counter:counter-v1".to_string()));
    assert!(tx.deployments.contains(&"production/1/Token:token-v1".to_string()));

    // Record everything to the registry
    registry.insert_deployment(dep1).unwrap();
    registry.insert_deployment(dep2).unwrap();
    registry.insert_transaction(tx.clone()).unwrap();

    // Verify registry state
    assert_eq!(registry.deployment_count(), 2);
    assert_eq!(registry.transaction_count(), 1);
    let stored_tx = registry.get_transaction(tx_id_hex).unwrap();
    assert_eq!(stored_tx.deployments.len(), 2);
}

// ---------------------------------------------------------------------------
// Test 4: Duplicate skip
// ---------------------------------------------------------------------------

#[test]
fn duplicate_skip_produces_skipped_entries() {
    let ctx = test_context();
    let (_dir, mut registry) = init_registry();

    // Pre-seed the registry with an existing deployment
    let extracted_existing = make_extracted_create(
        "Counter",
        "counter-v1",
        address!("5FbDB2315678afecb367f032d93F642f64180aa3"),
        b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    );
    let existing = hydrate_deployment(&extracted_existing, None, &ctx);
    registry.insert_deployment(existing).unwrap();

    // Now try to record the same deployment again (same ID)
    let extracted_dup = make_extracted_create(
        "Counter",
        "counter-v1",
        address!("1111111111111111111111111111111111111111"),
        b256!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
    );
    let duplicate = hydrate_deployment(&extracted_dup, None, &ctx);

    // Also add a non-conflicting deployment
    let extracted_new = make_extracted_create(
        "Token",
        "token-v1",
        address!("e7f1725E7734CE288F8367e1Bb143E90bb3F0512"),
        b256!("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"),
    );
    let new_dep = hydrate_deployment(&extracted_new, None, &ctx);

    let resolved =
        resolve_duplicates(vec![duplicate, new_dep], &registry, DuplicateStrategy::Skip).unwrap();

    // The duplicate should be skipped
    assert_eq!(resolved.skipped.len(), 1);
    assert_eq!(resolved.skipped[0].deployment.id, "production/1/Counter:counter-v1");
    assert!(
        resolved.skipped[0].reason.contains("already exists"),
        "reason should explain: {}",
        resolved.skipped[0].reason
    );

    // The new deployment should pass through
    assert_eq!(resolved.to_insert.len(), 1);
    assert_eq!(resolved.to_insert[0].id, "production/1/Token:token-v1");

    // Record only the inserts
    for dep in resolved.to_insert {
        registry.insert_deployment(dep).unwrap();
    }

    // Build the result
    let result = build_pipeline_result(vec![], vec![], vec![], resolved.skipped, false);
    assert_eq!(result.skipped.len(), 1);
    assert!(result.success);

    // Registry should have 2 deployments total (original + new)
    assert_eq!(registry.deployment_count(), 2);
}

// ---------------------------------------------------------------------------
// Test 5: Duplicate address detection
// ---------------------------------------------------------------------------

#[test]
fn duplicate_address_detection_same_chain() {
    let ctx = test_context();
    let (_dir, mut registry) = init_registry();

    // Seed existing deployment at a known address
    let extracted_existing = make_extracted_create(
        "OldToken",
        "old-v1",
        address!("5FbDB2315678afecb367f032d93F642f64180aa3"),
        b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    );
    let existing = hydrate_deployment(&extracted_existing, None, &ctx);
    registry.insert_deployment(existing).unwrap();

    // Try to record a DIFFERENT contract at the SAME address on the same chain
    let extracted_new = make_extracted_create(
        "NewToken",
        "new-v1",
        address!("5FbDB2315678afecb367f032d93F642f64180aa3"), // same address
        b256!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
    );
    let new_dep = hydrate_deployment(&extracted_new, None, &ctx);

    // check_duplicate should detect SameAddress
    let conflict = check_duplicate(&new_dep, &registry).expect("should detect conflict");
    assert_eq!(conflict.conflict_type, treb_forge::pipeline::ConflictType::SameAddress);
    assert_eq!(conflict.existing_id, "production/1/OldToken:old-v1");

    // resolve_duplicates with Skip strategy
    let resolved = resolve_duplicates(vec![new_dep], &registry, DuplicateStrategy::Skip).unwrap();

    assert_eq!(resolved.skipped.len(), 1);
    assert!(resolved.skipped[0].reason.contains("already registered"));
    assert!(resolved.to_insert.is_empty());
}

// ---------------------------------------------------------------------------
// Test 6: Dry-run mode
// ---------------------------------------------------------------------------

#[test]
fn dry_run_leaves_registry_unchanged() {
    let ctx = test_context();
    let (_dir, registry) = init_registry();

    // Hydrate deployments
    let extracted = make_extracted_create(
        "Counter",
        "counter-v1",
        address!("5FbDB2315678afecb367f032d93F642f64180aa3"),
        b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    );
    let deployment = hydrate_deployment(&extracted, None, &ctx);

    // Duplicate check (pass — empty registry)
    let resolved =
        resolve_duplicates(vec![deployment.clone()], &registry, DuplicateStrategy::Skip).unwrap();
    assert_eq!(resolved.to_insert.len(), 1);

    // Hydrate a transaction
    let tx_events = vec![TransactionSimulated {
        transactions: vec![SimulatedTransaction {
            transactionId: b256!(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            ),
            senderId: "deployer".to_string(),
            sender: address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
            returnData: Bytes::new(),
            transaction: treb_forge::events::abi::Transaction {
                to: address!("5FbDB2315678afecb367f032d93F642f64180aa3"),
                data: Bytes::new(),
                value: alloy_primitives::U256::ZERO,
            },
        }],
    }];
    let transactions = hydrate_transactions(&tx_events, &[deployment.clone()], &ctx);

    // Build PipelineResult in dry-run mode — do NOT write to registry
    let recorded_deps: Vec<RecordedDeployment> = resolved
        .to_insert
        .into_iter()
        .map(|d| RecordedDeployment { deployment: d, safe_transaction: None })
        .collect();
    let recorded_txs: Vec<RecordedTransaction> =
        transactions.into_iter().map(|t| RecordedTransaction { transaction: t }).collect();

    let result = build_pipeline_result(recorded_deps, recorded_txs, vec![], vec![], true);

    // Result should be fully populated
    assert!(result.dry_run);
    assert!(result.success);
    assert_eq!(result.deployments.len(), 1);
    assert_eq!(result.transactions.len(), 1);
    assert_eq!(result.deployments[0].deployment.id, "production/1/Counter:counter-v1");

    // Registry should be completely empty — dry-run did NOT write
    assert_eq!(registry.deployment_count(), 0);
    assert_eq!(registry.transaction_count(), 0);
}

// ---------------------------------------------------------------------------
// Test 7: Multi-deployment (CREATE, CREATE2, proxy)
// ---------------------------------------------------------------------------

#[test]
fn multi_deployment_create_create2_and_proxy() {
    let ctx = test_context();
    let (_dir, mut registry) = init_registry();

    let tx_id = b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");

    // 1. CREATE deployment
    let extracted_create = make_extracted_create(
        "Counter",
        "counter-v1",
        address!("5FbDB2315678afecb367f032d93F642f64180aa3"),
        tx_id,
    );

    // 2. CREATE2 deployment
    let extracted_create2 = make_extracted_create2(
        "Token",
        "token-v1",
        address!("e7f1725E7734CE288F8367e1Bb143E90bb3F0512"),
        tx_id,
    );

    // 3. Proxy deployment (CREATE with proxy relationship)
    let extracted_proxy = make_extracted_create(
        "ProxyContract",
        "proxy-v1",
        address!("9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0"),
        tx_id,
    );
    let proxy_rel = ProxyRelationship {
        proxy_address: address!("9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0"),
        proxy_type: ProxyType::UUPS,
        implementation: Some(address!("Cf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9")),
        admin: None,
        beacon: None,
    };

    let dep_create = hydrate_deployment(&extracted_create, None, &ctx);
    let dep_create2 = hydrate_deployment(&extracted_create2, None, &ctx);
    let dep_proxy = hydrate_deployment(&extracted_proxy, Some(&proxy_rel), &ctx);

    // Verify types
    assert_eq!(dep_create.deployment_type, DeploymentType::Singleton);
    assert_eq!(dep_create.deployment_strategy.method, DeploymentMethod::Create);

    assert_eq!(dep_create2.deployment_type, DeploymentType::Singleton);
    assert_eq!(dep_create2.deployment_strategy.method, DeploymentMethod::Create2);
    assert_eq!(
        dep_create2.deployment_strategy.salt,
        "0x0000000000000000000000000000000000000000000000000000000000000042"
    );
    assert_eq!(dep_create2.deployment_strategy.entropy, "entropy-value");

    assert_eq!(dep_proxy.deployment_type, DeploymentType::Proxy);
    let proxy_info = dep_proxy.proxy_info.as_ref().unwrap();
    assert_eq!(proxy_info.proxy_type, "UUPS");

    // Resolve duplicates (all new)
    let all_deps = vec![dep_create.clone(), dep_create2.clone(), dep_proxy.clone()];
    let resolved = resolve_duplicates(all_deps, &registry, DuplicateStrategy::Skip).unwrap();
    assert_eq!(resolved.to_insert.len(), 3);
    assert!(resolved.skipped.is_empty());

    // Record all
    for dep in resolved.to_insert {
        registry.insert_deployment(dep).unwrap();
    }

    assert_eq!(registry.deployment_count(), 3);

    // Verify each stored correctly
    let stored_create = registry.get_deployment("production/1/Counter:counter-v1").unwrap();
    assert_eq!(stored_create.deployment_strategy.method, DeploymentMethod::Create);
    assert_eq!(stored_create.deployment_type, DeploymentType::Singleton);

    let stored_create2 = registry.get_deployment("production/1/Token:token-v1").unwrap();
    assert_eq!(stored_create2.deployment_strategy.method, DeploymentMethod::Create2);
    assert!(!stored_create2.deployment_strategy.salt.is_empty());

    let stored_proxy = registry.get_deployment("production/1/ProxyContract:proxy-v1").unwrap();
    assert_eq!(stored_proxy.deployment_type, DeploymentType::Proxy);
    assert!(stored_proxy.proxy_info.is_some());
}

// ---------------------------------------------------------------------------
// Test 8: Collision reporting
// ---------------------------------------------------------------------------

#[test]
fn collision_events_reported_in_pipeline_result() {
    let ctx = test_context();
    let (_dir, mut registry) = init_registry();

    // Simulate extracting a deployment that succeeds
    let extracted = make_extracted_create(
        "Counter",
        "counter-v1",
        address!("5FbDB2315678afecb367f032d93F642f64180aa3"),
        b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    );
    let deployment = hydrate_deployment(&extracted, None, &ctx);

    // Simulate collision events detected during extraction
    let collisions = vec![
        ExtractedCollision {
            existing_address: address!("e7f1725E7734CE288F8367e1Bb143E90bb3F0512"),
            contract_name: "Token".to_string(),
            label: "token-v1".to_string(),
            strategy: DeploymentMethod::Create2,
            salt: b256!("2222222222222222222222222222222222222222222222222222222222222222"),
            bytecode_hash: b256!(
                "3333333333333333333333333333333333333333333333333333333333333333"
            ),
            init_code_hash: b256!(
                "4444444444444444444444444444444444444444444444444444444444444444"
            ),
        },
        ExtractedCollision {
            existing_address: address!("9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0"),
            contract_name: "Vault".to_string(),
            label: "vault-v1".to_string(),
            strategy: DeploymentMethod::Create,
            salt: B256::ZERO,
            bytecode_hash: B256::ZERO,
            init_code_hash: B256::ZERO,
        },
    ];

    // Record the successful deployment
    let resolved =
        resolve_duplicates(vec![deployment.clone()], &registry, DuplicateStrategy::Skip).unwrap();
    for dep in &resolved.to_insert {
        registry.insert_deployment(dep.clone()).unwrap();
    }

    let recorded_deps: Vec<RecordedDeployment> = resolved
        .to_insert
        .into_iter()
        .map(|d| RecordedDeployment { deployment: d, safe_transaction: None })
        .collect();

    // Build result with collisions
    let result = build_pipeline_result(recorded_deps, vec![], collisions, vec![], false);

    assert!(result.success);
    assert!(!result.dry_run);
    assert_eq!(result.deployments.len(), 1);
    assert_eq!(result.collisions.len(), 2);

    // Verify collision details
    assert_eq!(result.collisions[0].contract_name, "Token");
    assert_eq!(result.collisions[0].label, "token-v1");
    assert_eq!(
        result.collisions[0].existing_address,
        address!("e7f1725E7734CE288F8367e1Bb143E90bb3F0512")
    );
    assert_eq!(result.collisions[1].contract_name, "Vault");
    assert_eq!(result.collisions[1].label, "vault-v1");
}

// ---------------------------------------------------------------------------
// Test 9 (bonus): Safe transaction hydration in integration flow
// ---------------------------------------------------------------------------

#[test]
fn safe_transaction_hydration_and_recording() {
    let ctx = test_context();
    let (_dir, mut registry) = init_registry();

    let tx_id = b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    let safe_tx_hash = b256!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");

    // Hydrate deployment
    let extracted = make_extracted_create(
        "Counter",
        "counter-v1",
        address!("5FbDB2315678afecb367f032d93F642f64180aa3"),
        tx_id,
    );
    let deployment = hydrate_deployment(&extracted, None, &ctx);

    // Hydrate safe transaction
    let safe_events = vec![SafeTransactionQueued {
        safeTxHash: safe_tx_hash,
        safe: address!("1234567890123456789012345678901234567890"),
        proposer: address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
        transactionIds: vec![tx_id],
    }];
    let safe_txs = hydrate_safe_transactions(&safe_events, &ctx);
    assert_eq!(safe_txs.len(), 1);

    let safe_tx = &safe_txs[0];
    assert_eq!(
        safe_tx.safe_tx_hash,
        "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    );
    assert_eq!(safe_tx.status, TransactionStatus::Queued);
    assert_eq!(safe_tx.transaction_ids.len(), 1);

    // Record to registry
    registry.insert_deployment(deployment).unwrap();
    registry.insert_safe_transaction(safe_tx.clone()).unwrap();

    assert_eq!(registry.deployment_count(), 1);
    assert_eq!(registry.safe_transaction_count(), 1);

    let stored = registry
        .get_safe_transaction("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
        .unwrap();
    assert_eq!(stored.status, TransactionStatus::Queued);
    assert_eq!(stored.safe_address, "0x1234567890123456789012345678901234567890");
}

// ---------------------------------------------------------------------------
// Test 10 (bonus): Error strategy on duplicate
// ---------------------------------------------------------------------------

#[test]
fn error_strategy_returns_error_on_duplicate() {
    let ctx = test_context();
    let (_dir, mut registry) = init_registry();

    // Pre-seed
    let extracted = make_extracted_create(
        "Counter",
        "counter-v1",
        address!("5FbDB2315678afecb367f032d93F642f64180aa3"),
        b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    );
    let existing = hydrate_deployment(&extracted, None, &ctx);
    registry.insert_deployment(existing).unwrap();

    // Try to insert same ID with Error strategy
    let extracted_dup = make_extracted_create(
        "Counter",
        "counter-v1",
        address!("1111111111111111111111111111111111111111"),
        b256!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
    );
    let duplicate = hydrate_deployment(&extracted_dup, None, &ctx);

    let result = resolve_duplicates(vec![duplicate], &registry, DuplicateStrategy::Error);
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("Duplicate deployment"), "error: {err_msg}");
}
