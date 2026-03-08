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
    fs::write(
        tmp.path().join("script").join("TrebDeploySimple.s.sol"),
        TREB_DEPLOY_SCRIPT,
    )
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

/// Run a treb subcommand with `--json` and return the parsed JSON value.
///
/// Panics if the command fails or stdout is not valid JSON.
pub async fn run_json(
    tmp_path: std::path::PathBuf,
    args: Vec<String>,
) -> serde_json::Value {
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
    let arr = json.as_array().expect("treb list --json must be an array");
    assert_eq!(
        arr.len(),
        expected,
        "expected {expected} deployments, got {}",
        arr.len()
    );
    arr.clone()
}

/// Extract the deployment ID from the first entry in `treb list --json`.
///
/// Panics if there are no deployments or the `id` field is missing.
pub async fn get_deployment_id(tmp_path: std::path::PathBuf) -> String {
    let json = run_json(tmp_path, vec!["list".into()]).await;
    let arr = json.as_array().expect("treb list --json must be an array");
    assert!(!arr.is_empty(), "no deployments found");
    arr[0]["id"]
        .as_str()
        .expect("deployment must have 'id' field")
        .to_string()
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
    deps.as_object()
        .expect("deployments.json must be an object")
        .len()
}
