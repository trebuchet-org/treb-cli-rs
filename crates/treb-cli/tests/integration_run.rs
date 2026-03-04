//! Golden-file integration tests for `treb run`.
//!
//! Tests exercise basic deployment, dry-run, and JSON output modes using
//! in-process Anvil nodes via `TestContext::with_anvil()`.

mod framework;
mod helpers;

use framework::context::TestContext;
use framework::integration_test::{run_integration_test, IntegrationTest};
use framework::normalizer::{
    BlockNumberNormalizer, CompilerOutputNormalizer, DurationNormalizer, GasNormalizer,
    PathNormalizer,
};

/// Basic deployment with broadcast against a live Anvil node.
///
/// Verifies deployment table output and registry artifact writes
/// (deployments.json + transactions.json).
#[tokio::test(flavor = "multi_thread")]
async fn run_basic() {
    let ctx = TestContext::new("project")
        .with_anvil("anvil-31337")
        .await
        .expect("failed to spawn anvil");

    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("run_basic")
        .setup(&["init"])
        .test(&[
            "run",
            "script/Deploy.s.sol",
            "--network",
            "anvil-31337",
            "--broadcast",
            "--non-interactive",
        ])
        .output_artifact(".treb/deployments.json")
        .output_artifact(".treb/transactions.json")
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(CompilerOutputNormalizer))
        .extra_normalizer(Box::new(GasNormalizer))
        .extra_normalizer(Box::new(BlockNumberNormalizer))
        .extra_normalizer(Box::new(DurationNormalizer));

    run_integration_test(&test, &ctx);
}

/// Dry-run execution — no broadcast, no registry writes.
///
/// Verifies `[DRY RUN]` banner appears and no output artifacts are written.
#[tokio::test(flavor = "multi_thread")]
async fn run_dry_run() {
    let ctx = TestContext::new("project")
        .with_anvil("anvil-31337")
        .await
        .expect("failed to spawn anvil");

    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("run_dry_run")
        .setup(&["init"])
        .test(&[
            "run",
            "script/Deploy.s.sol",
            "--network",
            "anvil-31337",
            "--dry-run",
            "--non-interactive",
        ])
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(CompilerOutputNormalizer))
        .extra_normalizer(Box::new(GasNormalizer))
        .extra_normalizer(Box::new(BlockNumberNormalizer))
        .extra_normalizer(Box::new(DurationNormalizer));

    run_integration_test(&test, &ctx);
}

/// JSON output mode for a basic deployment.
///
/// Verifies JSON structure with `success`, `dry_run`, `deployments`,
/// and `transactions` fields.
#[tokio::test(flavor = "multi_thread")]
async fn run_basic_json() {
    let ctx = TestContext::new("project")
        .with_anvil("anvil-31337")
        .await
        .expect("failed to spawn anvil");

    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("run_basic_json")
        .setup(&["init"])
        .test(&[
            "run",
            "script/Deploy.s.sol",
            "--network",
            "anvil-31337",
            "--broadcast",
            "--non-interactive",
            "--json",
        ])
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(CompilerOutputNormalizer))
        .extra_normalizer(Box::new(GasNormalizer))
        .extra_normalizer(Box::new(BlockNumberNormalizer))
        .extra_normalizer(Box::new(DurationNormalizer));

    run_integration_test(&test, &ctx);
}
