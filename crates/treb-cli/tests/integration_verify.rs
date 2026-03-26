//! Golden-file integration tests for `treb verify`.
//!
//! Tests exercise already-verified skip, no unverified deployments,
//! JSON output, unknown verifier error, uninitialized project,
//! and no foundry project error paths.

mod framework;
mod helpers;

use std::collections::HashMap;

use chrono::Utc;
use predicates::prelude::*;
use treb_core::types::{
    ArtifactInfo, Deployment, DeploymentMethod, DeploymentStrategy, DeploymentType,
    TransactionStatus, VerificationInfo, VerificationStatus, VerifierStatus,
};
use treb_registry::Registry;

use framework::{
    context::TestContext,
    integration_test::{IntegrationTest, run_integration_test},
    normalizer::PathNormalizer,
};

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
        execution: None,
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
        broadcast_file: None,
        environment: "testnet".to_string(),
        created_at: ts,
    }
}

/// Build a verified deployment whose `verifiers` map has per-verifier entries.
fn make_verified_deployment_with_verifiers(
    id: &str,
    tx_id: &str,
    chain_id: u64,
) -> treb_core::types::Deployment {
    let mut dep = make_verified_deployment(id, tx_id, chain_id);
    let mut verifiers = HashMap::new();
    verifiers.insert(
        "etherscan".to_string(),
        VerifierStatus {
            status: "VERIFIED".to_string(),
            url: format!("https://etherscan.io/address/0x{:040x}#code", 1u64),
            reason: String::new(),
        },
    );
    verifiers.insert(
        "sourcify".to_string(),
        VerifierStatus {
            status: "VERIFIED".to_string(),
            url: format!("https://repo.sourcify.dev/contracts/full_match/1/0x{:040x}/", 1u64),
            reason: String::new(),
        },
    );
    dep.verification.verifiers = verifiers;
    dep
}

/// Seed the registry with one already-verified deployment and its transaction.
fn seed_verified_registry(project_root: &std::path::Path) {
    let mut registry = Registry::open(project_root).expect("registry should open");

    registry
        .insert_transaction(make_transaction("tx-1", vec!["dep-verified".to_string()], 1))
        .unwrap();
    registry.insert_deployment(make_verified_deployment("dep-verified", "tx-1", 1)).unwrap();
}

/// Seed the registry with a verified deployment that has per-verifier breakdown.
fn seed_verified_registry_with_verifiers(project_root: &std::path::Path) {
    let mut registry = Registry::open(project_root).expect("registry should open");

    registry
        .insert_transaction(make_transaction("tx-1", vec!["dep-verified".to_string()], 1))
        .unwrap();
    registry
        .insert_deployment(make_verified_deployment_with_verifiers("dep-verified", "tx-1", 1))
        .unwrap();
}

fn init_project_with_custom_deployments(
    ctx: &TestContext,
    deployments: impl IntoIterator<Item = Deployment>,
) {
    ctx.run(["init"]).success();

    let mut registry = Registry::open(ctx.path()).expect("registry should open");
    for deployment in deployments {
        registry.insert_deployment(deployment).expect("deployment insert should succeed");
    }
}

fn make_scoped_verify_deployment(
    namespace: &str,
    chain_id: u64,
    contract_name: &str,
    label: &str,
    address: &str,
    verification_status: VerificationStatus,
) -> Deployment {
    let ts = Utc::now();

    Deployment {
        id: format!("{namespace}/{chain_id}/{contract_name}:{label}"),
        namespace: namespace.to_string(),
        chain_id,
        contract_name: contract_name.to_string(),
        label: label.to_string(),
        address: address.to_string(),
        deployment_type: DeploymentType::Singleton,
        execution: None,
        transaction_id: format!("tx-{namespace}-{chain_id}-{contract_name}"),
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
            status: verification_status,
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

// ── New multi-verifier golden file tests ────────────────────────────────

/// JSON output with --sourcify shorthand shows verifier field as "sourcify".
#[test]
fn verify_json_already_verified_sourcify() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("verify_json_already_verified_sourcify")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_verified_registry(ctx.path()))
        .test(&["verify", "dep-verified", "--json", "--sourcify"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// JSON output with per-verifier breakdown from populated verifiers map.
#[test]
fn verify_json_already_verified_with_verifier_breakdown() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("verify_json_already_verified_with_verifier_breakdown")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_verified_registry_with_verifiers(ctx.path()))
        .test(&["verify", "dep-verified", "--json"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Already-verified deployment with --etherscan --sourcify multi-shorthand prints skip.
#[test]
fn verify_already_verified_multi_shorthand() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("verify_already_verified_multi_shorthand")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_verified_registry(ctx.path()))
        .test(&["verify", "dep-verified", "--etherscan", "--sourcify"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Short verifier flags and their combined `-ebs` form should be accepted.
#[test]
fn verify_short_flags_accepted() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("verify_short_flags_accepted")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_verified_registry(ctx.path()))
        .test(&["verify", "dep-verified", "-e"])
        .test(&["verify", "dep-verified", "-b"])
        .test(&["verify", "dep-verified", "-s"])
        .test(&["verify", "dep-verified", "-ebs"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// The remaining verify flag additions should be accepted together on the happy skip path.
#[test]
fn verify_extended_flag_surface_accepted() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("verify_extended_flag_surface_accepted")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_verified_registry(ctx.path()))
        .test(&[
            "verify",
            "dep-verified",
            "-b",
            "--blockscout-verifier-url",
            "https://example.com/api",
            "--contract-path",
            "./src/Counter.sol:Counter",
            "--debug",
            "--namespace",
            "default",
            "-n",
            "mainnet",
        ])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// --all --json with no unverified deployments returns empty JSON array.
#[test]
fn verify_all_none_unverified_json() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("verify_all_none_unverified_json")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_verified_registry(ctx.path()))
        .test(&["verify", "--all", "--json"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// JSON output with --etherscan --sourcify multi-shorthand shows first verifier.
#[test]
fn verify_json_already_verified_multi_shorthand() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("verify_json_already_verified_multi_shorthand")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_verified_registry(ctx.path()))
        .test(&["verify", "dep-verified", "--json", "--etherscan", "--sourcify"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

#[test]
fn verify_namespace_scope_resolves_the_filtered_deployment() {
    let ctx = TestContext::new("minimal-project");
    init_project_with_custom_deployments(
        &ctx,
        [
            make_scoped_verify_deployment(
                "mainnet",
                42220,
                "Counter",
                "v1",
                "0x0000000000000000000000000000000000001111",
                VerificationStatus::Verified,
            ),
            make_scoped_verify_deployment(
                "staging",
                42220,
                "Counter",
                "v1",
                "0x0000000000000000000000000000000000002222",
                VerificationStatus::Verified,
            ),
        ],
    );

    ctx.run(["verify", "Counter", "--namespace", "staging", "--json"])
        .success()
        .stdout(predicate::str::contains("\"deploymentId\": \"staging/42220/Counter:v1\""))
        .stdout(predicate::str::contains("\"deploymentId\": \"mainnet/42220/Counter:v1\"").not());
}

#[test]
fn verify_network_scope_resolves_the_filtered_deployment() {
    let ctx = TestContext::new("minimal-project");
    init_project_with_custom_deployments(
        &ctx,
        [
            make_scoped_verify_deployment(
                "default",
                1,
                "Counter",
                "v1",
                "0x0000000000000000000000000000000000001111",
                VerificationStatus::Verified,
            ),
            make_scoped_verify_deployment(
                "default",
                11155111,
                "Counter",
                "v1",
                "0x0000000000000000000000000000000000002222",
                VerificationStatus::Verified,
            ),
        ],
    );

    ctx.run(["verify", "Counter", "-n", "mainnet", "--json"])
        .success()
        .stdout(predicate::str::contains("\"deploymentId\": \"default/1/Counter:v1\""))
        .stdout(
            predicate::str::contains("\"deploymentId\": \"default/11155111/Counter:v1\"").not(),
        );
}

#[test]
fn verify_all_namespace_scope_only_updates_the_matching_deployment() {
    let ctx = TestContext::new("minimal-project");
    init_project_with_custom_deployments(
        &ctx,
        [
            make_scoped_verify_deployment(
                "mainnet",
                42220,
                "Counter",
                "v1",
                "not-an-address",
                VerificationStatus::Unverified,
            ),
            make_scoped_verify_deployment(
                "staging",
                42220,
                "Counter",
                "v1",
                "still-not-an-address",
                VerificationStatus::Unverified,
            ),
        ],
    );

    ctx.run(["verify", "--all", "--namespace", "staging", "--json"])
        .success()
        .stdout(predicate::str::contains("\"deploymentId\": \"staging/42220/Counter:v1\""))
        .stdout(predicate::str::contains("\"deploymentId\": \"mainnet/42220/Counter:v1\"").not());

    let registry = Registry::open(ctx.path()).expect("registry should open");
    assert_eq!(
        registry.get_deployment("staging/42220/Counter:v1").unwrap().verification.status,
        VerificationStatus::Failed
    );
    assert_eq!(
        registry.get_deployment("mainnet/42220/Counter:v1").unwrap().verification.status,
        VerificationStatus::Unverified
    );
}

#[test]
fn verify_queryless_scope_uses_filtered_deployments_before_prompting() {
    let ctx = TestContext::new("minimal-project");
    init_project_with_custom_deployments(
        &ctx,
        [make_scoped_verify_deployment(
            "mainnet",
            42220,
            "Counter",
            "v1",
            "0x0000000000000000000000000000000000001111",
            VerificationStatus::Verified,
        )],
    );

    ctx.run(["verify", "--namespace", "staging"])
        .failure()
        .stderr(predicate::str::contains("no deployments found in namespace 'staging'"));
}

#[test]
fn verify_scope_errors_include_context_suffix() {
    let ctx = TestContext::new("minimal-project");
    init_project_with_custom_deployments(
        &ctx,
        [make_scoped_verify_deployment(
            "mainnet",
            42220,
            "Counter",
            "v1",
            "0x0000000000000000000000000000000000001111",
            VerificationStatus::Verified,
        )],
    );
    ctx.run(["config", "set", "namespace", "mainnet"]).success();

    ctx.run(["verify", "--network", "1", "Counter"]).failure().stderr(predicate::str::contains(
        "no deployment found matching 'Counter' in namespace 'mainnet' on network '1'",
    ));
}
