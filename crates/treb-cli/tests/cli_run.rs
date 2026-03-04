//! Integration tests for `treb run`.
//!
//! These tests verify argument parsing, error handling, and flag validation.
//! Full pipeline execution tests require Solidity compilation and are not
//! covered here (they would be in forge-level integration tests).

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

// ── Argument parsing tests ──────────────────────────────────────────────

#[test]
fn run_without_script_argument_fails() {
    treb().arg("run").assert().failure().stderr(predicate::str::contains("<SCRIPT>"));
}

#[test]
fn run_help_shows_all_flags() {
    let output = treb().args(["run", "--help"]).output().expect("failed to run treb run --help");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Verify key flags appear in help
    assert!(stdout.contains("--sig"), "help should show --sig");
    assert!(stdout.contains("--args"), "help should show --args");
    assert!(stdout.contains("--network"), "help should show --network");
    assert!(stdout.contains("--rpc-url"), "help should show --rpc-url");
    assert!(stdout.contains("--namespace"), "help should show --namespace");
    assert!(stdout.contains("--broadcast"), "help should show --broadcast");
    assert!(stdout.contains("--dry-run"), "help should show --dry-run");
    assert!(stdout.contains("--slow"), "help should show --slow");
    assert!(stdout.contains("--legacy"), "help should show --legacy");
    assert!(stdout.contains("--verify"), "help should show --verify");
    assert!(stdout.contains("--verbose"), "help should show --verbose");
    assert!(stdout.contains("--debug"), "help should show --debug");
    assert!(stdout.contains("--json"), "help should show --json");
    assert!(stdout.contains("--env"), "help should show --env");
    assert!(stdout.contains("--target-contract"), "help should show --target-contract");
    assert!(stdout.contains("--non-interactive"), "help should show --non-interactive");
}

#[test]
fn run_sig_defaults_to_run() {
    // Run --help shows default value for --sig
    treb().args(["run", "--help"]).assert().success().stdout(predicate::str::contains("run()"));
}

// ── Error path tests ────────────────────────────────────────────────────

#[test]
fn run_without_foundry_toml_fails() {
    let tmp = tempfile::tempdir().unwrap();

    treb()
        .args(["run", "script/Deploy.s.sol"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("foundry.toml"));
}

#[test]
fn run_without_treb_init_fails() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("foundry.toml"), MINIMAL_FOUNDRY_TOML).unwrap();
    // Don't run init.

    treb()
        .args(["run", "script/Deploy.s.sol"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("treb init"));
}

#[test]
fn run_bad_env_var_format_fails() {
    let tmp = tempfile::tempdir().unwrap();
    init_project(&tmp);

    treb()
        .args(["run", "script/Deploy.s.sol", "--env", "INVALID_NO_EQUALS"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("missing '='"));
}

#[test]
fn run_env_var_empty_key_fails() {
    let tmp = tempfile::tempdir().unwrap();
    init_project(&tmp);

    treb()
        .args(["run", "script/Deploy.s.sol", "--env", "=value"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("key cannot be empty"));
}

// ── Flag combination tests ──────────────────────────────────────────────

#[test]
fn run_dry_run_and_json_flags_accepted() {
    // Both flags should be accepted together (parsing succeeds; will fail
    // at pipeline execution since there's no actual script, but that's OK).
    let tmp = tempfile::tempdir().unwrap();
    init_project(&tmp);

    // This will fail because the script doesn't exist / forge compilation
    // will fail, but the important thing is that the arg parsing succeeds
    // and the error is about pipeline execution, not arg parsing.
    let output = treb()
        .args(["run", "script/Deploy.s.sol", "--dry-run", "--json"])
        .current_dir(tmp.path())
        .output()
        .expect("failed to run command");

    // The command will fail (no script file), but stderr should NOT contain
    // clap arg-parsing errors.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("error: unexpected argument"),
        "should not have arg parsing error: {stderr}"
    );
}

#[test]
fn run_broadcast_non_interactive_flags_accepted() {
    let tmp = tempfile::tempdir().unwrap();
    init_project(&tmp);

    let output = treb()
        .args(["run", "script/Deploy.s.sol", "--broadcast", "--non-interactive"])
        .current_dir(tmp.path())
        .output()
        .expect("failed to run command");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("error: unexpected argument"),
        "should not have arg parsing error: {stderr}"
    );
}

#[test]
fn run_multiple_env_vars_accepted() {
    let tmp = tempfile::tempdir().unwrap();
    init_project(&tmp);

    let output = treb()
        .args(["run", "script/Deploy.s.sol", "--env", "FOO=bar", "--env", "BAZ=qux"])
        .current_dir(tmp.path())
        .output()
        .expect("failed to run command");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("error: unexpected argument"),
        "should not have arg parsing error: {stderr}"
    );
}

#[test]
fn run_all_flags_accepted_together() {
    let tmp = tempfile::tempdir().unwrap();
    init_project(&tmp);

    let output = treb()
        .args([
            "run",
            "script/Deploy.s.sol",
            "--sig",
            "deploy(uint256)",
            "--args",
            "42",
            "--network",
            "sepolia",
            "--namespace",
            "staging",
            "--dry-run",
            "--slow",
            "--legacy",
            "--verify",
            "--verbose",
            "--debug",
            "--json",
            "--env",
            "KEY=value",
            "--target-contract",
            "MyContract",
            "--non-interactive",
        ])
        .current_dir(tmp.path())
        .output()
        .expect("failed to run command");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("error: unexpected argument"),
        "should not have arg parsing error: {stderr}"
    );
}

#[test]
fn run_env_var_with_equals_in_value_accepted() {
    let tmp = tempfile::tempdir().unwrap();
    init_project(&tmp);

    let output = treb()
        .args(["run", "script/Deploy.s.sol", "--env", "KEY=value=with=equals"])
        .current_dir(tmp.path())
        .output()
        .expect("failed to run command");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("missing '='"), "should accept equals in value: {stderr}");
}

#[test]
fn run_env_var_empty_value_accepted() {
    let tmp = tempfile::tempdir().unwrap();
    init_project(&tmp);

    let output = treb()
        .args(["run", "script/Deploy.s.sol", "--env", "KEY="])
        .current_dir(tmp.path())
        .output()
        .expect("failed to run command");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("key cannot be empty"), "should accept empty value: {stderr}");
}
