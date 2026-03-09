//! Integration tests for `treb list` and `treb show`.

use assert_cmd::cargo::cargo_bin_cmd;
use chrono::Utc;
use predicates::prelude::*;
use std::{collections::HashMap, fs};
use treb_core::types::{
    ArtifactInfo, Deployment, DeploymentMethod, DeploymentStrategy, DeploymentType,
    VerificationInfo, VerificationStatus,
};

fn treb() -> assert_cmd::Command {
    cargo_bin_cmd!("treb-cli")
}

const MINIMAL_FOUNDRY_TOML: &str = "[profile.default]\n";

/// Helper: create a temp dir with foundry.toml, run `treb init`, then insert
/// fixture deployments and rebuild the lookup index.
fn init_project_with_deployments(tmp: &tempfile::TempDir) {
    fs::write(tmp.path().join("foundry.toml"), MINIMAL_FOUNDRY_TOML).unwrap();
    treb().arg("init").current_dir(tmp.path()).assert().success();

    // Read fixture deployments from treb-core test fixtures.
    let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../treb-core/tests/fixtures/deployments_map.json");
    let fixture_json = fs::read_to_string(&fixture_path).expect("fixture file should exist");

    // Write deployments directly to .treb/deployments.json.
    fs::write(tmp.path().join(".treb/deployments.json"), &fixture_json).unwrap();

    // Rebuild the lookup index using the registry API.
    let registry = treb_registry::Registry::open(tmp.path()).expect("registry should open");
    registry.rebuild_lookup_index().expect("lookup index rebuild should succeed");
}

fn init_project_with_custom_deployments(
    tmp: &tempfile::TempDir,
    deployments: impl IntoIterator<Item = Deployment>,
) {
    init_empty_project(tmp);

    let mut registry = treb_registry::Registry::open(tmp.path()).expect("registry should open");
    for deployment in deployments {
        registry.insert_deployment(deployment).expect("deployment insert should succeed");
    }
}

/// Helper: create a temp dir with foundry.toml and run `treb init` (no deployments).
fn init_empty_project(tmp: &tempfile::TempDir) {
    fs::write(tmp.path().join("foundry.toml"), MINIMAL_FOUNDRY_TOML).unwrap();
    treb().arg("init").current_dir(tmp.path()).assert().success();
}

fn make_list_deployment(namespace: &str, chain_id: u64, contract_name: &str) -> Deployment {
    let ts = Utc::now();
    let label = "v1";

    Deployment {
        id: format!("{namespace}/{chain_id}/{contract_name}:{label}"),
        namespace: namespace.to_string(),
        chain_id,
        contract_name: contract_name.to_string(),
        label: label.to_string(),
        address: format!("0x{chain_id:040x}"),
        deployment_type: DeploymentType::Singleton,
        transaction_id: format!("tx-{chain_id}"),
        deployment_strategy: DeploymentStrategy {
            method: DeploymentMethod::Create,
            salt: String::new(),
            init_code_hash: String::new(),
            factory: String::new(),
            constructor_args: String::new(),
            entropy: String::new(),
        },
        proxy_info: None,
        artifact: ArtifactInfo {
            path: "contracts/Test.sol".to_string(),
            compiler_version: "0.8.24".to_string(),
            bytecode_hash: "0xabc".to_string(),
            script_path: "script/Deploy.s.sol".to_string(),
            git_commit: "abc123".to_string(),
        },
        verification: VerificationInfo {
            status: VerificationStatus::Unverified,
            etherscan_url: String::new(),
            verified_at: None,
            reason: String::new(),
            verifiers: HashMap::new(),
        },
        tags: None,
        created_at: ts,
        updated_at: ts,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// treb list
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn list_shows_table_with_deployments() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_deployments(&tmp);

    treb()
        .arg("list")
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("FPMM"))
        .stdout(predicate::str::contains("FPMMFactory"))
        .stdout(predicate::str::contains("TransparentUpgradeableProxy"))
        .stdout(predicate::str::contains("MAINNET"))
        .stdout(predicate::str::contains("42220"))
        .stdout(predicate::str::contains("SINGLETONS"))
        .stdout(predicate::str::contains("PROXIES"));
}

#[test]
fn list_table_shows_full_addresses() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_deployments(&tmp);

    let output = treb().arg("list").current_dir(tmp.path()).output().unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    // Address should be full (not truncated) in the table format
    assert!(
        stdout.contains("0x42eddd7dC046da254A93659CA9b02f294606833D"),
        "expected full address, got:\n{stdout}"
    );
}

#[test]
fn list_adds_separator_between_chains_in_same_namespace() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_custom_deployments(
        &tmp,
        [make_list_deployment("shared", 1, "Alpha"), make_list_deployment("shared", 42220, "Beta")],
    );

    let output = treb().arg("list").current_dir(tmp.path()).output().unwrap();

    assert!(output.status.success(), "treb list should exit 0");
    let stdout = String::from_utf8(output.stdout).unwrap();
    // The separator between chains is a blank continuation line (│ ) followed
    // by the next chain header (└─). ANSI codes may appear between └─ and the
    // chain label, so just check for the structural separator pattern.
    assert!(
        stdout.contains("\n│ \n└─"),
        "expected a post-chain separator before the next chain header, got:\n{stdout}"
    );
}

#[test]
fn list_json_outputs_valid_json_array() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_deployments(&tmp);

    let output = treb().args(["list", "--json"]).current_dir(tmp.path()).output().unwrap();

    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("output is not valid JSON");
    let arr = json.as_array().expect("JSON output should be an array");
    assert_eq!(arr.len(), 4);

    // Verify deployment objects have expected fields.
    let first = &arr[0];
    assert!(first.get("id").is_some());
    assert!(first.get("contractName").is_some());
    assert!(first.get("address").is_some());
}

#[test]
fn list_filter_by_namespace() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_deployments(&tmp);

    // All fixtures are namespace "mainnet", so filtering should return all.
    treb()
        .args(["list", "--namespace", "mainnet"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("FPMM"));

    // A non-existent namespace should show "No deployments found".
    treb()
        .args(["list", "--namespace", "nonexistent"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("No deployments found"));
}

#[test]
fn list_filter_by_contract() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_deployments(&tmp);

    let output = treb()
        .args(["list", "--contract", "FPMM", "--json"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let arr = json.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["contractName"], "FPMM");
}

#[test]
fn list_filtered_implementation_stays_in_implementations_group() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_deployments(&tmp);

    treb()
        .args(["list", "--contract", "FPMMFactory"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("IMPLEMENTATIONS"))
        .stdout(predicate::str::contains("FPMMFactory"))
        .stdout(predicate::str::contains("SINGLETONS").not());
}

#[test]
fn list_filter_by_type() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_deployments(&tmp);

    let output = treb()
        .args(["list", "--type", "PROXY", "--json"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let arr = json.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["type"], "PROXY");
}

#[test]
fn list_empty_registry_shows_message() {
    let tmp = tempfile::tempdir().unwrap();
    init_empty_project(&tmp);

    treb()
        .arg("list")
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("No deployments found"));
}

#[test]
fn list_ls_alias_works() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_deployments(&tmp);

    treb()
        .arg("ls")
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("FPMM"));
}

#[test]
fn list_uninitialized_project_fails() {
    let tmp = tempfile::tempdir().unwrap();
    // No foundry.toml, no init.

    treb()
        .arg("list")
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("treb init").or(predicate::str::contains("foundry.toml")));
}

#[test]
fn list_without_treb_dir_fails() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("foundry.toml"), MINIMAL_FOUNDRY_TOML).unwrap();
    // Don't run init.

    treb()
        .arg("list")
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("treb init"));
}

// ═══════════════════════════════════════════════════════════════════════════
// treb show
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn show_by_full_id() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_deployments(&tmp);

    treb()
        .args(["show", "mainnet/42220/FPMM:v3.0.0"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Identity"))
        .stdout(predicate::str::contains("FPMM"))
        .stdout(predicate::str::contains("v3.0.0"))
        .stdout(predicate::str::contains("mainnet"))
        .stdout(predicate::str::contains("On-Chain"))
        .stdout(predicate::str::contains("42220"))
        .stdout(predicate::str::contains("0x42eddd7dC046da254A93659CA9b02f294606833D"))
        .stdout(predicate::str::contains("Transaction"))
        .stdout(predicate::str::contains("Artifact"))
        .stdout(predicate::str::contains("Verification"))
        .stdout(predicate::str::contains("Timestamps"));
}

#[test]
fn show_json_outputs_full_deployment() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_deployments(&tmp);

    let output = treb()
        .args(["show", "mainnet/42220/FPMM:v3.0.0", "--json"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("output is not valid JSON");
    let obj = json.as_object().expect("JSON output should be an object");
    assert_eq!(obj["id"], "mainnet/42220/FPMM:v3.0.0");
    assert_eq!(obj["contractName"], "FPMM");
    assert_eq!(obj["chainId"], 42220);
    assert_eq!(obj["address"], "0x42eddd7dC046da254A93659CA9b02f294606833D");
}

#[test]
fn show_by_contract_name() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_deployments(&tmp);

    // FPMMFactory is unique, should resolve.
    treb()
        .args(["show", "FPMMFactory"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("FPMMFactory"))
        .stdout(predicate::str::contains("v3.0.0"));
}

#[test]
fn show_by_address() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_deployments(&tmp);

    treb()
        .args(["show", "0x42eddd7dC046da254A93659CA9b02f294606833D"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("FPMM"))
        .stdout(predicate::str::contains("mainnet/42220/FPMM:v3.0.0"));
}

#[test]
fn show_nonexistent_fails() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_deployments(&tmp);

    treb()
        .args(["show", "NonexistentContract"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("no deployment found"))
        .stderr(predicate::str::contains("treb list"));
}

#[test]
fn show_without_argument_fails() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_deployments(&tmp);

    treb().arg("show").current_dir(tmp.path()).assert().failure();
}

#[test]
fn show_uninitialized_project_fails() {
    let tmp = tempfile::tempdir().unwrap();

    treb()
        .args(["show", "anything"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("treb init").or(predicate::str::contains("foundry.toml")));
}

#[test]
fn show_proxy_deployment_shows_proxy_info() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_deployments(&tmp);

    treb()
        .args(["show", "TransparentUpgradeableProxy"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Proxy Info"))
        .stdout(predicate::str::contains("UUPS"))
        .stdout(predicate::str::contains("Implementation"));
}

#[test]
fn show_non_proxy_deployment_hides_proxy_info() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_deployments(&tmp);

    let output = treb().args(["show", "FPMMFactory"]).current_dir(tmp.path()).output().unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        !stdout.contains("Proxy Info"),
        "non-proxy deployment should not show Proxy Info section"
    );
}
