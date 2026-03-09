#![allow(dead_code)]

//! Shared E2E test infrastructure for multi-command workflow tests.
//!
//! Provides reusable helpers for project setup, Anvil spawning, CLI invocation,
//! JSON assertion, and direct registry file reading.

use assert_cmd::cargo::cargo_bin_cmd;
use std::{fs, path::Path};

// ── CLI ─────────────────────────────────────────────────────────────────────

/// Build an `assert_cmd::Command` pointing at the `treb-cli` binary.
pub fn treb() -> assert_cmd::Command {
    cargo_bin_cmd!("treb-cli")
}

// ── Constants ───────────────────────────────────────────────────────────────

/// treb.toml using Anvil account 0's well-known private key.
pub const TREB_TOML: &str = r#"[accounts.deployer]
type = "private_key"
private_key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"

[namespace.default]
senders = { deployer = "deployer" }
"#;

/// Solidity deploy script that emits treb-compatible ContractDeployed and
/// TransactionSimulated events so the pipeline records a clean registry entry.
pub const TREB_DEPLOY_SCRIPT: &str = r#"// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Script} from "forge-std/Script.sol";
import {SimpleContract} from "../src/SimpleContract.sol";

struct DeploymentDetails {
    string artifact;
    string label;
    string entropy;
    bytes32 salt;
    bytes32 bytecodeHash;
    bytes32 initCodeHash;
    bytes constructorArgs;
    string createStrategy;
}

struct TxDetails {
    address to;
    bytes data;
    uint256 value;
}

struct SimTx {
    bytes32 transactionId;
    string senderId;
    address sender;
    bytes returnData;
    TxDetails transaction;
}

contract TrebDeploySimple is Script {
    event ContractDeployed(
        address indexed deployer,
        address indexed location,
        bytes32 indexed transactionId,
        DeploymentDetails deployment
    );

    event TransactionSimulated(SimTx[] transactions);

    function run() public {
        vm.startBroadcast();

        SimpleContract simple = new SimpleContract();

        bytes32 txId = keccak256(
            abi.encode(block.chainid, block.number, address(simple))
        );
        bytes32 initCodeHash = keccak256(type(SimpleContract).creationCode);
        bytes32 bytecodeHash = keccak256(address(simple).code);

        emit ContractDeployed(
            msg.sender,
            address(simple),
            txId,
            DeploymentDetails({
                artifact: "SimpleContract",
                label: "SimpleContract",
                entropy: "",
                salt: bytes32(0),
                bytecodeHash: bytecodeHash,
                initCodeHash: initCodeHash,
                constructorArgs: bytes(""),
                createStrategy: "create"
            })
        );

        SimTx[] memory txs = new SimTx[](1);
        txs[0] = SimTx({
            transactionId: txId,
            senderId: "deployer",
            sender: msg.sender,
            returnData: bytes(""),
            transaction: TxDetails({to: address(0), data: bytes(""), value: 0})
        });
        emit TransactionSimulated(txs);

        vm.stopBroadcast();
    }
}
"#;

// ── Filesystem Helpers ──────────────────────────────────────────────────────

/// Recursively copy a directory tree from `src` to `dst`.
pub fn copy_dir_recursive(src: &Path, dst: &Path) {
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            fs::create_dir_all(&dst_path).unwrap();
            copy_dir_recursive(&src_path, &dst_path);
        } else {
            fs::copy(&src_path, &dst_path).unwrap();
        }
    }
}

// ── Anvil ───────────────────────────────────────────────────────────────────

/// Spawn Anvil for e2e tests. In restricted environments where process
/// forking is disallowed, return `None` so tests can skip cleanly.
pub async fn spawn_anvil_or_skip() -> Option<treb_forge::AnvilInstance> {
    match treb_forge::anvil::AnvilConfig::new().port(0).spawn().await {
        Ok(anvil) => Some(anvil),
        Err(err) if err.to_string().contains("Operation not permitted") => None,
        Err(err) => panic!("failed to spawn Anvil: {err}"),
    }
}

// ── Project Setup ───────────────────────────────────────────────────────────

/// Set up an isolated project directory for an e2e test.
///
/// Copies the gen-deploy-project fixture (includes forge-std and
/// SimpleContract.sol), writes a treb deploy script that emits treb events,
/// adds treb.toml with the Anvil deployer key, and runs `treb init`.
pub async fn setup_project() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();

    // Copy gen-deploy-project as the base: provides forge-std, SimpleContract.sol, foundry.toml.
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("gen-deploy-project");
    copy_dir_recursive(&fixture, tmp.path());

    // Write the treb deploy script.
    fs::create_dir_all(tmp.path().join("script")).unwrap();
    fs::write(tmp.path().join("script").join("TrebDeploySimple.s.sol"), TREB_DEPLOY_SCRIPT)
        .unwrap();

    // Write treb.toml with the Anvil deployer private key.
    fs::write(tmp.path().join("treb.toml"), TREB_TOML).unwrap();

    // Run `treb init` to create the .treb/ directory.
    let tmp_path = tmp.path().to_path_buf();
    tokio::task::spawn_blocking(move || {
        treb().arg("init").current_dir(&tmp_path).assert().success();
    })
    .await
    .expect("treb init should not panic");

    tmp
}

/// Deploy SimpleContract against the given Anvil RPC URL and assert success.
pub async fn run_deployment(tmp_path: std::path::PathBuf, rpc_url: String) {
    tokio::task::spawn_blocking(move || {
        treb()
            .args([
                "run",
                "script/TrebDeploySimple.s.sol",
                "--rpc-url",
                &rpc_url,
                "--broadcast",
                "--non-interactive",
            ])
            .current_dir(&tmp_path)
            .assert()
            .success();
    })
    .await
    .expect("treb run should not panic");
}

// ── JSON Assertion Helpers ──────────────────────────────────────────────────

/// Run a treb subcommand in human (non-JSON) mode and return stdout as a String.
///
/// Panics if the command fails.
pub async fn run_human(tmp_path: std::path::PathBuf, args: Vec<String>) -> String {
    let output = tokio::task::spawn_blocking(move || {
        treb()
            .args(args.iter().map(|s| s.as_str()).collect::<Vec<_>>())
            .current_dir(&tmp_path)
            .output()
            .unwrap()
    })
    .await
    .unwrap();

    assert!(
        output.status.success(),
        "treb command failed (exit {:?}):\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
    );

    String::from_utf8(output.stdout).expect("stdout must be valid UTF-8")
}

/// Run a treb subcommand with `--json` and return the parsed JSON value.
///
/// Panics if the command fails or stdout is not valid JSON.
pub async fn run_json(tmp_path: std::path::PathBuf, args: Vec<String>) -> serde_json::Value {
    let output = tokio::task::spawn_blocking(move || {
        treb()
            .args(args.iter().map(|s| s.as_str()).collect::<Vec<_>>())
            .arg("--json")
            .current_dir(&tmp_path)
            .output()
            .unwrap()
    })
    .await
    .unwrap();

    assert!(
        output.status.success(),
        "treb command failed (exit {:?}):\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
    );

    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "invalid JSON from treb --json: {e}\nstdout: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

/// Assert that `treb list --json` returns exactly `expected` deployments.
///
/// Returns the parsed JSON array for further inspection.
pub async fn assert_deployment_count(
    tmp_path: std::path::PathBuf,
    expected: usize,
) -> Vec<serde_json::Value> {
    let json = run_json(tmp_path, vec!["list".into()]).await;
    let arr = json["deployments"].as_array().expect("treb list --json must have deployments array");
    assert_eq!(arr.len(), expected, "expected {expected} deployments, got {}", arr.len());
    arr.clone()
}

/// Extract the deployment ID from the first entry in `treb list --json`.
///
/// Panics if there are no deployments or the `id` field is missing.
pub async fn get_deployment_id(tmp_path: std::path::PathBuf) -> String {
    let json = run_json(tmp_path, vec!["list".into()]).await;
    let arr = json["deployments"].as_array().expect("treb list --json must have deployments array");
    assert!(!arr.is_empty(), "no deployments found");
    arr[0]["id"].as_str().expect("deployment must have 'id' field").to_string()
}

// ── Registry File Readers ───────────────────────────────────────────────────

/// Read and parse a JSON file from the `.treb/` registry directory.
///
/// `project_root` is the project directory; `filename` is relative to `.treb/`
/// (e.g., `"deployments.json"`).
pub fn read_registry_file(project_root: &Path, filename: &str) -> serde_json::Value {
    let path = project_root.join(".treb").join(filename);
    let data = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    serde_json::from_str(&data)
        .unwrap_or_else(|e| panic!("invalid JSON in {}: {e}", path.display()))
}

/// Read `.treb/deployments.json` and return the parsed map.
///
/// The returned value is a JSON object mapping deployment ID → deployment record.
pub fn read_deployments(project_root: &Path) -> serde_json::Value {
    read_registry_file(project_root, "deployments.json")
}

/// Read `.treb/transactions.json` and return the parsed map.
///
/// The returned value is a JSON object mapping transaction ID → transaction record.
pub fn read_transactions(project_root: &Path) -> serde_json::Value {
    read_registry_file(project_root, "transactions.json")
}

/// Count the number of deployments in `.treb/deployments.json`.
pub fn deployment_count(project_root: &Path) -> usize {
    let deps = read_deployments(project_root);
    deps.as_object().expect("deployments.json must be an object").len()
}

// ── Registry Consistency ────────────────────────────────────────────────────

/// Assert that lookup.json cross-references are consistent with deployments.json.
///
/// Deployment IDs are the object keys in `deployments.json`; `lookup.json`
/// only stores secondary indexes for name, address, and tag lookups.
///
/// Validates:
/// - Every deployment ID appears in `byName[contractName.toLowerCase()]`
/// - Every deployment with a non-empty address appears in `byAddress[address.toLowerCase()]`
/// - Every tagged deployment has its ID in `byTag[tag]`
/// - Every ID referenced in lookup.json actually exists in deployments.json
pub fn assert_registry_consistent(project_root: &Path) {
    let deps_json = read_deployments(project_root);
    let deps = deps_json.as_object().expect("deployments.json must be an object");

    let lookup_json = read_registry_file(project_root, "lookup.json");
    let by_name = lookup_json["byName"].as_object().expect("lookup must have byName");
    let by_address = lookup_json["byAddress"].as_object().expect("lookup must have byAddress");
    let by_tag = lookup_json["byTag"].as_object().expect("lookup must have byTag");

    // Forward: every deployment should be indexed in lookup.
    for (dep_id, dep) in deps {
        let contract_name = dep["contractName"]
            .as_str()
            .unwrap_or_else(|| panic!("deployment {dep_id} missing contractName"));
        let name_key = contract_name.to_lowercase();

        // byName must contain this deployment ID.
        let name_ids = by_name
            .get(&name_key)
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| panic!("byName missing key '{name_key}' for deployment {dep_id}"));
        assert!(
            name_ids.iter().any(|v| v.as_str() == Some(dep_id)),
            "byName['{name_key}'] does not contain deployment ID '{dep_id}'"
        );

        // byAddress must contain this deployment ID (if address is non-empty).
        let address = dep["address"].as_str().unwrap_or("");
        if !address.is_empty() {
            let addr_key = address.to_lowercase();
            let addr_id = by_address.get(&addr_key).and_then(|v| v.as_str()).unwrap_or_else(|| {
                panic!("byAddress missing key '{addr_key}' for deployment {dep_id}")
            });
            assert_eq!(
                addr_id, dep_id,
                "byAddress['{addr_key}'] = '{addr_id}', expected '{dep_id}'"
            );
        }

        // byTag must contain this deployment ID for each tag.
        if let Some(tags) = dep["tags"].as_array() {
            for tag_val in tags {
                let tag = tag_val.as_str().expect("tag must be a string");
                let tag_ids = by_tag
                    .get(tag)
                    .and_then(|v| v.as_array())
                    .unwrap_or_else(|| panic!("byTag missing key '{tag}' for deployment {dep_id}"));
                assert!(
                    tag_ids.iter().any(|v| v.as_str() == Some(dep_id)),
                    "byTag['{tag}'] does not contain deployment ID '{dep_id}'"
                );
            }
        }
    }

    // Reverse: every ID in lookup must exist in deployments.
    for (name_key, ids_val) in by_name {
        for id_val in ids_val.as_array().unwrap_or(&vec![]) {
            let id = id_val.as_str().expect("byName entry must be string");
            assert!(
                deps.contains_key(id),
                "byName['{name_key}'] references non-existent ID '{id}'"
            );
        }
    }
    for (addr_key, id_val) in by_address {
        let id = id_val.as_str().expect("byAddress entry must be string");
        assert!(deps.contains_key(id), "byAddress['{addr_key}'] references non-existent ID '{id}'");
    }
    for (tag, ids_val) in by_tag {
        for id_val in ids_val.as_array().unwrap_or(&vec![]) {
            let id = id_val.as_str().expect("byTag entry must be string");
            assert!(deps.contains_key(id), "byTag['{tag}'] references non-existent ID '{id}'");
        }
    }
}
