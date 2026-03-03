//! Integration tests for `treb networks`.

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use std::fs;

fn treb() -> assert_cmd::Command {
    cargo_bin_cmd!("treb-cli")
}

/// Minimal foundry.toml with two RPC endpoints using unresolved env vars
/// so that no real HTTP calls are needed (deterministic).
const FIXTURE_FOUNDRY_TOML: &str = r#"
[profile.default]
src = "src"

[rpc_endpoints]
mainnet = "${MAINNET_RPC_URL}"
sepolia = "${SEPOLIA_RPC_URL}"
"#;

#[test]
fn networks_errors_without_foundry_toml() {
    let tmp = tempfile::tempdir().unwrap();

    treb()
        .arg("networks")
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("foundry.toml"));
}

#[test]
fn networks_table_shows_endpoint_names() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("foundry.toml"), FIXTURE_FOUNDRY_TOML).unwrap();

    treb()
        .arg("networks")
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("mainnet"))
        .stdout(predicate::str::contains("sepolia"));
}

#[test]
fn networks_json_parses_with_expected_fields() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("foundry.toml"), FIXTURE_FOUNDRY_TOML).unwrap();

    let output = treb()
        .args(["networks", "--json"])
        .current_dir(tmp.path())
        .output()
        .expect("failed to run treb networks --json");

    assert!(output.status.success());

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("output is not valid JSON");

    let arr = json.as_array().expect("JSON output is not an array");
    assert!(!arr.is_empty(), "JSON array is empty");

    let names: Vec<&str> = arr
        .iter()
        .map(|v| v.get("name").unwrap().as_str().unwrap())
        .collect();

    assert!(names.contains(&"mainnet"));
    assert!(names.contains(&"sepolia"));

    // Verify all entries have the expected fields
    for entry in arr {
        let obj = entry.as_object().expect("entry is not an object");
        assert!(obj.contains_key("name"), "missing field: name");
        assert!(obj.contains_key("rpc_url"), "missing field: rpc_url");
        assert!(obj.contains_key("chain_id"), "missing field: chain_id");
        assert!(obj.contains_key("status"), "missing field: status");
    }
}
