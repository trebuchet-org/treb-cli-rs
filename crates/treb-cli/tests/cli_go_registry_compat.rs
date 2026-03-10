//! CLI integration coverage for Go-created registry data.

mod helpers;

use std::fs;

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;

const MINIMAL_FOUNDRY_TOML: &str = "[profile.default]\n";
const GO_DEPLOYMENT_COUNT: usize = 13;
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
    assert!(deployment_ids.contains(&"virtual/42220/MentoRouter:v1.0.0"));
    assert!(deployment_ids.contains(&"virtual/8453/ConstantProductPricingModule:v2.6.5"));

    let lookup: Value = serde_json::from_str(
        &fs::read_to_string(tmp.path().join(".treb/lookup.json"))
            .expect("lookup.json should exist"),
    )
    .expect("lookup.json should be valid JSON");

    assert_eq!(
        lookup["byAddress"]["0x959597fd009876e6f53ebdb2f1c1bc3f994579df"].as_str(),
        Some(EXISTING_CORE_ID)
    );
    assert!(
        lookup["byName"]["transparentupgradeableproxy"]
            .as_array()
            .expect("proxy name index should exist")
            .iter()
            .any(|id| id.as_str() == Some(GO_PROXY_ID)),
        "lookup.json should index Go-created proxy deployments by name"
    );
    assert!(
        lookup["byTag"]["core"]
            .as_array()
            .expect("core tag index should exist")
            .iter()
            .any(|id| id.as_str() == Some(EXISTING_CORE_ID)),
        "lookup.json should preserve tags from Go-created deployments"
    );
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
