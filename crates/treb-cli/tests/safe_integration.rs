//! Integration tests for Safe and Governor sender flows.
//!
//! Tests Safe and Governor sender detection from config, dry-run proposal
//! output, and sync fixture deserialization.

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

// ── Safe sender detection from config ────────────────────────────────────

#[test]
fn config_show_json_includes_safe_sender() {
    let tmp = tempfile::tempdir().unwrap();
    init_project(&tmp);

    // Write treb.toml with Safe sender
    let safe_config = include_str!("fixtures/safe/safe-config.toml");
    fs::write(tmp.path().join("treb.toml"), safe_config).unwrap();

    let output = treb()
        .args(["config", "show", "--json"])
        .current_dir(tmp.path())
        .output()
        .expect("failed to run treb config show --json");

    assert!(output.status.success(), "config show should succeed");

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("output is not valid JSON");
    let senders = json["senders"].as_object().expect("senders should be an object");

    // The deployer role should map to the multisig account
    assert!(senders.contains_key("deployer"), "senders should contain 'deployer' role");

    let deployer = &senders["deployer"];
    assert_eq!(deployer["type"], "safe", "deployer should be of type 'safe'");
    assert_eq!(
        deployer["safe"], "0x1234567890123456789012345678901234567890",
        "should have correct safe address"
    );
    assert_eq!(deployer["signer"], "deployer", "safe sender should reference 'deployer' as signer");
}

#[test]
fn config_show_plaintext_includes_safe_type() {
    let tmp = tempfile::tempdir().unwrap();
    init_project(&tmp);

    let safe_config = include_str!("fixtures/safe/safe-config.toml");
    fs::write(tmp.path().join("treb.toml"), safe_config).unwrap();

    treb()
        .args(["config", "show"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("safe"));
}

// ── Governor sender detection from config ────────────────────────────────

#[test]
fn config_show_json_includes_governor_sender() {
    let tmp = tempfile::tempdir().unwrap();
    init_project(&tmp);

    // Write treb.toml with Governor sender
    let gov_config = include_str!("fixtures/safe/governor-config.toml");
    fs::write(tmp.path().join("treb.toml"), gov_config).unwrap();

    let output = treb()
        .args(["config", "show", "--json"])
        .current_dir(tmp.path())
        .output()
        .expect("failed to run treb config show --json");

    assert!(output.status.success(), "config show should succeed");

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("output is not valid JSON");
    let senders = json["senders"].as_object().expect("senders should be an object");

    assert!(senders.contains_key("deployer"), "senders should contain 'deployer' role");

    let deployer = &senders["deployer"];
    assert_eq!(deployer["type"], "oz_governor", "deployer should be of type 'oz_governor'");
    assert_eq!(
        deployer["governor"], "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "should have correct governor address"
    );
    assert_eq!(
        deployer["timelock"], "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "should have correct timelock address"
    );
    assert_eq!(
        deployer["proposer"], "deployer",
        "governor sender should reference 'deployer' as proposer"
    );
}

#[test]
fn config_show_plaintext_includes_governor_type() {
    let tmp = tempfile::tempdir().unwrap();
    init_project(&tmp);

    let gov_config = include_str!("fixtures/safe/governor-config.toml");
    fs::write(tmp.path().join("treb.toml"), gov_config).unwrap();

    treb()
        .args(["config", "show"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("oz_governor"));
}

// ── Dry-run proposal output: Safe sender ─────────────────────────────────

#[test]
fn dry_run_with_safe_sender_shows_safe_type_in_config() {
    let tmp = tempfile::tempdir().unwrap();
    init_project(&tmp);

    let safe_config = include_str!("fixtures/safe/safe-config.toml");
    fs::write(tmp.path().join("treb.toml"), safe_config).unwrap();

    // Running with --dry-run will fail because there's no script file, but the
    // important thing is that the error is about the pipeline (script not found),
    // not about sender resolution or config parsing.
    let output = treb()
        .args(["run", "script/Deploy.s.sol", "--dry-run"])
        .current_dir(tmp.path())
        .output()
        .expect("failed to run command");

    let stderr = String::from_utf8_lossy(&output.stderr);
    // The command should fail at pipeline execution (no script), not at config parsing
    assert!(
        !stderr.contains("unknown sender type"),
        "should not fail on sender type parsing: {stderr}"
    );
    assert!(!stderr.contains("missing required"), "should not fail on missing fields: {stderr}");
}

// ── Dry-run proposal output: Governor sender ─────────────────────────────

#[test]
fn dry_run_with_governor_sender_shows_governor_type_in_config() {
    let tmp = tempfile::tempdir().unwrap();
    init_project(&tmp);

    let gov_config = include_str!("fixtures/safe/governor-config.toml");
    fs::write(tmp.path().join("treb.toml"), gov_config).unwrap();

    let output = treb()
        .args(["run", "script/Deploy.s.sol", "--dry-run"])
        .current_dir(tmp.path())
        .output()
        .expect("failed to run command");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unknown sender type"),
        "should not fail on sender type parsing: {stderr}"
    );
    assert!(!stderr.contains("missing required"), "should not fail on missing fields: {stderr}");
}

// ── Sync with fixture registry ───────────────────────────────────────────

#[test]
fn sync_without_init_fails() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("foundry.toml"), MINIMAL_FOUNDRY_TOML).unwrap();

    treb()
        .args(["registry", "sync"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("treb init"));
}

#[test]
fn sync_help_shows_expected_flags() {
    let output = treb().args(["registry", "sync", "--help"]).output().expect("failed to run treb registry sync --help");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--network"), "help should show --network");
    assert!(stdout.contains("--clean"), "help should show --clean");
    assert!(stdout.contains("--json"), "help should show --json");
}

#[test]
fn sync_with_empty_registry_succeeds() {
    let tmp = tempfile::tempdir().unwrap();
    init_project(&tmp);

    // Sync with no safe transactions in registry should succeed gracefully
    treb().args(["registry", "sync", "--json"]).current_dir(tmp.path()).assert().success();

    // Verify JSON output
    let output = treb()
        .args(["registry", "sync", "--json"])
        .current_dir(tmp.path())
        .output()
        .expect("failed to run sync");

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("sync output should be valid JSON");

    assert_eq!(json["synced"], 0);
    assert_eq!(json["updated"], 0);
    assert_eq!(json["newlyExecuted"], 0);
}

#[test]
fn sync_plaintext_output_with_empty_registry() {
    let tmp = tempfile::tempdir().unwrap();
    init_project(&tmp);

    treb()
        .args(["registry", "sync"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Syncing registry..."))
        .stdout(predicate::str::contains("No pending Safe transactions found"))
        .stdout(predicate::str::contains("Registry synced successfully"));
}

// ── Fixture deserialization tests ─────────────────────────────────────────

#[test]
fn safe_service_response_fixture_is_valid_json() {
    let fixture = include_str!("fixtures/safe/safe-service-response.json");
    let json: serde_json::Value =
        serde_json::from_str(fixture).expect("fixture should be valid JSON");

    assert_eq!(json["count"], 2);
    assert!(json["results"].is_array());
    assert_eq!(json["results"].as_array().unwrap().len(), 2);

    // Verify executed transaction
    let tx0 = &json["results"][0];
    assert_eq!(tx0["isExecuted"], true);
    assert!(tx0["transactionHash"].is_string());
    assert!(tx0["confirmations"].is_array());
    assert_eq!(tx0["confirmations"].as_array().unwrap().len(), 2);

    // Verify pending transaction
    let tx1 = &json["results"][1];
    assert_eq!(tx1["isExecuted"], false);
    assert!(tx1["transactionHash"].is_null());
    assert_eq!(tx1["confirmations"].as_array().unwrap().len(), 1);
}

#[test]
fn propose_request_fixture_is_valid_json() {
    let fixture = include_str!("fixtures/safe/propose-request.json");
    let json: serde_json::Value =
        serde_json::from_str(fixture).expect("fixture should be valid JSON");

    assert!(json["safeTxGas"].is_string());
    assert!(json["baseGas"].is_string());
    assert!(json["gasPrice"].is_string());
    assert!(json["contractTransactionHash"].is_string());
    assert!(json["sender"].is_string());
    assert!(json["signature"].is_string());
    assert_eq!(json["nonce"], 7);
    assert_eq!(json["origin"], "treb");
}

#[test]
fn safe_config_fixture_is_valid_toml() {
    let fixture = include_str!("fixtures/safe/safe-config.toml");
    let parsed: toml::Value =
        toml::from_str(fixture).expect("safe-config fixture should be valid TOML");

    let accounts = parsed["accounts"].as_table().unwrap();
    assert!(accounts.contains_key("deployer"));
    assert!(accounts.contains_key("multisig"));

    let multisig = &accounts["multisig"];
    assert_eq!(multisig["type"].as_str(), Some("safe"));
    assert!(multisig["safe"].is_str());
    assert_eq!(multisig["signer"].as_str(), Some("deployer"));
}

#[test]
fn governor_config_fixture_is_valid_toml() {
    let fixture = include_str!("fixtures/safe/governor-config.toml");
    let parsed: toml::Value =
        toml::from_str(fixture).expect("governor-config fixture should be valid TOML");

    let accounts = parsed["accounts"].as_table().unwrap();
    assert!(accounts.contains_key("deployer"));
    assert!(accounts.contains_key("governance"));

    let governance = &accounts["governance"];
    assert_eq!(governance["type"].as_str(), Some("oz_governor"));
    assert!(governance["governor"].is_str());
    assert!(governance["timelock"].is_str());
    assert_eq!(governance["proposer"].as_str(), Some("deployer"));
}
