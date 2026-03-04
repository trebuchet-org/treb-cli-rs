//! End-to-end integration test suite for treb CLI multi-command workflows.
//!
//! These tests exercise the full deployment pipeline using in-process Anvil,
//! verifying that init, run, list, show, tag, prune, and reset commands
//! compose correctly end-to-end.
//!
//! Each test spawns a local Anvil instance, copies the gen-deploy-project
//! fixture (which includes forge-std and SimpleContract.sol), deploys a
//! contract using a treb-compatible script that emits `ContractDeployed`
//! events, and then exercises the relevant treb command.

use assert_cmd::cargo::cargo_bin_cmd;
use std::{fs, path::Path};

fn treb() -> assert_cmd::Command {
    cargo_bin_cmd!("treb-cli")
}

// ── Constants ─────────────────────────────────────────────────────────────────

/// treb.toml using Anvil account 0's well-known private key.
const TREB_TOML: &str = r#"[accounts.deployer]
type = "private_key"
private_key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"

[namespace.default]
senders = { deployer = "deployer" }
"#;

/// Solidity deploy script that emits treb-compatible ContractDeployed and
/// TransactionSimulated events so the pipeline records a clean registry entry.
const TREB_DEPLOY_SCRIPT: &str = r#"// SPDX-License-Identifier: UNLICENSED
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

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Recursively copy a directory tree from `src` to `dst`.
fn copy_dir_recursive(src: &Path, dst: &Path) {
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

/// Set up an isolated project directory for an e2e test.
///
/// Copies the gen-deploy-project fixture (includes forge-std and
/// SimpleContract.sol), writes a treb deploy script that emits treb events,
/// adds treb.toml with the Anvil deployer key, and runs `treb init`.
async fn setup_project() -> tempfile::TempDir {
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
async fn run_deployment(tmp_path: std::path::PathBuf, rpc_url: String) {
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

// ── Tests ─────────────────────────────────────────────────────────────────────

/// init → run → list: `treb list --json` returns exactly one deployment with
/// a non-zero EVM address.
#[tokio::test(flavor = "multi_thread")]
async fn e2e_init_run_list() {
    use treb_forge::anvil::AnvilConfig;

    let anvil = AnvilConfig::new().port(0).spawn().await.expect("failed to spawn Anvil");
    let rpc_url = anvil.rpc_url().to_string();

    let tmp = setup_project().await;
    run_deployment(tmp.path().to_path_buf(), rpc_url).await;

    let tmp_path = tmp.path().to_path_buf();
    let output = tokio::task::spawn_blocking(move || {
        treb().args(["list", "--json"]).current_dir(&tmp_path).output().unwrap()
    })
    .await
    .unwrap();

    assert!(output.status.success(), "treb list should exit 0");

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("treb list --json must emit valid JSON");
    let arr = json.as_array().expect("JSON output must be an array");
    assert_eq!(arr.len(), 1, "exactly one deployment should be recorded");

    let address = arr[0]["address"].as_str().expect("deployment must have 'address'");
    assert!(address.starts_with("0x"), "address must be 0x-prefixed: {address}");
    assert_ne!(
        address, "0x0000000000000000000000000000000000000000",
        "deployed address must be non-zero: {address}"
    );

    drop(anvil);
}

/// run → show: `treb show <id> --json` output contains `address`, `contractName`,
/// and `chainId` fields.
#[tokio::test(flavor = "multi_thread")]
async fn e2e_run_show() {
    use treb_forge::anvil::AnvilConfig;

    let anvil = AnvilConfig::new().port(0).spawn().await.expect("failed to spawn Anvil");
    let rpc_url = anvil.rpc_url().to_string();

    let tmp = setup_project().await;
    run_deployment(tmp.path().to_path_buf(), rpc_url).await;

    // Retrieve the deployment ID via list.
    let tmp_path = tmp.path().to_path_buf();
    let list_output = tokio::task::spawn_blocking(move || {
        treb().args(["list", "--json"]).current_dir(&tmp_path).output().unwrap()
    })
    .await
    .unwrap();

    let list_json: serde_json::Value = serde_json::from_slice(&list_output.stdout).unwrap();
    let arr = list_json.as_array().unwrap();
    assert_eq!(arr.len(), 1, "expected one deployment before show");
    let deployment_id = arr[0]["id"].as_str().expect("deployment must have 'id'").to_string();

    // Run `treb show <id> --json`.
    let tmp_path = tmp.path().to_path_buf();
    let dep_id = deployment_id.clone();
    let output = tokio::task::spawn_blocking(move || {
        treb().args(["show", &dep_id, "--json"]).current_dir(&tmp_path).output().unwrap()
    })
    .await
    .unwrap();

    assert!(output.status.success(), "treb show should exit 0");

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("treb show --json must emit valid JSON");

    assert!(json.get("address").is_some(), "show output must contain 'address'");
    assert!(json.get("contractName").is_some(), "show output must contain 'contractName'");
    assert!(json.get("chainId").is_some(), "show output must contain 'chainId'");

    drop(anvil);
}

/// run → tag → list-with-tag-filter: `treb list --tag v1.0.0 --json` returns
/// exactly one result after tagging.
#[tokio::test(flavor = "multi_thread")]
async fn e2e_run_tag_list_with_filter() {
    use treb_forge::anvil::AnvilConfig;

    let anvil = AnvilConfig::new().port(0).spawn().await.expect("failed to spawn Anvil");
    let rpc_url = anvil.rpc_url().to_string();

    let tmp = setup_project().await;
    run_deployment(tmp.path().to_path_buf(), rpc_url).await;

    // Retrieve the deployment ID.
    let tmp_path = tmp.path().to_path_buf();
    let list_output = tokio::task::spawn_blocking(move || {
        treb().args(["list", "--json"]).current_dir(&tmp_path).output().unwrap()
    })
    .await
    .unwrap();

    let list_json: serde_json::Value = serde_json::from_slice(&list_output.stdout).unwrap();
    let arr = list_json.as_array().unwrap();
    let deployment_id = arr[0]["id"].as_str().unwrap().to_string();

    // Tag the deployment with "v1.0.0".
    let tmp_path = tmp.path().to_path_buf();
    let dep_id = deployment_id.clone();
    tokio::task::spawn_blocking(move || {
        treb().args(["tag", &dep_id, "--add", "v1.0.0"]).current_dir(&tmp_path).assert().success();
    })
    .await
    .unwrap();

    // List with tag filter — should return exactly one result.
    let tmp_path = tmp.path().to_path_buf();
    let output = tokio::task::spawn_blocking(move || {
        treb().args(["list", "--tag", "v1.0.0", "--json"]).current_dir(&tmp_path).output().unwrap()
    })
    .await
    .unwrap();

    assert!(output.status.success(), "treb list --tag should exit 0");

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("treb list --json must emit valid JSON");
    let arr = json.as_array().expect("JSON output must be an array");
    assert_eq!(arr.len(), 1, "exactly one deployment should match tag v1.0.0");
    assert_eq!(
        arr[0]["id"].as_str().unwrap(),
        deployment_id.as_str(),
        "tagged deployment id should match"
    );

    drop(anvil);
}

/// run → prune --dry-run on clean registry: exits 0 and stdout contains
/// "Nothing to prune" because the registry has no broken cross-references.
#[tokio::test(flavor = "multi_thread")]
async fn e2e_run_prune_dry_run_clean() {
    use treb_forge::anvil::AnvilConfig;

    let anvil = AnvilConfig::new().port(0).spawn().await.expect("failed to spawn Anvil");
    let rpc_url = anvil.rpc_url().to_string();

    let tmp = setup_project().await;
    run_deployment(tmp.path().to_path_buf(), rpc_url).await;

    let tmp_path = tmp.path().to_path_buf();
    let output = tokio::task::spawn_blocking(move || {
        treb().args(["prune", "--dry-run"]).current_dir(&tmp_path).output().unwrap()
    })
    .await
    .unwrap();

    assert!(output.status.success(), "treb prune --dry-run should exit 0");

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("Nothing to prune"),
        "prune should report 'Nothing to prune' on a clean registry; got:\n{stdout}"
    );

    drop(anvil);
}

/// run → reset → list: `treb list --json` returns an empty array after
/// `treb reset --yes` wipes the registry.
#[tokio::test(flavor = "multi_thread")]
async fn e2e_run_reset_list() {
    use treb_forge::anvil::AnvilConfig;

    let anvil = AnvilConfig::new().port(0).spawn().await.expect("failed to spawn Anvil");
    let rpc_url = anvil.rpc_url().to_string();

    let tmp = setup_project().await;
    run_deployment(tmp.path().to_path_buf(), rpc_url).await;

    // Reset the registry without prompting.
    let tmp_path = tmp.path().to_path_buf();
    tokio::task::spawn_blocking(move || {
        treb().args(["reset", "--yes"]).current_dir(&tmp_path).assert().success();
    })
    .await
    .unwrap();

    // List should now return an empty array.
    let tmp_path = tmp.path().to_path_buf();
    let output = tokio::task::spawn_blocking(move || {
        treb().args(["list", "--json"]).current_dir(&tmp_path).output().unwrap()
    })
    .await
    .unwrap();

    assert!(output.status.success(), "treb list should exit 0 after reset");

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("treb list --json must emit valid JSON");
    let arr = json.as_array().expect("JSON output must be an array");
    assert!(arr.is_empty(), "registry should be empty after reset, but got {} entries", arr.len());

    drop(anvil);
}

/// `treb list --no-color` stdout must not contain ANSI escape sequences.
#[tokio::test(flavor = "multi_thread")]
async fn e2e_list_no_color_has_no_ansi() {
    use treb_forge::anvil::AnvilConfig;

    let anvil = AnvilConfig::new().port(0).spawn().await.expect("failed to spawn Anvil");
    let rpc_url = anvil.rpc_url().to_string();

    let tmp = setup_project().await;
    run_deployment(tmp.path().to_path_buf(), rpc_url).await;

    let tmp_path = tmp.path().to_path_buf();
    let output = tokio::task::spawn_blocking(move || {
        treb().args(["--no-color", "list"]).current_dir(&tmp_path).output().unwrap()
    })
    .await
    .unwrap();

    assert!(output.status.success(), "treb --no-color list should exit 0");

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        !stdout.contains("\x1b["),
        "treb --no-color list must not contain ANSI escape sequences;\nstdout:\n{stdout}"
    );

    drop(anvil);
}
