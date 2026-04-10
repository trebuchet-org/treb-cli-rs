//! CLI integration coverage for Go-created registry data.

mod helpers;

use std::fs;

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;

const MINIMAL_FOUNDRY_TOML: &str = "[profile.default]\n";
/// The go-compat fixture has 13 total entries across namespaces.
/// The seeder sets namespace to `mainnet` in local config, so `treb list`
/// scopes to the 5 mainnet deployments.
const GO_DEPLOYMENT_COUNT: usize = 5;
const EXISTING_CORE_ID: &str = "mainnet/42220/FPMMFactory:v3.0.0";
const GO_PROXY_ID: &str = "mainnet/143/TransparentUpgradeableProxy:GBPm";

fn treb() -> assert_cmd::Command {
    cargo_bin_cmd!("treb-cli")
}

fn init_project_with_go_registry(tmp: &tempfile::TempDir) {
    fs::write(tmp.path().join("foundry.toml"), MINIMAL_FOUNDRY_TOML).unwrap();
    treb().arg("init").current_dir(tmp.path()).assert().success();
    helpers::seed_go_compat_registry(tmp.path());
}

#[test]
fn list_against_go_registry_data_shows_correct_count_and_builds_lookup() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_go_registry(&tmp);

    let output = treb()
        .args(["list", "--json"])
        .env("NO_COLOR", "1")
        .current_dir(tmp.path())
        .output()
        .unwrap();

    assert!(output.status.success(), "treb list should exit 0");

    let json: Value =
        serde_json::from_slice(&output.stdout).expect("list output should be valid JSON");
    let deployments = json["deployments"].as_array().expect("list JSON must include deployments");
    assert_eq!(deployments.len(), GO_DEPLOYMENT_COUNT);

    let deployment_ids: Vec<_> =
        deployments.iter().filter_map(|entry| entry["id"].as_str()).collect();
    assert!(deployment_ids.contains(&EXISTING_CORE_ID));
    assert!(deployment_ids.contains(&GO_PROXY_ID));
}

#[test]
fn show_against_go_registry_data_displays_correct_proxy_details() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_go_registry(&tmp);

    let output = treb()
        .args(["show", GO_PROXY_ID])
        .env("NO_COLOR", "1")
        .current_dir(tmp.path())
        .output()
        .unwrap();

    assert!(output.status.success(), "treb show should exit 0");

    let stdout = String::from_utf8(output.stdout).expect("show output should be utf-8");
    assert!(stdout.contains("Deployment: mainnet/143/TransparentUpgradeableProxy:GBPm"));
    assert!(stdout.contains("Contract: TransparentUpgradeableProxy:GBPm"));
    assert!(stdout.contains("Address: 0x39bb4E0a204412bB98e821d25e7d955e69d40Fd1"));
    assert!(stdout.contains("Type: PROXY"));
    assert!(stdout.contains("Namespace: mainnet"));
    assert!(stdout.contains("Network: 143"));
    assert!(stdout.contains("Method: CREATE3"));
    assert!(stdout.contains("Type: UUPS"));
    assert!(stdout.contains("Implementation ID: mainnet/143/StableTokenSpoke:v3.0.0"));
}
