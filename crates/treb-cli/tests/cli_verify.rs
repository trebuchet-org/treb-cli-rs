//! Integration tests for `treb verify`.
//!
//! These tests verify argument parsing, error handling, and verification-status
//! paths.  Full end-to-end verification requires network access / a running
//! block-explorer API and is not covered here.

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use std::fs;

fn treb() -> assert_cmd::Command {
    cargo_bin_cmd!("treb-cli")
}

const MINIMAL_FOUNDRY_TOML: &str = "[profile.default]\n";

/// Helper: create a temp dir with foundry.toml and run `treb init`.
fn init_project(tmp: &tempfile::TempDir) {
    fs::write(tmp.path().join("foundry.toml"), MINIMAL_FOUNDRY_TOML).unwrap();
    treb().arg("init").current_dir(tmp.path()).assert().success();
}

/// Helper: create a temp dir with foundry.toml, run `treb init`, then insert
/// fixture deployments and rebuild the lookup index.
fn init_project_with_deployments(tmp: &tempfile::TempDir) {
    init_project(tmp);

    let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../treb-core/tests/fixtures/deployments_map.json");
    let fixture_json = fs::read_to_string(&fixture_path).expect("fixture file should exist");

    fs::write(tmp.path().join(".treb/deployments.json"), &fixture_json).unwrap();

    let registry = treb_registry::Registry::open(tmp.path()).expect("registry should open");
    registry.rebuild_lookup_index().expect("lookup index rebuild should succeed");
}

/// Helper: create a project where ALL deployments have verification.status = VERIFIED.
fn init_project_with_verified_deployments(tmp: &tempfile::TempDir) {
    init_project_with_deployments(tmp);

    let path = tmp.path().join(".treb/deployments.json");
    let json_str = fs::read_to_string(&path).unwrap();
    let mut map: serde_json::Value = serde_json::from_str(&json_str).unwrap();

    for (_key, dep) in map.as_object_mut().unwrap() {
        dep["verification"]["status"] = serde_json::json!("VERIFIED");
        dep["verification"]["etherscanUrl"] =
            serde_json::json!("https://etherscan.io/address/0x123#code");
        dep["verification"]["verifiedAt"] = serde_json::json!("2026-01-01T00:00:00Z");
    }

    fs::write(&path, serde_json::to_string_pretty(&map).unwrap()).unwrap();

    let registry = treb_registry::Registry::open(tmp.path()).expect("registry should open");
    registry.rebuild_lookup_index().expect("lookup index rebuild should succeed");
}

/// Helper: create a project with one unverified deployment whose address is invalid,
/// so verification fails before any network call.
fn init_project_with_invalid_address_deployment(tmp: &tempfile::TempDir) {
    init_project_with_deployments(tmp);

    let path = tmp.path().join(".treb/deployments.json");
    let json_str = fs::read_to_string(&path).unwrap();
    let mut map: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    let deployment =
        map.get_mut("mainnet/42220/FPMM:v3.0.0").expect("fixture deployment should exist");

    deployment["address"] = serde_json::json!("not-an-address");
    deployment["verification"]["status"] = serde_json::json!("UNVERIFIED");
    deployment["verification"]["etherscanUrl"] = serde_json::json!("");
    deployment["verification"]["verifiedAt"] = serde_json::Value::Null;
    deployment["verification"]["reason"] = serde_json::json!("");
    deployment["verification"]["verifiers"] = serde_json::json!({});

    fs::write(&path, serde_json::to_string_pretty(&map).unwrap()).unwrap();

    let registry = treb_registry::Registry::open(tmp.path()).expect("registry should open");
    registry.rebuild_lookup_index().expect("lookup index rebuild should succeed");
}

/// Helper: create a project where only a single labeled deployment remains
/// unverified and fails locally before any network call.
fn init_project_with_single_labeled_batch_deployment(tmp: &tempfile::TempDir) {
    init_project_with_verified_deployments(tmp);

    let path = tmp.path().join(".treb/deployments.json");
    let json_str = fs::read_to_string(&path).unwrap();
    let mut map: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    let deployment =
        map.get_mut("mainnet/42220/FPMM:v3.0.0").expect("fixture deployment should exist");

    deployment["address"] = serde_json::json!("not-an-address");
    deployment["verification"]["status"] = serde_json::json!("UNVERIFIED");
    deployment["verification"]["etherscanUrl"] = serde_json::json!("");
    deployment["verification"]["verifiedAt"] = serde_json::Value::Null;
    deployment["verification"]["reason"] = serde_json::json!("");
    deployment["verification"]["verifiers"] = serde_json::json!({});

    fs::write(&path, serde_json::to_string_pretty(&map).unwrap()).unwrap();

    let registry = treb_registry::Registry::open(tmp.path()).expect("registry should open");
    registry.rebuild_lookup_index().expect("lookup index rebuild should succeed");
}

// ═══════════════════════════════════════════════════════════════════════════
// --help output
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn verify_help_shows_all_flags() {
    let output =
        treb().args(["verify", "--help"]).output().expect("failed to run treb verify --help");

    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains("--all"), "help should show --all");
    assert!(stdout.contains("--namespace"), "help should show --namespace");
    assert!(stdout.contains("-n, --network"), "help should show -n, --network");
    assert!(stdout.contains("--verifier"), "help should show --verifier");
    assert!(stdout.contains("-e, --etherscan"), "help should show -e, --etherscan");
    assert!(stdout.contains("-b, --blockscout"), "help should show -b, --blockscout");
    assert!(stdout.contains("-s, --sourcify"), "help should show -s, --sourcify");
    assert!(stdout.contains("--verifier-url"), "help should show --verifier-url");
    assert!(
        stdout.contains("blockscout-verifier-url"),
        "help should show the blockscout verifier URL alias"
    );
    assert!(stdout.contains("--contract-path"), "help should show --contract-path");
    assert!(stdout.contains("--debug"), "help should show --debug");
    assert!(stdout.contains("--verifier-api-key"), "help should show --verifier-api-key");
    assert!(stdout.contains("--force"), "help should show --force");
    assert!(stdout.contains("--watch"), "help should show --watch");
    assert!(stdout.contains("--retries"), "help should show --retries");
    assert!(stdout.contains("--delay"), "help should show --delay");
    assert!(stdout.contains("--json"), "help should show --json");
    assert!(
        stdout.contains("<DEPLOYMENT>") || stdout.contains("[DEPLOYMENT]"),
        "help should show <DEPLOYMENT> positional arg"
    );
}

#[test]
fn verify_help_shows_retries_delay_defaults() {
    treb()
        .args(["verify", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[default: 5]"));
}

// ═══════════════════════════════════════════════════════════════════════════
// Missing deployment / --all validation
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn verify_without_deployment_or_all_fails() {
    let tmp = tempfile::tempdir().unwrap();
    init_project(&tmp);

    // Without a deployment argument or --all in non-TTY mode, the fuzzy
    // selector is invoked. On an empty registry it returns "no deployment
    // selected"; on a non-empty registry in non-TTY it returns a TTY error.
    // Either way the command should exit non-zero with a clear message.
    treb()
        .arg("verify")
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("no deployment").or(predicate::str::contains("no TTY")));
}

// ═══════════════════════════════════════════════════════════════════════════
// --verifier values
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn verify_rejects_invalid_verifier() {
    let tmp = tempfile::tempdir().unwrap();
    init_project(&tmp);

    treb()
        .args(["verify", "SomeContract", "--verifier", "invalid"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown verifier"));
}

#[test]
fn verify_accepts_etherscan_verifier() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_deployments(&tmp);

    // Will fail at deployment resolution (NonexistentContract), but should
    // NOT fail at verifier validation.
    let output = treb()
        .args(["verify", "NonexistentContract", "--verifier", "etherscan"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("unknown verifier"), "etherscan should be a valid verifier: {stderr}");
}

#[test]
fn verify_accepts_sourcify_verifier() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_deployments(&tmp);

    let output = treb()
        .args(["verify", "NonexistentContract", "--verifier", "sourcify"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("unknown verifier"), "sourcify should be a valid verifier: {stderr}");
}

#[test]
fn verify_accepts_blockscout_verifier() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_deployments(&tmp);

    let output = treb()
        .args(["verify", "NonexistentContract", "--verifier", "blockscout"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unknown verifier"),
        "blockscout should be a valid verifier: {stderr}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// --etherscan / --blockscout / --sourcify shorthand flags
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn verify_etherscan_shorthand_flag_accepted() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_deployments(&tmp);

    let output = treb()
        .args(["verify", "NonexistentContract", "--etherscan"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("unexpected argument"), "--etherscan should be accepted: {stderr}");
}

#[test]
fn verify_blockscout_shorthand_flag_accepted() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_deployments(&tmp);

    let output = treb()
        .args(["verify", "NonexistentContract", "--blockscout"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("unexpected argument"), "--blockscout should be accepted: {stderr}");
}

#[test]
fn verify_sourcify_shorthand_flag_accepted() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_deployments(&tmp);

    let output = treb()
        .args(["verify", "NonexistentContract", "--sourcify"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("unexpected argument"), "--sourcify should be accepted: {stderr}");
}

#[test]
fn verify_multiple_shorthand_flags_combined() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_deployments(&tmp);

    let output = treb()
        .args(["verify", "NonexistentContract", "--etherscan", "--sourcify"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unexpected argument"),
        "--etherscan --sourcify combined should be accepted: {stderr}"
    );
}

#[test]
fn verify_all_three_shorthand_flags() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_deployments(&tmp);

    let output = treb()
        .args(["verify", "NonexistentContract", "--etherscan", "--blockscout", "--sourcify"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unexpected argument"),
        "all three shorthand flags should be accepted: {stderr}"
    );
}

#[test]
fn verify_shorthand_overrides_verifier_flag() {
    // When both --verifier and a shorthand flag are provided, the shorthand
    // should take precedence. The command should not reject the combination.
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_deployments(&tmp);

    let output = treb()
        .args(["verify", "NonexistentContract", "--verifier", "sourcify", "--etherscan"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unexpected argument"),
        "--etherscan should override --verifier without conflict: {stderr}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// --retries / --delay overrides
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn verify_custom_retries_and_delay_accepted() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_deployments(&tmp);

    let output = treb()
        .args(["verify", "NonexistentContract", "--retries", "10", "--delay", "3"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("error: unexpected argument"),
        "custom retries/delay should be accepted: {stderr}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Error paths: uninitialized project
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn verify_without_foundry_toml_fails() {
    let tmp = tempfile::tempdir().unwrap();

    treb()
        .args(["verify", "MyContract"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("foundry.toml"));
}

#[test]
fn verify_without_treb_dir_fails() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("foundry.toml"), MINIMAL_FOUNDRY_TOML).unwrap();

    treb()
        .args(["verify", "MyContract"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("treb init"));
}

// ═══════════════════════════════════════════════════════════════════════════
// Error paths: nonexistent deployment
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn verify_nonexistent_deployment_fails() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_deployments(&tmp);

    treb()
        .args(["verify", "NonexistentContract"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("no deployment found"));
}

// ═══════════════════════════════════════════════════════════════════════════
// Already-verified / --all / --force paths
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn verify_already_verified_skips_with_message() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_verified_deployments(&tmp);

    treb()
        .args(["verify", "FPMM"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("already verified"));
}

#[test]
fn verify_single_failure_uses_go_style_output_without_generic_error_line() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_invalid_address_deployment(&tmp);

    let output = treb()
        .args(["--no-color", "verify", "FPMM"])
        .env("NO_COLOR", "1")
        .current_dir(tmp.path())
        .output()
        .unwrap();

    assert!(!output.status.success(), "verification failure should exit non-zero");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("✗ Verification failed: invalid address 'not-an-address'"),
        "single verify failure should print the handled summary: {stderr}"
    );
    assert!(
        stderr.contains("Verification Status:"),
        "single verify failure should include the status section: {stderr}"
    );
    assert!(
        stderr.contains("✗ Etherscan Failed"),
        "single verify failure should print title-cased verifier status: {stderr}"
    );
    assert!(
        !stderr.contains("  etherscan: FAILED"),
        "legacy lowercase verifier line should be suppressed: {stderr}"
    );
    assert!(
        !stderr.contains("Error:"),
        "generic main error line should be suppressed for handled verification failures: {stderr}"
    );
}

#[test]
fn verify_all_with_all_verified_prints_noop() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_verified_deployments(&tmp);

    treb()
        .args(["verify", "--all"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("No unverified deployed contracts found"));
}

#[test]
fn verify_all_force_proceeds_with_reverification() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_verified_deployments(&tmp);

    let output =
        treb().args(["verify", "--all", "--force"]).current_dir(tmp.path()).output().unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    // With --force, the batch should attempt verification (not print noop).
    assert!(
        !stderr.contains("No unverified deployed contracts found"),
        "--all --force should not print noop message: {stderr}"
    );
    // Should show to-verify header indicating it found contracts to verify.
    assert!(
        stderr.contains("Found") && stderr.contains("to verify"),
        "--all --force should show to-verify header: {stderr}"
    );
}

#[test]
fn verify_all_uses_labeled_display_name_without_synthetic_skip_section() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_single_labeled_batch_deployment(&tmp);

    let output = treb()
        .args(["--no-color", "verify", "--all"])
        .env("NO_COLOR", "1")
        .current_dir(tmp.path())
        .output()
        .unwrap();

    assert!(output.status.success(), "batch verify should complete with handled failures");

    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !stderr.contains("Skipping "),
        "batch verify should not synthesize a skipped section for already verified deployments: {stderr}"
    );
    assert!(
        !stderr.contains("(already verified)"),
        "batch verify should not render already verified deployments as skipped batch results: {stderr}"
    );
    // Per-result line should contain the labeled display name in chain:CHAINID/NS/NAME format.
    assert!(
        stderr.contains("chain:42220/mainnet/FPMM:v3.0.0"),
        "batch per-result should show the labeled display name: {stderr}"
    );
    assert!(
        !stderr.contains("chain:42220/mainnet/FPMM\n"),
        "batch per-result should not fall back to the raw contract name: {stderr}"
    );
    // Summary line should appear.
    assert!(stderr.contains("Verification complete:"), "batch should show summary line: {stderr}");
}

#[test]
fn verify_all_batch_output_uses_aggregate_failure_details() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_single_labeled_batch_deployment(&tmp);

    let output = treb()
        .args(["--no-color", "verify", "--all", "--etherscan", "--sourcify"])
        .env("NO_COLOR", "1")
        .current_dir(tmp.path())
        .output()
        .unwrap();

    assert!(output.status.success(), "batch verify should complete with handled failures");

    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stderr.contains("✗ invalid address 'not-an-address'"),
        "batch verify should print aggregate failure details instead of verifier status lines: {stderr}"
    );
    assert!(
        !stderr.contains("✗ Etherscan Failed"),
        "batch verify should not reuse single-verify verifier breakdown lines: {stderr}"
    );
    assert!(
        !stderr.contains("✗ Sourcify Failed"),
        "batch verify should not reuse single-verify verifier breakdown lines: {stderr}"
    );
    assert!(
        !stderr.contains("Verification Status:"),
        "batch verify should not print the single-verify status section: {stderr}"
    );
}
