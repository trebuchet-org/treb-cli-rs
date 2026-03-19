//! P8-US-002: Mixed wallet + Safe sender integration tests.
//!
//! Verifies that partition_into_runs() correctly separates transaction runs
//! when a single script broadcasts from both a wallet sender and a Safe sender.

mod e2e;

use alloy_primitives::Address;
use e2e::deploy_safe::deploy_safe;
use e2e::{
    assert_registry_consistent, copy_dir_recursive, read_deployments, read_transactions,
    spawn_anvil_or_skip, treb,
};
use std::path::Path;
use std::str::FromStr;

/// Well-known Anvil account #0.
const ACCOUNT_0: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";

fn fixture_project() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("project")
}

/// Write a treb.toml that configures both a wallet sender (deployer) and a
/// Safe sender (safe_deployer) with the deployer as the Safe signer.
fn write_mixed_wallet_safe_treb_toml(project_dir: &Path, safe_address: &Address) {
    let toml = format!(
        r#"[accounts.deployer]
type = "private_key"
private_key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"

[accounts.safe_deployer]
type = "safe"
safe = "{safe_address}"
signer = "deployer"

[namespace.default]
senders = {{ deployer = "deployer", safe_deployer = "safe_deployer" }}
"#,
    );
    std::fs::write(project_dir.join("treb.toml"), toml).unwrap();
}

/// Mixed wallet + Safe(1/1) broadcast on fork: deploy Safe → configure both
/// senders → fork enter → treb run DeployMixedWalletSafe.s.sol → verify that
/// both sender types produce EXECUTED transactions with correct sender addresses.
#[tokio::test(flavor = "multi_thread")]
async fn mixed_wallet_safe_broadcast_on_fork() {
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

    // 4. Write treb.toml with mixed wallet + Safe sender config
    write_mixed_wallet_safe_treb_toml(tmp.path(), &proxy_addr);

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

    // 7. Run mixed-sender deployment
    let safe_addr_str = format!("{}", proxy_addr);
    let safe_env = format!("SAFE_ADDRESS={}", safe_addr_str);
    let multisend_env = format!("MULTISEND_ADDRESS={}", multisend_addr);
    let tmp_path = tmp.path().to_path_buf();
    let rpc = rpc_url.clone();
    tokio::task::spawn_blocking(move || {
        let output = treb()
            .args([
                "run",
                "script/DeployMixedWalletSafe.s.sol",
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

    // 8a. Check deployments — should have exactly 2 entries
    let deps = read_deployments(tmp.path());
    let deps_map = deps.as_object().expect("deployments.json must be object");
    assert_eq!(
        deps_map.len(),
        2,
        "should have exactly 2 deployments (wallet + safe), got {}",
        deps_map.len()
    );

    // 8b. Identify wallet and safe deployments by label
    let mut wallet_dep = None;
    let mut safe_dep = None;
    for (_, dep) in deps_map {
        match dep["label"].as_str() {
            Some("WalletCounter") => wallet_dep = Some(dep),
            Some("SafeCounter") => safe_dep = Some(dep),
            other => panic!("unexpected deployment label: {other:?}"),
        }
    }
    let wallet_dep = wallet_dep.expect("should have WalletCounter deployment");
    let safe_dep = safe_dep.expect("should have SafeCounter deployment");

    // Both should have contractName "Counter" (artifact name)
    assert_eq!(wallet_dep["contractName"].as_str(), Some("Counter"));
    assert_eq!(safe_dep["contractName"].as_str(), Some("Counter"));

    // 8c. Verify all transactions have status EXECUTED
    let txs = read_transactions(tmp.path());
    let txs_map = txs.as_object().expect("transactions.json must be object");
    assert!(
        !txs_map.is_empty(),
        "should have at least 1 transaction record"
    );
    for (tx_id, tx) in txs_map {
        assert_eq!(
            tx["status"].as_str(),
            Some("EXECUTED"),
            "transaction {tx_id} should be EXECUTED"
        );
    }

    // 8d. Verify wallet-sender transaction's sender matches the wallet address
    let has_wallet_tx = txs_map.values().any(|tx| {
        tx["sender"]
            .as_str()
            .is_some_and(|s| s.eq_ignore_ascii_case(ACCOUNT_0))
    });
    assert!(
        has_wallet_tx,
        "should have a transaction with sender matching the wallet address {ACCOUNT_0}"
    );

    // 8e. Verify safe-sender transaction's sender matches the Safe proxy address
    let has_safe_tx = txs_map.values().any(|tx| {
        tx["sender"]
            .as_str()
            .is_some_and(|s| s.eq_ignore_ascii_case(&safe_addr_str))
    });
    assert!(
        has_safe_tx,
        "should have a transaction with sender matching the Safe proxy address {safe_addr_str}"
    );

    // 9. Verify registry consistency (lookup.json cross-references)
    assert_registry_consistent(tmp.path());

    drop(anvil);
}
