//! Live integration coverage for `treb compose --resume`.
//!
//! The flaky component uses two wallet senders:
//! - tx 1 broadcasts from a funded Anvil account and succeeds
//! - tx 2 broadcasts from an unfunded private key and fails with insufficient funds
//!
//! This gives us a deterministic partial broadcast checkpoint that can be
//! resumed after funding the second sender.

mod framework;

use std::{fs, path::Path, str::FromStr};

use alloy_primitives::{Address, U256};
use framework::context::TestContext;

const RESUMER_PRIVATE_KEY: &str =
    "0x1111111111111111111111111111111111111111111111111111111111111111";
const RESUMER_ADDRESS: &str = "0x19E7E376E7C213B7E7e7e46cc70A5dD086DAff2A";

async fn compose_resume_test_context() -> Option<TestContext> {
    match TestContext::new("project").with_anvil("anvil-31337").await {
        Ok(ctx) => Some(ctx),
        Err(err) if err.to_string().contains("Operation not permitted") => None,
        Err(err) => panic!("failed to spawn anvil: {err}"),
    }
}

fn install_resume_fixture(ctx: &TestContext) {
    write_resume_treb_toml(ctx.path());
    write_seed_script(ctx.path());
    write_flaky_script(ctx.path());
    write_compose_file(ctx.path());
}

fn write_resume_treb_toml(project_dir: &Path) {
    let toml = format!(
        r#"[accounts.deployer]
type = "private_key"
private_key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"

[accounts.resumer]
type = "private_key"
private_key = "{RESUMER_PRIVATE_KEY}"

[namespace.default]
profile = "default"

[namespace.default.senders]
deployer = "deployer"
resumer = "resumer"
"#,
    );
    fs::write(project_dir.join("treb.toml"), toml).unwrap();
}

fn write_seed_script(project_dir: &Path) {
    fs::write(
        project_dir.join("script").join("SeedResume.s.sol"),
        r#"// SPDX-License-Identifier: MIT
pragma solidity =0.8.30;

import "forge-std/Script.sol";

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

contract SeedResumeScript is Script {
    event TransactionSimulated(SimTx[] transactions);

    function run() public {
        address target = address(0x2001);

        vm.startBroadcast();
        (bool ok, bytes memory data) = target.call("");
        require(ok, "seed call failed");

        SimTx[] memory txs = new SimTx[](1);
        txs[0] = SimTx({
            transactionId: keccak256("seed"),
            senderId: "deployer",
            sender: msg.sender,
            returnData: data,
            transaction: TxDetails({to: target, data: bytes(""), value: 0})
        });
        emit TransactionSimulated(txs);
        vm.stopBroadcast();
    }
}
"#,
    )
    .unwrap();
}

fn write_flaky_script(project_dir: &Path) {
    fs::write(
        project_dir.join("script").join("FlakyResume.s.sol"),
        format!(
            r#"// SPDX-License-Identifier: MIT
pragma solidity =0.8.30;

import "forge-std/Script.sol";

struct TxDetails {{
    address to;
    bytes data;
    uint256 value;
}}

struct SimTx {{
    bytes32 transactionId;
    string senderId;
    address sender;
    bytes returnData;
    TxDetails transaction;
}}

contract FlakyResumeScript is Script {{
    event TransactionSimulated(SimTx[] transactions);

    address internal constant RESUMER = {RESUMER_ADDRESS};

    function run() public {{
        address firstTarget = address(0x2002);
        address secondTarget = address(0x2003);

        vm.startBroadcast();
        (bool ok1, bytes memory data1) = firstTarget.call("");
        require(ok1, "first call failed");

        SimTx[] memory first = new SimTx[](1);
        first[0] = SimTx({{
            transactionId: keccak256("flaky-first"),
            senderId: "deployer",
            sender: msg.sender,
            returnData: data1,
            transaction: TxDetails({{to: firstTarget, data: bytes(""), value: 0}})
        }});
        emit TransactionSimulated(first);
        vm.stopBroadcast();

        vm.startBroadcast(RESUMER);
        (bool ok2, bytes memory data2) = secondTarget.call("");
        require(ok2, "second call failed");

        SimTx[] memory second = new SimTx[](1);
        second[0] = SimTx({{
            transactionId: keccak256("flaky-second"),
            senderId: "resumer",
            sender: msg.sender,
            returnData: data2,
            transaction: TxDetails({{to: secondTarget, data: bytes(""), value: 0}})
        }});
        emit TransactionSimulated(second);
        vm.stopBroadcast();
    }}
}}
"#,
        ),
    )
    .unwrap();
}

fn write_compose_file(project_dir: &Path) {
    fs::write(
        project_dir.join("resume.yaml"),
        r#"group: resume-test
components:
  seed:
    script: script/SeedResume.s.sol
  flaky:
    script: script/FlakyResume.s.sol
    deps:
      - seed
"#,
    )
    .unwrap();
}

fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_str(
        &fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display())),
    )
    .unwrap_or_else(|e| panic!("invalid json in {}: {e}", path.display()))
}

fn read_registry_entries(project_root: &Path, filename: &str) -> serde_json::Value {
    let value = read_json(&project_root.join(".treb").join(filename));
    value
        .as_object()
        .and_then(|object| object.get("entries").filter(|_| object.contains_key("_format")))
        .cloned()
        .unwrap_or(value)
}

fn archived_compose_plans(plan_dir: &Path) -> Vec<std::path::PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(plan_dir)
        .unwrap()
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let file_name = path.file_name()?.to_string_lossy();
            if file_name.starts_with("compose-") && file_name != "compose-latest.json" {
                Some(path)
            } else {
                None
            }
        })
        .collect();
    paths.sort();
    paths
}

fn compose_component<'a>(plan: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    plan["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|component| component["name"].as_str() == Some(name))
        .unwrap_or_else(|| panic!("missing compose component {name}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn compose_resume_reuses_partial_wallet_checkpoint() {
    let Some(ctx) = compose_resume_test_context().await else {
        return;
    };

    install_resume_fixture(&ctx);
    ctx.run(["init"]).success();
    let rpc_url = ctx.anvil("anvil-31337").unwrap().rpc_url().to_string();

    let first_run = ctx.run([
        "compose",
        "resume.yaml",
        "--network",
        "anvil-31337",
        "--rpc-url",
        rpc_url.as_str(),
        "--broadcast",
        "--non-interactive",
    ]);
    let first_stderr = String::from_utf8_lossy(&first_run.get_output().stderr).to_string();
    first_run.failure();
    assert!(
        first_stderr.to_lowercase().contains("insufficient funds"),
        "expected insufficient funds failure, got:\n{first_stderr}"
    );

    let plan_path =
        ctx.path().join("broadcast").join("resume.yaml").join("31337").join("compose-latest.json");
    assert!(plan_path.exists(), "compose plan should be written after partial failure");

    let plan = read_json(&plan_path);
    assert_eq!(compose_component(&plan, "seed")["status"].as_str(), Some("broadcast"));
    assert_eq!(compose_component(&plan, "flaky")["status"].as_str(), Some("failed"));

    let first_registry_txs = read_registry_entries(ctx.path(), "transactions.json");
    assert_eq!(
        first_registry_txs.as_object().unwrap().len(),
        1,
        "only the completed seed component should be recorded before resume"
    );

    let seed_broadcast_path =
        ctx.path().join("broadcast").join("SeedResume.s.sol").join("31337").join("run-latest.json");
    let seed_broadcast = read_json(&seed_broadcast_path);
    let seed_hash = seed_broadcast["transactions"][0]["hash"].as_str().unwrap().to_string();

    let flaky_broadcast_path = ctx
        .path()
        .join("broadcast")
        .join("FlakyResume.s.sol")
        .join("31337")
        .join("run-latest.json");
    assert!(
        flaky_broadcast_path.exists(),
        "failed component should leave a resumable broadcast checkpoint"
    );

    let flaky_checkpoint = read_json(&flaky_broadcast_path);
    let flaky_txs = flaky_checkpoint["transactions"].as_array().unwrap();
    assert_eq!(flaky_txs.len(), 2, "flaky component should checkpoint both transactions");
    assert!(flaky_txs[0]["hash"].as_str().is_some(), "first tx hash should be checkpointed");
    assert!(flaky_txs[1]["hash"].is_null(), "second tx should remain unsent after failure");
    assert_eq!(
        flaky_checkpoint["receipts"].as_array().unwrap().len(),
        1,
        "checkpoint should contain the confirmed receipt for the first tx"
    );

    ctx.anvil("anvil-31337")
        .unwrap()
        .instance()
        .set_balance(
            Address::from_str(RESUMER_ADDRESS).unwrap(),
            U256::from(1_000_000_000_000_000_000u128),
        )
        .await
        .unwrap();

    let resumed = ctx.run([
        "compose",
        "resume.yaml",
        "--network",
        "anvil-31337",
        "--rpc-url",
        rpc_url.as_str(),
        "--broadcast",
        "--resume",
        "--non-interactive",
    ]);
    let resumed_stderr = String::from_utf8_lossy(&resumed.get_output().stderr).to_string();
    resumed.success();
    assert!(
        resumed_stderr.contains("Resuming compose from step 2 of 2"),
        "expected resume banner, got:\n{resumed_stderr}"
    );

    let resumed_plan = read_json(&plan_path);
    assert_eq!(compose_component(&resumed_plan, "seed")["status"].as_str(), Some("broadcast"));
    assert_eq!(compose_component(&resumed_plan, "flaky")["status"].as_str(), Some("broadcast"));
    let seed_archive_rel = compose_component(&resumed_plan, "seed")["broadcastFile"]
        .as_str()
        .expect("seed should reference an archived broadcast file");
    let flaky_archive_rel = compose_component(&resumed_plan, "flaky")["broadcastFile"]
        .as_str()
        .expect("flaky should reference an archived broadcast file");
    assert!(
        seed_archive_rel.contains("/run-") && !seed_archive_rel.ends_with("run-latest.json"),
        "seed should point to an immutable broadcast archive, got {seed_archive_rel}"
    );
    assert!(
        flaky_archive_rel.contains("/run-") && !flaky_archive_rel.ends_with("run-latest.json"),
        "flaky should point to an immutable broadcast archive, got {flaky_archive_rel}"
    );
    let seed_archive_path = ctx.path().join(seed_archive_rel);
    let flaky_archive_path = ctx.path().join(flaky_archive_rel);
    assert!(seed_archive_path.exists(), "seed archived broadcast file should exist");
    assert!(flaky_archive_path.exists(), "flaky archived broadcast file should exist");

    let seed_broadcast_after = read_json(&seed_broadcast_path);
    assert_eq!(
        seed_broadcast_after["transactions"][0]["hash"].as_str(),
        Some(seed_hash.as_str()),
        "resume should skip re-broadcasting the completed seed component"
    );
    let seed_archive = read_json(&seed_archive_path);
    assert_eq!(
        seed_archive["transactions"][0]["hash"].as_str(),
        Some(seed_hash.as_str()),
        "seed archived broadcast should preserve the original completed hash"
    );

    let flaky_broadcast_after = read_json(&flaky_broadcast_path);
    let flaky_after_txs = flaky_broadcast_after["transactions"].as_array().unwrap();
    assert!(flaky_after_txs[0]["hash"].as_str().is_some(), "first tx hash should still be present");
    assert!(flaky_after_txs[1]["hash"].as_str().is_some(), "resume should fill the second tx hash");
    assert_eq!(
        flaky_broadcast_after["receipts"].as_array().unwrap().len(),
        2,
        "resume should write both receipts after the component completes"
    );

    let final_registry_txs = read_registry_entries(ctx.path(), "transactions.json");
    assert_eq!(
        final_registry_txs.as_object().unwrap().len(),
        3,
        "resume should record the two flaky component txs without duplicating seed"
    );
    for tx in final_registry_txs.as_object().unwrap().values() {
        let broadcast_file = tx["broadcastFile"]
            .as_str()
            .expect("recorded transactions should carry a broadcast file");
        assert!(
            broadcast_file.contains("/run-") && !broadcast_file.ends_with("run-latest.json"),
            "registry transactions should point to immutable broadcast archives, got {broadcast_file}"
        );
    }

    let plan_dir = ctx.path().join("broadcast").join("resume.yaml").join("31337");
    let archived_plans = archived_compose_plans(&plan_dir);
    assert_eq!(archived_plans.len(), 1, "compose should emit one archived plan on success");
    let archived_plan = read_json(&archived_plans[0]);
    assert_eq!(
        compose_component(&archived_plan, "seed")["broadcastFile"].as_str(),
        Some(seed_archive_rel)
    );
    assert_eq!(
        compose_component(&archived_plan, "flaky")["broadcastFile"].as_str(),
        Some(flaky_archive_rel)
    );
}
