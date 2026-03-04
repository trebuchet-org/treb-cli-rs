//! Golden-file integration tests for `treb sync`.
//!
//! Tests exercise empty-state paths: no safe transactions (plain/JSON),
//! network filter with no matches, uninitialized project, and no foundry project.

mod framework;
mod helpers;

use framework::{
    context::TestContext,
    integration_test::{IntegrationTest, run_integration_test},
    normalizer::PathNormalizer,
};

// ── Tests ────────────────────────────────────────────────────────────────

/// Empty registry with no safe transactions prints informational message.
#[test]
fn sync_no_safe_txs() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("sync_no_safe_txs")
        .setup(&["init"])
        .test(&["sync"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Empty registry with --network filter prints network-specific message.
#[test]
fn sync_no_safe_txs_network_filter() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("sync_no_safe_txs_network_filter")
        .setup(&["init"])
        .test(&["sync", "--network", "1"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Empty registry with --json outputs zero-count JSON.
#[test]
fn sync_json_empty() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("sync_json_empty")
        .setup(&["init"])
        .test(&["sync", "--json"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Uninitialized project (no .treb/) produces an error.
#[test]
fn sync_uninitialized() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("sync_uninitialized")
        .pre_setup_hook(|ctx| {
            std::fs::remove_dir_all(ctx.path().join(".treb")).unwrap();
        })
        .test(&["sync"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// No foundry project produces an error.
#[test]
fn sync_no_foundry_project() {
    let ctx = TestContext::new("compose-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("sync_no_foundry_project")
        .test(&["sync"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}
