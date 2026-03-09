//! Golden-file integration tests for `treb list`.

mod framework;
mod helpers;

use framework::{
    context::TestContext,
    integration_test::{IntegrationTest, run_integration_test},
    normalizer::PathNormalizer,
};

/// Table output shows all 3 seeded deployments with correct columns.
#[test]
fn list_table() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("list_table")
        .setup(&["init"])
        .post_setup_hook(|ctx| helpers::seed_registry(ctx.path()))
        .test(&["list"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// JSON output is a valid JSON array with 3 elements.
#[test]
fn list_json() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("list_json")
        .setup(&["init"])
        .post_setup_hook(|ctx| helpers::seed_registry(ctx.path()))
        .test(&["list", "--json"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Empty registry (no deployments) shows "No deployments found."
#[test]
fn list_empty() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("list_empty")
        .setup(&["init"])
        .test(&["list"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// `treb ls` alias works identically to `treb list`.
#[test]
fn list_ls_alias() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("list_ls_alias")
        .setup(&["init"])
        .post_setup_hook(|ctx| helpers::seed_registry(ctx.path()))
        .test(&["ls"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Filter by namespace returns only matching deployments.
#[test]
fn list_filter_namespace() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("list_filter_namespace")
        .setup(&["init"])
        .post_setup_hook(|ctx| helpers::seed_registry(ctx.path()))
        .test(&["list", "--namespace", "mainnet"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Filter by non-matching namespace shows "No deployments found."
#[test]
fn list_filter_namespace_no_match() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("list_filter_namespace_no_match")
        .setup(&["init"])
        .post_setup_hook(|ctx| helpers::seed_registry(ctx.path()))
        .test(&["list", "--namespace", "nonexistent"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Filter by contract name returns only matching deployments.
#[test]
fn list_filter_contract() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("list_filter_contract")
        .setup(&["init"])
        .post_setup_hook(|ctx| helpers::seed_registry(ctx.path()))
        .test(&["list", "--contract", "FPMM"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Filter by deployment type returns only matching deployments.
#[test]
fn list_filter_type() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("list_filter_type")
        .setup(&["init"])
        .post_setup_hook(|ctx| helpers::seed_registry(ctx.path()))
        .test(&["list", "--type", "PROXY"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Fork-namespace deployments show [fork] badge in tree output.
#[test]
fn list_with_fork_badge() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("list_with_fork_badge")
        .setup(&["init"])
        .post_setup_hook(|ctx| helpers::seed_registry(ctx.path()))
        .test(&["list", "--fork"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Namespace discovery hint shows when filtering by non-existent namespace.
#[test]
fn list_namespace_discovery_hint() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("list_namespace_discovery_hint")
        .setup(&["init"])
        .post_setup_hook(|ctx| helpers::seed_registry(ctx.path()))
        .test(&["list", "--namespace", "staging"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Tag display shows first tag as '(tag_name)' in deployment rows.
#[test]
fn list_with_tags() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("list_with_tags")
        .setup(&["init"])
        .post_setup_hook(|ctx| helpers::seed_registry(ctx.path()))
        .test(&["list", "--tag", "v3-release"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// JSON output wraps deployments in {"deployments": [...]} object.
#[test]
fn list_json_wrapped() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("list_json_wrapped")
        .setup(&["init"])
        .post_setup_hook(|ctx| helpers::seed_registry(ctx.path()))
        .test(&["list", "--json", "--namespace", "nonexistent"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// List without initialized project fails with error mentioning treb init.
#[test]
fn list_uninitialized() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("list_uninitialized")
        .pre_setup_hook(|ctx| {
            std::fs::remove_dir_all(ctx.treb_dir()).ok();
        })
        .test(&["list"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}
