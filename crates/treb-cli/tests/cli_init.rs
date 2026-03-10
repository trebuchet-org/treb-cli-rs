//! Integration tests for `treb init`.

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

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
        .stdout(predicate::str::contains("Initialized registry in .treb/"))
        .stdout(predicate::str::contains("treb initialized successfully!"))
        .stdout(predicate::str::contains("Next steps:"))
        .stdout(predicate::str::contains("treb config show"));

    assert!(
        !tmp.path().join(".treb/registry.json").exists(),
        "init should not create registry.json metadata"
    );

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
    treb().arg("init").current_dir(tmp.path()).assert().success();

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
    treb().arg("init").current_dir(tmp.path()).assert().success();

    // Modify config.
    let config = r#"{"namespace":"production","network":"mainnet"}"#;
    fs::write(tmp.path().join(".treb/config.local.json"), config).unwrap();

    // Init with --force should reset config.
    treb().args(["init", "--force"]).current_dir(tmp.path()).assert().success();

    // Config should be reset to defaults.
    let config_json = fs::read_to_string(tmp.path().join(".treb/config.local.json")).unwrap();
    let config_val: serde_json::Value = serde_json::from_str(&config_json).unwrap();
    assert_eq!(config_val["namespace"], "default");
    assert_eq!(config_val["network"], "");

    assert!(tmp.path().join(".treb").exists());
    assert!(
        !tmp.path().join(".treb/registry.json").exists(),
        "init --force should not create registry.json metadata"
    );
}

#[cfg(unix)]
#[test]
fn init_prints_step_failure_when_registry_init_fails() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("foundry.toml"), MINIMAL_FOUNDRY_TOML).unwrap();

    let original_mode = fs::metadata(tmp.path()).unwrap().permissions().mode();
    let mut readonly = fs::metadata(tmp.path()).unwrap().permissions();
    readonly.set_mode(0o500);
    fs::set_permissions(tmp.path(), readonly).unwrap();

    let output = treb().env("NO_COLOR", "1").arg("init").current_dir(tmp.path()).output().unwrap();

    let mut restored = fs::metadata(tmp.path()).unwrap().permissions();
    restored.set_mode(original_mode);
    fs::set_permissions(tmp.path(), restored).unwrap();

    assert!(!output.status.success(), "init should fail when project root is not writable");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stdout.contains("❌ Failed to initialize registry in .treb/"),
        "stdout should include the init step failure line: {stdout}"
    );
    assert!(
        stdout.contains("Permission denied"),
        "stdout should include the underlying OS error: {stdout}"
    );
    assert!(
        stderr.contains("Error: failed to initialize registry"),
        "stderr should still include the bubbled error: {stderr}"
    );
}

#[cfg(unix)]
#[test]
fn init_force_prints_step_failure_when_config_write_fails() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("foundry.toml"), MINIMAL_FOUNDRY_TOML).unwrap();
    treb().arg("init").current_dir(tmp.path()).assert().success();

    let config_path = tmp.path().join(".treb/config.local.json");
    let original_mode = fs::metadata(&config_path).unwrap().permissions().mode();
    let mut readonly = fs::metadata(&config_path).unwrap().permissions();
    readonly.set_mode(0o400);
    fs::set_permissions(&config_path, readonly).unwrap();

    let output = treb()
        .env("NO_COLOR", "1")
        .args(["init", "--force"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    let mut restored = fs::metadata(&config_path).unwrap().permissions();
    restored.set_mode(original_mode);
    fs::set_permissions(&config_path, restored).unwrap();

    assert!(
        !output.status.success(),
        "init --force should fail when config.local.json is not writable"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stdout.contains("❌ Failed to reset local config"),
        "stdout should include the reset failure line: {stdout}"
    );
    assert!(
        stdout.contains("Permission denied"),
        "stdout should include the underlying OS error: {stdout}"
    );
    assert!(
        stderr.contains("Error: failed to write config.local.json"),
        "stderr should still include the bubbled error: {stderr}"
    );
}
