//! Golden-file integration tests for `treb migrate`.
//!
//! Tests exercise config migration (v1→v2 dry-run, v1→v2 with backup, already-v2
//! detection) paths.

mod framework;
mod helpers;

use framework::{
    context::TestContext,
    integration_test::{IntegrationTest, run_integration_test},
    normalizer::{EpochNormalizer, PathNormalizer},
};

// ── Fixture helpers ──────────────────────────────────────────────────────

const V1_TREB_TOML: &str = r#"[ns.default.senders.deployer]
type = "private_key"
address = "0xDeployerAddr"
"#;

const V2_TREB_TOML: &str = r#"[accounts.deployer]
type = "private_key"
address = "0xDeployerAddr"
"#;

fn write_v1_treb_toml(ctx: &TestContext) {
    std::fs::write(ctx.path().join("treb.toml"), V1_TREB_TOML).expect("write v1 treb.toml");
}

fn write_v2_treb_toml(ctx: &TestContext) {
    std::fs::write(ctx.path().join("treb.toml"), V2_TREB_TOML).expect("write v2 treb.toml");
}

const FOUNDRY_TOML_WITH_TREB: &str = r#"[profile.default]
src = "src"
out = "out"
libs = ["lib"]

[profile.default.treb.senders.deployer]
type = "private_key"
address = "0xDeployerAddr"

[rpc_endpoints]
localhost = "http://localhost:8545"
"#;

fn write_foundry_with_treb_senders(ctx: &TestContext) {
    std::fs::write(ctx.path().join("foundry.toml"), FOUNDRY_TOML_WITH_TREB)
        .expect("write foundry.toml with treb senders");
}

// ── Config migration tests ───────────────────────────────────────────────

/// Dry-run v1→v2 migration prints the v2 TOML to stdout without modifying files.
#[test]
fn migrate_config_dry_run_v1() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("migrate_config_dry_run_v1")
        .setup(&["init"])
        .post_setup_hook(write_v1_treb_toml)
        .test(&["migrate", "config", "--dry-run"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// v1→v2 migration rewrites treb.toml with backup and prints completion message.
#[test]
fn migrate_config_v1_to_v2() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("migrate_config_v1_to_v2")
        .setup(&["init"])
        .post_setup_hook(write_v1_treb_toml)
        .test(&["migrate", "config"])
        .output_artifact("treb.toml")
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(EpochNormalizer));

    run_integration_test(&test, &ctx);
}

/// Already-v2 config prints "already v2 format" message.
#[test]
fn migrate_config_already_v2() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("migrate_config_already_v2")
        .setup(&["init"])
        .post_setup_hook(write_v2_treb_toml)
        .test(&["migrate", "config"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// JSON output for already-v2 config produces valid JSON with status field.
#[test]
fn migrate_config_json_already_v2() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("migrate_config_json_already_v2")
        .setup(&["init"])
        .post_setup_hook(write_v2_treb_toml)
        .test(&["migrate", "config", "--json"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// JSON dry-run output for v1 config produces valid JSON with dryRun and v2Content.
#[test]
fn migrate_config_json_dry_run() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("migrate_config_json_dry_run")
        .setup(&["init"])
        .post_setup_hook(write_v1_treb_toml)
        .test(&["migrate", "config", "--dry-run", "--json"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// No treb.toml (and no foundry.toml senders) produces an error.
#[test]
fn migrate_config_no_treb_toml() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("migrate_config_no_treb_toml")
        .setup(&["init"])
        .test(&["migrate", "config"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Foundry-only migration prints the Go-style warning and writes v2 config.
#[test]
fn migrate_config_foundry_only() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("migrate_config_foundry_only")
        .setup(&["init"])
        .post_setup_hook(write_foundry_with_treb_senders)
        .test(&["migrate", "config", "--yes"])
        .output_artifact("treb.toml")
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// v1→v2 migration with --yes flag skips prompts and writes directly.
#[test]
fn migrate_config_v1_to_v2_yes() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("migrate_config_v1_to_v2_yes")
        .setup(&["init"])
        .post_setup_hook(write_v1_treb_toml)
        .test(&["migrate", "config", "--yes"])
        .output_artifact("treb.toml")
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(EpochNormalizer));

    run_integration_test(&test, &ctx);
}

/// v1→v2 migration with --cleanup-foundry removes treb sections from foundry.toml.
#[test]
fn migrate_config_cleanup_foundry() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("migrate_config_cleanup_foundry")
        .setup(&["init"])
        .post_setup_hook(|ctx| {
            write_v1_treb_toml(ctx);
            write_foundry_with_treb_senders(ctx);
        })
        .test(&["migrate", "config", "--yes", "--cleanup-foundry"])
        .output_artifact("treb.toml")
        .output_artifact("foundry.toml")
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(EpochNormalizer));

    run_integration_test(&test, &ctx);
}
