//! E2E tests: fork exec receipt processing for Safe multisig and Governor proposals.
//!
//! All tests fork Celo Sepolia where a real Safe (2-of-3) and several ERC-1967
//! proxies are deployed. The fork exec command simulates queued transactions on
//! Anvil and the receipt processing detects proxy upgrades, new creations, and
//! updates the registry accordingly.
//!
//! ## On-chain fixtures (Celo Sepolia, chain ID 11142220)
//!
//! | Contract       | Address                                      | Admin/Owner           |
//! |----------------|----------------------------------------------|-----------------------|
//! | Safe (2-of-3)  | 0x4f0407700215b8f8875abfbcedd5b27b988af136   | Anvil accounts 0-2   |
//! | BoxV1          | 0x1b2B5490F192228d775295cFfeE9a71aE14dbd81   | —                     |
//! | BoxV2          | 0xc936b27D09CC71Adf1092767f8bda7ECc0F8FDAC   | —                     |
//! | Proxy          | 0x24b9Ac3f2Fa593fee972F5Cd184d2681FA05BDE2   | admin=Safe, val=42    |
//! | GovProxy       | 0xfa9633243ef18C4eF60E5Bb075e31fe783dDF8b0   | admin=Timelock, val=99|
//! | Proxy3         | 0x974181b85135d0E20f8fE9056446142cAf499810   | admin=Safe, val=77    |
//!
//! Anvil deterministic accounts used as identities:
//! - #0 0xf39F...2266  Safe owner
//! - #1 0x7099...79C8  Safe owner
//! - #2 0x3C44...93BC  Safe owner
//! - #3 0x90F7...b906  "Timelock" (admin of GovProxy)
//! - #4 0x15d3...6A65  "Governor"

mod e2e;

use std::{collections::HashMap, fs};

use chrono::Utc;
use e2e::treb;
use treb_core::types::{
    ArtifactInfo, Deployment, DeploymentMethod, DeploymentStrategy, DeploymentType, GovernorAction,
    GovernorProposal, ProposalStatus, ProxyInfo, Transaction, TransactionStatus, VerificationInfo,
    VerificationStatus,
    safe_transaction::{Confirmation, SafeTransaction, SafeTxData},
};
use treb_registry::Registry;

// ── Constants ───────────────────────────────────────────────────────────

const CHAIN_ID: u64 = 11142220; // Celo Sepolia
const CELO_SEPOLIA_RPC: &str = "https://forno.celo-sepolia.celo-testnet.org";

const SAFE_ADDRESS: &str = "0x4f0407700215b8f8875abfbcedd5b27b988af136";
const BOX_V1: &str = "0x1b2B5490F192228d775295cFfeE9a71aE14dbd81";
const BOX_V2: &str = "0xc936b27D09CC71Adf1092767f8bda7ECc0F8FDAC";
const PROXY_ADDRESS: &str = "0x24b9Ac3f2Fa593fee972F5Cd184d2681FA05BDE2";
const GOV_PROXY_ADDRESS: &str = "0xfa9633243ef18C4eF60E5Bb075e31fe783dDF8b0";
const PROXY3_ADDRESS: &str = "0x974181b85135d0E20f8fE9056446142cAf499810";
const TIMELOCK_ADDRESS: &str = "0x90F79bf6EB2c4f870365E785982E1f101E93b906";
const GOVERNOR_ADDRESS: &str = "0x15d34AAf54267DB7D7c367839AAf71A00a2C6A65";

// ── Shared helpers ──────────────────────────────────────────────────────

async fn spawn_celo_sepolia_fork() -> Option<treb_forge::AnvilInstance> {
    match treb_forge::anvil::AnvilConfig::new().port(0).fork_url(CELO_SEPOLIA_RPC).spawn().await {
        Ok(anvil) => Some(anvil),
        Err(err) => {
            let msg = err.to_string();
            if msg.contains("Operation not permitted") || msg.contains("Connection refused") {
                eprintln!("skipping: anvil fork unavailable ({msg})");
                None
            } else {
                panic!("failed to spawn Anvil fork of Celo Sepolia: {err}");
            }
        }
    }
}

/// Encode `upgradeTo(address)` calldata.
fn upgrade_to_calldata(new_impl: &str) -> String {
    let addr = new_impl.strip_prefix("0x").unwrap_or(new_impl);
    format!("0x3659cfe6000000000000000000000000{}", &addr.to_lowercase())
}

/// Encode `setValue(uint256)` calldata.
fn set_value_calldata(value: u64) -> String {
    format!("0x55241077{:064x}", value)
}

fn setup_minimal_project(tmp: &std::path::Path) {
    fs::write(
        tmp.join("foundry.toml"),
        format!(
            "[profile.default]\nsrc = \"src\"\n\n[rpc_endpoints]\ncelo-sepolia = \"{CELO_SEPOLIA_RPC}\"\n"
        ),
    )
    .unwrap();
    fs::create_dir_all(tmp.join(".treb")).unwrap();
}

fn write_fork_state(tmp: &std::path::Path, rpc_url: &str, port: u16) {
    let now = Utc::now().to_rfc3339();
    let fork_state = serde_json::json!({
        "forks": {
            "celo-sepolia": {
                "network": "celo-sepolia",
                "rpcUrl": rpc_url,
                "port": port,
                "chainId": CHAIN_ID,
                "forkUrl": CELO_SEPOLIA_RPC,
                "snapshotDir": "",
                "startedAt": &now,
                "envVarName": "",
                "originalRpc": "",
                "anvilPid": 0,
                "pidFile": "",
                "logFile": "",
                "enteredAt": &now,
            }
        },
        "history": []
    });
    fs::write(
        tmp.join(".treb").join("fork.json"),
        serde_json::to_string_pretty(&fork_state).unwrap(),
    )
    .unwrap();
}

fn make_deployment(id: &str, address: &str, name: &str, admin: &str) -> Deployment {
    Deployment {
        id: id.to_string(),
        namespace: "default".to_string(),
        chain_id: CHAIN_ID,
        contract_name: name.to_string(),
        label: String::new(),
        address: address.to_string(),
        deployment_type: DeploymentType::Proxy,
        execution: None,
        transaction_id: format!("tx-deploy-{}", name.to_lowercase()),
        deployment_strategy: DeploymentStrategy {
            method: DeploymentMethod::Create,
            salt: String::new(),
            init_code_hash: String::new(),
            factory: String::new(),
            constructor_args: String::new(),
            entropy: String::new(),
        },
        proxy_info: Some(ProxyInfo {
            proxy_type: "transparent".to_string(),
            implementation: BOX_V1.to_string(),
            admin: admin.to_string(),
            history: vec![],
        }),
        artifact: ArtifactInfo {
            path: String::new(),
            compiler_version: String::new(),
            bytecode_hash: String::new(),
            script_path: String::new(),
            git_commit: String::new(),
        },
        verification: VerificationInfo {
            status: VerificationStatus::Unverified,
            etherscan_url: String::new(),
            verified_at: None,
            reason: String::new(),
            verifiers: HashMap::new(),
        },
        tags: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn make_transaction(id: &str, deployments: Vec<String>) -> Transaction {
    Transaction {
        id: id.to_string(),
        chain_id: CHAIN_ID,
        hash: String::new(),
        status: TransactionStatus::Executed,
        block_number: 0,
        sender: String::new(),
        nonce: 0,
        deployments,
        operations: vec![],
        safe_context: None,
        broadcast_file: None,
        environment: "default".to_string(),
        created_at: Utc::now(),
    }
}

fn make_safe_tx(hash: &str, operations: Vec<SafeTxData>) -> SafeTransaction {
    SafeTransaction {
        safe_tx_hash: hash.to_string(),
        safe_address: SAFE_ADDRESS.to_string(),
        chain_id: CHAIN_ID,
        status: TransactionStatus::Queued,
        nonce: 0,
        transactions: operations,
        transaction_ids: vec![],
        proposed_by: "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".to_string(),
        proposed_at: Utc::now(),
        confirmations: vec![Confirmation {
            signer: "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".to_string(),
            signature: "0x".to_string(),
            confirmed_at: Utc::now(),
        }],
        executed_at: None,
        execution_tx_hash: String::new(),
        fork_executed_at: None,
    }
}

fn make_governor_proposal(id: &str, actions: Vec<GovernorAction>) -> GovernorProposal {
    GovernorProposal {
        proposal_id: id.to_string(),
        governor_address: GOVERNOR_ADDRESS.to_string(),
        timelock_address: TIMELOCK_ADDRESS.to_string(),
        chain_id: CHAIN_ID,
        status: ProposalStatus::Queued,
        transaction_ids: vec![],
        proposed_by: "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".to_string(),
        proposed_at: Utc::now(),
        description: String::new(),
        actions,
        executed_at: None,
        execution_tx_hash: String::new(),
        fork_executed_at: None,
    }
}

fn read_deployment(tmp: &std::path::Path, dep_id: &str) -> serde_json::Value {
    let content =
        fs::read_to_string(tmp.join(".treb").join("deployments.json")).expect("read deployments");
    let map: serde_json::Value = serde_json::from_str(&content).expect("parse deployments");
    map.get(dep_id).cloned().unwrap_or_else(|| panic!("deployment {dep_id} not found"))
}

fn read_safe_tx(tmp: &std::path::Path, hash: &str) -> serde_json::Value {
    let content =
        fs::read_to_string(tmp.join(".treb").join("safe-txs.json")).expect("read safe-txs");
    let map: serde_json::Value = serde_json::from_str(&content).expect("parse safe-txs");
    map.get(hash).cloned().unwrap_or_else(|| panic!("safe tx {hash} not found"))
}

fn read_governor_proposal(tmp: &std::path::Path, id: &str) -> serde_json::Value {
    let content =
        fs::read_to_string(tmp.join(".treb").join("governor-txs.json")).expect("read governor-txs");
    let map: serde_json::Value = serde_json::from_str(&content).expect("parse governor-txs");
    map.get(id).cloned().unwrap_or_else(|| panic!("governor proposal {id} not found"))
}

/// Run `treb fork exec --all` and return (success, stdout, stderr).
async fn run_fork_exec(tmp: &std::path::Path) -> (bool, String, String) {
    let tmp_path = tmp.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let output = treb()
            .args(["fork", "exec", "--all"])
            .current_dir(&tmp_path)
            .output()
            .expect("spawn treb fork exec");
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        (output.status.success(), stdout, stderr)
    })
    .await
    .unwrap()
}

fn cast_call(rpc_url: &str, to: &str, sig: &str) -> String {
    let output = std::process::Command::new("cast")
        .args(["call", to, sig, "--rpc-url", rpc_url])
        .output()
        .expect("cast call");
    String::from_utf8_lossy(&output.stdout).trim().trim_matches('"').to_string()
}

// ── Test 1: Single Safe upgrade ─────────────────────────────────────────

/// Safe multisig executes upgradeTo(BoxV2) on a proxy.
/// Receipt processing detects the Upgraded event and updates the registry.
#[tokio::test(flavor = "multi_thread")]
async fn fork_exec_safe_upgrades_proxy() {
    let Some(anvil) = spawn_celo_sepolia_fork().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    let tmp = tempfile::tempdir().unwrap();
    setup_minimal_project(tmp.path());
    write_fork_state(tmp.path(), &rpc_url, anvil.port());

    let dep_id = "default/11142220/UpgradeableBox/";
    let mut registry = Registry::open(tmp.path()).unwrap();
    registry
        .insert_deployment(make_deployment(dep_id, PROXY_ADDRESS, "UpgradeableBox", SAFE_ADDRESS))
        .unwrap();
    registry
        .insert_transaction(make_transaction("tx-deploy-upgradeablebox", vec![dep_id.to_string()]))
        .unwrap();
    registry
        .insert_safe_transaction(make_safe_tx(
            "0xsafe_upgrade_single",
            vec![SafeTxData {
                to: PROXY_ADDRESS.to_string(),
                value: "0".to_string(),
                data: upgrade_to_calldata(BOX_V2),
                operation: 0,
            }],
        ))
        .unwrap();
    drop(registry);

    let (success, stdout, stderr) = run_fork_exec(tmp.path()).await;
    eprintln!("stdout: {stdout}\nstderr: {stderr}");
    assert!(success, "fork exec failed");
    assert!(stderr.contains("upgraded"), "should report upgrade in stderr");

    // Registry updated
    let dep = read_deployment(tmp.path(), dep_id);
    assert!(
        dep["proxyInfo"]["implementation"].as_str().unwrap().eq_ignore_ascii_case(BOX_V2),
        "impl should be BoxV2"
    );
    assert!(
        !dep["proxyInfo"]["history"].as_array().unwrap().is_empty(),
        "history should have entry"
    );

    // On-chain verification
    assert_eq!(cast_call(&rpc_url, PROXY_ADDRESS, "version()(string)"), "v2");
}

// ── Test 2: Multi-operation Safe batch ──────────────────────────────────

/// Safe multisig batch: upgradeTo(BoxV2) + setValue(100) in one transaction.
/// Both operations execute atomically. Receipt detects the upgrade.
#[tokio::test(flavor = "multi_thread")]
async fn fork_exec_safe_multi_op_batch() {
    let Some(anvil) = spawn_celo_sepolia_fork().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    let tmp = tempfile::tempdir().unwrap();
    setup_minimal_project(tmp.path());
    write_fork_state(tmp.path(), &rpc_url, anvil.port());

    let dep_id = "default/11142220/UpgradeableBox/";
    let mut registry = Registry::open(tmp.path()).unwrap();
    registry
        .insert_deployment(make_deployment(dep_id, PROXY_ADDRESS, "UpgradeableBox", SAFE_ADDRESS))
        .unwrap();
    registry
        .insert_transaction(make_transaction("tx-deploy-upgradeablebox", vec![dep_id.to_string()]))
        .unwrap();
    // Two operations in one Safe tx: upgrade then set value
    registry
        .insert_safe_transaction(make_safe_tx(
            "0xsafe_multi_op",
            vec![
                SafeTxData {
                    to: PROXY_ADDRESS.to_string(),
                    value: "0".to_string(),
                    data: upgrade_to_calldata(BOX_V2),
                    operation: 0,
                },
                SafeTxData {
                    to: PROXY_ADDRESS.to_string(),
                    value: "0".to_string(),
                    data: set_value_calldata(100),
                    operation: 0,
                },
            ],
        ))
        .unwrap();
    drop(registry);

    let (success, stdout, stderr) = run_fork_exec(tmp.path()).await;
    eprintln!("stdout: {stdout}\nstderr: {stderr}");
    assert!(success, "fork exec failed");

    // Proxy upgraded
    let dep = read_deployment(tmp.path(), dep_id);
    assert!(
        dep["proxyInfo"]["implementation"].as_str().unwrap().eq_ignore_ascii_case(BOX_V2),
        "impl should be BoxV2"
    );

    // On-chain: version=v2, value=100
    assert_eq!(cast_call(&rpc_url, PROXY_ADDRESS, "version()(string)"), "v2");
    assert_eq!(cast_call(&rpc_url, PROXY_ADDRESS, "value()(uint256)"), "100");
}

// ── Test 3: Governor proposal upgrades proxy ────────────────────────────

/// Governor proposal executes upgradeTo(BoxV2) on GovProxy (admin=Timelock).
/// Simplified simulation impersonates the timelock and sends the action directly.
#[tokio::test(flavor = "multi_thread")]
async fn fork_exec_governor_upgrades_proxy() {
    let Some(anvil) = spawn_celo_sepolia_fork().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    let tmp = tempfile::tempdir().unwrap();
    setup_minimal_project(tmp.path());
    write_fork_state(tmp.path(), &rpc_url, anvil.port());

    let dep_id = "default/11142220/GovBox/";
    let mut registry = Registry::open(tmp.path()).unwrap();
    registry
        .insert_deployment(make_deployment(dep_id, GOV_PROXY_ADDRESS, "GovBox", TIMELOCK_ADDRESS))
        .unwrap();
    registry
        .insert_transaction(make_transaction("tx-deploy-govbox", vec![dep_id.to_string()]))
        .unwrap();
    registry
        .insert_governor_proposal(make_governor_proposal(
            "0xgov_proposal_upgrade",
            vec![GovernorAction {
                target: GOV_PROXY_ADDRESS.to_string(),
                value: "0".to_string(),
                calldata: upgrade_to_calldata(BOX_V2),
            }],
        ))
        .unwrap();
    drop(registry);

    let (success, stdout, stderr) = run_fork_exec(tmp.path()).await;
    eprintln!("stdout: {stdout}\nstderr: {stderr}");
    assert!(success, "fork exec failed");
    assert!(stderr.contains("simulated proposal"), "should report proposal simulation");

    // Governor proposal marked as fork-executed
    let proposal = read_governor_proposal(tmp.path(), "0xgov_proposal_upgrade");
    assert!(proposal.get("forkExecutedAt").is_some(), "should have forkExecutedAt");

    // Registry updated
    let dep = read_deployment(tmp.path(), dep_id);
    assert!(
        dep["proxyInfo"]["implementation"].as_str().unwrap().eq_ignore_ascii_case(BOX_V2),
        "impl should be BoxV2"
    );

    // On-chain
    assert_eq!(cast_call(&rpc_url, GOV_PROXY_ADDRESS, "version()(string)"), "v2");
}

// ── Test 4: Mixed Safe + Governor in one exec ───────────────────────────

/// Both a Safe tx and a Governor proposal queued. Fork exec processes both.
/// Each upgrades a different proxy.
#[tokio::test(flavor = "multi_thread")]
async fn fork_exec_mixed_safe_and_governor() {
    let Some(anvil) = spawn_celo_sepolia_fork().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    let tmp = tempfile::tempdir().unwrap();
    setup_minimal_project(tmp.path());
    write_fork_state(tmp.path(), &rpc_url, anvil.port());

    let safe_dep_id = "default/11142220/SafeBox/";
    let gov_dep_id = "default/11142220/GovBox/";

    let mut registry = Registry::open(tmp.path()).unwrap();
    registry
        .insert_deployment(make_deployment(safe_dep_id, PROXY_ADDRESS, "SafeBox", SAFE_ADDRESS))
        .unwrap();
    registry
        .insert_transaction(make_transaction("tx-deploy-safebox", vec![safe_dep_id.to_string()]))
        .unwrap();
    registry
        .insert_deployment(make_deployment(
            gov_dep_id,
            GOV_PROXY_ADDRESS,
            "GovBox",
            TIMELOCK_ADDRESS,
        ))
        .unwrap();
    registry
        .insert_transaction(make_transaction("tx-deploy-govbox", vec![gov_dep_id.to_string()]))
        .unwrap();

    // Safe tx upgrades PROXY
    registry
        .insert_safe_transaction(make_safe_tx(
            "0xsafe_mixed",
            vec![SafeTxData {
                to: PROXY_ADDRESS.to_string(),
                value: "0".to_string(),
                data: upgrade_to_calldata(BOX_V2),
                operation: 0,
            }],
        ))
        .unwrap();

    // Governor proposal upgrades GOV_PROXY
    registry
        .insert_governor_proposal(make_governor_proposal(
            "0xgov_mixed",
            vec![GovernorAction {
                target: GOV_PROXY_ADDRESS.to_string(),
                value: "0".to_string(),
                calldata: upgrade_to_calldata(BOX_V2),
            }],
        ))
        .unwrap();
    drop(registry);

    let (success, stdout, stderr) = run_fork_exec(tmp.path()).await;
    eprintln!("stdout: {stdout}\nstderr: {stderr}");
    assert!(success, "fork exec failed");
    assert!(stdout.contains("Executed 2"), "should execute 2 items");

    // Both proxies upgraded
    let safe_dep = read_deployment(tmp.path(), safe_dep_id);
    assert!(
        safe_dep["proxyInfo"]["implementation"].as_str().unwrap().eq_ignore_ascii_case(BOX_V2),
        "Safe proxy impl should be BoxV2"
    );
    let gov_dep = read_deployment(tmp.path(), gov_dep_id);
    assert!(
        gov_dep["proxyInfo"]["implementation"].as_str().unwrap().eq_ignore_ascii_case(BOX_V2),
        "Gov proxy impl should be BoxV2"
    );

    // On-chain
    assert_eq!(cast_call(&rpc_url, PROXY_ADDRESS, "version()(string)"), "v2");
    assert_eq!(cast_call(&rpc_url, GOV_PROXY_ADDRESS, "version()(string)"), "v2");
}

// ── Test 5: Multiple Safe txs ───────────────────────────────────────────

/// Two separate queued Safe transactions, each upgrading a different proxy.
/// Both execute in one `fork exec --all` invocation.
#[tokio::test(flavor = "multi_thread")]
async fn fork_exec_multiple_safe_txs() {
    let Some(anvil) = spawn_celo_sepolia_fork().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    let tmp = tempfile::tempdir().unwrap();
    setup_minimal_project(tmp.path());
    write_fork_state(tmp.path(), &rpc_url, anvil.port());

    let dep1_id = "default/11142220/Proxy1/";
    let dep2_id = "default/11142220/Proxy3/";

    let mut registry = Registry::open(tmp.path()).unwrap();
    registry
        .insert_deployment(make_deployment(dep1_id, PROXY_ADDRESS, "Proxy1", SAFE_ADDRESS))
        .unwrap();
    registry
        .insert_transaction(make_transaction("tx-deploy-proxy1", vec![dep1_id.to_string()]))
        .unwrap();
    registry
        .insert_deployment(make_deployment(dep2_id, PROXY3_ADDRESS, "Proxy3", SAFE_ADDRESS))
        .unwrap();
    registry
        .insert_transaction(make_transaction("tx-deploy-proxy3", vec![dep2_id.to_string()]))
        .unwrap();

    // First Safe tx: upgrade PROXY
    registry
        .insert_safe_transaction(make_safe_tx(
            "0xsafe_multi_1",
            vec![SafeTxData {
                to: PROXY_ADDRESS.to_string(),
                value: "0".to_string(),
                data: upgrade_to_calldata(BOX_V2),
                operation: 0,
            }],
        ))
        .unwrap();
    // Second Safe tx: upgrade PROXY3
    let mut safe_tx2 = make_safe_tx(
        "0xsafe_multi_2",
        vec![SafeTxData {
            to: PROXY3_ADDRESS.to_string(),
            value: "0".to_string(),
            data: upgrade_to_calldata(BOX_V2),
            operation: 0,
        }],
    );
    // Safe nonce must be different (the first tx consumed nonce 0)
    safe_tx2.nonce = 1;
    registry.insert_safe_transaction(safe_tx2).unwrap();
    drop(registry);

    let (success, stdout, stderr) = run_fork_exec(tmp.path()).await;
    eprintln!("stdout: {stdout}\nstderr: {stderr}");
    assert!(success, "fork exec failed");
    assert!(stdout.contains("Executed 2"), "should execute 2 items");

    // Both proxies upgraded
    let dep1 = read_deployment(tmp.path(), dep1_id);
    assert!(
        dep1["proxyInfo"]["implementation"].as_str().unwrap().eq_ignore_ascii_case(BOX_V2),
        "Proxy1 impl should be BoxV2"
    );
    let dep2 = read_deployment(tmp.path(), dep2_id);
    assert!(
        dep2["proxyInfo"]["implementation"].as_str().unwrap().eq_ignore_ascii_case(BOX_V2),
        "Proxy3 impl should be BoxV2"
    );

    assert_eq!(cast_call(&rpc_url, PROXY_ADDRESS, "version()(string)"), "v2");
    assert_eq!(cast_call(&rpc_url, PROXY3_ADDRESS, "version()(string)"), "v2");
}

// ── Test 6: Already fork-executed items are skipped ─────────────────────

/// Safe tx with `fork_executed_at` already set is skipped by fork exec.
#[tokio::test(flavor = "multi_thread")]
async fn fork_exec_skips_already_executed() {
    let Some(anvil) = spawn_celo_sepolia_fork().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    let tmp = tempfile::tempdir().unwrap();
    setup_minimal_project(tmp.path());
    write_fork_state(tmp.path(), &rpc_url, anvil.port());

    let mut registry = Registry::open(tmp.path()).unwrap();
    let mut stx = make_safe_tx(
        "0xsafe_already_done",
        vec![SafeTxData {
            to: PROXY_ADDRESS.to_string(),
            value: "0".to_string(),
            data: upgrade_to_calldata(BOX_V2),
            operation: 0,
        }],
    );
    stx.fork_executed_at = Some(Utc::now()); // Already done
    registry.insert_safe_transaction(stx).unwrap();
    drop(registry);

    let (success, stdout, _stderr) = run_fork_exec(tmp.path()).await;
    assert!(success);
    assert!(stdout.contains("Executed 0") || stdout.contains("No queued items"));

    // Proxy NOT upgraded (still v1)
    assert_eq!(cast_call(&rpc_url, PROXY_ADDRESS, "version()(string)"), "v1");
}

// ── Test 7: Governor multi-action proposal ──────────────────────────────

/// Governor proposal with two actions: upgrade proxy + set value.
#[tokio::test(flavor = "multi_thread")]
async fn fork_exec_governor_multi_action() {
    let Some(anvil) = spawn_celo_sepolia_fork().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    let tmp = tempfile::tempdir().unwrap();
    setup_minimal_project(tmp.path());
    write_fork_state(tmp.path(), &rpc_url, anvil.port());

    let dep_id = "default/11142220/GovBox/";
    let mut registry = Registry::open(tmp.path()).unwrap();
    registry
        .insert_deployment(make_deployment(dep_id, GOV_PROXY_ADDRESS, "GovBox", TIMELOCK_ADDRESS))
        .unwrap();
    registry
        .insert_transaction(make_transaction("tx-deploy-govbox", vec![dep_id.to_string()]))
        .unwrap();
    registry
        .insert_governor_proposal(make_governor_proposal(
            "0xgov_multi_action",
            vec![
                GovernorAction {
                    target: GOV_PROXY_ADDRESS.to_string(),
                    value: "0".to_string(),
                    calldata: upgrade_to_calldata(BOX_V2),
                },
                GovernorAction {
                    target: GOV_PROXY_ADDRESS.to_string(),
                    value: "0".to_string(),
                    calldata: set_value_calldata(200),
                },
            ],
        ))
        .unwrap();
    drop(registry);

    let (success, stdout, stderr) = run_fork_exec(tmp.path()).await;
    eprintln!("stdout: {stdout}\nstderr: {stderr}");
    assert!(success, "fork exec failed");

    // Proxy upgraded and value set
    let dep = read_deployment(tmp.path(), dep_id);
    assert!(
        dep["proxyInfo"]["implementation"].as_str().unwrap().eq_ignore_ascii_case(BOX_V2),
        "impl should be BoxV2"
    );

    assert_eq!(cast_call(&rpc_url, GOV_PROXY_ADDRESS, "version()(string)"), "v2");
    assert_eq!(cast_call(&rpc_url, GOV_PROXY_ADDRESS, "value()(uint256)"), "200");
}

// ── Test 8: sync --tx-hash after fork exec ──────────────────────────────

/// After fork exec upgrades a proxy, `sync --tx-hash` on the Anvil fork
/// also detects the upgrade from the same receipt.
#[tokio::test(flavor = "multi_thread")]
async fn sync_tx_hash_detects_upgrade_on_fork() {
    let Some(anvil) = spawn_celo_sepolia_fork().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    let tmp = tempfile::tempdir().unwrap();
    setup_minimal_project(tmp.path());
    write_fork_state(tmp.path(), &rpc_url, anvil.port());

    let dep_id = "default/11142220/SyncBox/";
    let mut registry = Registry::open(tmp.path()).unwrap();
    registry
        .insert_deployment(make_deployment(dep_id, PROXY_ADDRESS, "SyncBox", SAFE_ADDRESS))
        .unwrap();
    registry
        .insert_transaction(make_transaction("tx-deploy-syncbox", vec![dep_id.to_string()]))
        .unwrap();
    registry
        .insert_safe_transaction(make_safe_tx(
            "0xsafe_for_sync",
            vec![SafeTxData {
                to: PROXY_ADDRESS.to_string(),
                value: "0".to_string(),
                data: upgrade_to_calldata(BOX_V2),
                operation: 0,
            }],
        ))
        .unwrap();
    drop(registry);

    // First: fork exec to get a real tx hash on the fork
    let (success, _, _) = run_fork_exec(tmp.path()).await;
    assert!(success, "fork exec failed");

    // Read the execution tx hash from the receipt
    let stx = read_safe_tx(tmp.path(), "0xsafe_for_sync");
    assert!(stx.get("forkExecutedAt").is_some());

    // Now reset the deployment impl back to BoxV1 (simulating a fresh sync scenario)
    let mut registry = Registry::open(tmp.path()).unwrap();
    if let Some(dep) = registry.get_deployment(dep_id).cloned() {
        let mut updated = dep;
        if let Some(ref mut pi) = updated.proxy_info {
            pi.implementation = BOX_V1.to_string();
            pi.history.clear();
        }
        registry.update_deployment(updated).unwrap();
    }
    drop(registry);

    // Verify reset
    let dep_before = read_deployment(tmp.path(), dep_id);
    assert!(
        dep_before["proxyInfo"]["implementation"].as_str().unwrap().eq_ignore_ascii_case(BOX_V1),
        "impl should be reset to BoxV1"
    );

    // Get the actual tx hash that was executed on the fork. We need to find it from Anvil.
    // The easiest approach: query the latest block for the tx hash.
    let latest_block = std::process::Command::new("cast")
        .args(["block", "latest", "--json", "--rpc-url", &rpc_url])
        .output()
        .expect("cast block");
    let block_json: serde_json::Value =
        serde_json::from_slice(&latest_block.stdout).expect("parse block");
    let tx_hashes = block_json["transactions"].as_array().expect("transactions array");

    // Use the last transaction hash (the Safe execution)
    if let Some(last_tx) = tx_hashes.last() {
        let tx_hash = last_tx.as_str().expect("tx hash string");

        // Run sync --tx-hash
        let tmp_path = tmp.path().to_path_buf();
        let rpc = rpc_url.clone();
        let hash = tx_hash.to_string();
        let sync_output = tokio::task::spawn_blocking(move || {
            let output = treb()
                .args(["registry", "sync", "--tx-hash", &hash, "--rpc-url", &rpc])
                .current_dir(&tmp_path)
                .output()
                .expect("spawn treb sync");
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            (output.status.success(), stdout, stderr)
        })
        .await
        .unwrap();

        let (sync_ok, sync_stdout, sync_stderr) = sync_output;
        eprintln!("sync stdout: {sync_stdout}\nsync stderr: {sync_stderr}");
        assert!(sync_ok, "sync --tx-hash should succeed");

        // Verify: proxy impl updated back to BoxV2 by sync
        let dep_after = read_deployment(tmp.path(), dep_id);
        assert!(
            dep_after["proxyInfo"]["implementation"].as_str().unwrap().eq_ignore_ascii_case(BOX_V2),
            "sync should have detected upgrade to BoxV2"
        );
    }
}
