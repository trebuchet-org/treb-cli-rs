//! Integration tests for `treb init`.

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use std::fs;

fn treb() -> assert_cmd::Command {
    cargo_bin_cmd!("treb-cli")
}

const MINIMAL_FOUNDRY_TOML: &str = "[profile.default]\n";

#[test]
fn init_creates_treb_directory_with_correct_files() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("foundry.toml"), MINIMAL_FOUNDRY_TOML).unwrap();

    treb()
        .arg("init")
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains(".treb"))
        .stdout(predicate::str::contains("treb config show"));

    // Verify registry.json exists with version 1.
    let registry_json = fs::read_to_string(tmp.path().join(".treb/registry.json")).unwrap();
    let registry: serde_json::Value = serde_json::from_str(&registry_json).unwrap();
    assert_eq!(registry["version"], 1);

    // Verify config.local.json has defaults.
    let config_json = fs::read_to_string(tmp.path().join(".treb/config.local.json")).unwrap();
    let config: serde_json::Value = serde_json::from_str(&config_json).unwrap();
    assert_eq!(config["namespace"], "default");
    assert_eq!(config["network"], "");
}

#[test]
fn init_without_foundry_toml_fails() {
    let tmp = tempfile::tempdir().unwrap();

    treb()
        .arg("init")
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("foundry.toml"));
}

#[test]
fn init_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("foundry.toml"), MINIMAL_FOUNDRY_TOML).unwrap();

    // First init.
    treb()
        .arg("init")
        .current_dir(tmp.path())
        .assert()
        .success();

    // Modify config to detect overwrites.
    let config = r#"{"namespace":"production","network":"mainnet"}"#;
    fs::write(tmp.path().join(".treb/config.local.json"), config).unwrap();

    // Second init without --force should not modify files.
    treb()
        .arg("init")
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("already initialized"));

    // Config should be unchanged.
    let config_json = fs::read_to_string(tmp.path().join(".treb/config.local.json")).unwrap();
    let config_val: serde_json::Value = serde_json::from_str(&config_json).unwrap();
    assert_eq!(config_val["namespace"], "production");
}

#[test]
fn init_force_resets_local_config() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("foundry.toml"), MINIMAL_FOUNDRY_TOML).unwrap();

    // First init.
    treb()
        .arg("init")
        .current_dir(tmp.path())
        .assert()
        .success();

    // Modify config.
    let config = r#"{"namespace":"production","network":"mainnet"}"#;
    fs::write(tmp.path().join(".treb/config.local.json"), config).unwrap();

    // Init with --force should reset config.
    treb()
        .args(["init", "--force"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Config should be reset to defaults.
    let config_json = fs::read_to_string(tmp.path().join(".treb/config.local.json")).unwrap();
    let config_val: serde_json::Value = serde_json::from_str(&config_json).unwrap();
    assert_eq!(config_val["namespace"], "default");
    assert_eq!(config_val["network"], "");

    // Registry data should still exist.
    assert!(tmp.path().join(".treb/registry.json").exists());
}
