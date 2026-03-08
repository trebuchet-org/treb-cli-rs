//! P16-US-002: Basic Deployment Workflow with Full Assertions.
//!
//! Comprehensive E2E tests for the init → run → list → show → tag → list-with-filter
//! workflow, run JSON output field validation, and dry-run registry mutation checks.

mod e2e;

use e2e::{
    assert_deployment_count, run_deployment, run_json, setup_project, spawn_anvil_or_skip, treb,
};

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Full deployment lifecycle: init → run → list → show → tag add → list-with-tag →
/// list-with-nonexistent-tag → tag remove → show-verify-tags.
#[tokio::test(flavor = "multi_thread")]
async fn e2e_full_deployment_lifecycle() {
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    // Step 1: init (via setup_project)
    let tmp = setup_project().await;

    // Step 2: run (deploy SimpleContract)
    run_deployment(tmp.path().to_path_buf(), rpc_url).await;

    // Step 3: list → assert exactly 1 deployment
    let deployments = assert_deployment_count(tmp.path().to_path_buf(), 1).await;
    let dep_id = deployments[0]["id"].as_str().expect("deployment must have 'id'").to_string();
    let dep_address =
        deployments[0]["address"].as_str().expect("deployment must have 'address'").to_string();
    let dep_contract_name = deployments[0]["contractName"]
        .as_str()
        .expect("deployment must have 'contractName'")
        .to_string();
    let dep_chain_id = deployments[0]["chainId"].as_u64().expect("deployment must have 'chainId'");
    assert!(dep_address.starts_with("0x"), "address must be 0x-prefixed");

    // Step 4: show <id> --json → validate fields
    let show_json = run_json(tmp.path().to_path_buf(), vec!["show".into(), dep_id.clone()]).await;
    assert_eq!(show_json["id"].as_str().unwrap(), dep_id, "show id must match");
    assert_eq!(show_json["address"].as_str().unwrap(), dep_address, "show address must match");
    assert_eq!(
        show_json["contractName"].as_str().unwrap(),
        dep_contract_name,
        "show contractName must match"
    );
    assert_eq!(show_json["chainId"].as_u64().unwrap(), dep_chain_id, "show chainId must match");

    // Step 5: tag add "v1.0.0"
    let tmp_path = tmp.path().to_path_buf();
    let dep_id_clone = dep_id.clone();
    tokio::task::spawn_blocking(move || {
        treb()
            .args(["tag", &dep_id_clone, "--add", "v1.0.0"])
            .current_dir(&tmp_path)
            .assert()
            .success();
    })
    .await
    .unwrap();

    // Step 6: list --tag v1.0.0 --json → exactly 1 result
    let filtered =
        run_json(tmp.path().to_path_buf(), vec!["list".into(), "--tag".into(), "v1.0.0".into()])
            .await;
    let filtered_arr = filtered.as_array().expect("filtered list must be array");
    assert_eq!(filtered_arr.len(), 1, "exactly 1 deployment with tag v1.0.0");
    assert_eq!(
        filtered_arr[0]["id"].as_str().unwrap(),
        dep_id,
        "filtered deployment id must match"
    );

    // Step 7: list --tag nonexistent --json → 0 results
    let empty = run_json(
        tmp.path().to_path_buf(),
        vec!["list".into(), "--tag".into(), "nonexistent".into()],
    )
    .await;
    let empty_arr = empty.as_array().expect("empty list must be array");
    assert!(empty_arr.is_empty(), "no deployments should match nonexistent tag");

    // Step 8: tag remove "v1.0.0"
    let tmp_path = tmp.path().to_path_buf();
    let dep_id_clone = dep_id.clone();
    tokio::task::spawn_blocking(move || {
        treb()
            .args(["tag", &dep_id_clone, "--remove", "v1.0.0"])
            .current_dir(&tmp_path)
            .assert()
            .success();
    })
    .await
    .unwrap();

    // Step 9: show --json → verify tags absent or empty
    let show_after = run_json(tmp.path().to_path_buf(), vec!["show".into(), dep_id.clone()]).await;
    let tags = &show_after["tags"];
    assert!(
        tags.is_null() || tags.as_array().map_or(true, |a| a.is_empty()),
        "tags should be absent or empty after removal, got: {tags}"
    );

    drop(anvil);
}

/// Validate all RunOutputJson fields from `treb run --json`.
#[tokio::test(flavor = "multi_thread")]
async fn e2e_run_json_output_fields() {
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    let tmp = setup_project().await;

    // The shared JSON helper should work for `run` without stripping preamble text.
    let json = run_json(
        tmp.path().to_path_buf(),
        vec![
            "run".into(),
            "script/TrebDeploySimple.s.sol".into(),
            "--rpc-url".into(),
            rpc_url,
            "--broadcast".into(),
            "--non-interactive".into(),
        ],
    )
    .await;

    // Validate all expected RunOutputJson fields exist with correct types.
    assert_eq!(json["success"].as_bool(), Some(true), "success must be true");
    assert_eq!(json["dryRun"].as_bool(), Some(false), "dryRun must be false for broadcast run");

    let deployments = json["deployments"].as_array().expect("deployments must be an array");
    assert_eq!(deployments.len(), 1, "exactly 1 deployment");

    // Validate deployment fields.
    let dep = &deployments[0];
    assert!(dep["id"].is_string(), "deployment must have id");
    assert!(dep["contractName"].is_string(), "deployment must have contractName");
    assert!(dep["address"].is_string(), "deployment must have address");
    assert!(dep["namespace"].is_string(), "deployment must have namespace");
    assert!(dep["chainId"].is_u64(), "deployment must have chainId");
    assert!(dep["deploymentType"].is_string(), "deployment must have deploymentType");

    let transactions = json["transactions"].as_array().expect("transactions must be an array");
    assert!(!transactions.is_empty(), "must have at least 1 transaction");

    // Validate transaction fields.
    let tx = &transactions[0];
    assert!(tx["id"].is_string(), "transaction must have id");
    assert!(tx["hash"].is_string(), "transaction must have hash");
    assert!(tx["status"].is_string(), "transaction must have status");

    assert!(json["gasUsed"].is_u64(), "gasUsed must be a number");
    assert!(json["skipped"].is_array(), "skipped must be an array");
    assert!(json["consoleLogs"].is_array(), "consoleLogs must be an array");
    let governor_proposals =
        json["governorProposals"].as_array().expect("governorProposals must be an array");
    assert!(
        governor_proposals.is_empty(),
        "governorProposals must be empty for a standard deployer run"
    );

    drop(anvil);
}

/// Dry-run does not write state to deployments.json.
#[tokio::test(flavor = "multi_thread")]
async fn e2e_dry_run_no_registry_mutation() {
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    let tmp = setup_project().await;

    // Run with --dry-run — should simulate but not persist.
    let tmp_path = tmp.path().to_path_buf();
    let rpc = rpc_url.clone();
    tokio::task::spawn_blocking(move || {
        treb()
            .args([
                "run",
                "script/TrebDeploySimple.s.sol",
                "--rpc-url",
                &rpc,
                "--broadcast",
                "--dry-run",
                "--non-interactive",
            ])
            .current_dir(&tmp_path)
            .assert()
            .success();
    })
    .await
    .unwrap();

    // Verify deployments.json either doesn't exist or has no entries.
    let deployments_path = tmp.path().join(".treb").join("deployments.json");
    if deployments_path.exists() {
        let data: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&deployments_path).unwrap()).unwrap();
        let count = data.as_object().map_or(0, |m| m.len());
        assert_eq!(count, 0, "dry-run must not write deployments to registry");
    }
    // If deployments.json doesn't exist at all, that's also correct — no mutation occurred.

    drop(anvil);
}
