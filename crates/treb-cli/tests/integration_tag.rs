//! Golden-file integration tests for `treb tag`.

mod framework;
mod helpers;

use framework::{
    context::TestContext,
    integration_test::{IntegrationTest, run_integration_test},
    normalizer::PathNormalizer,
};

/// Seed the registry and pre-add a "v3-release" tag to the FPMM:v3.0.0 deployment.
/// Used by remove and duplicate-add tests that need a tag already present.
fn seed_registry_with_tag(project_root: &std::path::Path) {
    helpers::seed_registry(project_root);
    let dep_path = project_root.join(".treb/deployments.json");
    let data = std::fs::read_to_string(&dep_path).unwrap();
    let mut map: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&data).unwrap();
    let dep = map.get_mut("mainnet/42220/FPMM:v3.0.0").unwrap();
    dep.as_object_mut().unwrap().insert("tags".to_string(), serde_json::json!(["v3-release"]));
    std::fs::write(&dep_path, serde_json::to_string_pretty(&map).unwrap()).unwrap();
}

/// Show tags on a deployment with no tags displays "No tags".
#[test]
fn tag_show_empty() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("tag_show_empty")
        .setup(&["init"])
        .post_setup_hook(|ctx| helpers::seed_registry(ctx.path()))
        .test(&["tag", "mainnet/42220/FPMM:v3.0.0"])
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
        .test(&["tag", "--json", "mainnet/42220/FPMM:v3.0.0"])
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
        .test(&["tag", "--add", "v3-release", "mainnet/42220/FPMM:v3.0.0"])
        .post_test_hook(|ctx| {
            // Extract tags for the modified deployment into a small deterministic artifact.
            // deployments.json uses HashMap so key order is non-deterministic;
            // writing just the tags avoids golden file flakiness.
            let deployments_path = ctx.path().join(".treb/deployments.json");
            let data = std::fs::read_to_string(&deployments_path).unwrap();
            let map: serde_json::Map<String, serde_json::Value> =
                serde_json::from_str(&data).unwrap();
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

/// Adding a tag with --json produces valid JSON with action and tag fields.
#[test]
fn tag_add_json() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("tag_add_json")
        .setup(&["init"])
        .post_setup_hook(|ctx| helpers::seed_registry(ctx.path()))
        .test(&["tag", "--json", "--add", "v3-release", "mainnet/42220/FPMM:v3.0.0"])
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
        .test(&["tag", "--add", "v3-release", "mainnet/42220/FPMM:v3.0.0"])
        .test(&["tag", "mainnet/42220/FPMM:v3.0.0"])
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
        .test(&["tag", "--remove", "v3-release", "mainnet/42220/FPMM:v3.0.0"])
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
        .test(&["tag", "--json", "--remove", "v3-release", "mainnet/42220/FPMM:v3.0.0"])
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
        .test(&["tag", "--add", "v3-release", "mainnet/42220/FPMM:v3.0.0"])
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
        .test(&["tag", "--remove", "v3-release", "mainnet/42220/FPMM:v3.0.0"])
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
        .test(&["tag", "mainnet/42220/FPMM:v3.0.0"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}
