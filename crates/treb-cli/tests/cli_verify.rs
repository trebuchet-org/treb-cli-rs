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
    treb()
        .arg("init")
        .current_dir(tmp.path())
        .assert()
        .success();
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
    registry
        .rebuild_lookup_index()
        .expect("lookup index rebuild should succeed");
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
    registry
        .rebuild_lookup_index()
        .expect("lookup index rebuild should succeed");
}

// ═══════════════════════════════════════════════════════════════════════════
// --help output
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn verify_help_shows_all_flags() {
    let output = treb()
        .args(["verify", "--help"])
        .output()
        .expect("failed to run treb verify --help");

    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains("--all"), "help should show --all");
    assert!(stdout.contains("--verifier"), "help should show --verifier");
    assert!(
        stdout.contains("--verifier-url"),
        "help should show --verifier-url"
    );
    assert!(
        stdout.contains("--verifier-api-key"),
        "help should show --verifier-api-key"
    );
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
        .stderr(
            predicate::str::contains("no deployment").or(predicate::str::contains("no TTY")),
        );
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
    assert!(
        !stderr.contains("unknown verifier"),
        "etherscan should be a valid verifier: {stderr}"
    );
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
    assert!(
        !stderr.contains("unknown verifier"),
        "sourcify should be a valid verifier: {stderr}"
    );
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
// --retries / --delay overrides
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn verify_custom_retries_and_delay_accepted() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_deployments(&tmp);

    let output = treb()
        .args([
            "verify",
            "NonexistentContract",
            "--retries",
            "10",
            "--delay",
            "3",
        ])
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
fn verify_all_with_all_verified_prints_noop() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_verified_deployments(&tmp);

    treb()
        .args(["verify", "--all"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("No unverified deployments found"));
}

#[test]
fn verify_all_force_proceeds_with_reverification() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_verified_deployments(&tmp);

    let output = treb()
        .args(["verify", "--all", "--force"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    // With --force, the batch should attempt verification (not print noop).
    assert!(
        !stderr.contains("No unverified deployments found"),
        "--all --force should not print noop message: {stderr}"
    );
    // Should show progress messages indicating it tried to verify.
    assert!(
        stderr.contains("Verifying"),
        "--all --force should attempt verification: {stderr}"
    );
}
