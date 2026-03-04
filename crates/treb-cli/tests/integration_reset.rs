//! Golden-file integration tests for `treb reset`.
//!
//! Tests exercise full reset with backup, network-scoped reset, namespace-scoped
//! reset, empty registry, JSON output, and uninitialized project error paths.

mod framework;
mod helpers;

use std::collections::HashMap;

use chrono::Utc;
use treb_core::types::{
    ArtifactInfo, DeploymentMethod, DeploymentStrategy, DeploymentType, ProposalStatus,
    SafeTxData, TransactionStatus, VerificationInfo, VerificationStatus,
};
use treb_registry::Registry;

use framework::context::TestContext;
use framework::integration_test::{run_integration_test, IntegrationTest};
use framework::normalizer::{EpochNormalizer, PathNormalizer};

// ── Fixture builders ─────────────────────────────────────────────────────

fn make_deployment(
    id: &str,
    chain_id: u64,
    namespace: &str,
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
        transaction_id: String::new(),
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
        deployments: vec![],
        operations: vec![],
        safe_context: None,
        environment: "testnet".to_string(),
        created_at: ts,
    }
}

fn make_safe_transaction(
    hash: &str,
    chain_id: u64,
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
        transaction_ids: vec![],
        proposed_by: "0x0000000000000000000000000000000000000003".to_string(),
        proposed_at: ts,
        confirmations: vec![],
        executed_at: None,
        execution_tx_hash: String::new(),
    }
}

fn make_governor_proposal(
    id: &str,
    chain_id: u64,
) -> treb_core::types::GovernorProposal {
    let ts = Utc::now();
    treb_core::types::GovernorProposal {
        proposal_id: id.to_string(),
        governor_address: "0x0000000000000000000000000000000000000004".to_string(),
        timelock_address: String::new(),
        chain_id,
        status: ProposalStatus::Pending,
        transaction_ids: vec![],
        proposed_by: "0x0000000000000000000000000000000000000005".to_string(),
        proposed_at: ts,
        description: String::new(),
        executed_at: None,
        execution_tx_hash: String::new(),
    }
}

/// Seed the registry with entries across two chains and two namespaces.
///
/// Creates:
/// - dep-1: chain 1, namespace "default"
/// - dep-2: chain 1, namespace "staging"
/// - dep-3: chain 42220, namespace "default"
/// - tx-1: chain 1
/// - tx-2: chain 42220
/// - safe-tx-1: chain 1
/// - safe-tx-2: chain 42220
/// - gov-1: chain 1
/// - gov-2: chain 42220
fn seed_reset_registry(project_root: &std::path::Path) {
    let mut registry = Registry::open(project_root).expect("registry should open");

    // Deployments
    registry.insert_deployment(make_deployment("dep-1", 1, "default")).unwrap();
    registry.insert_deployment(make_deployment("dep-2", 1, "staging")).unwrap();
    registry.insert_deployment(make_deployment("dep-3", 42220, "default")).unwrap();

    // Transactions
    registry.insert_transaction(make_transaction("tx-1", 1)).unwrap();
    registry.insert_transaction(make_transaction("tx-2", 42220)).unwrap();

    // Safe transactions
    registry.insert_safe_transaction(make_safe_transaction("safe-tx-1", 1)).unwrap();
    registry.insert_safe_transaction(make_safe_transaction("safe-tx-2", 42220)).unwrap();

    // Governor proposals
    registry.insert_governor_proposal(make_governor_proposal("gov-1", 1)).unwrap();
    registry.insert_governor_proposal(make_governor_proposal("gov-2", 42220)).unwrap();
}

// ── Tests ────────────────────────────────────────────────────────────────

/// Full reset removes all entries and prints per-type removal counts with backup path.
#[test]
fn reset_full() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("reset_full")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_reset_registry(ctx.path()))
        .test(&["reset", "--yes"])
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(EpochNormalizer));

    run_integration_test(&test, &ctx);
}

/// Network filter restricts reset to entries on the specified chain only.
#[test]
fn reset_network_filter() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("reset_network_filter")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_reset_registry(ctx.path()))
        .test(&["reset", "--yes", "--network", "1"])
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(EpochNormalizer));

    run_integration_test(&test, &ctx);
}

/// Namespace filter restricts reset to deployments in the specified namespace only.
#[test]
fn reset_namespace_filter() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("reset_namespace_filter")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_reset_registry(ctx.path()))
        .test(&["reset", "--yes", "--namespace", "staging"])
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(EpochNormalizer));

    run_integration_test(&test, &ctx);
}

/// Empty registry outputs "Nothing to reset."
#[test]
fn reset_empty() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("reset_empty")
        .setup(&["init"])
        .test(&["reset", "--yes"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// JSON output produces valid JSON with removal counts and backup path.
#[test]
fn reset_json() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("reset_json")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_reset_registry(ctx.path()))
        .test(&["reset", "--yes", "--json"])
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(EpochNormalizer));

    run_integration_test(&test, &ctx);
}

/// Uninitialized project (no foundry.toml) produces an error.
#[test]
fn reset_uninitialized() {
    let ctx = TestContext::new("compose-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("reset_uninitialized")
        .test(&["reset", "--yes"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}
