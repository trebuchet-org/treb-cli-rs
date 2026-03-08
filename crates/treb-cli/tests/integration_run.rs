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
        BlockNumberNormalizer, CompilerOutputNormalizer, DebugLogNormalizer, DurationNormalizer,
        GasNormalizer, PathNormalizer,
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
        .extra_normalizer(Box::new(DurationNormalizer))
        .extra_normalizer(Box::new(DebugLogNormalizer));

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

/// Verbose + JSON mode should emit JSON only (no verbose human output).
///
/// Verifies `--verbose --json` does not print key/value verbose lines that
/// would break machine-readable JSON output.
#[test]
fn run_verbose_json() {
    let ctx = TestContext::new("project");

    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("run_verbose_json")
        .setup(&["init"])
        .test(&[
            "run",
            "script/Deploy.s.sol",
            "--dry-run",
            "--non-interactive",
            "--verbose",
            "--json",
        ])
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(CompilerOutputNormalizer))
        .extra_normalizer(Box::new(GasNormalizer))
        .extra_normalizer(Box::new(DurationNormalizer));

    run_integration_test(&test, &ctx);
}

/// Dump command — prints equivalent forge script CLI command and exits.
///
/// Verifies that `--dump-command` outputs the forge command and exits without executing.
#[tokio::test(flavor = "multi_thread")]
async fn run_dump_command() {
    let ctx =
        TestContext::new("project").with_anvil("anvil-31337").await.expect("failed to spawn anvil");

    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("run_dump_command")
        .setup(&["init"])
        .test(&[
            "run",
            "script/Deploy.s.sol",
            "--network",
            "anvil-31337",
            "--broadcast",
            "--non-interactive",
            "--dump-command",
        ])
        .extra_normalizer(Box::new(path_normalizer));

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

// ── Governor sender tests ───────────────────────────────────────────────

/// Governor sender config for tests.
///
/// Defines an "anvil" private-key account and a "governance" oz_governor account
/// whose proposer is "anvil".  The namespace maps "deployer" → "governance" so
/// that the CLI detects `is_governor_sender = true`.
const GOVERNOR_TREB_TOML: &str = r#"[accounts.anvil]
type = "private_key"
private_key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"

[accounts.governance]
type = "oz_governor"
governor = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
timelock = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
proposer = "anvil"

[namespace.default]
profile = "default"

[namespace.default.senders]
anvil = "anvil"
deployer = "governance"
"#;

const GOVERNOR_SCRIPT: &str = "script/GovernorProposal.s.sol";

/// Broadcast with governor sender — human output.
///
/// Verifies populated governor proposal output and registry persistence.
#[tokio::test(flavor = "multi_thread")]
async fn run_governor_human() {
    let ctx =
        TestContext::new("project").with_anvil("anvil-31337").await.expect("failed to spawn anvil");

    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("run_governor_human")
        .pre_setup_hook(|ctx| {
            std::fs::write(ctx.path().join("treb.toml"), GOVERNOR_TREB_TOML).unwrap();
        })
        .setup(&["init"])
        .test(&[
            "run",
            GOVERNOR_SCRIPT,
            "--network",
            "anvil-31337",
            "--broadcast",
            "--non-interactive",
        ])
        .output_artifact(".treb/governor-txs.json")
        .output_artifact(".treb/transactions.json")
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(CompilerOutputNormalizer))
        .extra_normalizer(Box::new(GasNormalizer))
        .extra_normalizer(Box::new(BlockNumberNormalizer))
        .extra_normalizer(Box::new(DurationNormalizer));

    run_integration_test(&test, &ctx);
}

/// Broadcast with governor sender — JSON output.
///
/// Verifies JSON structure is correct with populated governor proposal data.
#[tokio::test(flavor = "multi_thread")]
async fn run_governor_json() {
    let ctx =
        TestContext::new("project").with_anvil("anvil-31337").await.expect("failed to spawn anvil");

    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("run_governor_json")
        .pre_setup_hook(|ctx| {
            std::fs::write(ctx.path().join("treb.toml"), GOVERNOR_TREB_TOML).unwrap();
        })
        .setup(&["init"])
        .test(&[
            "run",
            GOVERNOR_SCRIPT,
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

/// Dry-run with governor sender — verifies governor config doesn't break
/// the dry-run path and uses proposal wording.
#[test]
fn run_governor_dry_run() {
    let ctx = TestContext::new("project");

    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("run_governor_dry_run")
        .pre_setup_hook(|ctx| {
            std::fs::write(ctx.path().join("treb.toml"), GOVERNOR_TREB_TOML).unwrap();
        })
        .setup(&["init"])
        .test(&["run", GOVERNOR_SCRIPT, "--dry-run", "--non-interactive"])
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(CompilerOutputNormalizer))
        .extra_normalizer(Box::new(GasNormalizer))
        .extra_normalizer(Box::new(DurationNormalizer));

    run_integration_test(&test, &ctx);
}

/// Verbose governor broadcast shows governor sender context and proposal counts.
#[tokio::test(flavor = "multi_thread")]
async fn run_governor_verbose() {
    let ctx =
        TestContext::new("project").with_anvil("anvil-31337").await.expect("failed to spawn anvil");

    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("run_governor_verbose")
        .pre_setup_hook(|ctx| {
            std::fs::write(ctx.path().join("treb.toml"), GOVERNOR_TREB_TOML).unwrap();
        })
        .setup(&["init"])
        .test(&[
            "run",
            GOVERNOR_SCRIPT,
            "--network",
            "anvil-31337",
            "--broadcast",
            "--non-interactive",
            "--verbose",
        ])
        .output_artifact(".treb/governor-txs.json")
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(CompilerOutputNormalizer))
        .extra_normalizer(Box::new(GasNormalizer))
        .extra_normalizer(Box::new(BlockNumberNormalizer))
        .extra_normalizer(Box::new(DurationNormalizer));

    run_integration_test(&test, &ctx);
}

// ── Error tests ─────────────────────────────────────────────────────────

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
