//! P16-US-003: Fork Mode E2E Workflow.
//!
//! E2E tests for the full fork lifecycle (enter → deploy → diff → revert → exit)
//! verifying fork mode properly isolates and restores registry state with live
//! Anvil execution.

mod e2e;

use std::{
    net::TcpListener,
    path::Path,
    process::{Child, Stdio},
    time::Duration,
};

use e2e::{
    assert_deployment_count, get_deployment_id, read_registry_file, run_deployment, run_json,
    setup_project, spawn_anvil_or_skip, treb,
};

struct BackgroundProcess {
    child: Child,
}

impl Drop for BackgroundProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn spawn_tracked_anvil(project_root: &Path, network: &str) -> (BackgroundProcess, String) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("failed to reserve an Anvil port");
    let port = listener.local_addr().expect("reserved listener must have a local addr").port();
    drop(listener);

    #[allow(deprecated)]
    let treb_bin = assert_cmd::cargo::cargo_bin("treb-cli");

    let child = std::process::Command::new(&treb_bin)
        .args(["dev", "anvil", "start", "--network", network, "--port", &port.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .current_dir(project_root)
        .spawn()
        .expect("failed to spawn `treb dev anvil start`");
    let process = BackgroundProcess { child };

    let addr = format!("127.0.0.1:{port}");
    let ready_deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        if tokio::net::TcpStream::connect(&addr).await.is_ok() {
            break;
        }
        if tokio::time::Instant::now() >= ready_deadline {
            panic!("tracked Anvil did not become reachable on {addr} within 60 seconds");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    (process, format!("http://127.0.0.1:{port}"))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Full fork lifecycle: enter → deploy → diff → revert → exit.
///
/// 11 verification steps covering registry state checks at each transition:
///  1. init → empty deployment baseline
///  2. fork enter → snapshot registry (0 deployments)
///  3. verify fork.json has active fork entry with correct chain ID
///  4. deploy during fork mode → create fork-only deployment
///  5. verify list --json shows the new deployment
///  6. fork diff --json → shows "added" deployment entry
///  7. fork revert → restores from snapshot (deployment removed)
///  8. verify deployment count returns to 0
///  9. verify fork.json still active + history has "enter" and "revert"
/// 10. fork exit → restores registry, removes fork entry, cleans snapshot
/// 11. verify no active forks, history has "exit", snapshot dir gone, count stays 0
#[tokio::test(flavor = "multi_thread")]
async fn e2e_fork_enter_deploy_diff_revert_exit() {
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
    run_deployment(tmp.path().to_path_buf(), rpc_url.clone()).await;

    // Step 5: verify list --json shows the new deployment
    let deployments = assert_deployment_count(tmp.path().to_path_buf(), 1).await;
    let dep_id = deployments[0]["id"].as_str().expect("deployment must have an id").to_string();

    // Step 6: fork diff → shows added deployment
    let diff = run_json(
        tmp.path().to_path_buf(),
        vec!["fork".into(), "diff".into(), "--network".into(), "anvil-31337".into()],
    )
    .await;
    assert_eq!(diff["network"].as_str(), Some("anvil-31337"));
    assert_eq!(diff["hasChanges"].as_bool(), Some(true), "diff must have changes after deployment");
    let new_deps = diff["newDeployments"].as_array().expect("newDeployments must be an array");
    assert!(
        new_deps.iter().any(|d| {
            d["changeType"].as_str() == Some("added") && d["id"].as_str() == Some(dep_id.as_str())
        }),
        "diff must show the fork-only deployment as added in newDeployments"
    );

    // Step 7: fork revert → restores from snapshot (deployment removed)
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

    // Step 8: verify deployment count returns to the pre-fork baseline
    assert_deployment_count(tmp.path().to_path_buf(), 0).await;

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
            .args(["tag", &id, "--add", "fork-only-tag"])
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

    let (tracked_anvil, expected_rpc_url) = spawn_tracked_anvil(tmp.path(), "anvil-31337").await;

    // Fork status --json
    let ready_deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let status = loop {
        let status = run_json(tmp.path().to_path_buf(), vec!["fork".into(), "status".into()]).await;
        let statuses = status.as_array().expect("fork status --json must be array");
        if statuses.len() == 1 && statuses[0]["rpcUrl"].as_str() == Some(expected_rpc_url.as_str())
        {
            break status;
        }
        if tokio::time::Instant::now() >= ready_deadline {
            panic!("fork status never reported rpcUrl {expected_rpc_url}");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    let statuses = status.as_array().expect("fork status --json must be array");
    assert_eq!(statuses.len(), 1, "exactly 1 active fork");

    let s = &statuses[0];
    assert_eq!(s["network"].as_str(), Some("anvil-31337"), "network must match");
    assert_eq!(s["chainId"].as_u64(), Some(31337), "chainId must be 31337");
    assert_eq!(s["rpcUrl"].as_str(), Some(expected_rpc_url.as_str()), "rpcUrl must match");
    assert_eq!(
        s["status"].as_str(),
        Some("running"),
        "status is running once the tracked Anvil is started"
    );

    drop(tracked_anvil);

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
