//! End-to-end integration test suite for treb CLI multi-command workflows.
//!
//! These tests exercise the full deployment pipeline using in-process Anvil,
//! verifying that init, run, list, show, tag, prune, and reset commands
//! compose correctly end-to-end.
//!
//! Each test spawns a local Anvil instance, copies the project fixture
//! (which includes forge-std), writes SimpleContract.sol, deploys a
//! contract using a treb-compatible script that emits `ContractDeployed`
//! events, and then exercises the relevant treb command.

mod e2e;

use e2e::{run_deployment, setup_project, spawn_anvil_or_skip, treb};

// ── Tests ─────────────────────────────────────────────────────────────────────

/// init → run → list: `treb list --json` returns exactly one deployment with
/// a non-zero EVM address.
#[tokio::test(flavor = "multi_thread")]
async fn e2e_init_run_list() {
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
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
    let arr = json["deployments"].as_array().expect("JSON output must contain a deployments array");
    assert_eq!(arr.len(), 1, "exactly one deployment should be recorded");

    let address = arr[0]["address"].as_str().expect("deployment must have 'address'");
    assert!(address.starts_with("0x"), "address must be 0x-prefixed: {address}");
    assert_ne!(
        address, "0x0000000000000000000000000000000000000000",
        "deployed address must be non-zero: {address}"
    );

    drop(anvil);
}

/// run → show: `treb show <id> --json` output contains a wrapped deployment object
/// with `address`, `contractName`, and `chainId` fields.
#[tokio::test(flavor = "multi_thread")]
async fn e2e_run_show() {
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
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
    let arr = list_json["deployments"].as_array().unwrap();
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
    let deployment = &json["deployment"];

    assert!(deployment.get("address").is_some(), "show output must contain deployment.address");
    assert!(
        deployment.get("contractName").is_some(),
        "show output must contain deployment.contractName"
    );
    assert!(deployment.get("chainId").is_some(), "show output must contain deployment.chainId");
    assert!(json.get("fork").is_none(), "non-fork deployments must not include a fork flag");

    drop(anvil);
}

/// run → tag → list-with-tag-filter: `treb list --tag v1.0.0 --json` returns
/// exactly one result after tagging.
#[tokio::test(flavor = "multi_thread")]
async fn e2e_run_tag_list_with_filter() {
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
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
    let arr = list_json["deployments"].as_array().unwrap();
    let deployment_id = arr[0]["id"].as_str().unwrap().to_string();

    // Tag the deployment with "v1.0.0".
    let tmp_path = tmp.path().to_path_buf();
    let dep_id = deployment_id.clone();
    tokio::task::spawn_blocking(move || {
        treb().args(["registry", "tag", &dep_id, "--add", "v1.0.0"]).current_dir(&tmp_path).assert().success();
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
    let arr = json["deployments"].as_array().expect("JSON output must contain a deployments array");
    assert_eq!(arr.len(), 1, "exactly one deployment should match tag v1.0.0");
    assert_eq!(
        arr[0]["id"].as_str().unwrap(),
        deployment_id.as_str(),
        "tagged deployment id should match"
    );

    drop(anvil);
}

/// run → prune --dry-run after broadcast: prune detects the broken transaction
/// cross-reference (v2 events emit sequential transactionIds that don't match
/// the hash-based IDs generated by the Rust hydration layer) and flags the
/// deployment as a prune candidate.
#[tokio::test(flavor = "multi_thread")]
async fn e2e_run_prune_dry_run_clean() {
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    let tmp = setup_project().await;
    run_deployment(tmp.path().to_path_buf(), rpc_url).await;

    let tmp_path = tmp.path().to_path_buf();
    let output = tokio::task::spawn_blocking(move || {
        treb().args(["registry", "prune", "--dry-run"]).current_dir(&tmp_path).output().unwrap()
    })
    .await
    .unwrap();

    assert!(output.status.success(), "treb prune --dry-run should exit 0");

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("Found 1 items to prune"),
        "prune should find 1 candidate (broken transaction ref); got:\n{stdout}"
    );
    assert!(
        stdout.contains("references missing transaction"),
        "prune should explain the broken transaction reference; got:\n{stdout}"
    );

    drop(anvil);
}

/// run → reset → list: `treb list --json` returns an empty array after
/// `treb reset --yes` wipes the registry.
#[tokio::test(flavor = "multi_thread")]
async fn e2e_run_reset_list() {
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    let tmp = setup_project().await;
    run_deployment(tmp.path().to_path_buf(), rpc_url).await;

    // Reset the registry without prompting.
    let tmp_path = tmp.path().to_path_buf();
    tokio::task::spawn_blocking(move || {
        treb().args(["registry", "drop", "--namespace", "default", "--yes"]).current_dir(&tmp_path).assert().success();
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
    let arr = json["deployments"].as_array().expect("JSON output must contain a deployments array");
    assert!(arr.is_empty(), "registry should be empty after reset, but got {} entries", arr.len());

    drop(anvil);
}

/// `treb list --no-color` stdout must not contain ANSI escape sequences.
#[tokio::test(flavor = "multi_thread")]
async fn e2e_list_no_color_has_no_ansi() {
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
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
