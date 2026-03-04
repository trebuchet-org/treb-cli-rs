//! Golden-file integration tests for `treb run`.
//!
//! Tests exercise basic deployment, dry-run, JSON output, verbose, debug,
//! and error paths using in-process Anvil nodes via `TestContext::with_anvil()`.

mod framework;
mod helpers;

use framework::{
    context::TestContext,
    integration_test::{IntegrationTest, run_integration_test},
    normalizer::{
        BlockNumberNormalizer, CompilerOutputNormalizer, DurationNormalizer, GasNormalizer,
        PathNormalizer,
    },
};

/// Basic deployment with broadcast against a live Anvil node.
///
/// Verifies deployment table output and registry artifact writes
/// (deployments.json + transactions.json).
#[tokio::test(flavor = "multi_thread")]
async fn run_basic() {
    let ctx =
        TestContext::new("project").with_anvil("anvil-31337").await.expect("failed to spawn anvil");

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
    let ctx =
        TestContext::new("project").with_anvil("anvil-31337").await.expect("failed to spawn anvil");

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
    let ctx =
        TestContext::new("project").with_anvil("anvil-31337").await.expect("failed to spawn anvil");

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

/// Debug mode — enables Forge debugger flag which adds trace output.
///
/// Verifies that `--debug` produces output beyond the basic deployment.
#[tokio::test(flavor = "multi_thread")]
async fn run_debug() {
    let ctx =
        TestContext::new("project").with_anvil("anvil-31337").await.expect("failed to spawn anvil");

    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("run_debug")
        .setup(&["init"])
        .test(&[
            "run",
            "script/Deploy.s.sol",
            "--network",
            "anvil-31337",
            "--broadcast",
            "--non-interactive",
            "--debug",
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

/// Verbose mode — shows config source, namespace, RPC, senders, and result summary.
///
/// Verifies extra verbose context appears in golden output.
#[tokio::test(flavor = "multi_thread")]
async fn run_verbose() {
    let ctx =
        TestContext::new("project").with_anvil("anvil-31337").await.expect("failed to spawn anvil");

    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("run_verbose")
        .setup(&["init"])
        .test(&[
            "run",
            "script/Deploy.s.sol",
            "--network",
            "anvil-31337",
            "--broadcast",
            "--non-interactive",
            "--verbose",
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

/// Error: missing script file.
///
/// Verifies the error message mentions the nonexistent script path.
#[test]
fn run_missing_script() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("run_missing_script")
        .setup(&["init"])
        .test(&[
            "run",
            "script/NonExistent.s.sol",
            "--network",
            "anvil-31337",
            "--broadcast",
            "--non-interactive",
        ])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Error: uninitialized project (no foundry.toml or .treb/).
///
/// Verifies the error message mentions `foundry.toml` or `treb init`.
#[test]
fn run_no_init() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("run_no_init")
        .pre_setup_hook(|ctx| {
            std::fs::remove_dir_all(ctx.treb_dir()).ok();
            std::fs::remove_file(ctx.path().join("foundry.toml")).ok();
        })
        .test(&["run", "script/Deploy.s.sol"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Error: bad function signature.
///
/// Verifies the error message mentions the invalid function signature.
#[tokio::test(flavor = "multi_thread")]
async fn run_bad_signature() {
    let ctx =
        TestContext::new("project").with_anvil("anvil-31337").await.expect("failed to spawn anvil");

    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("run_bad_signature")
        .setup(&["init"])
        .test(&[
            "run",
            "script/Deploy.s.sol",
            "--network",
            "anvil-31337",
            "--sig",
            "nonexistent()",
            "--broadcast",
            "--non-interactive",
        ])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(CompilerOutputNormalizer))
        .extra_normalizer(Box::new(GasNormalizer))
        .extra_normalizer(Box::new(BlockNumberNormalizer))
        .extra_normalizer(Box::new(DurationNormalizer));

    run_integration_test(&test, &ctx);
}
