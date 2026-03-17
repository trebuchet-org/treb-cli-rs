//! Golden-file integration tests for `treb registry tag`.

mod framework;
mod helpers;

use std::{collections::HashMap, fs};

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
use treb_registry::{read_versioned_file, write_versioned_file};

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

fn make_tag_deployment(
    namespace: &str,
    chain_id: u64,
    contract_name: &str,
    label: &str,
    address: &str,
    tags: Option<Vec<&str>>,
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
        tags: tags.map(|entries| entries.into_iter().map(str::to_string).collect()),
        created_at: ts,
        updated_at: ts,
    }
}

/// Seed the registry and pre-add a "v3-release" tag to the FPMM:v3.0.0 deployment.
/// Used by remove and duplicate-add tests that need a tag already present.
fn seed_registry_with_tag(project_root: &std::path::Path) {
    helpers::seed_registry(project_root);
    let dep_path = project_root.join(".treb/deployments.json");
    let mut map: serde_json::Map<String, serde_json::Value> =
        read_versioned_file(&dep_path).unwrap();
    let dep = map.get_mut("mainnet/42220/FPMM:v3.0.0").unwrap();
    dep.as_object_mut().unwrap().insert("tags".to_string(), serde_json::json!(["v3-release"]));
    write_versioned_file(&dep_path, &map).unwrap();
}

/// Show tags on a deployment with no tags displays "No tags".
#[test]
fn tag_show_empty() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("tag_show_empty")
        .setup(&["init"])
        .post_setup_hook(|ctx| helpers::seed_registry(ctx.path()))
        .test(&["registry", "tag", "mainnet/42220/FPMM:v3.0.0"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// JSON show tags on a deployment with no tags produces valid JSON with empty tags array.
#[test]
fn tag_show_json_empty() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("tag_show_json_empty")
        .setup(&["init"])
        .post_setup_hook(|ctx| helpers::seed_registry(ctx.path()))
        .test(&["registry", "tag", "--json", "mainnet/42220/FPMM:v3.0.0"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Adding a tag displays confirmation and persists to deployments.json.
#[test]
fn tag_add() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("tag_add")
        .setup(&["init"])
        .post_setup_hook(|ctx| helpers::seed_registry(ctx.path()))
        .test(&["registry", "tag", "--add", "v3-release", "mainnet/42220/FPMM:v3.0.0"])
        .post_test_hook(|ctx| {
            // Extract tags for the modified deployment into a small deterministic artifact.
            // deployments.json uses HashMap so key order is non-deterministic;
            // writing just the tags avoids golden file flakiness.
            let deployments_path = ctx.path().join(".treb/deployments.json");
            let map: serde_json::Map<String, serde_json::Value> =
                read_versioned_file(&deployments_path).unwrap();
            let dep = map.get("mainnet/42220/FPMM:v3.0.0").unwrap();
            let tags = dep.get("tags").unwrap();
            let artifact = serde_json::json!({
                "deploymentId": "mainnet/42220/FPMM:v3.0.0",
                "tags": tags
            });
            let out = serde_json::to_string_pretty(&artifact).unwrap();
            std::fs::write(ctx.path().join(".treb/tag_check.json"), out).unwrap();
        })
        .output_artifact(".treb/tag_check.json")
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// A mutating write against Go-created registry data preserves the bare JSON map format.
#[test]
fn tag_add_preserves_go_compatible_bare_deployments_file() {
    let ctx = TestContext::new("project");
    ctx.run(["init"]).success();
    helpers::seed_go_compat_registry(ctx.path());

    let deployments_path = ctx.path().join(".treb/deployments.json");
    let before = fs::read_to_string(&deployments_path).expect("read go deployments fixture");
    assert!(
        !before.contains("\"_format\""),
        "seeded fixture should start as the Go-compatible bare-map format"
    );

    ctx.run(["registry", "tag", "--add", "core", "mainnet/42220/CDPLiquidityStrategy:v3.0.0"]).success();

    let after = fs::read_to_string(&deployments_path).expect("read updated deployments file");
    let json: serde_json::Value =
        serde_json::from_str(&after).expect("updated deployments file should be valid json");

    let entries = json.as_object().expect("deployments file should remain a bare JSON map");
    assert!(
        !entries.contains_key("_format"),
        "Go-compatible deployments.json must not reintroduce the legacy wrapper"
    );

    let deployment = entries
        .get("mainnet/42220/CDPLiquidityStrategy:v3.0.0")
        .and_then(serde_json::Value::as_object)
        .expect("updated deployments should preserve the tagged deployment");

    assert_eq!(deployment.get("contractName"), Some(&serde_json::json!("CDPLiquidityStrategy")));
    assert_eq!(deployment.get("tags"), Some(&serde_json::json!(["core"])));
}

/// Adding a tag with --json produces valid JSON with action and tag fields.
#[test]
fn tag_add_json() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("tag_add_json")
        .setup(&["init"])
        .post_setup_hook(|ctx| helpers::seed_registry(ctx.path()))
        .test(&["registry", "tag", "--json", "--add", "v3-release", "mainnet/42220/FPMM:v3.0.0"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Adding a tag then showing tags displays both the confirmation and the tag list.
#[test]
fn tag_add_then_show() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("tag_add_then_show")
        .setup(&["init"])
        .post_setup_hook(|ctx| helpers::seed_registry(ctx.path()))
        .test(&["registry", "tag", "--add", "v3-release", "mainnet/42220/FPMM:v3.0.0"])
        .test(&["registry", "tag", "mainnet/42220/FPMM:v3.0.0"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Removing an existing tag displays confirmation.
#[test]
fn tag_remove() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("tag_remove")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_registry_with_tag(ctx.path()))
        .test(&["registry", "tag", "--remove", "v3-release", "mainnet/42220/FPMM:v3.0.0"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Removing an existing tag with --json produces valid JSON with action "remove".
#[test]
fn tag_remove_json() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("tag_remove_json")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_registry_with_tag(ctx.path()))
        .test(&["registry", "tag", "--json", "--remove", "v3-release", "mainnet/42220/FPMM:v3.0.0"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Adding a tag that already exists produces an error containing "already exists".
#[test]
fn tag_add_duplicate_error() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("tag_add_duplicate_error")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_registry_with_tag(ctx.path()))
        .test(&["registry", "tag", "--add", "v3-release", "mainnet/42220/FPMM:v3.0.0"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Removing a nonexistent tag produces an error containing "not found".
#[test]
fn tag_remove_nonexistent_error() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("tag_remove_nonexistent_error")
        .setup(&["init"])
        .post_setup_hook(|ctx| helpers::seed_registry(ctx.path()))
        .test(&["registry", "tag", "--remove", "v3-release", "mainnet/42220/FPMM:v3.0.0"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Running tag on an uninitialized project produces an error mentioning treb init.
#[test]
fn tag_uninitialized() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("tag_uninitialized")
        .pre_setup_hook(|ctx| {
            std::fs::remove_dir_all(ctx.treb_dir()).ok();
        })
        .test(&["registry", "tag", "mainnet/42220/FPMM:v3.0.0"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

#[test]
fn tag_show_namespace_scope_resolves_the_filtered_deployment() {
    let ctx = TestContext::new("project");
    init_project_with_custom_deployments(
        &ctx,
        [
            make_tag_deployment(
                "mainnet",
                42220,
                "Counter",
                "v1",
                "0x0000000000000000000000000000000000001111",
                Some(vec!["stable"]),
            ),
            make_tag_deployment(
                "staging",
                42220,
                "Counter",
                "v1",
                "0x0000000000000000000000000000000000002222",
                Some(vec!["beta"]),
            ),
        ],
    );

    ctx.run_with_env(["registry", "tag", "--namespace", "mainnet", "Counter"], [("NO_COLOR", "1")])
        .success()
        .stdout(predicate::str::contains("mainnet/42220/Counter:v1"))
        .stdout(predicate::str::contains("stable"))
        .stdout(predicate::str::contains("beta").not());
}

#[test]
fn tag_add_namespace_and_network_scope_only_updates_the_matching_deployment() {
    let ctx = TestContext::new("project");
    init_project_with_custom_deployments(
        &ctx,
        [
            make_tag_deployment(
                "mainnet",
                42220,
                "Counter",
                "v1",
                "0x0000000000000000000000000000000000001111",
                None,
            ),
            make_tag_deployment(
                "mainnet",
                1,
                "Counter",
                "v1",
                "0x0000000000000000000000000000000000002222",
                None,
            ),
        ],
    );

    ctx.run(["registry", "tag", "--add", "v2", "-s", "mainnet", "-n", "42220", "Counter"])
        .success()
        .stdout(predicate::str::contains("mainnet/42220/Counter:v1"));

    let registry = treb_registry::Registry::open(ctx.path()).expect("registry should open");
    assert_eq!(
        registry.get_deployment("mainnet/42220/Counter:v1").unwrap().tags,
        Some(vec!["v2".to_string()])
    );
    assert_eq!(registry.get_deployment("mainnet/1/Counter:v1").unwrap().tags, None);
}

#[test]
fn tag_remove_namespace_scope_only_updates_the_matching_deployment() {
    let ctx = TestContext::new("project");
    init_project_with_custom_deployments(
        &ctx,
        [
            make_tag_deployment(
                "mainnet",
                42220,
                "Counter",
                "v1",
                "0x0000000000000000000000000000000000001111",
                Some(vec!["v2"]),
            ),
            make_tag_deployment(
                "staging",
                42220,
                "Counter",
                "v1",
                "0x0000000000000000000000000000000000002222",
                Some(vec!["v2"]),
            ),
        ],
    );

    ctx.run(["registry", "tag", "--remove", "v2", "--namespace", "mainnet", "Counter"])
        .success()
        .stdout(predicate::str::contains("mainnet/42220/Counter:v1"));

    let registry = treb_registry::Registry::open(ctx.path()).expect("registry should open");
    assert_eq!(registry.get_deployment("mainnet/42220/Counter:v1").unwrap().tags, None);
    assert_eq!(
        registry.get_deployment("staging/42220/Counter:v1").unwrap().tags,
        Some(vec!["v2".to_string()])
    );
}

#[test]
fn tag_network_scope_errors_when_the_match_is_outside_the_filter() {
    let ctx = TestContext::new("project");
    init_project_with_custom_deployments(
        &ctx,
        [make_tag_deployment(
            "mainnet",
            42220,
            "Counter",
            "v1",
            "0x0000000000000000000000000000000000001111",
            None,
        )],
    );

    ctx.run(["registry", "tag", "--network", "1", "Counter"])
        .failure()
        .stderr(predicate::str::contains("no deployment found matching 'Counter' on network '1'"));
}
