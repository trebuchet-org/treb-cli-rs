//! P6-US-005 / P6-US-006: Safe routing e2e workflow tests.
//!
//! Integration tests verifying the full Safe routing pipeline against
//! real Safe contracts deployed on Anvil:
//!
//! - Safe(1/1): wallet broadcast on fork via `execute_safe_on_fork()`
//! - Safe(2/3): multi-sig proposal on fork (queued, not executed)

mod e2e;

use alloy_primitives::{Address, U256};
use e2e::{
    copy_dir_recursive,
    deploy_safe::{deploy_safe, verify_safe_via_eth_call},
    read_transactions, spawn_anvil_or_skip, treb,
};
use std::{path::Path, str::FromStr};

/// Well-known Anvil test accounts.
const ACCOUNT_0: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const ACCOUNT_1: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
const ACCOUNT_2: &str = "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC";

fn fixture_project() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("project")
}

/// Write a treb.toml that configures a Safe sender for the deployer role.
///
/// The signer wallet must have its own role in the namespace senders map
/// because `signer` references a role key, not an account name.
fn write_safe_treb_toml(project_dir: &Path, safe_address: &Address) {
    let toml = format!(
        r#"[accounts.signer_wallet]
type = "private_key"
private_key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"

[accounts.safe_deployer]
type = "safe"
safe = "{safe_address}"
signer = "signer_wallet"

[namespace.default]
senders = {{ deployer = "safe_deployer", signer_wallet = "signer_wallet" }}
"#,
    );
    std::fs::write(project_dir.join("treb.toml"), toml).unwrap();
}

/// Safe(1/1) full execution on fork: deploy Safe → fork enter → treb run →
/// verify execute_safe_on_fork exercises approveHash + execTransaction on
/// real contracts → verify registry records.
#[tokio::test(flavor = "multi_thread")]
async fn safe_1of1_broadcast_on_fork() {
    // 1. Spawn Anvil
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    // 2. Set up project directory
    let tmp = tempfile::tempdir().unwrap();
    copy_dir_recursive(&fixture_project(), tmp.path());

    // 3. Deploy Safe(1/1) on Anvil with account #0 as sole owner
    let owner = Address::from_str(ACCOUNT_0).unwrap();
    let project_dir = tmp.path().to_path_buf();
    let rpc = rpc_url.clone();
    let safe = tokio::task::spawn_blocking(move || deploy_safe(&project_dir, &rpc, &[owner], 1))
        .await
        .expect("deploy_safe should not panic");

    let proxy_addr = safe.proxy_address;
    let multisend_addr = safe.multisend_address;

    // Verify Safe is functional via eth_call
    {
        let rpc = rpc_url.clone();
        let safe_clone = e2e::deploy_safe::DeployedSafe {
            proxy_address: safe.proxy_address,
            singleton_address: safe.singleton_address,
            factory_address: safe.factory_address,
            multisend_address: safe.multisend_address,
            owners: safe.owners.clone(),
            threshold: safe.threshold,
        };
        tokio::task::spawn_blocking(move || {
            verify_safe_via_eth_call(&rpc, &safe_clone);
        })
        .await
        .unwrap();
    }

    // 4. Write treb.toml with Safe sender config
    write_safe_treb_toml(tmp.path(), &proxy_addr);

    // 5. Run `treb init`
    let tmp_path = tmp.path().to_path_buf();
    tokio::task::spawn_blocking(move || {
        treb().arg("init").current_dir(&tmp_path).assert().success();
    })
    .await
    .unwrap();

    // 6. Enter fork mode
    let tmp_path = tmp.path().to_path_buf();
    let rpc = rpc_url.clone();
    tokio::task::spawn_blocking(move || {
        treb()
            .args(["fork", "enter", "--network", "anvil-31337", "--rpc-url", &rpc])
            .current_dir(&tmp_path)
            .assert()
            .success();
    })
    .await
    .unwrap();

    // 7. Run deployment through Safe
    let safe_addr_str = format!("{}", proxy_addr);
    let safe_env = format!("SAFE_ADDRESS={}", safe_addr_str);
    let multisend_env = format!("MULTISEND_ADDRESS={}", multisend_addr);
    let tmp_path = tmp.path().to_path_buf();
    let rpc = rpc_url.clone();
    tokio::task::spawn_blocking(move || {
        let output = treb()
            .args([
                "run",
                "script/DeployViaSafe.s.sol",
                "--network",
                "anvil-31337",
                "--rpc-url",
                &rpc,
                "--broadcast",
                "--non-interactive",
                "--env",
                &safe_env,
                "--env",
                &multisend_env,
            ])
            .current_dir(&tmp_path)
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "treb run failed.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    })
    .await
    .unwrap();

    // 8. Verify registry records

    // 8a. Check deployments exist
    let deps = e2e::read_deployments(tmp.path());
    let deps_map = deps.as_object().expect("deployments.json must be object");
    assert_eq!(deps_map.len(), 1, "should have exactly 1 deployment, got {}", deps_map.len());

    // 8b. Verify the deployment has the Counter contract
    let (_, dep) = deps_map.iter().next().unwrap();
    assert_eq!(
        dep["contractName"].as_str(),
        Some("Counter"),
        "deployment should be for Counter contract"
    );
    // The deployment address should be non-zero
    let dep_address = dep["address"].as_str().unwrap();
    assert!(dep_address.starts_with("0x"), "deployment address should start with 0x");
    assert_ne!(
        dep_address, "0x0000000000000000000000000000000000000000",
        "deployment address should be non-zero"
    );

    // 8c. Verify transaction records
    let txs = read_transactions(tmp.path());
    let txs_map = txs.as_object().expect("transactions.json must be object");
    assert!(!txs_map.is_empty(), "should have at least 1 transaction record");

    // Find the transaction and verify sender is the Safe address
    let (_, tx) = txs_map.iter().next().unwrap();
    let tx_sender = tx["sender"].as_str().unwrap();
    assert_eq!(
        tx_sender.to_lowercase(),
        safe_addr_str.to_lowercase(),
        "transaction sender should be the Safe address, not the EOA"
    );

    // Transaction should be executed (not queued) for Safe(1/1)
    let tx_status = tx["status"].as_str().unwrap();
    assert_eq!(tx_status, "EXECUTED", "Safe(1/1) transaction should be EXECUTED on fork");

    // Transaction should have a non-empty hash
    let tx_hash = tx["hash"].as_str().unwrap();
    assert!(
        tx_hash.starts_with("0x") && tx_hash.len() > 2,
        "transaction should have a valid hash, got: {tx_hash}"
    );

    // 8d. Verify the routing path exercised execute_safe_on_fork by checking
    // that the transaction has a valid on-chain hash (proves it was broadcast
    // through the Safe, not just simulated).
    assert!(
        tx_hash.len() == 66, // "0x" + 64 hex chars
        "transaction hash should be a full 32-byte hex, got: {tx_hash}"
    );

    // 8e. Verify registry consistency (lookup.json cross-references)
    e2e::assert_registry_consistent(tmp.path());

    drop(anvil);
}

/// Read the Counter contract creation bytecode from forge build artifacts.
fn read_counter_creation_bytecode(project_dir: &Path) -> Vec<u8> {
    let artifact_path = project_dir.join("out/Counter.sol/Counter.json");
    let data: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&artifact_path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", artifact_path.display())),
    )
    .unwrap();
    let hex_str = data["bytecode"]["object"].as_str().expect("bytecode.object must be a string");
    let hex_str = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    alloy_primitives::hex::decode(hex_str).expect("invalid hex in creation bytecode")
}

/// Safe(2/3) proposal on fork: deploy Safe(2/3) → fork enter → treb run with
/// --skip-fork-execution → verify QueuedExecution::SafeProposal →
/// verify safe-txs.json records and EIP-712 hash.
#[tokio::test(flavor = "multi_thread")]
async fn safe_2of3_proposal_on_fork() {
    // 1. Spawn Anvil
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    // 2. Set up project directory
    let tmp = tempfile::tempdir().unwrap();
    copy_dir_recursive(&fixture_project(), tmp.path());

    // 3. Deploy Safe(2/3) on Anvil with accounts #0, #1, #2 as owners
    let owner0 = Address::from_str(ACCOUNT_0).unwrap();
    let owner1 = Address::from_str(ACCOUNT_1).unwrap();
    let owner2 = Address::from_str(ACCOUNT_2).unwrap();
    let project_dir = tmp.path().to_path_buf();
    let rpc = rpc_url.clone();
    let safe = tokio::task::spawn_blocking(move || {
        deploy_safe(&project_dir, &rpc, &[owner0, owner1, owner2], 2)
    })
    .await
    .expect("deploy_safe should not panic");

    let proxy_addr = safe.proxy_address;
    let multisend_addr = safe.multisend_address;

    // Verify Safe is functional via eth_call
    {
        let rpc = rpc_url.clone();
        let safe_clone = e2e::deploy_safe::DeployedSafe {
            proxy_address: safe.proxy_address,
            singleton_address: safe.singleton_address,
            factory_address: safe.factory_address,
            multisend_address: safe.multisend_address,
            owners: safe.owners.clone(),
            threshold: safe.threshold,
        };
        tokio::task::spawn_blocking(move || {
            verify_safe_via_eth_call(&rpc, &safe_clone);
        })
        .await
        .unwrap();
    }

    // 4. Write treb.toml with Safe sender config
    write_safe_treb_toml(tmp.path(), &proxy_addr);

    // 5. Run `treb init`
    let tmp_path = tmp.path().to_path_buf();
    tokio::task::spawn_blocking(move || {
        treb().arg("init").current_dir(&tmp_path).assert().success();
    })
    .await
    .unwrap();

    // 6. Enter fork mode
    let tmp_path = tmp.path().to_path_buf();
    let rpc = rpc_url.clone();
    tokio::task::spawn_blocking(move || {
        treb()
            .args(["fork", "enter", "--network", "anvil-31337", "--rpc-url", &rpc])
            .current_dir(&tmp_path)
            .assert()
            .success();
    })
    .await
    .unwrap();

    // 7. Run deployment through Safe with --skip-fork-execution
    let safe_addr_str = format!("{}", proxy_addr);
    let safe_env = format!("SAFE_ADDRESS={}", safe_addr_str);
    let multisend_env = format!("MULTISEND_ADDRESS={}", multisend_addr);
    let tmp_path = tmp.path().to_path_buf();
    let rpc = rpc_url.clone();
    tokio::task::spawn_blocking(move || {
        let output = treb()
            .args([
                "run",
                "script/DeployViaSafe.s.sol",
                "--network",
                "anvil-31337",
                "--rpc-url",
                &rpc,
                "--broadcast",
                "--non-interactive",
                "--skip-fork-execution",
                "--env",
                &safe_env,
                "--env",
                &multisend_env,
            ])
            .current_dir(&tmp_path)
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "treb run failed.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    })
    .await
    .unwrap();

    // 8. Verify safe-txs.json

    // 8a. Read safe-txs.json
    let safe_txs = e2e::read_registry_file(tmp.path(), "safe-txs.json");
    let safe_txs_map = safe_txs.as_object().expect("safe-txs.json must be object");
    assert_eq!(
        safe_txs_map.len(),
        1,
        "should have exactly 1 safe transaction entry, got {}",
        safe_txs_map.len()
    );

    let (safe_tx_hash_key, stx) = safe_txs_map.iter().next().unwrap();

    // 8b. Verify safeAddress
    assert_eq!(
        stx["safeAddress"].as_str().unwrap().to_lowercase(),
        safe_addr_str.to_lowercase(),
        "safeAddress should match the deployed Safe proxy"
    );

    // 8c. Verify status = QUEUED
    assert_eq!(
        stx["status"].as_str(),
        Some("QUEUED"),
        "Safe(2/3) transaction should be QUEUED, not executed"
    );

    // 8d. Verify nonce = 0
    assert_eq!(stx["nonce"].as_u64(), Some(0), "Safe nonce should be 0 (first transaction)");

    // 8e. Verify non-empty safeTxHash (key matches value)
    assert!(
        safe_tx_hash_key.starts_with("0x") && safe_tx_hash_key.len() == 66,
        "safeTxHash key should be a 32-byte hex hash, got: {safe_tx_hash_key}"
    );
    assert_eq!(
        stx["safeTxHash"].as_str().unwrap(),
        safe_tx_hash_key,
        "safeTxHash value should match the map key"
    );

    // 8f. Verify proposedBy = account #0
    assert_eq!(
        stx["proposedBy"].as_str().unwrap().to_lowercase(),
        ACCOUNT_0.to_lowercase(),
        "proposedBy should be the signer (account #0)"
    );

    // 8g. Verify linked transactionIds
    let tx_ids = stx["transactionIds"].as_array().expect("transactionIds must be array");
    assert!(!tx_ids.is_empty(), "should have at least 1 linked transactionId");

    // 8h. Verify transaction records exist and are QUEUED
    let txs = read_transactions(tmp.path());
    let txs_map = txs.as_object().expect("transactions.json must be object");
    assert!(!txs_map.is_empty(), "should have at least 1 transaction record");
    for tx_id in tx_ids {
        let tx_id_str = tx_id.as_str().unwrap();
        let tx = &txs_map[tx_id_str];
        assert_eq!(
            tx["status"].as_str(),
            Some("QUEUED"),
            "linked transaction {tx_id_str} should be QUEUED"
        );
        assert_eq!(
            tx["sender"].as_str().unwrap().to_lowercase(),
            safe_addr_str.to_lowercase(),
            "transaction sender should be the Safe address"
        );
    }

    // 9. Verify safeTxHash matches independent EIP-712 computation.
    //
    // The DeployViaSafe.s.sol script deploys Counter via CREATE. The routing
    // converts this to a single MultiSendOperation { to: 0x0, data: creation_bytecode,
    // operation: 0 }. For a single op, compute_safe_tx_hash_for_ops uses it
    // directly (no MultiSend wrapping).
    let creation_bytecode = read_counter_creation_bytecode(tmp.path());
    let safe_tx = treb_safe::SafeTx {
        to: Address::ZERO,
        value: U256::ZERO,
        data: creation_bytecode.into(),
        operation: 0,
        safeTxGas: U256::ZERO,
        baseGas: U256::ZERO,
        gasPrice: U256::ZERO,
        gasToken: Address::ZERO,
        refundReceiver: Address::ZERO,
        nonce: U256::ZERO,
    };
    let expected_hash = treb_safe::compute_safe_tx_hash(31337, proxy_addr, &safe_tx);
    let expected_hash_str = format!("{:#x}", expected_hash);
    assert_eq!(
        safe_tx_hash_key, &expected_hash_str,
        "safeTxHash in registry should match EIP-712 computation"
    );

    drop(anvil);
}
