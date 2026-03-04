//! Golden-file integration tests for `treb migrate`.
//!
//! Tests exercise config migration (v1→v2 dry-run, v1→v2 with backup, already-v2
//! detection) and registry migration (up-to-date, dry-run) paths.

mod framework;
mod helpers;

use framework::context::TestContext;
use framework::integration_test::{run_integration_test, IntegrationTest};
use framework::normalizer::{EpochNormalizer, PathNormalizer};

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
    std::fs::write(ctx.path().join("treb.toml"), V1_TREB_TOML)
        .expect("write v1 treb.toml");
}

fn write_v2_treb_toml(ctx: &TestContext) {
    std::fs::write(ctx.path().join("treb.toml"), V2_TREB_TOML)
        .expect("write v2 treb.toml");
}

// ── Config migration tests ───────────────────────────────────────────────

/// Dry-run v1→v2 migration prints the v2 TOML to stdout without modifying files.
#[test]
fn migrate_config_dry_run_v1() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("migrate_config_dry_run_v1")
        .setup(&["init"])
        .post_setup_hook(|ctx| write_v1_treb_toml(ctx))
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
        .post_setup_hook(|ctx| write_v1_treb_toml(ctx))
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
        .post_setup_hook(|ctx| write_v2_treb_toml(ctx))
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
        .post_setup_hook(|ctx| write_v2_treb_toml(ctx))
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
        .post_setup_hook(|ctx| write_v1_treb_toml(ctx))
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

// ── Registry migration tests ────────────────────────────────────────────

/// Up-to-date registry prints "up to date" message.
#[test]
fn migrate_registry_up_to_date() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("migrate_registry_up_to_date")
        .setup(&["init"])
        .test(&["migrate", "registry"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Dry-run on up-to-date registry prints "up to date" (same as non-dry-run
/// since no migrations are pending).
#[test]
fn migrate_registry_dry_run_up_to_date() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("migrate_registry_dry_run_up_to_date")
        .setup(&["init"])
        .test(&["migrate", "registry", "--dry-run"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}
