//! Golden-file integration tests for `treb verify`.
//!
//! Tests exercise already-verified skip, no unverified deployments,
//! JSON output, unknown verifier error, uninitialized project,
//! and no foundry project error paths.

mod framework;
mod helpers;

use std::collections::HashMap;

use chrono::Utc;
use treb_core::types::{
    ArtifactInfo, DeploymentMethod, DeploymentStrategy, DeploymentType, TransactionStatus,
    VerificationInfo, VerificationStatus,
};
use treb_registry::Registry;

use framework::context::TestContext;
use framework::integration_test::{run_integration_test, IntegrationTest};
use framework::normalizer::PathNormalizer;

// ── Fixture builders ─────────────────────────────────────────────────────

fn make_verified_deployment(id: &str, tx_id: &str, chain_id: u64) -> treb_core::types::Deployment {
    let ts = Utc::now();
    treb_core::types::Deployment {
        id: id.to_string(),
        namespace: "default".to_string(),
        chain_id,
        contract_name: "TestContract".to_string(),
        label: "v1".to_string(),
        address: format!("0x{:040x}", 1u64),
        deployment_type: DeploymentType::Singleton,
        transaction_id: tx_id.to_string(),
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
            status: VerificationStatus::Verified,
            etherscan_url: format!("https://etherscan.io/address/0x{:040x}", 1u64),
            verified_at: Some(ts),
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
    dep_ids: Vec<String>,
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
        deployments: dep_ids,
        operations: vec![],
        safe_context: None,
        environment: "testnet".to_string(),
        created_at: ts,
    }
}

/// Seed the registry with one already-verified deployment and its transaction.
fn seed_verified_registry(project_root: &std::path::Path) {
    let mut registry = Registry::open(project_root).expect("registry should open");

    registry
        .insert_transaction(make_transaction(
            "tx-1",
            vec!["dep-verified".to_string()],
            1,
        ))
        .unwrap();
    registry
        .insert_deployment(make_verified_deployment("dep-verified", "tx-1", 1))
        .unwrap();
}

// ── Tests ────────────────────────────────────────────────────────────────

/// Single deployment that is already verified prints skip message.
#[test]
fn verify_already_verified() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("verify_already_verified")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_verified_registry(ctx.path()))
        .test(&["verify", "dep-verified"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// --all with no unverified deployments prints empty-state message.
#[test]
fn verify_all_none_unverified() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("verify_all_none_unverified")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_verified_registry(ctx.path()))
        .test(&["verify", "--all"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// JSON output for already-verified deployment has correct camelCase fields.
#[test]
fn verify_json_already_verified() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("verify_json_already_verified")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_verified_registry(ctx.path()))
        .test(&["verify", "dep-verified", "--json"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Unknown verifier produces an error with valid verifier list.
#[test]
fn verify_error_unknown_verifier() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("verify_error_unknown_verifier")
        .setup(&["init"])
        .test(&["verify", "--all", "--verifier", "custom"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Uninitialized project (no .treb/) produces an error.
#[test]
fn verify_uninitialized() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("verify_uninitialized")
        .pre_setup_hook(|ctx| {
            std::fs::remove_dir_all(ctx.path().join(".treb")).unwrap();
        })
        .test(&["verify", "--all"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// No foundry project produces an error.
#[test]
fn verify_no_foundry_project() {
    let ctx = TestContext::new("compose-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("verify_no_foundry_project")
        .test(&["verify", "--all"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}
