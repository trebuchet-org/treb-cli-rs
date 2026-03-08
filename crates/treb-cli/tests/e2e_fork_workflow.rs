//! P16-US-003: Fork Mode E2E Workflow.
//!
//! E2E tests for the full fork lifecycle (enter → deploy → diff → revert → exit)
//! verifying fork mode properly isolates and restores registry state with live
//! Anvil execution.

mod e2e;

use e2e::{
    assert_deployment_count, get_deployment_id, read_registry_file, run_deployment, run_json,
    setup_project, spawn_anvil_or_skip, treb,
};

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Full fork lifecycle: enter → deploy → diff → revert → exit.
///
/// 11 verification steps covering registry state checks at each transition:
///  1. init + deploy → 1 deployment baseline
///  2. fork enter → snapshot registry (1 deployment, no tags)
///  3. verify fork.json has active fork entry with correct chain ID
///  4. tag the deployment → modify registry during fork mode
///  5. verify deployment has tag via show --json
///  6. fork diff --json → shows "modified" deployment entry
///  7. fork revert → restores from snapshot (tag removed)
///  8. verify deployment tag is gone (restored)
///  9. verify fork.json still active + history has "enter" and "revert"
/// 10. fork exit → restores registry, removes fork entry, cleans snapshot
/// 11. verify no active forks, history has "exit", snapshot dir gone
#[tokio::test(flavor = "multi_thread")]
async fn e2e_fork_enter_deploy_diff_revert_exit() {
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    // Step 1: init + deploy → 1 deployment baseline
    let tmp = setup_project().await;
    run_deployment(tmp.path().to_path_buf(), rpc_url.clone()).await;
    let dep_id = get_deployment_id(tmp.path().to_path_buf()).await;

    // Step 2: fork enter (snapshots registry with 1 deployment)
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

    // Step 3: verify fork.json has active fork for "anvil-31337"
    let fork_state = read_registry_file(tmp.path(), "fork.json");
    let forks = fork_state["forks"].as_object().expect("forks must be an object");
    assert!(forks.contains_key("anvil-31337"), "fork entry must exist for anvil-31337");
    let fork_entry = &forks["anvil-31337"];
    assert_eq!(fork_entry["network"].as_str(), Some("anvil-31337"));
    assert_eq!(fork_entry["chainId"].as_u64(), Some(31337));

    // Step 4: tag the deployment during fork mode (modifies deployments.json)
    let tmp_path = tmp.path().to_path_buf();
    let id = dep_id.clone();
    tokio::task::spawn_blocking(move || {
        treb()
            .args(["tag", &id, "--add", "fork-tag"])
            .current_dir(&tmp_path)
            .assert()
            .success();
    })
    .await
    .unwrap();

    // Step 5: verify deployment has tag
    let show_json = run_json(tmp.path().to_path_buf(), vec!["show".into(), dep_id.clone()]).await;
    let tags = show_json["tags"].as_array().expect("tags must be array after tagging");
    assert!(tags.iter().any(|t| t.as_str() == Some("fork-tag")), "deployment must have fork-tag");

    // Step 6: fork diff → shows modified deployment
    let diff = run_json(
        tmp.path().to_path_buf(),
        vec!["fork".into(), "diff".into(), "--network".into(), "anvil-31337".into()],
    )
    .await;
    assert_eq!(diff["network"].as_str(), Some("anvil-31337"));
    assert_eq!(diff["clean"].as_bool(), Some(false), "diff must not be clean after modification");
    let changes = diff["changes"].as_array().expect("changes must be an array");
    assert!(
        changes.iter().any(|c| c["change"].as_str() == Some("modified")
            && c["file"].as_str() == Some("deployments")),
        "diff must show modified deployment"
    );

    // Step 7: fork revert → restores from snapshot (tag removed)
    let tmp_path = tmp.path().to_path_buf();
    tokio::task::spawn_blocking(move || {
        treb()
            .args(["fork", "revert", "--network", "anvil-31337"])
            .current_dir(&tmp_path)
            .assert()
            .success();
    })
    .await
    .unwrap();

    // Step 8: verify tag is gone (restored from snapshot)
    let show_json = run_json(tmp.path().to_path_buf(), vec!["show".into(), dep_id.clone()]).await;
    let tags = &show_json["tags"];
    assert!(
        tags.is_null() || tags.as_array().map_or(true, |a| a.is_empty()),
        "tags must be absent or empty after revert, got: {tags}"
    );

    // Step 9: verify fork.json still has active fork (revert ≠ exit)
    let fork_state = read_registry_file(tmp.path(), "fork.json");
    let forks = fork_state["forks"].as_object().expect("forks must be object");
    assert!(forks.contains_key("anvil-31337"), "fork entry must still exist after revert");
    let history = fork_state["history"].as_array().expect("history must be array");
    assert!(
        history.iter().any(|h| h["action"].as_str() == Some("enter")),
        "history must contain 'enter' action"
    );
    assert!(
        history.iter().any(|h| h["action"].as_str() == Some("revert")),
        "history must contain 'revert' action"
    );

    // Step 10: fork exit → restores registry, removes fork entry
    let tmp_path = tmp.path().to_path_buf();
    tokio::task::spawn_blocking(move || {
        treb()
            .args(["fork", "exit", "--network", "anvil-31337"])
            .current_dir(&tmp_path)
            .assert()
            .success();
    })
    .await
    .unwrap();

    // Step 11: verify no active forks, history has "exit", snapshot dir cleaned
    let fork_state = read_registry_file(tmp.path(), "fork.json");
    let forks = fork_state["forks"].as_object().expect("forks must be object");
    assert!(forks.is_empty(), "no active forks after exit");
    let history = fork_state["history"].as_array().expect("history must be array");
    assert!(
        history.iter().any(|h| h["action"].as_str() == Some("exit")),
        "history must contain 'exit' action"
    );
    let snapshot_dir = tmp.path().join(".treb").join("snapshots").join("anvil-31337");
    assert!(!snapshot_dir.exists(), "snapshot directory must be removed after exit");
    assert_deployment_count(tmp.path().to_path_buf(), 1).await;

    drop(anvil);
}

/// Exit without revert restores registry to pre-fork state.
///
/// Deploys, enters fork mode, tags the deployment (modifying registry),
/// then exits without reverting. Verifies the tag added during fork mode
/// is gone after exit.
#[tokio::test(flavor = "multi_thread")]
async fn e2e_fork_enter_deploy_exit_restores_state() {
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    let tmp = setup_project().await;
    run_deployment(tmp.path().to_path_buf(), rpc_url.clone()).await;
    let dep_id = get_deployment_id(tmp.path().to_path_buf()).await;

    // Fork enter (snapshots registry with 1 untagged deployment)
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

    // Tag deployment during fork mode (modifies registry)
    let tmp_path = tmp.path().to_path_buf();
    let id = dep_id.clone();
    tokio::task::spawn_blocking(move || {
        treb()
            .args(["tag", &id, "--add", "fork-only-tag"])
            .current_dir(&tmp_path)
            .assert()
            .success();
    })
    .await
    .unwrap();

    // Verify tag exists during fork
    let show_json = run_json(tmp.path().to_path_buf(), vec!["show".into(), dep_id.clone()]).await;
    let tags = show_json["tags"].as_array().expect("tags must be array");
    assert!(
        tags.iter().any(|t| t.as_str() == Some("fork-only-tag")),
        "deployment must have fork-only-tag during fork"
    );

    // Exit fork → should restore to pre-fork state (no tag)
    let tmp_path = tmp.path().to_path_buf();
    tokio::task::spawn_blocking(move || {
        treb()
            .args(["fork", "exit", "--network", "anvil-31337"])
            .current_dir(&tmp_path)
            .assert()
            .success();
    })
    .await
    .unwrap();

    // Registry should be restored to pre-fork state (1 deployment, no tags)
    assert_deployment_count(tmp.path().to_path_buf(), 1).await;
    let show_json = run_json(tmp.path().to_path_buf(), vec!["show".into(), dep_id.clone()]).await;
    let tags = &show_json["tags"];
    assert!(
        tags.is_null() || tags.as_array().map_or(true, |a| a.is_empty()),
        "tag must be gone after fork exit, got: {tags}"
    );

    // Fork state should be clean
    let fork_state = read_registry_file(tmp.path(), "fork.json");
    let forks = fork_state["forks"].as_object().expect("forks must be object");
    assert!(forks.is_empty(), "no active forks after exit");

    drop(anvil);
}

/// Fork status --json reports correct network and chain ID fields.
#[tokio::test(flavor = "multi_thread")]
async fn e2e_fork_status_shows_active_fork() {
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    let tmp = setup_project().await;

    // Fork enter
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

    // Fork status --json
    let status = run_json(tmp.path().to_path_buf(), vec!["fork".into(), "status".into()]).await;
    let statuses = status.as_array().expect("fork status --json must be array");
    assert_eq!(statuses.len(), 1, "exactly 1 active fork");

    let s = &statuses[0];
    assert_eq!(s["network"].as_str(), Some("anvil-31337"), "network must match");
    assert_eq!(s["chainId"].as_u64(), Some(31337), "chainId must be 31337");
    assert!(s.get("rpcUrl").is_some(), "rpcUrl field must be present");
    assert_eq!(
        s["status"].as_str(),
        Some("stopped"),
        "status is stopped (no tracked Anvil running)"
    );

    // Clean up: exit fork mode
    let tmp_path = tmp.path().to_path_buf();
    tokio::task::spawn_blocking(move || {
        treb()
            .args(["fork", "exit", "--network", "anvil-31337"])
            .current_dir(&tmp_path)
            .assert()
            .success();
    })
    .await
    .unwrap();

    drop(anvil);
}
