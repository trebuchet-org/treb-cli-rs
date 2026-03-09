//! Integration tests for `treb config` (show, set, remove).

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

// ── config show ──────────────────────────────────────────────────────────

#[test]
fn config_show_displays_namespace_and_network() {
    let tmp = tempfile::tempdir().unwrap();
    init_project(&tmp);

    treb()
        .args(["config", "show"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("📋 Current config:"))
        .stdout(predicate::str::contains("Namespace: default"))
        .stdout(predicate::str::contains("Network:   (not set)"))
        .stdout(predicate::str::contains("📦 Config source:"))
        .stdout(predicate::str::contains("📁 config file:"));
}

#[test]
fn config_show_json_is_valid() {
    let tmp = tempfile::tempdir().unwrap();
    init_project(&tmp);

    let output = treb()
        .args(["config", "show", "--json"])
        .current_dir(tmp.path())
        .output()
        .expect("failed to run treb config show --json");

    assert!(output.status.success());

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("output is not valid JSON");

    let obj = json.as_object().expect("JSON output is not an object");
    assert_eq!(obj["namespace"], "default");
    assert!(obj.contains_key("network"));
    assert!(obj.contains_key("profile"));
    assert!(obj.contains_key("configSource"));
    assert!(obj.contains_key("projectRoot"));
    assert!(obj.contains_key("senders"));
}

#[test]
fn config_show_without_init_fails() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("foundry.toml"), MINIMAL_FOUNDRY_TOML).unwrap();
    // Don't run init.

    treb()
        .args(["config", "show"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("treb init"));
}

// ── config set ───────────────────────────────────────────────────────────

#[test]
fn config_set_updates_local_config() {
    let tmp = tempfile::tempdir().unwrap();
    init_project(&tmp);

    // Set namespace.
    treb()
        .args(["config", "set", "namespace", "production"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Set namespace = production"));

    let config_json = fs::read_to_string(tmp.path().join(".treb/config.local.json")).unwrap();
    let config: serde_json::Value = serde_json::from_str(&config_json).unwrap();
    assert_eq!(config["namespace"], "production");

    // Set network.
    treb().args(["config", "set", "network", "mainnet"]).current_dir(tmp.path()).assert().success();

    let config_json = fs::read_to_string(tmp.path().join(".treb/config.local.json")).unwrap();
    let config: serde_json::Value = serde_json::from_str(&config_json).unwrap();
    assert_eq!(config["network"], "mainnet");

    // Verify 2-space JSON formatting with trailing newline.
    assert!(config_json.contains("  \"namespace\""));
    assert!(config_json.ends_with('\n'));
}

#[test]
fn config_set_invalid_key_fails() {
    let tmp = tempfile::tempdir().unwrap();
    init_project(&tmp);

    treb()
        .args(["config", "set", "invalid_key", "value"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("namespace"))
        .stderr(predicate::str::contains("network"));
}

// ── config remove ────────────────────────────────────────────────────────

#[test]
fn config_remove_resets_to_default() {
    let tmp = tempfile::tempdir().unwrap();
    init_project(&tmp);

    // Set custom values first.
    treb()
        .args(["config", "set", "namespace", "production"])
        .current_dir(tmp.path())
        .assert()
        .success();
    treb().args(["config", "set", "network", "mainnet"]).current_dir(tmp.path()).assert().success();

    // Remove namespace — should reset to "default".
    treb()
        .args(["config", "remove", "namespace"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed namespace"));

    let config_json = fs::read_to_string(tmp.path().join(".treb/config.local.json")).unwrap();
    let config: serde_json::Value = serde_json::from_str(&config_json).unwrap();
    assert_eq!(config["namespace"], "default");
    assert_eq!(config["network"], "mainnet"); // network unchanged

    // Remove network — should reset to "".
    treb().args(["config", "remove", "network"]).current_dir(tmp.path()).assert().success();

    let config_json = fs::read_to_string(tmp.path().join(".treb/config.local.json")).unwrap();
    let config: serde_json::Value = serde_json::from_str(&config_json).unwrap();
    assert_eq!(config["network"], "");
}

#[test]
fn config_remove_invalid_key_fails() {
    let tmp = tempfile::tempdir().unwrap();
    init_project(&tmp);

    treb()
        .args(["config", "remove", "invalid_key"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("namespace"))
        .stderr(predicate::str::contains("network"));
}

#[test]
fn config_set_without_init_fails() {
    let tmp = tempfile::tempdir().unwrap();
    // No foundry.toml, no init.

    treb()
        .args(["config", "set", "namespace", "test"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("treb init"));
}
