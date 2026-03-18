//! Golden-file integration tests for `treb registry drop`.
//!
//! Tests exercise full drop with backup, network-scoped drop, namespace-scoped
//! drop, empty registry, JSON output, and uninitialized project error paths.

mod framework;
mod helpers;

use std::collections::HashMap;

use chrono::Utc;
use treb_core::types::{
    ArtifactInfo, DeploymentMethod, DeploymentStrategy, DeploymentType, ProposalStatus, SafeTxData,
    TransactionStatus, VerificationInfo, VerificationStatus,
};
use treb_registry::Registry;

use framework::{
    context::TestContext,
    integration_test::{IntegrationTest, run_integration_test},
    normalizer::{EpochNormalizer, PathNormalizer},
};

// ── Fixture builders ─────────────────────────────────────────────────────

fn make_deployment(
    id: &str,
    chain_id: u64,
    namespace: &str,
    transaction_id: &str,
) -> treb_core::types::Deployment {
    let ts = Utc::now();
    treb_core::types::Deployment {
        id: id.to_string(),
        namespace: namespace.to_string(),
        chain_id,
        contract_name: "TestContract".to_string(),
        label: "v1".to_string(),
        address: format!("0x{:040x}", 1u64),
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
            path: "contracts/Test.sol".to_string(),
            compiler_version: "0.8.24".to_string(),
            bytecode_hash: "0xabc".to_string(),
            script_path: "script/Deploy.s.sol".to_string(),
            git_commit: "abc123".to_string(),
        },
        verification: VerificationInfo {
            status: VerificationStatus::Unverified,
            etherscan_url: String::new(),
            verified_at: None,
            reason: String::new(),
            verifiers: HashMap::new(),
        },
        tags: None,
        created_at: ts,
        updated_at: ts,
    }
}

fn make_transaction(
    id: &str,
    chain_id: u64,
    deployments: &[&str],
) -> treb_core::types::Transaction {
    let ts = Utc::now();
    treb_core::types::Transaction {
        id: id.to_string(),
        chain_id,
        hash: format!("0x{:064x}", 0u64),
        status: TransactionStatus::Executed,
        block_number: 1000,
        sender: "0x56fD3F2bEE130e9867942D0F463a16fBE49B8d81".to_string(),
        nonce: 0,
        deployments: deployments.iter().map(|id| (*id).to_string()).collect(),
        operations: vec![],
        safe_context: None,
        broadcast_file: None,
        environment: "testnet".to_string(),
        created_at: ts,
    }
}

fn make_safe_transaction(
    hash: &str,
    chain_id: u64,
    transaction_ids: &[&str],
) -> treb_core::types::SafeTransaction {
    let ts = Utc::now();
    treb_core::types::SafeTransaction {
        safe_tx_hash: hash.to_string(),
        safe_address: "0x0000000000000000000000000000000000000001".to_string(),
        chain_id,
        status: TransactionStatus::Queued,
        nonce: 0,
        transactions: vec![SafeTxData {
            to: "0x0000000000000000000000000000000000000002".to_string(),
            value: "0".to_string(),
            data: "0x".to_string(),
            operation: 0,
        }],
        transaction_ids: transaction_ids.iter().map(|id| (*id).to_string()).collect(),
        proposed_by: "0x0000000000000000000000000000000000000003".to_string(),
        proposed_at: ts,
        confirmations: vec![],
        executed_at: None,
        execution_tx_hash: String::new(),
        fork_executed_at: None,
    }
}

fn make_governor_proposal(
    id: &str,
    chain_id: u64,
    transaction_ids: &[&str],
) -> treb_core::types::GovernorProposal {
    let ts = Utc::now();
    treb_core::types::GovernorProposal {
        proposal_id: id.to_string(),
        governor_address: "0x0000000000000000000000000000000000000004".to_string(),
        timelock_address: String::new(),
        chain_id,
        status: ProposalStatus::Pending,
        transaction_ids: transaction_ids.iter().map(|id| (*id).to_string()).collect(),
        proposed_by: "0x0000000000000000000000000000000000000005".to_string(),
        proposed_at: ts,
        description: String::new(),
        executed_at: None,
        execution_tx_hash: String::new(),
        fork_executed_at: None,
        actions: Vec::new(),
    }
}

/// Seed the registry with linked entries across two chains, all in "default" namespace.
///
/// Creates:
/// - dep-1: chain 1, namespace "default", linked to tx-1
/// - dep-2: chain 1, namespace "default", linked to tx-1
/// - dep-3: chain 42220, namespace "default", linked to tx-2
/// - tx-1: chain 1, linked to [dep-1, dep-2]
/// - tx-2: chain 42220, linked to [dep-3]
/// - safe-tx-1: chain 1, linked to [tx-1]
/// - safe-tx-2: chain 42220, linked to [tx-2]
/// - gov-1: chain 1, linked to [tx-1]
/// - gov-2: chain 42220, linked to [tx-2]
fn seed_drop_registry(project_root: &std::path::Path) {
    let mut registry = Registry::open(project_root).expect("registry should open");

    // Deployments — all in "default" namespace
    registry.insert_deployment(make_deployment("dep-1", 1, "default", "tx-1")).unwrap();
    registry.insert_deployment(make_deployment("dep-2", 1, "default", "tx-1")).unwrap();
    registry.insert_deployment(make_deployment("dep-3", 42220, "default", "tx-2")).unwrap();

    // Transactions — linked to deployments
    registry
        .insert_transaction(make_transaction("tx-1", 1, &["dep-1", "dep-2"]))
        .unwrap();
    registry
        .insert_transaction(make_transaction("tx-2", 42220, &["dep-3"]))
        .unwrap();

    // Safe transactions — linked to transactions
    registry
        .insert_safe_transaction(make_safe_transaction("safe-tx-1", 1, &["tx-1"]))
        .unwrap();
    registry
        .insert_safe_transaction(make_safe_transaction("safe-tx-2", 42220, &["tx-2"]))
        .unwrap();

    // Governor proposals — linked to transactions
    registry
        .insert_governor_proposal(make_governor_proposal("gov-1", 1, &["tx-1"]))
        .unwrap();
    registry
        .insert_governor_proposal(make_governor_proposal("gov-2", 42220, &["tx-2"]))
        .unwrap();
}

/// Seed namespace-scoped drop fixtures with linked entries in two namespaces.
fn seed_namespace_drop_registry(project_root: &std::path::Path) {
    let mut registry = Registry::open(project_root).expect("registry should open");

    registry.insert_deployment(make_deployment("dep-default", 1, "default", "tx-default")).unwrap();
    registry.insert_deployment(make_deployment("dep-staging", 1, "staging", "tx-staging")).unwrap();

    registry.insert_transaction(make_transaction("tx-default", 1, &["dep-default"])).unwrap();
    registry.insert_transaction(make_transaction("tx-staging", 1, &["dep-staging"])).unwrap();

    registry
        .insert_safe_transaction(make_safe_transaction("safe-tx-default", 1, &["tx-default"]))
        .unwrap();
    registry
        .insert_safe_transaction(make_safe_transaction("safe-tx-staging", 1, &["tx-staging"]))
        .unwrap();

    registry
        .insert_governor_proposal(make_governor_proposal("gov-default", 1, &["tx-default"]))
        .unwrap();
    registry
        .insert_governor_proposal(make_governor_proposal("gov-staging", 1, &["tx-staging"]))
        .unwrap();
}

// ── Tests ────────────────────────────────────────────────────────────────

/// Full drop removes all entries and prints per-type removal counts with backup path.
#[test]
fn drop_full() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("reset_full")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_drop_registry(ctx.path()))
        .test(&["registry", "drop", "--namespace", "default", "--yes"])
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(EpochNormalizer));

    run_integration_test(&test, &ctx);
}

/// Network filter restricts drop to entries on the specified chain only.
#[test]
fn drop_network_filter() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("reset_network_filter")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_drop_registry(ctx.path()))
        .test(&["registry", "drop", "--yes", "--network", "1"])
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(EpochNormalizer));

    run_integration_test(&test, &ctx);
}

/// Namespace filter restricts drop to deployments in the specified namespace only.
#[test]
fn drop_namespace_filter() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("reset_namespace_filter")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_namespace_drop_registry(ctx.path()))
        .test(&["registry", "drop", "--yes", "--namespace", "staging"])
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(EpochNormalizer));

    run_integration_test(&test, &ctx);
}

/// Empty registry outputs "Nothing to drop."
#[test]
fn drop_empty() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("reset_empty")
        .setup(&["init"])
        .test(&["registry", "drop", "--namespace", "default", "--yes"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// JSON output produces valid JSON with removal counts and backup path.
#[test]
fn drop_json() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("reset_json")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_drop_registry(ctx.path()))
        .test(&["registry", "drop", "--namespace", "default", "--yes", "--json"])
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(EpochNormalizer));

    run_integration_test(&test, &ctx);
}

/// Uninitialized project (no foundry.toml) produces an error.
#[test]
fn drop_uninitialized() {
    let ctx = TestContext::new("compose-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("reset_uninitialized")
        .test(&["registry", "drop", "--namespace", "default", "--yes"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}
