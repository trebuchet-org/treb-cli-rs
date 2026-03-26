//! Integration coverage for compose behavior around linked external libraries.
//!
//! These tests exercise a two-step compose where:
//! 1. step 1 explicitly deploys an external library contract
//! 2. step 2 deploys a contract that links against that same library source
//!
//! The expected behavior is that step 1 is recorded as a `LIBRARY`
//! deployment and step 2 reuses that address through Foundry `--libraries`
//! linker flags instead of predeploying the library again.

mod framework;

use std::{
    fs,
    path::Path,
    sync::{Arc, OnceLock},
};

use framework::context::TestContext;

fn compose_test_lock() -> Arc<tokio::sync::Semaphore> {
    static LOCK: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    LOCK.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(1))).clone()
}

async fn compose_library_test_context() -> Option<TestContext> {
    match TestContext::new("project").with_anvil("anvil-31337").await {
        Ok(ctx) => Some(ctx),
        Err(err) if err.to_string().contains("Operation not permitted") => None,
        Err(err) => panic!("failed to spawn anvil: {err}"),
    }
}

fn install_library_fixture(ctx: &TestContext) {
    write_library_script(ctx.path());
    write_consumer_script(ctx.path());
    write_compose_file(ctx.path());
}

fn install_artifact_library_fixture(ctx: &TestContext) {
    write_library_script(ctx.path());
    write_artifact_consumer_script(ctx.path());
    write_artifact_compose_file(ctx.path());
}

fn write_library_script(project_dir: &Path) {
    fs::write(
        project_dir.join("script").join("DeployStringUtilsV2.s.sol"),
        r#"// SPDX-License-Identifier: MIT
pragma solidity =0.8.30;

import "forge-std/Script.sol";
import "../src/StringUtilsV2.sol";

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

contract DeployStringUtilsV2Script is Script {
    event ContractDeployed(
        address indexed deployer,
        address indexed location,
        bytes32 indexed transactionId,
        DeploymentDetails deployment
    );

    event TransactionSimulated(SimTx[] transactions);

    function run() public {
        bytes memory creationCode = type(StringUtilsV2).creationCode;

        vm.startBroadcast();

        address deployed;
        assembly {
            deployed := create(0, add(creationCode, 0x20), mload(creationCode))
            if iszero(deployed) { revert(0, 0) }
        }

        bytes32 txId = keccak256(abi.encode(block.chainid, block.number, deployed));

        emit ContractDeployed(
            msg.sender,
            deployed,
            txId,
            DeploymentDetails({
                artifact: "StringUtilsV2",
                label: "StringUtilsV2",
                entropy: "",
                salt: bytes32(0),
                bytecodeHash: keccak256(deployed.code),
                initCodeHash: keccak256(type(StringUtilsV2).creationCode),
                constructorArgs: bytes(""),
                createStrategy: "create"
            })
        );

        SimTx[] memory txs = new SimTx[](1);
        txs[0] = SimTx({
            transactionId: txId,
            senderId: "anvil",
            sender: msg.sender,
            returnData: bytes(""),
            transaction: TxDetails({to: address(0), data: creationCode, value: 0})
        });
        emit TransactionSimulated(txs);

        vm.stopBroadcast();
    }
}
"#,
    )
    .expect("failed to write library deployment script");
}

fn write_consumer_script(project_dir: &Path) {
    fs::write(
        project_dir.join("script").join("DeployMessageStorageV08.s.sol"),
        r#"// SPDX-License-Identifier: MIT
pragma solidity =0.8.30;

import "forge-std/Script.sol";
import "../src/MessageStorageV08.sol";

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

contract DeployMessageStorageV08Script is Script {
    event ContractDeployed(
        address indexed deployer,
        address indexed location,
        bytes32 indexed transactionId,
        DeploymentDetails deployment
    );

    event TransactionSimulated(SimTx[] transactions);

    function run() public {
        bytes memory constructorArgs = abi.encode("hello from compose");
        bytes memory creationCode = abi.encodePacked(
            type(MessageStorageV08).creationCode,
            constructorArgs
        );

        vm.startBroadcast();

        MessageStorageV08 deployed = new MessageStorageV08("hello from compose");

        bytes32 txId = keccak256(abi.encode(block.chainid, block.number, address(deployed)));

        emit ContractDeployed(
            msg.sender,
            address(deployed),
            txId,
            DeploymentDetails({
                artifact: "MessageStorageV08",
                label: "MessageStorageV08",
                entropy: "",
                salt: bytes32(0),
                bytecodeHash: keccak256(address(deployed).code),
                initCodeHash: keccak256(creationCode),
                constructorArgs: constructorArgs,
                createStrategy: "create"
            })
        );

        SimTx[] memory txs = new SimTx[](1);
        txs[0] = SimTx({
            transactionId: txId,
            senderId: "anvil",
            sender: msg.sender,
            returnData: bytes(""),
            transaction: TxDetails({to: address(0), data: creationCode, value: 0})
        });
        emit TransactionSimulated(txs);

        vm.stopBroadcast();
    }
}
"#,
    )
    .expect("failed to write consumer deployment script");
}

fn write_artifact_consumer_script(project_dir: &Path) {
    fs::write(
        project_dir.join("script").join("DeployMessageStorageArtifact.s.sol"),
        r#"// SPDX-License-Identifier: MIT
pragma solidity =0.8.30;

interface VmExt {
    function startBroadcast() external;
    function stopBroadcast() external;
    function getCode(string calldata artifactPath) external view returns (bytes memory creationBytecode);
}

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

contract DeployMessageStorageArtifactScript {
    VmExt private constant vm = VmExt(address(uint160(uint256(keccak256("hevm cheat code")))));

    event ContractDeployed(
        address indexed deployer,
        address indexed location,
        bytes32 indexed transactionId,
        DeploymentDetails deployment
    );

    event TransactionSimulated(SimTx[] transactions);

    function run() public {
        bytes memory constructorArgs = abi.encode("hello from artifact");
        bytes memory creationCode = abi.encodePacked(
            vm.getCode("src/MessageStorageV08.sol:MessageStorageV08"),
            constructorArgs
        );

        vm.startBroadcast();

        address deployed;
        assembly {
            deployed := create(0, add(creationCode, 0x20), mload(creationCode))
            if iszero(deployed) { revert(0, 0) }
        }

        bytes32 txId = keccak256(abi.encode(block.chainid, block.number, deployed));

        emit ContractDeployed(
            msg.sender,
            deployed,
            txId,
            DeploymentDetails({
                artifact: "src/MessageStorageV08.sol:MessageStorageV08",
                label: "MessageStorageV08",
                entropy: "",
                salt: bytes32(0),
                bytecodeHash: keccak256(deployed.code),
                initCodeHash: keccak256(creationCode),
                constructorArgs: constructorArgs,
                createStrategy: "create"
            })
        );

        SimTx[] memory txs = new SimTx[](1);
        txs[0] = SimTx({
            transactionId: txId,
            senderId: "anvil",
            sender: msg.sender,
            returnData: bytes(""),
            transaction: TxDetails({to: address(0), data: creationCode, value: 0})
        });
        emit TransactionSimulated(txs);

        vm.stopBroadcast();
    }
}
"#,
    )
    .expect("failed to write artifact consumer deployment script");
}

fn write_compose_file(project_dir: &Path) {
    fs::write(
        project_dir.join("library-link.yaml"),
        r#"group: linked-library
components:
  library:
    script: script/DeployStringUtilsV2.s.sol
  consumer:
    script: script/DeployMessageStorageV08.s.sol
    deps:
      - library
"#,
    )
    .expect("failed to write compose file");
}

fn write_artifact_compose_file(project_dir: &Path) {
    fs::write(
        project_dir.join("library-link-artifact.yaml"),
        r#"group: linked-library-artifact
components:
  library:
    script: script/DeployStringUtilsV2.s.sol
  consumer:
    script: script/DeployMessageStorageArtifact.s.sol
    deps:
      - library
"#,
    )
    .expect("failed to write artifact compose file");
}

fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_str(
        &fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display())),
    )
    .unwrap_or_else(|e| panic!("invalid json in {}: {e}", path.display()))
}

fn read_deployments(project_root: &Path, namespace: &str, chain_id: u64) -> serde_json::Value {
    let value = read_json(
        &project_root.join("deployments").join(namespace).join(format!("{chain_id}.json")),
    );
    value
        .as_object()
        .and_then(|object| object.get("entries").filter(|_| object.contains_key("_format")))
        .cloned()
        .unwrap_or(value)
}

fn deployment_address(deployments: &serde_json::Value, contract_name: &str) -> String {
    deployment_entry(deployments, contract_name)["address"]
        .as_str()
        .unwrap_or_else(|| panic!("missing address for contract {contract_name}"))
        .to_string()
}

fn deployment_entry<'a>(
    deployments: &'a serde_json::Value,
    contract_name: &str,
) -> &'a serde_json::Value {
    deployments
        .as_object()
        .unwrap()
        .values()
        .find(|deployment| {
            deployment["contractName"].as_str().is_some_and(|name| {
                name == contract_name || name.ends_with(&format!(":{contract_name}"))
            })
        })
        .unwrap_or_else(|| panic!("missing deployment for contract {contract_name}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn compose_linked_library_step_reuses_previous_library_in_next_script() {
    let _permit = compose_test_lock().acquire_owned().await.unwrap();
    let Some(ctx) = compose_library_test_context().await else {
        return;
    };

    install_library_fixture(&ctx);
    ctx.run(["init"]).success();
    let rpc_url = ctx.anvil("anvil-31337").unwrap().rpc_url().to_string();

    let compose = ctx.run([
        "compose",
        "library-link.yaml",
        "--network",
        "anvil-31337",
        "--rpc-url",
        rpc_url.as_str(),
        "--broadcast",
        "--non-interactive",
    ]);
    let stderr = String::from_utf8_lossy(&compose.get_output().stderr).to_string();
    compose.success();

    let library_broadcast_path = ctx
        .path()
        .join("broadcast")
        .join("DeployStringUtilsV2.s.sol")
        .join("31337")
        .join("run-latest.json");
    assert!(
        library_broadcast_path.exists(),
        "library step should write a broadcast file, stderr:\n{stderr}"
    );
    let deployments = read_deployments(ctx.path(), "default", 31337);
    assert!(
        !deployment_address(&deployments, "StringUtilsV2").is_empty(),
        "library step should register the explicit library deployment"
    );
    assert_eq!(
        deployment_entry(&deployments, "StringUtilsV2")["type"].as_str(),
        Some("LIBRARY"),
        "library step should be classified as a library deployment for later linker reuse"
    );

    let consumer_broadcast_path = ctx
        .path()
        .join("broadcast")
        .join("DeployMessageStorageV08.s.sol")
        .join("31337")
        .join("run-latest.json");
    assert!(
        consumer_broadcast_path.exists(),
        "consumer step should write a broadcast file, stderr:\n{stderr}"
    );

    let consumer_broadcast = read_json(&consumer_broadcast_path);
    let consumer_txs = consumer_broadcast["transactions"].as_array().unwrap_or_else(|| {
        panic!("missing transactions array in {}", consumer_broadcast_path.display())
    });
    assert_eq!(
        consumer_txs.len(),
        1,
        "consumer step should deploy directly once the prior library address is passed via --libraries"
    );
    assert_ne!(
        consumer_txs[0]["transaction"]["to"].as_str(),
        Some("0x4e59b44847b379578588920ca78fbf26c0b4956c"),
        "consumer step should not predeploy the library through the CREATE2 deployer"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn compose_artifact_deploy_via_vm_get_code_uses_linked_library_from_previous_step() {
    let _permit = compose_test_lock().acquire_owned().await.unwrap();
    let Some(ctx) = compose_library_test_context().await else {
        return;
    };

    install_artifact_library_fixture(&ctx);
    ctx.run(["init"]).success();
    let rpc_url = ctx.anvil("anvil-31337").unwrap().rpc_url().to_string();

    let compose = ctx.run([
        "compose",
        "library-link-artifact.yaml",
        "--network",
        "anvil-31337",
        "--rpc-url",
        rpc_url.as_str(),
        "--broadcast",
        "--non-interactive",
    ]);
    let stderr = String::from_utf8_lossy(&compose.get_output().stderr).to_string();
    compose.success();

    let consumer_broadcast_path = ctx
        .path()
        .join("broadcast")
        .join("DeployMessageStorageArtifact.s.sol")
        .join("31337")
        .join("run-latest.json");
    assert!(
        consumer_broadcast_path.exists(),
        "artifact consumer step should write a broadcast file, stderr:\n{stderr}"
    );

    let deployments = read_deployments(ctx.path(), "default", 31337);
    assert!(
        !deployment_address(&deployments, "MessageStorageV08").is_empty(),
        "artifact consumer step should still register the consumer deployment"
    );

    let consumer_broadcast = read_json(&consumer_broadcast_path);
    let consumer_txs = consumer_broadcast["transactions"].as_array().unwrap_or_else(|| {
        panic!("missing transactions array in {}", consumer_broadcast_path.display())
    });
    assert_eq!(
        consumer_txs.len(),
        1,
        "artifact-path deploy should succeed directly once the prior library address is linked"
    );
    assert_ne!(
        consumer_txs[0]["transaction"]["to"].as_str(),
        Some("0x4e59b44847b379578588920ca78fbf26c0b4956c"),
        "artifact-path deploy should not predeploy the library through the CREATE2 deployer"
    );
}
