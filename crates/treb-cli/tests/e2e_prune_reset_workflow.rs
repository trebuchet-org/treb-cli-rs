//! P16-US-005: Deployment Lifecycle with Prune and Reset
//!
//! Tests prune (with on-chain bytecode verification) and scoped reset
//! to verify the full deployment create-destroy-recreate lifecycle.

mod e2e;

use e2e::{
    assert_deployment_count, read_deployments, read_transactions, run_deployment, run_human,
    run_json, setup_project, spawn_anvil_or_skip, treb,
};

// ── Anvil RPC helpers ───────────────────────────────────────────────────────

/// Deploy a minimal contract directly on Anvil via `eth_sendTransaction` and
/// return `(tx_hash, contract_address)`.
async fn deploy_contract_on_anvil(rpc_url: &str) -> (String, String) {
    let client = reqwest::Client::new();

    // Creation bytecode that deploys a 1-byte runtime (STOP opcode = 0x00).
    // The key difference from 0x6001600a... is that the code offset (0x0c = 12)
    // correctly points past the creation code so RETURN produces non-empty output.
    //   PUSH1 01 (size=1)  PUSH1 0c (offset=12)  PUSH1 00 (dest=0)  CODECOPY
    //   PUSH1 01 (size=1)  PUSH1 00 (offset=0)   RETURN   00 (runtime)
    let creation_bytecode = "0x6001600c60003960016000f300";

    let resp: serde_json::Value = client
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_sendTransaction",
            "params": [{
                "from": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
                "data": creation_bytecode,
                "gas": "0x100000"
            }],
            "id": 1
        }))
        .send()
        .await
        .expect("failed to send deploy tx to Anvil")
        .json()
        .await
        .expect("invalid JSON response from Anvil");

    let tx_hash = resp["result"]
        .as_str()
        .unwrap_or_else(|| {
            panic!("eth_sendTransaction failed: {}", serde_json::to_string_pretty(&resp).unwrap())
        })
        .to_string();

    // Poll for the receipt (Anvil auto-mines but there may be a brief delay).
    let receipt = loop {
        let resp: serde_json::Value = client
            .post(rpc_url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "eth_getTransactionReceipt",
                "params": [&tx_hash],
                "id": 2
            }))
            .send()
            .await
            .expect("failed to get receipt")
            .json()
            .await
            .expect("invalid JSON receipt");

        if !resp["result"].is_null() {
            break resp;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };

    let contract_address = receipt["result"]["contractAddress"]
        .as_str()
        .expect("receipt must have contractAddress")
        .to_string();

    (tx_hash, contract_address)
}

/// Call `anvil_setCode` to replace the bytecode at `address` with `code`.
async fn anvil_set_code(rpc_url: &str, address: &str, code: &str) {
    let client = reqwest::Client::new();
    let resp: serde_json::Value = client
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "anvil_setCode",
            "params": [address, code],
            "id": 1
        }))
        .send()
        .await
        .expect("failed to send anvil_setCode")
        .json()
        .await
        .expect("invalid JSON from anvil_setCode");
    assert!(resp.get("error").is_none(), "anvil_setCode failed: {resp}");
}

/// Register a deployment from a transaction hash in the requested namespace.
async fn register_deployment(
    tmp_path: std::path::PathBuf,
    tx_hash: String,
    rpc_url: String,
    namespace: &str,
) {
    let namespace = namespace.to_string();
    tokio::task::spawn_blocking(move || {
        treb()
            .args([
                "registry",
                "add",
                "--tx-hash",
                &tx_hash,
                "--rpc-url",
                &rpc_url,
                "--namespace",
                &namespace,
                "--skip-verify",
            ])
            .current_dir(&tmp_path)
            .assert()
            .success();
    })
    .await
    .unwrap();
}

// ── Tests ───────────────────────────────────────────────────────────────────

/// Prune with --check-onchain on a clean registry (all contracts have live
/// bytecode) should find zero candidates and leave the registry untouched.
#[tokio::test(flavor = "multi_thread")]
async fn e2e_prune_onchain_clean_registry() {
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();
    let tmp = setup_project().await;

    // Step 1: Deploy directly on Anvil and register so the contract is both
    //         in the registry AND on-chain with valid bytecode.
    let (tx_hash, _) = deploy_contract_on_anvil(&rpc_url).await;
    register_deployment(tmp.path().to_path_buf(), tx_hash, rpc_url.clone(), "default").await;
    assert_deployment_count(tmp.path().to_path_buf(), 1).await;

    // Step 2a: Prune human output with --check-onchain --dry-run → clean message.
    let prune_output = run_human(
        tmp.path().to_path_buf(),
        vec![
            "registry".into(),
            "prune".into(),
            "--check-onchain".into(),
            "--rpc-url".into(),
            rpc_url.clone(),
            "--network".into(),
            "31337".into(),
            "--dry-run".into(),
        ],
    )
    .await;
    assert!(
        prune_output.contains("✅ All registry entries are valid. Nothing to prune."),
        "clean prune must show '✅ All registry entries are valid. Nothing to prune.', got: {prune_output}"
    );

    // Step 2b: Prune with --json --check-onchain --dry-run → no candidates.
    let result = run_json(
        tmp.path().to_path_buf(),
        vec![
            "registry".into(),
            "prune".into(),
            "--check-onchain".into(),
            "--rpc-url".into(),
            rpc_url.clone(),
            "--network".into(),
            "31337".into(),
            "--dry-run".into(),
        ],
    )
    .await;

    // When no candidates are found, prune outputs {"candidates": []}.
    let candidates =
        result["candidates"].as_array().expect("prune --json must have 'candidates' array");
    assert!(
        candidates.is_empty(),
        "expected no prune candidates for clean registry, got: {candidates:?}"
    );

    // Step 3: Verify deployment is still intact.
    assert_deployment_count(tmp.path().to_path_buf(), 1).await;

    drop(anvil);
}

/// Prune with --check-onchain after zeroing a contract's bytecode via
/// anvil_setCode should detect the "destroyed" contract and remove it.
#[tokio::test(flavor = "multi_thread")]
async fn e2e_prune_detects_selfdestructed() {
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();
    let tmp = setup_project().await;

    // Step 1: Deploy directly on Anvil and register.
    let (tx_hash, contract_address) = deploy_contract_on_anvil(&rpc_url).await;
    register_deployment(tmp.path().to_path_buf(), tx_hash, rpc_url.clone(), "default").await;
    let deployments = assert_deployment_count(tmp.path().to_path_buf(), 1).await;
    let dep_id = deployments[0]["id"].as_str().expect("deployment must have id");

    // Step 2: Zero the bytecode at the contract address (simulate selfdestruct).
    anvil_set_code(&rpc_url, &contract_address, "0x").await;

    // Step 3a: Prune human dry-run to verify section listings and emoji messages.
    let prune_output = run_human(
        tmp.path().to_path_buf(),
        vec![
            "registry".into(),
            "prune".into(),
            "--check-onchain".into(),
            "--rpc-url".into(),
            rpc_url.clone(),
            "--network".into(),
            "31337".into(),
            "--dry-run".into(),
        ],
    )
    .await;
    assert!(
        prune_output.contains("🔍 Checking registry entries..."),
        "prune dry-run must show '🔍 Checking registry entries...', got: {prune_output}"
    );
    assert!(
        prune_output.contains("🗑️  Found 1 items to prune:"),
        "prune dry-run must show '🗑️  Found 1 items to prune:', got: {prune_output}"
    );
    assert!(
        prune_output.contains("Deployments ("),
        "prune dry-run must show 'Deployments (N):' section, got: {prune_output}"
    );

    // Step 3b: Prune with --check-onchain --yes → detect and remove (JSON).
    let result = run_json(
        tmp.path().to_path_buf(),
        vec![
            "registry".into(),
            "prune".into(),
            "--check-onchain".into(),
            "--rpc-url".into(),
            rpc_url.clone(),
            "--network".into(),
            "31337".into(),
            "--yes".into(),
        ],
    )
    .await;

    // Destructive prune outputs {removed: [...], backupPath: "..."}.
    let removed =
        result["removed"].as_array().expect("prune --yes --json must have 'removed' array");
    assert_eq!(removed.len(), 1, "expected 1 pruned deployment");
    assert_eq!(removed[0]["id"].as_str(), Some(dep_id));
    assert_eq!(removed[0]["kind"].as_str(), Some("DestroyedOnChain"));
    assert!(result["backupPath"].as_str().is_some(), "must include backupPath");

    // Step 4: Verify deployment removed from registry.
    assert_deployment_count(tmp.path().to_path_buf(), 0).await;

    drop(anvil);
}

/// Reset with --namespace removes only matching deployments and linked
/// transactions, leaving other namespaces intact.
#[tokio::test(flavor = "multi_thread")]
async fn e2e_reset_scoped_by_namespace() {
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();
    let tmp = setup_project().await;

    // Step 1: Seed two deployments in different namespaces.
    let (default_tx_hash, _) = deploy_contract_on_anvil(&rpc_url).await;
    register_deployment(tmp.path().to_path_buf(), default_tx_hash, rpc_url.clone(), "default")
        .await;
    let (staging_tx_hash, _) = deploy_contract_on_anvil(&rpc_url).await;
    register_deployment(tmp.path().to_path_buf(), staging_tx_hash, rpc_url.clone(), "staging")
        .await;
    assert_deployment_count(tmp.path().to_path_buf(), 2).await;

    let deployments_before = read_deployments(tmp.path());
    let transactions_before = read_transactions(tmp.path());

    // Step 2: Reset with a non-matching namespace → no files change.
    let reset_output = run_human(
        tmp.path().to_path_buf(),
        vec![
            "registry".into(),
            "drop".into(),
            "--namespace".into(),
            "nonexistent".into(),
            "--yes".into(),
        ],
    )
    .await;
    assert!(
        reset_output
            .contains("Nothing to drop. No registry entries found matching the given filters."),
        "non-matching namespace drop must show the full empty-state message, got: {reset_output}"
    );
    assert_eq!(
        read_deployments(tmp.path()),
        deployments_before,
        "deployments.json should be unchanged for a non-matching namespace reset"
    );
    assert_eq!(
        read_transactions(tmp.path()),
        transactions_before,
        "transactions.json should be unchanged for a non-matching namespace reset"
    );

    // Step 3: Reset the default namespace only.
    let result = run_json(
        tmp.path().to_path_buf(),
        vec![
            "registry".into(),
            "drop".into(),
            "--namespace".into(),
            "default".into(),
            "--yes".into(),
        ],
    )
    .await;
    assert_eq!(
        result["removedDeployments"].as_u64(),
        Some(1),
        "matching namespace should remove 1 deployment"
    );
    assert_eq!(
        result["removedTransactions"].as_u64(),
        Some(1),
        "matching namespace should remove only the linked transaction"
    );
    assert!(result["backupPath"].as_str().is_some(), "must include backupPath");

    // Step 4: Verify the staging deployment and its transaction linkage survive.
    let deps_after = read_deployments(tmp.path());
    let txs_after = read_transactions(tmp.path());
    let deps_after_obj =
        deps_after.as_object().expect("deployments.json must be an object after namespace reset");
    let txs_after_obj =
        txs_after.as_object().expect("transactions.json must be an object after namespace reset");
    assert_eq!(deps_after_obj.len(), 1, "exactly one deployment should remain");
    assert_eq!(txs_after_obj.len(), 1, "exactly one transaction should remain");

    let remaining_dep = deps_after_obj.values().next().expect("remaining deployment must exist");
    assert_eq!(remaining_dep["namespace"].as_str(), Some("staging"));
    let remaining_dep_id = remaining_dep["id"].as_str().expect("remaining deployment must have id");
    let remaining_tx_id = remaining_dep["transactionId"]
        .as_str()
        .expect("remaining deployment must reference a transaction");
    let remaining_tx = txs_after_obj
        .get(remaining_tx_id)
        .expect("remaining deployment should keep a live transaction reference");
    assert_eq!(remaining_tx["deployments"].as_array().map(Vec::len), Some(1));
    assert_eq!(remaining_tx["deployments"][0].as_str(), Some(remaining_dep_id));

    drop(anvil);
}

/// Full create → destroy → recreate lifecycle: deploy, reset, redeploy.
///
/// NOTE: The broadcast pipeline's v2 transaction IDs don't match the deployment's
/// transactionId (Solidity emits sequential counters, Rust generates hash-based IDs).
/// As a result, scoped `drop --namespace` cannot cascade to the unlinked transaction,
/// so transactions accumulate across deploy-reset-redeploy cycles.
#[tokio::test(flavor = "multi_thread")]
async fn e2e_deploy_reset_redeploy() {
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();
    let tmp = setup_project().await;

    // Step 1: Initial deploy.
    run_deployment(tmp.path().to_path_buf(), rpc_url.clone()).await;
    assert_deployment_count(tmp.path().to_path_buf(), 1).await;

    // Verify registry has 1 deployment and 1 transaction.
    assert_eq!(read_deployments(tmp.path()).as_object().unwrap().len(), 1);
    assert_eq!(read_transactions(tmp.path()).as_object().unwrap().len(), 1);

    // Step 2a: Reset human output check.
    // Scoped drop only removes the deployment (1 item) — the transaction survives
    // because its `deployments` list is empty (unlinked due to v2 ID mismatch).
    let reset_output = run_human(
        tmp.path().to_path_buf(),
        vec![
            "registry".into(),
            "drop".into(),
            "--namespace".into(),
            "default".into(),
            "--yes".into(),
        ],
    )
    .await;
    assert!(
        reset_output.contains("Successfully dropped 1 items from the registry."),
        "drop must show 'Successfully dropped 1 items from the registry.', got: {reset_output}"
    );
    assert_deployment_count(tmp.path().to_path_buf(), 0).await;

    // Verify deployment removed; transaction survives (unlinked, empty deployments list).
    assert_eq!(read_deployments(tmp.path()).as_object().unwrap().len(), 0);
    assert_eq!(
        read_transactions(tmp.path()).as_object().unwrap().len(),
        1,
        "unlinked transaction survives scoped drop"
    );

    // Step 3: Redeploy the same script — create-destroy-recreate cycle.
    run_deployment(tmp.path().to_path_buf(), rpc_url.clone()).await;
    assert_deployment_count(tmp.path().to_path_buf(), 1).await;

    // Step 4: Verify registry files after redeploy (1 dep + 1 tx).
    // The second deploy generates the same hash-based transaction ID (same script
    // path + index), so it overwrites the orphan from the first deploy.
    let deps = read_deployments(tmp.path());
    assert_eq!(deps.as_object().unwrap().len(), 1, "must have 1 deployment after redeploy");
    let txns = read_transactions(tmp.path());
    assert_eq!(txns.as_object().unwrap().len(), 1, "must have 1 transaction after redeploy");

    drop(anvil);
}
