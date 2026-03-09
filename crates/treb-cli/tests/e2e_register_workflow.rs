//! P16-US-004: Register and Tag E2E Workflow.
//!
//! E2E tests for the register → tag → show workflow, verifying that `treb register`
//! correctly imports deployments by transaction hash and that registered deployments
//! can be tagged and queried.

mod e2e;

use e2e::{assert_deployment_count, run_human, run_json, setup_project, spawn_anvil_or_skip, treb};

/// Deploy a minimal contract directly on Anvil via `eth_sendTransaction` and
/// return `(tx_hash, contract_address)`.
///
/// Uses Anvil's default funded account (0xf39F...92266) to send a contract
/// creation transaction with minimal bytecode.  Anvil auto-mines, so the
/// transaction is included in a block immediately.
async fn deploy_contract_on_anvil(rpc_url: &str) -> (String, String) {
    let client = reqwest::Client::new();

    // Minimal creation bytecode: deploys a contract with 1-byte runtime (STOP).
    let creation_bytecode = "0x6001600a600039600160006000f300";

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

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Register a deployment from a real transaction hash and verify it appears
/// in list output with the correct address.
#[tokio::test(flavor = "multi_thread")]
async fn e2e_register_from_tx_hash() {
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    let tmp = setup_project().await;

    // Deploy a contract directly on Anvil (bypassing treb run).
    let (tx_hash, expected_address) = deploy_contract_on_anvil(&rpc_url).await;

    // Register the deployment from the transaction hash and verify human output.
    let register_output = run_human(
        tmp.path().to_path_buf(),
        vec![
            "register".into(),
            "--tx-hash".into(),
            tx_hash.clone(),
            "--rpc-url".into(),
            rpc_url.clone(),
            "--skip-verify".into(),
        ],
    )
    .await;
    assert!(
        register_output.contains("✓ Successfully registered"),
        "register output must contain '✓ Successfully registered', got: {register_output}"
    );
    assert!(
        register_output.contains("Deployment ID:"),
        "register output must contain 'Deployment ID:', got: {register_output}"
    );

    // Verify deployment appears in list.
    let deployments = assert_deployment_count(tmp.path().to_path_buf(), 1).await;
    let dep_address =
        deployments[0]["address"].as_str().expect("deployment must have address").to_string();

    // Address must match (case-insensitive for checksummed vs lowercase).
    assert_eq!(
        dep_address.to_lowercase(),
        expected_address.to_lowercase(),
        "registered deployment address must match the originally deployed contract"
    );

    drop(anvil);
}

/// Full tag lifecycle on a registered deployment: add 3 tags, verify all present,
/// remove 1, verify 2 remain.
#[tokio::test(flavor = "multi_thread")]
async fn e2e_register_tag_show_roundtrip() {
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    let tmp = setup_project().await;

    // Deploy and register a contract, verifying human output.
    let (tx_hash, _) = deploy_contract_on_anvil(&rpc_url).await;

    let register_output = run_human(
        tmp.path().to_path_buf(),
        vec![
            "register".into(),
            "--tx-hash".into(),
            tx_hash.clone(),
            "--rpc-url".into(),
            rpc_url.clone(),
            "--skip-verify".into(),
        ],
    )
    .await;
    assert!(
        register_output.contains("✓ Successfully registered"),
        "register output must contain '✓ Successfully registered', got: {register_output}"
    );

    // Get the registered deployment ID.
    let dep_id = {
        let json = run_json(tmp.path().to_path_buf(), vec!["list".into()]).await;
        let arr = json["deployments"].as_array().expect("list must contain deployments array");
        assert_eq!(arr.len(), 1, "exactly 1 registered deployment");
        arr[0]["id"].as_str().expect("deployment must have id").to_string()
    };

    // Add 3 tags.
    for tag in &["stable", "production", "v2.0.0"] {
        let tmp_path = tmp.path().to_path_buf();
        let id = dep_id.clone();
        let t = tag.to_string();
        tokio::task::spawn_blocking(move || {
            treb().args(["tag", &id, "--add", &t]).current_dir(&tmp_path).assert().success();
        })
        .await
        .unwrap();
    }

    // Verify all 3 tags present via show --json.
    let show = run_json(tmp.path().to_path_buf(), vec!["show".into(), dep_id.clone()]).await;
    let tags =
        show["deployment"]["tags"].as_array().expect("tags must be array after adding 3 tags");
    assert_eq!(tags.len(), 3, "must have 3 tags");
    let tag_strs: Vec<&str> = tags.iter().map(|t| t.as_str().unwrap()).collect();
    assert!(tag_strs.contains(&"stable"), "must have 'stable' tag");
    assert!(tag_strs.contains(&"production"), "must have 'production' tag");
    assert!(tag_strs.contains(&"v2.0.0"), "must have 'v2.0.0' tag");

    // Remove 1 tag ("production").
    let tmp_path = tmp.path().to_path_buf();
    let id = dep_id.clone();
    tokio::task::spawn_blocking(move || {
        treb()
            .args(["tag", &id, "--remove", "production"])
            .current_dir(&tmp_path)
            .assert()
            .success();
    })
    .await
    .unwrap();

    // Verify 2 tags remain.
    let show = run_json(tmp.path().to_path_buf(), vec!["show".into(), dep_id.clone()]).await;
    let tags = show["deployment"]["tags"].as_array().expect("tags must be array after removal");
    assert_eq!(tags.len(), 2, "must have 2 tags after removing one");
    let tag_strs: Vec<&str> = tags.iter().map(|t| t.as_str().unwrap()).collect();
    assert!(tag_strs.contains(&"stable"), "'stable' must still be present");
    assert!(tag_strs.contains(&"v2.0.0"), "'v2.0.0' must still be present");
    assert!(!tag_strs.contains(&"production"), "'production' must be gone");

    drop(anvil);
}
