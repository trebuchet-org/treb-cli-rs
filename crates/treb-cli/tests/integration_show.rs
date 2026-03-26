//! Golden-file integration tests for `treb show`.

mod framework;
mod helpers;

use std::collections::HashMap;

use chrono::Utc;
use framework::{
    context::TestContext,
    integration_test::{IntegrationTest, run_integration_test},
    normalizer::PathNormalizer,
};
use predicates::prelude::*;
use treb_core::types::{
    ArtifactInfo, Deployment, DeploymentMethod, DeploymentStrategy, DeploymentType,
    VerificationInfo, VerificationStatus,
};

fn init_project_with_custom_deployments(
    ctx: &TestContext,
    deployments: impl IntoIterator<Item = Deployment>,
) {
    ctx.run(["init"]).success();

    let mut registry = treb_registry::Registry::open(ctx.path()).expect("registry should open");
    for deployment in deployments {
        registry.insert_deployment(deployment).expect("deployment insert should succeed");
    }
}

fn make_show_deployment(
    namespace: &str,
    chain_id: u64,
    contract_name: &str,
    label: &str,
    address: &str,
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

/// Show by full deployment ID displays all section headers.
#[test]
fn show_full_id() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("show_full_id")
        .setup(&["init"])
        .post_setup_hook(|ctx| helpers::seed_registry(ctx.path()))
        .test(&["show", "mainnet/42220/FPMM:v3.0.0"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// JSON output for a deployment is a valid JSON object.
#[test]
fn show_json() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("show_json")
        .setup(&["init"])
        .post_setup_hook(|ctx| helpers::seed_registry(ctx.path()))
        .test(&["show", "--json", "mainnet/42220/FPMM:v3.0.0"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Proxy deployment shows Proxy Info section.
#[test]
fn show_proxy() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("show_proxy")
        .setup(&["init"])
        .post_setup_hook(|ctx| helpers::seed_registry(ctx.path()))
        .test(&["show", "mainnet/42220/TransparentUpgradeableProxy:FPMMFactory"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Non-proxy deployment does NOT show Proxy Info section.
#[test]
fn show_non_proxy() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("show_non_proxy")
        .setup(&["init"])
        .post_setup_hook(|ctx| helpers::seed_registry(ctx.path()))
        .test(&["show", "mainnet/42220/FPMMFactory:v3.0.0"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Resolution by unique contract name finds the deployment.
#[test]
fn show_by_contract_name() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("show_by_contract_name")
        .setup(&["init"])
        .post_setup_hook(|ctx| helpers::seed_registry(ctx.path()))
        .test(&["show", "FPMMFactory"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Show deployment with populated verifiers displays per-verifier detail lines.
#[test]
fn show_with_verifiers() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("show_with_verifiers")
        .setup(&["init"])
        .post_setup_hook(|ctx| helpers::seed_registry(ctx.path()))
        .test(&["show", "mainnet/42220/FPMM:v3.0.0"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Show deployment with tags displays the Tags section.
#[test]
fn show_with_tags() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("show_with_tags")
        .setup(&["init"])
        .post_setup_hook(|ctx| helpers::seed_registry(ctx.path()))
        .test(&["show", "mainnet/42220/FPMMFactory:v3.0.0"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Nonexistent deployment produces an error with 'no deployment found'.
#[test]
fn show_nonexistent() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("show_nonexistent")
        .setup(&["init"])
        .post_setup_hook(|ctx| helpers::seed_registry(ctx.path()))
        .test(&["show", "nonexistent/1/Foo:bar"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Show without initialized project fails with error mentioning treb init.
#[test]
fn show_uninitialized() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("show_uninitialized")
        .pre_setup_hook(|ctx| {
            std::fs::remove_dir_all(ctx.treb_dir()).ok();
        })
        .test(&["show", "anything"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

#[test]
fn show_namespace_filter_scopes_resolution_and_errors_outside_scope() {
    let ctx = TestContext::new("project");
    init_project_with_custom_deployments(
        &ctx,
        [
            make_show_deployment(
                "mainnet",
                42220,
                "Counter",
                "v1",
                "0x0000000000000000000000000000000000000001",
            ),
            make_show_deployment(
                "staging",
                42220,
                "Counter",
                "v1",
                "0x0000000000000000000000000000000000000002",
            ),
        ],
    );

    ctx.run_with_env(["show", "--namespace", "mainnet", "Counter"], [("NO_COLOR", "1")])
        .success()
        .stdout(predicate::str::contains("Deployment: mainnet/42220/Counter:v1"))
        .stdout(predicate::str::contains("Namespace: mainnet"))
        .stdout(predicate::str::contains("Namespace: staging").not());

    ctx.run(["show", "--namespace", "prod", "Counter"]).failure().stderr(predicate::str::contains(
        "no deployment found matching 'Counter' in namespace 'prod'",
    ));
}

#[test]
fn show_network_filter_scopes_resolution_and_errors_outside_scope() {
    let ctx = TestContext::new("project");
    init_project_with_custom_deployments(
        &ctx,
        [
            make_show_deployment(
                "mainnet",
                1,
                "Counter",
                "v1",
                "0x0000000000000000000000000000000000000001",
            ),
            make_show_deployment(
                "mainnet",
                42220,
                "Counter",
                "v1",
                "0x0000000000000000000000000000000000000002",
            ),
        ],
    );
    ctx.run(["config", "set", "namespace", "mainnet"]).success();

    ctx.run_with_env(["show", "--network", "42220", "Counter"], [("NO_COLOR", "1")])
        .success()
        .stdout(predicate::str::contains("Deployment: mainnet/42220/Counter:v1"))
        .stdout(predicate::str::contains("Network: 42220"))
        .stdout(predicate::str::contains("Network: 1").not());

    ctx.run(["show", "--network", "11155111", "Counter"]).failure().stderr(
        predicate::str::contains(
            "no deployment found matching 'Counter' in namespace 'mainnet' on network '11155111'",
        ),
    );
}

#[test]
fn show_no_fork_filter_excludes_fork_deployments() {
    let ctx = TestContext::new("project");
    ctx.run(["init"]).success();
    helpers::seed_registry(ctx.path());

    ctx.run(["show", "--namespace", "fork/42220", "MockToken"]).success();

    ctx.run(["show", "--namespace", "fork/42220", "--no-fork", "MockToken"])
        .failure()
        .stderr(predicate::str::contains(
            "no deployment found matching 'MockToken' in namespace 'fork/42220' excluding fork deployments",
        ));
}

#[test]
fn show_combined_filters_json_returns_only_the_matching_deployment() {
    let ctx = TestContext::new("project");
    init_project_with_custom_deployments(
        &ctx,
        [
            make_show_deployment(
                "mainnet",
                1,
                "Counter",
                "v1",
                "0x0000000000000000000000000000000000000001",
            ),
            make_show_deployment(
                "mainnet",
                42220,
                "Counter",
                "v1",
                "0x0000000000000000000000000000000000000002",
            ),
            make_show_deployment(
                "fork/42220",
                42220,
                "Counter",
                "v1",
                "0x0000000000000000000000000000000000000003",
            ),
        ],
    );

    let assert = ctx
        .run([
            "show",
            "--namespace",
            "mainnet",
            "--network",
            "42220",
            "--no-fork",
            "--json",
            "Counter",
        ])
        .success();

    let json: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("show output should be json");
    assert_eq!(json["deployment"]["id"], "mainnet/42220/Counter:v1");
    assert_eq!(json["deployment"]["namespace"], "mainnet");
    assert_eq!(json["deployment"]["chainId"], 42220);
    assert!(json.get("fork").is_none(), "combined filters should exclude fork deployments");
}
