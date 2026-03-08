//! Integration tests for exit-code behavior and non-interactive mode detection.
//!
//! Covers:
//! - Exit code 0 on successful JSON commands
//! - Exit code 1 on failed JSON commands
//! - `TREB_NON_INTERACTIVE=true` env var triggers non-interactive mode
//! - `CI=true` env var triggers non-interactive mode

mod framework;

use framework::{
    context::TestContext,
    integration_test::{IntegrationTest, run_integration_test},
    normalizer::{PathNormalizer, ShortHexNormalizer},
};

// ── Exit code tests ─────────────────────────────────────────────────────

/// Successful JSON command exits with code 0.
///
/// Uses `version --json` (no init required) and verifies exit code 0
/// with valid JSON on stdout via golden file comparison.
#[test]
fn exit_code_zero_on_json_success() {
    let ctx = TestContext::new("project");

    let test = IntegrationTest::new("exit_code_zero_on_json_success")
        .test(&["version", "--json"])
        .extra_normalizer(Box::new(ShortHexNormalizer));

    run_integration_test(&test, &ctx);
}

/// Failed JSON command exits with code 1 and emits JSON error to stderr.
///
/// Uses `config show --json` on an uninitialized project and verifies
/// exit code is non-zero with structured `{"error": "..."}` on stderr.
#[test]
fn exit_code_one_on_json_error() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("exit_code_one_on_json_error")
        .pre_setup_hook(|ctx| {
            std::fs::remove_dir_all(ctx.treb_dir()).ok();
        })
        .test(&["config", "show", "--json"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── Non-interactive env var tests ───────────────────────────────────────

/// `TREB_NON_INTERACTIVE=true` causes commands to skip interactive prompts.
///
/// Runs `config show --json` with the env var set on an initialized project.
/// The command completes without hanging — proving interactive prompts are
/// suppressed by the environment variable.
#[test]
fn treb_non_interactive_env_skips_prompts() {
    let ctx = TestContext::new("project");

    // Init the project first.
    ctx.run(&["init"]).success();

    // Run with TREB_NON_INTERACTIVE=true — should complete without hanging.
    let assertion =
        ctx.run_with_env(&["config", "show", "--json"], [("TREB_NON_INTERACTIVE", "true")]);

    // Capture output before consuming assertion with .success().
    let stdout = String::from_utf8_lossy(&assertion.get_output().stdout).to_string();
    assertion.success();

    // Verify stdout is valid JSON with expected fields.
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout should be valid JSON");
    assert!(parsed.get("configSource").is_some(), "JSON should contain configSource field");
}

/// `CI=true` causes commands to skip interactive prompts.
///
/// Same verification as the TREB_NON_INTERACTIVE test but using the CI
/// environment variable that many CI systems set.
#[test]
fn ci_env_skips_prompts() {
    let ctx = TestContext::new("project");

    // Init the project first.
    ctx.run(&["init"]).success();

    // Run with CI=true — should complete without hanging.
    let assertion = ctx.run_with_env(&["config", "show", "--json"], [("CI", "true")]);

    // Capture output before consuming assertion with .success().
    let stdout = String::from_utf8_lossy(&assertion.get_output().stdout).to_string();
    assertion.success();

    // Verify stdout is valid JSON with expected fields.
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout should be valid JSON");
    assert!(parsed.get("configSource").is_some(), "JSON should contain configSource field");
}
