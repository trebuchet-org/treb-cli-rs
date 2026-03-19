//! P16-US-003: Fork Mode E2E Workflow.
//!
//! E2E tests for the full fork lifecycle (enter → deploy → revert → exit)
//! verifying fork mode properly isolates and restores registry state with live
//! Anvil execution.

mod e2e;

use std::time::Duration;

use e2e::{
    assert_deployment_count, get_deployment_id, read_registry_file, run_deployment, run_json,
    setup_project, spawn_anvil_or_skip, treb,
};

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Full fork lifecycle: enter → deploy → revert → exit.
///
/// 10 verification steps covering registry state checks at each transition:
///  1. init → empty deployment baseline
///  2. fork enter → snapshot registry (0 deployments)
///  3. verify fork.json has active fork entry with correct chain ID
///  4. deploy during fork mode → create fork-only deployment
///  5. verify list --json shows the new deployment
///  6. fork revert → restores from snapshot (deployment removed)
///  7. verify deployment count returns to 0
///  8. verify fork.json still active + history has "enter" and "revert"
///  9. fork exit → restores registry, removes fork entry, cleans snapshot
/// 10. verify no active forks, history has "exit", snapshot dir gone, count stays 0
#[tokio::test(flavor = "multi_thread")]
async fn e2e_fork_enter_deploy_revert_exit() {
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    // Step 1: init → empty deployment baseline
    let tmp = setup_project().await;
    assert_deployment_count(tmp.path().to_path_buf(), 0).await;

    // Step 2: fork enter (snapshots registry with 0 deployments)
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

    // Step 4: deploy during fork mode (adds a fork-only deployment)
    // Pass --network so treb run detects the active fork and creates a run snapshot.
    let tmp_path = tmp.path().to_path_buf();
    let rpc = rpc_url.clone();
    tokio::task::spawn_blocking(move || {
        treb()
            .args([
                "run",
                "script/TrebDeploySimple.s.sol",
                "--rpc-url",
                &rpc,
                "--network",
                "anvil-31337",
                "--broadcast",
                "--non-interactive",
            ])
            .current_dir(&tmp_path)
            .assert()
            .success();
    })
    .await
    .unwrap();

    // Step 5: verify list --json shows the new deployment
    assert_deployment_count(tmp.path().to_path_buf(), 1).await;

    // Step 6: fork revert → restores from snapshot (deployment removed)
    let tmp_path = tmp.path().to_path_buf();
    tokio::task::spawn_blocking(move || {
        treb()
            .args(["fork", "revert"])
            .current_dir(&tmp_path)
            .assert()
            .success();
    })
    .await
    .unwrap();

    // Step 7: verify deployment count returns to the pre-fork baseline
    assert_deployment_count(tmp.path().to_path_buf(), 0).await;

    // Step 8: verify fork.json still has active fork (revert ≠ exit)
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

    // Step 9: fork exit → restores registry, removes fork entry
    let tmp_path = tmp.path().to_path_buf();
    tokio::task::spawn_blocking(move || {
        treb()
            .args(["fork", "exit"])
            .current_dir(&tmp_path)
            .assert()
            .success();
    })
    .await
    .unwrap();

    // Step 10: verify no active forks, history has "exit", snapshot dir cleaned
    let fork_state = read_registry_file(tmp.path(), "fork.json");
    let forks = fork_state["forks"].as_object().expect("forks must be object");
    assert!(forks.is_empty(), "no active forks after exit");
    let history = fork_state["history"].as_array().expect("history must be array");
    assert!(
        history.iter().any(|h| h["action"].as_str() == Some("exit")),
        "history must contain 'exit' action"
    );
    let snapshot_dir = tmp.path().join(".treb").join("priv/snapshots").join("holistic");
    assert!(!snapshot_dir.exists(), "snapshot directory must be removed after exit");
    assert_deployment_count(tmp.path().to_path_buf(), 0).await;

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
            .args(["registry", "tag", &id, "--add", "fork-only-tag"])
            .current_dir(&tmp_path)
            .assert()
            .success();
    })
    .await
    .unwrap();

    // Verify tag exists during fork
    let show_json = run_json(tmp.path().to_path_buf(), vec!["show".into(), dep_id.clone()]).await;
    let tags = show_json["deployment"]["tags"].as_array().expect("tags must be array");
    assert!(
        tags.iter().any(|t| t.as_str() == Some("fork-only-tag")),
        "deployment must have fork-only-tag during fork"
    );
    assert!(
        show_json.get("fork").is_none(),
        "non-fork namespaces must not include fork=true during fork mode"
    );

    // Exit fork → should restore to pre-fork state (no tag)
    let tmp_path = tmp.path().to_path_buf();
    tokio::task::spawn_blocking(move || {
        treb()
            .args(["fork", "exit"])
            .current_dir(&tmp_path)
            .assert()
            .success();
    })
    .await
    .unwrap();

    // Registry should be restored to pre-fork state (1 deployment, no tags)
    assert_deployment_count(tmp.path().to_path_buf(), 1).await;
    let show_json = run_json(tmp.path().to_path_buf(), vec!["show".into(), dep_id.clone()]).await;
    let tags = &show_json["deployment"]["tags"];
    assert!(
        tags.is_null() || tags.as_array().is_none_or(|a| a.is_empty()),
        "tag must be gone after fork exit, got: {tags}"
    );

    // Fork state should be clean
    let fork_state = read_registry_file(tmp.path(), "fork.json");
    let forks = fork_state["forks"].as_object().expect("forks must be object");
    assert!(forks.is_empty(), "no active forks after exit");

    drop(anvil);
}

/// Fork status --json reports the concrete network, chain ID, and rpcUrl values.
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

    // Fork status --json — poll until the spawned Anvil is reported as running
    let ready_deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let status = loop {
        let status = run_json(tmp.path().to_path_buf(), vec!["fork".into(), "status".into()]).await;
        let forks = status["forks"].as_array().expect("fork status --json must have forks array");
        if forks.len() == 1 && forks[0]["status"].as_str() == Some("running") {
            break status;
        }
        if tokio::time::Instant::now() >= ready_deadline {
            panic!("fork status never reported a running fork within deadline");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    };

    assert_eq!(status["active"].as_bool(), Some(true), "fork mode must be active");

    let forks = status["forks"].as_array().expect("fork status --json must have forks array");
    assert_eq!(forks.len(), 1, "exactly 1 active fork");

    let s = &forks[0];
    assert_eq!(s["network"].as_str(), Some("anvil-31337"), "network must match");
    assert_eq!(s["chainId"].as_u64(), Some(31337), "chainId must be 31337");
    assert!(s["rpcUrl"].as_str().is_some(), "rpcUrl must be set");
    assert_eq!(
        s["status"].as_str(),
        Some("running"),
        "status must be running"
    );

    // Clean up: exit fork mode
    let tmp_path = tmp.path().to_path_buf();
    tokio::task::spawn_blocking(move || {
        treb()
            .args(["fork", "exit"])
            .current_dir(&tmp_path)
            .assert()
            .success();
    })
    .await
    .unwrap();

    drop(anvil);
}
