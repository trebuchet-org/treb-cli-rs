//! Golden-file integration tests for `treb init`.

mod framework;

use framework::{
    context::TestContext,
    integration_test::{IntegrationTest, run_integration_test},
    normalizer::PathNormalizer,
};

/// Fresh init creates .treb/ with config.local.json.
#[test]
fn init_fresh() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("init_fresh")
        .pre_setup_hook(|ctx| {
            // TestWorkdir creates .treb/ by default; remove it so init can create fresh.
            std::fs::remove_dir_all(ctx.treb_dir()).ok();
        })
        .test(&["init"])
        .output_artifact(".treb/config.local.json")
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Re-running init without --force is idempotent.
#[test]
fn init_idempotent() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("init_idempotent")
        .pre_setup_hook(|ctx| {
            std::fs::remove_dir_all(ctx.treb_dir()).ok();
        })
        .setup(&["init"])
        .test(&["init"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Init with --force resets local config to defaults.
#[test]
fn init_force() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("init_force")
        .pre_setup_hook(|ctx| {
            std::fs::remove_dir_all(ctx.treb_dir()).ok();
        })
        .setup(&["init"])
        .test(&["init", "--force"])
        .output_artifact(".treb/config.local.json")
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Init without foundry.toml fails with an error.
#[test]
fn init_no_foundry_toml() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("init_no_foundry_toml")
        .pre_setup_hook(|ctx| {
            std::fs::remove_dir_all(ctx.treb_dir()).ok();
            std::fs::remove_file(ctx.path().join("foundry.toml")).ok();
        })
        .test(&["init"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}
