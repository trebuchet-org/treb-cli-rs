//! P7-US-005 / P7-US-006 / P7-US-007: Governor routing e2e workflow tests.
//!
//! Integration tests verifying the full Governor routing pipeline against
//! real governance contracts deployed on Anvil:
//!
//! - Governor→Wallet: propose() broadcast on fork via wallet proposer
//! - Governor→Safe(1/1): recursive depth-2 routing (Governor → Safe → Wallet)
//! - Governor with --skip-fork-execution: proposal recorded but not simulated

mod e2e;

use alloy_primitives::Address;
use e2e::{
    copy_dir_recursive,
    deploy_governor::{deploy_governor, verify_governor_via_eth_call},
    deploy_safe::{deploy_safe, verify_safe_via_eth_call},
    read_transactions, spawn_anvil_or_skip, treb,
};
use std::{path::Path, str::FromStr};

/// Well-known Anvil test account #0.
const ACCOUNT_0: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";

fn fixture_project() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("project")
}

/// Write a treb.toml that configures a Governor sender with a Wallet proposer.
///
/// The governor broadcasts from the timelock address, and the proposer (wallet)
/// submits the propose() transaction on fork.
fn write_governor_wallet_treb_toml(
    project_dir: &Path,
    governor_address: &alloy_primitives::Address,
    timelock_address: &alloy_primitives::Address,
) {
    let toml = format!(
        r#"[accounts.signer_wallet]
type = "private_key"
private_key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"

[accounts.governance]
type = "governance"
address = "{governor_address}"
timelock = "{timelock_address}"
proposer = "signer_wallet"

[namespace.default]
senders = {{ deployer = "governance", signer_wallet = "signer_wallet" }}
"#,
    );
    std::fs::write(project_dir.join("treb.toml"), toml).unwrap();
}

/// Governor with Wallet proposer: deploy governance stack → fork enter →
/// treb run DeployViaGovernor.s.sol → verify propose() is broadcast through
/// wallet → verify governor-txs.json and transactions.json registry records.
#[tokio::test(flavor = "multi_thread")]
async fn governor_wallet_propose_on_fork() {
    // 1. Spawn Anvil
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    // 2. Set up project directory
    let tmp = tempfile::tempdir().unwrap();
    copy_dir_recursive(&fixture_project(), tmp.path());

    // 3. Deploy governance stack (token, timelock, governor) on Anvil
    let project_dir = tmp.path().to_path_buf();
    let rpc = rpc_url.clone();
    let gov = tokio::task::spawn_blocking(move || deploy_governor(&project_dir, &rpc, 1))
        .await
        .expect("deploy_governor should not panic");

    // Verify governance stack is functional
    {
        let rpc = rpc_url.clone();
        let gov_ref = e2e::deploy_governor::DeployedGovernor {
            governor_address: gov.governor_address,
            timelock_address: gov.timelock_address,
            token_address: gov.token_address,
            timelock_delay: gov.timelock_delay,
        };
        tokio::task::spawn_blocking(move || {
            verify_governor_via_eth_call(&rpc, &gov_ref);
        })
        .await
        .unwrap();
    }

    // 4. Write treb.toml with Governor sender config (wallet proposer)
    write_governor_wallet_treb_toml(tmp.path(), &gov.governor_address, &gov.timelock_address);

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

    // 7. Run deployment through Governor
    let timelock_env = format!("TIMELOCK_ADDRESS={}", gov.timelock_address);
    let tmp_path = tmp.path().to_path_buf();
    let rpc = rpc_url.clone();
    tokio::task::spawn_blocking(move || {
        let output = treb()
            .args([
                "run",
                "script/DeployViaGovernor.s.sol",
                "--network",
                "anvil-31337",
                "--rpc-url",
                &rpc,
                "--broadcast",
                "--non-interactive",
                "--env",
                &timelock_env,
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

    // 8b. Verify the deployment has the Counter contract with non-zero address
    let (_, dep) = deps_map.iter().next().unwrap();
    assert_eq!(
        dep["contractName"].as_str(),
        Some("Counter"),
        "deployment should be for Counter contract"
    );
    let dep_address = dep["address"].as_str().unwrap();
    assert!(dep_address.starts_with("0x"), "deployment address should start with 0x");
    assert_ne!(
        dep_address, "0x0000000000000000000000000000000000000000",
        "deployment address should be non-zero"
    );

    // 8c. Verify governor-txs.json
    let gov_txs = e2e::read_registry_file(tmp.path(), "governor-txs.json");
    let gov_txs_map = gov_txs.as_object().expect("governor-txs.json must be object");
    assert_eq!(
        gov_txs_map.len(),
        1,
        "should have exactly 1 governor proposal, got {}",
        gov_txs_map.len()
    );

    let (proposal_id_key, proposal) = gov_txs_map.iter().next().unwrap();

    // proposalId should be non-empty
    let proposal_id = proposal["proposalId"].as_str().unwrap();
    assert!(!proposal_id.is_empty(), "proposalId should be non-empty");
    assert_eq!(proposal_id, proposal_id_key, "proposalId value should match the map key");

    // status should be pending
    assert_eq!(
        proposal["status"].as_str(),
        Some("pending"),
        "governor proposal status should be 'pending'"
    );

    // governorAddress should match deployed governor
    assert_eq!(
        proposal["governorAddress"].as_str().unwrap().to_lowercase(),
        format!("{}", gov.governor_address).to_lowercase(),
        "governorAddress should match deployed governor"
    );

    // timelockAddress should match deployed timelock
    assert_eq!(
        proposal["timelockAddress"].as_str().unwrap().to_lowercase(),
        format!("{}", gov.timelock_address).to_lowercase(),
        "timelockAddress should match deployed timelock"
    );

    // chainId should be 31337 (Anvil)
    assert_eq!(proposal["chainId"].as_u64(), Some(31337), "chainId should be 31337");

    // proposedBy should be account #0 (the wallet proposer)
    assert_eq!(
        proposal["proposedBy"].as_str().unwrap().to_lowercase(),
        ACCOUNT_0.to_lowercase(),
        "proposedBy should be the wallet proposer (account #0)"
    );

    // transactionIds should be non-empty
    let tx_ids = proposal["transactionIds"].as_array().expect("transactionIds must be array");
    assert!(!tx_ids.is_empty(), "should have at least 1 linked transactionId");

    // forkExecutedAt should be set (fork simulation runs automatically in non-interactive mode)
    assert!(
        proposal["forkExecutedAt"].as_str().is_some(),
        "forkExecutedAt should be set after fork simulation"
    );

    // 8d. Verify transaction records
    let txs = read_transactions(tmp.path());
    let txs_map = txs.as_object().expect("transactions.json must be object");
    assert!(!txs_map.is_empty(), "should have at least 1 transaction record");

    // Verify linked transactions
    let timelock_addr_str = format!("{}", gov.timelock_address);
    for tx_id in tx_ids {
        let tx_id_str = tx_id.as_str().unwrap();
        let tx = &txs_map[tx_id_str];

        // Sender should be the timelock address (broadcast_address for Governor+Timelock)
        assert_eq!(
            tx["sender"].as_str().unwrap().to_lowercase(),
            timelock_addr_str.to_lowercase(),
            "transaction sender should be the timelock address"
        );

        // Governor routing marks linked transactions as QUEUED (the proposal needs
        // on-chain governance to execute; fork simulation only sets forkExecutedAt
        // on the governor proposal, not the underlying transaction status).
        let tx_status = tx["status"].as_str().unwrap();
        assert!(
            tx_status == "QUEUED" || tx_status == "EXECUTED",
            "transaction status should be QUEUED or EXECUTED, got: {tx_status}"
        );
    }

    // 8e. Verify registry consistency (lookup.json cross-references)
    e2e::assert_registry_consistent(tmp.path());

    drop(anvil);
}

/// Write a treb.toml that configures a Governor sender with a Safe(1/1) proposer.
///
/// Depth-2 routing: Governor → Safe(1/1) → Wallet.
/// The Safe is the proposer for the governor, and the wallet signs for the Safe.
fn write_governor_safe_treb_toml(
    project_dir: &Path,
    governor_address: &alloy_primitives::Address,
    timelock_address: &alloy_primitives::Address,
    safe_address: &alloy_primitives::Address,
) {
    let toml = format!(
        r#"[accounts.signer_wallet]
type = "private_key"
private_key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"

[accounts.proposer_safe]
type = "safe"
safe = "{safe_address}"
signer = "signer_wallet"

[accounts.governance]
type = "governance"
address = "{governor_address}"
timelock = "{timelock_address}"
proposer = "proposer_safe"

[namespace.default]
senders = {{ deployer = "governance", signer_wallet = "signer_wallet", proposer_safe = "proposer_safe" }}
"#,
    );
    std::fs::write(project_dir.join("treb.toml"), toml).unwrap();
}

/// Governor with Safe(1/1) proposer: deploy governance stack + Safe(1/1) →
/// fork enter → treb run DeployViaGovernor.s.sol → verify depth-2 routing
/// (Governor → Safe → Wallet) → verify governor-txs.json and transactions.json.
#[tokio::test(flavor = "multi_thread")]
async fn governor_safe_propose_on_fork() {
    // 1. Spawn Anvil
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    // 2. Set up project directory
    let tmp = tempfile::tempdir().unwrap();
    copy_dir_recursive(&fixture_project(), tmp.path());

    // 3. Deploy Safe(1/1) with account #0 as sole owner
    let owner = Address::from_str(ACCOUNT_0).unwrap();
    let project_dir = tmp.path().to_path_buf();
    let rpc = rpc_url.clone();
    let safe = tokio::task::spawn_blocking(move || deploy_safe(&project_dir, &rpc, &[owner], 1))
        .await
        .expect("deploy_safe should not panic");

    let safe_proxy_addr = safe.proxy_address;

    // Verify Safe is functional
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

    // 4. Deploy governance stack (token, timelock, governor) on Anvil
    let project_dir = tmp.path().to_path_buf();
    let rpc = rpc_url.clone();
    let gov = tokio::task::spawn_blocking(move || deploy_governor(&project_dir, &rpc, 1))
        .await
        .expect("deploy_governor should not panic");

    // Verify governance stack is functional
    {
        let rpc = rpc_url.clone();
        let gov_ref = e2e::deploy_governor::DeployedGovernor {
            governor_address: gov.governor_address,
            timelock_address: gov.timelock_address,
            token_address: gov.token_address,
            timelock_delay: gov.timelock_delay,
        };
        tokio::task::spawn_blocking(move || {
            verify_governor_via_eth_call(&rpc, &gov_ref);
        })
        .await
        .unwrap();
    }

    // 5. Write treb.toml with Governor sender config (Safe proposer)
    write_governor_safe_treb_toml(
        tmp.path(),
        &gov.governor_address,
        &gov.timelock_address,
        &safe_proxy_addr,
    );

    // 6. Run `treb init`
    let tmp_path = tmp.path().to_path_buf();
    tokio::task::spawn_blocking(move || {
        treb().arg("init").current_dir(&tmp_path).assert().success();
    })
    .await
    .unwrap();

    // 7. Enter fork mode
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

    // 8. Run deployment through Governor (with Safe proposer)
    let timelock_env = format!("TIMELOCK_ADDRESS={}", gov.timelock_address);
    let tmp_path = tmp.path().to_path_buf();
    let rpc = rpc_url.clone();
    tokio::task::spawn_blocking(move || {
        let output = treb()
            .args([
                "run",
                "script/DeployViaGovernor.s.sol",
                "--network",
                "anvil-31337",
                "--rpc-url",
                &rpc,
                "--broadcast",
                "--non-interactive",
                "--env",
                &timelock_env,
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

    // 9. Verify registry records

    // 9a. Check deployments exist
    let deps = e2e::read_deployments(tmp.path());
    let deps_map = deps.as_object().expect("deployments.json must be object");
    assert_eq!(deps_map.len(), 1, "should have exactly 1 deployment, got {}", deps_map.len());

    // 9b. Verify the deployment has the Counter contract with non-zero address
    let (_, dep) = deps_map.iter().next().unwrap();
    assert_eq!(
        dep["contractName"].as_str(),
        Some("Counter"),
        "deployment should be for Counter contract"
    );
    let dep_address = dep["address"].as_str().unwrap();
    assert!(dep_address.starts_with("0x"), "deployment address should start with 0x");
    assert_ne!(
        dep_address, "0x0000000000000000000000000000000000000000",
        "deployment address should be non-zero"
    );

    // 9c. Verify governor-txs.json
    let gov_txs = e2e::read_registry_file(tmp.path(), "governor-txs.json");
    let gov_txs_map = gov_txs.as_object().expect("governor-txs.json must be object");
    assert_eq!(
        gov_txs_map.len(),
        1,
        "should have exactly 1 governor proposal, got {}",
        gov_txs_map.len()
    );

    let (proposal_id_key, proposal) = gov_txs_map.iter().next().unwrap();

    // proposalId should be non-empty
    let proposal_id = proposal["proposalId"].as_str().unwrap();
    assert!(!proposal_id.is_empty(), "proposalId should be non-empty");
    assert_eq!(proposal_id, proposal_id_key, "proposalId value should match the map key");

    // status should be pending
    assert_eq!(
        proposal["status"].as_str(),
        Some("pending"),
        "governor proposal status should be 'pending'"
    );

    // governorAddress should match deployed governor
    assert_eq!(
        proposal["governorAddress"].as_str().unwrap().to_lowercase(),
        format!("{}", gov.governor_address).to_lowercase(),
        "governorAddress should match deployed governor"
    );

    // timelockAddress should match deployed timelock
    assert_eq!(
        proposal["timelockAddress"].as_str().unwrap().to_lowercase(),
        format!("{}", gov.timelock_address).to_lowercase(),
        "timelockAddress should match deployed timelock"
    );

    // chainId should be 31337 (Anvil)
    assert_eq!(proposal["chainId"].as_u64(), Some(31337), "chainId should be 31337");

    // transactionIds should be non-empty
    let tx_ids = proposal["transactionIds"].as_array().expect("transactionIds must be array");
    assert!(!tx_ids.is_empty(), "should have at least 1 linked transactionId");

    // 9d. Verify transaction records
    let txs = read_transactions(tmp.path());
    let txs_map = txs.as_object().expect("transactions.json must be object");
    assert!(!txs_map.is_empty(), "should have at least 1 transaction record");

    // Verify linked transactions
    let timelock_addr_str = format!("{}", gov.timelock_address);
    for tx_id in tx_ids {
        let tx_id_str = tx_id.as_str().unwrap();
        let tx = &txs_map[tx_id_str];

        // Sender should be the timelock address (broadcast_address for Governor+Timelock)
        assert_eq!(
            tx["sender"].as_str().unwrap().to_lowercase(),
            timelock_addr_str.to_lowercase(),
            "transaction sender should be the timelock address"
        );

        // Governor routing marks linked transactions as QUEUED
        let tx_status = tx["status"].as_str().unwrap();
        assert!(
            tx_status == "QUEUED" || tx_status == "EXECUTED",
            "transaction status should be QUEUED or EXECUTED, got: {tx_status}"
        );
    }

    // 9e. Verify registry consistency (lookup.json cross-references)
    e2e::assert_registry_consistent(tmp.path());

    drop(anvil);
}

/// Governor with Wallet proposer and --skip-fork-execution: deploy governance
/// stack → fork enter → treb run DeployViaGovernor.s.sol --skip-fork-execution →
/// verify proposal is recorded but NOT fork-simulated → verify QUEUED status
/// and populated actions array.
#[tokio::test(flavor = "multi_thread")]
async fn governor_skip_fork_execution() {
    // 1. Spawn Anvil
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    // 2. Set up project directory
    let tmp = tempfile::tempdir().unwrap();
    copy_dir_recursive(&fixture_project(), tmp.path());

    // 3. Deploy governance stack (token, timelock, governor) on Anvil
    let project_dir = tmp.path().to_path_buf();
    let rpc = rpc_url.clone();
    let gov = tokio::task::spawn_blocking(move || deploy_governor(&project_dir, &rpc, 1))
        .await
        .expect("deploy_governor should not panic");

    // Verify governance stack is functional
    {
        let rpc = rpc_url.clone();
        let gov_ref = e2e::deploy_governor::DeployedGovernor {
            governor_address: gov.governor_address,
            timelock_address: gov.timelock_address,
            token_address: gov.token_address,
            timelock_delay: gov.timelock_delay,
        };
        tokio::task::spawn_blocking(move || {
            verify_governor_via_eth_call(&rpc, &gov_ref);
        })
        .await
        .unwrap();
    }

    // 4. Write treb.toml with Governor sender config (wallet proposer)
    write_governor_wallet_treb_toml(tmp.path(), &gov.governor_address, &gov.timelock_address);

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

    // 7. Run deployment through Governor with --skip-fork-execution
    let timelock_env = format!("TIMELOCK_ADDRESS={}", gov.timelock_address);
    let tmp_path = tmp.path().to_path_buf();
    let rpc = rpc_url.clone();
    tokio::task::spawn_blocking(move || {
        let output = treb()
            .args([
                "run",
                "script/DeployViaGovernor.s.sol",
                "--network",
                "anvil-31337",
                "--rpc-url",
                &rpc,
                "--broadcast",
                "--non-interactive",
                "--skip-fork-execution",
                "--env",
                &timelock_env,
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

    // 8b. Verify the deployment has the Counter contract with non-zero address
    let (_, dep) = deps_map.iter().next().unwrap();
    assert_eq!(
        dep["contractName"].as_str(),
        Some("Counter"),
        "deployment should be for Counter contract"
    );
    let dep_address = dep["address"].as_str().unwrap();
    assert!(dep_address.starts_with("0x"), "deployment address should start with 0x");
    assert_ne!(
        dep_address, "0x0000000000000000000000000000000000000000",
        "deployment address should be non-zero"
    );

    // 8c. Verify governor-txs.json
    let gov_txs = e2e::read_registry_file(tmp.path(), "governor-txs.json");
    let gov_txs_map = gov_txs.as_object().expect("governor-txs.json must be object");
    assert_eq!(
        gov_txs_map.len(),
        1,
        "should have exactly 1 governor proposal, got {}",
        gov_txs_map.len()
    );

    let (proposal_id_key, proposal) = gov_txs_map.iter().next().unwrap();

    // proposalId should be non-empty
    let proposal_id = proposal["proposalId"].as_str().unwrap();
    assert!(!proposal_id.is_empty(), "proposalId should be non-empty");
    assert_eq!(proposal_id, proposal_id_key, "proposalId value should match the map key");

    // status should be pending (not executed, since we skipped fork execution)
    assert_eq!(
        proposal["status"].as_str(),
        Some("pending"),
        "governor proposal status should be 'pending'"
    );

    // governorAddress should match deployed governor
    assert_eq!(
        proposal["governorAddress"].as_str().unwrap().to_lowercase(),
        format!("{}", gov.governor_address).to_lowercase(),
        "governorAddress should match deployed governor"
    );

    // timelockAddress should match deployed timelock
    assert_eq!(
        proposal["timelockAddress"].as_str().unwrap().to_lowercase(),
        format!("{}", gov.timelock_address).to_lowercase(),
        "timelockAddress should match deployed timelock"
    );

    // chainId should be 31337 (Anvil)
    assert_eq!(proposal["chainId"].as_u64(), Some(31337), "chainId should be 31337");

    // forkExecutedAt should NOT be set (--skip-fork-execution was used)
    assert!(
        proposal.get("forkExecutedAt").is_none() || proposal["forkExecutedAt"].is_null(),
        "forkExecutedAt should NOT be set when --skip-fork-execution is used, got: {:?}",
        proposal.get("forkExecutedAt")
    );

    // actions should be a non-empty array with target/value/calldata
    let actions = proposal["actions"].as_array().expect("actions must be an array");
    assert!(
        !actions.is_empty(),
        "actions array should have at least 1 entry for Counter deployment"
    );
    for action in actions {
        assert!(action["target"].as_str().is_some(), "action should have a target field");
        assert!(
            action["value"].is_string() || action["value"].is_number(),
            "action should have a value field"
        );
        assert!(action["calldata"].as_str().is_some(), "action should have a calldata field");
    }

    // transactionIds should be non-empty
    let tx_ids = proposal["transactionIds"].as_array().expect("transactionIds must be array");
    assert!(!tx_ids.is_empty(), "should have at least 1 linked transactionId");

    // 8d. Verify transaction records — all should be QUEUED (not EXECUTED)
    let txs = read_transactions(tmp.path());
    let txs_map = txs.as_object().expect("transactions.json must be object");
    assert!(!txs_map.is_empty(), "should have at least 1 transaction record");

    let timelock_addr_str = format!("{}", gov.timelock_address);
    for tx_id in tx_ids {
        let tx_id_str = tx_id.as_str().unwrap();
        let tx = &txs_map[tx_id_str];

        // Sender should be the timelock address
        assert_eq!(
            tx["sender"].as_str().unwrap().to_lowercase(),
            timelock_addr_str.to_lowercase(),
            "transaction sender should be the timelock address"
        );

        // With --skip-fork-execution, transactions must be strictly QUEUED
        assert_eq!(
            tx["status"].as_str(),
            Some("QUEUED"),
            "linked transaction {tx_id_str} should be QUEUED when --skip-fork-execution is used"
        );
    }

    // 8e. Verify registry consistency
    e2e::assert_registry_consistent(tmp.path());

    drop(anvil);
}

/// Governor proposal with title and description: deploy governance stack →
/// fork enter → treb run DeployViaGovernorWithDescription.s.sol → verify
/// governor-txs.json has description populated from GovernorBroadcast event.
#[tokio::test(flavor = "multi_thread")]
async fn governor_proposal_captures_description() {
    // 1. Spawn Anvil
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    // 2. Set up project directory
    let tmp = tempfile::tempdir().unwrap();
    copy_dir_recursive(&fixture_project(), tmp.path());

    // 3. Deploy governance stack
    let project_dir = tmp.path().to_path_buf();
    let rpc = rpc_url.clone();
    let gov = tokio::task::spawn_blocking(move || deploy_governor(&project_dir, &rpc, 1))
        .await
        .expect("deploy_governor should not panic");

    // 4. Write treb.toml with Governor sender
    write_governor_wallet_treb_toml(tmp.path(), &gov.governor_address, &gov.timelock_address);

    // 5. Init + fork enter
    let tmp_path = tmp.path().to_path_buf();
    tokio::task::spawn_blocking(move || {
        treb().arg("init").current_dir(&tmp_path).assert().success();
    })
    .await
    .unwrap();

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

    // 6. Run deployment with description script
    let timelock_env = format!("TIMELOCK_ADDRESS={}", gov.timelock_address);
    let governor_env = format!("GOVERNOR_ADDRESS={}", gov.governor_address);
    let tmp_path = tmp.path().to_path_buf();
    let rpc = rpc_url.clone();
    tokio::task::spawn_blocking(move || {
        let output = treb()
            .args([
                "run",
                "script/DeployViaGovernorWithDescription.s.sol",
                "--network",
                "anvil-31337",
                "--rpc-url",
                &rpc,
                "--broadcast",
                "--non-interactive",
                "--env",
                &timelock_env,
                "--env",
                &governor_env,
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

    // 7. Verify governor-txs.json has description
    let gov_txs = e2e::read_registry_file(tmp.path(), "governor-txs.json");
    let gov_txs_map = gov_txs.as_object().expect("governor-txs.json must be object");
    assert!(!gov_txs_map.is_empty(), "should have at least 1 governor proposal");

    // Find the proposal with a description (the event-hydrated one)
    let description = gov_txs_map
        .values()
        .filter_map(|p| p["description"].as_str())
        .find(|d| !d.is_empty())
        .unwrap_or("");

    assert!(
        description.contains("Deploy Counter v2"),
        "description should contain title 'Deploy Counter v2', got: {description}"
    );
    assert!(
        description.contains("deploys a new Counter contract via governance"),
        "description should contain body text, got: {description}"
    );

    drop(anvil);
}
