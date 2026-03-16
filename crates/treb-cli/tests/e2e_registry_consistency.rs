//! P16-US-006: Cross-Command Registry Consistency Assertions.
//!
//! Tests that verify registry file invariants (cross-references between
//! deployments.json, lookup.json, transactions.json) hold after complex
//! multi-command sequences, ensuring internal data integrity.

mod e2e;

use e2e::{
    assert_deployment_count, assert_registry_consistent, get_deployment_id, read_deployments,
    read_registry_file, read_transactions, run_deployment, run_json, setup_project,
    spawn_anvil_or_skip, treb,
};

// ── Tests ─────────────────────────────────────────────────────────────────────

/// After a deployment, lookup.json secondary indexes (byName, byAddress) must
/// match deployments.json, whose object keys are the canonical deployment IDs,
/// and transactions.json must link back to the deployment.
#[tokio::test(flavor = "multi_thread")]
async fn e2e_registry_consistency_after_deployment() {
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();
    let tmp = setup_project().await;

    // Deploy
    run_deployment(tmp.path().to_path_buf(), rpc_url).await;
    assert_deployment_count(tmp.path().to_path_buf(), 1).await;

    // Verify lookup.json ↔ deployments.json cross-references
    assert_registry_consistent(tmp.path());

    // Verify transactions.json ↔ deployments.json cross-references
    let deps = read_deployments(tmp.path());
    let deps_obj = deps.as_object().unwrap();
    let txns = read_transactions(tmp.path());
    let txns_obj = txns.as_object().unwrap();

    for (dep_id, dep) in deps_obj {
        let tx_id = dep["transactionId"]
            .as_str()
            .unwrap_or_else(|| panic!("deployment {dep_id} missing transactionId"));
        assert!(
            txns_obj.contains_key(tx_id),
            "deployment {dep_id} references transaction {tx_id} not in transactions.json"
        );
        let tx = &txns_obj[tx_id];
        let tx_deps = tx["deployments"]
            .as_array()
            .unwrap_or_else(|| panic!("transaction {tx_id} missing deployments array"));
        assert!(
            tx_deps.iter().any(|d| d.as_str() == Some(dep_id)),
            "transaction {tx_id} does not back-reference deployment {dep_id}"
        );
    }

    // Verify lookup.json byName key matches contractName (lowercase)
    let lookup = read_registry_file(tmp.path(), "lookup.json");
    let by_name = lookup["byName"].as_object().unwrap();
    for (dep_id, dep) in deps_obj {
        let name = dep["contractName"].as_str().unwrap();
        let name_key = name.to_lowercase();
        let ids = by_name[&name_key].as_array().unwrap();
        assert!(ids.iter().any(|v| v.as_str() == Some(dep_id)));
    }

    // Verify lookup.json byAddress key matches address (lowercase)
    let by_address = lookup["byAddress"].as_object().unwrap();
    for (dep_id, dep) in deps_obj {
        let addr = dep["address"].as_str().unwrap();
        if !addr.is_empty() {
            let addr_key = addr.to_lowercase();
            assert_eq!(by_address[&addr_key].as_str(), Some(dep_id.as_str()));
        }
    }

    drop(anvil);
}

/// After tagging, lookup.json byTag index must contain correct tag-to-deployment
/// mappings, and removing a tag must update the index accordingly.
#[tokio::test(flavor = "multi_thread")]
async fn e2e_registry_consistency_after_tag() {
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();
    let tmp = setup_project().await;

    // Deploy and get the deployment ID
    run_deployment(tmp.path().to_path_buf(), rpc_url).await;
    let dep_id = get_deployment_id(tmp.path().to_path_buf()).await;

    // Add two tags
    let tmp_path = tmp.path().to_path_buf();
    let id = dep_id.clone();
    tokio::task::spawn_blocking(move || {
        treb().args(["tag", &id, "--add", "stable"]).current_dir(&tmp_path).assert().success();
    })
    .await
    .unwrap();

    let tmp_path = tmp.path().to_path_buf();
    let id = dep_id.clone();
    tokio::task::spawn_blocking(move || {
        treb().args(["tag", &id, "--add", "production"]).current_dir(&tmp_path).assert().success();
    })
    .await
    .unwrap();

    // Full consistency check after tagging
    assert_registry_consistent(tmp.path());

    // Verify byTag has both tags with the correct deployment ID
    let lookup = read_registry_file(tmp.path(), "lookup.json");
    let by_tag = lookup["byTag"].as_object().unwrap();
    assert_eq!(by_tag.len(), 2, "exactly 2 tags should exist");

    let stable_ids = by_tag["stable"].as_array().expect("stable tag must exist");
    assert!(stable_ids.iter().any(|v| v.as_str() == Some(&dep_id)));

    let prod_ids = by_tag["production"].as_array().expect("production tag must exist");
    assert!(prod_ids.iter().any(|v| v.as_str() == Some(&dep_id)));

    // Remove one tag
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

    // Consistency must still hold after tag removal
    assert_registry_consistent(tmp.path());

    // byTag should now have only "stable"
    let lookup = read_registry_file(tmp.path(), "lookup.json");
    let by_tag = lookup["byTag"].as_object().unwrap();
    assert!(
        by_tag.get("production").is_none()
            || by_tag["production"].as_array().is_none_or(|a| a.is_empty()),
        "production tag should be absent or empty after removal"
    );
    let stable_ids = by_tag["stable"].as_array().expect("stable tag must still exist");
    assert!(stable_ids.iter().any(|v| v.as_str() == Some(&dep_id)));

    drop(anvil);
}

/// After a full reset, all registry files must be empty/reset with valid
/// structure (empty objects/maps, no synthesized metadata file).
#[tokio::test(flavor = "multi_thread")]
async fn e2e_registry_consistency_after_reset() {
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();
    let tmp = setup_project().await;

    // Deploy to populate registry
    run_deployment(tmp.path().to_path_buf(), rpc_url).await;
    assert_deployment_count(tmp.path().to_path_buf(), 1).await;

    // Reset everything
    let result = run_json(tmp.path().to_path_buf(), vec!["reset".into(), "--yes".into()]).await;
    assert_eq!(result["removedDeployments"].as_u64(), Some(1));

    // Verify all registry files are empty/reset
    let deps = read_deployments(tmp.path());
    let deps_obj = deps.as_object().expect("deployments.json must be an object");
    assert!(deps_obj.is_empty(), "deployments.json must be empty after reset");

    let txns = read_transactions(tmp.path());
    let txns_obj = txns.as_object().expect("transactions.json must be an object");
    assert!(txns_obj.is_empty(), "transactions.json must be empty after reset");

    // Lookup index must be empty
    let lookup = read_registry_file(tmp.path(), "lookup.json");
    let by_name = lookup["byName"].as_object().expect("lookup must have byName");
    let by_address = lookup["byAddress"].as_object().expect("lookup must have byAddress");
    let by_tag = lookup["byTag"].as_object().expect("lookup must have byTag");
    assert!(by_name.is_empty(), "byName must be empty after reset");
    assert!(by_address.is_empty(), "byAddress must be empty after reset");
    assert!(by_tag.is_empty(), "byTag must be empty after reset");

    assert!(
        !tmp.path().join(".treb/registry.json").exists(),
        "reset must not recreate registry.json metadata"
    );

    // Consistency check passes trivially on empty registry
    assert_registry_consistent(tmp.path());

    drop(anvil);
}

/// After a fork enter → modify → exit cycle, fork.json must be clean (no active
/// forks) and the snapshot directory must be removed.
#[tokio::test(flavor = "multi_thread")]
async fn e2e_registry_consistency_after_fork_cycle() {
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();
    let tmp = setup_project().await;

    // Deploy before fork (ensures registry files exist for snapshot)
    run_deployment(tmp.path().to_path_buf(), rpc_url.clone()).await;
    assert_deployment_count(tmp.path().to_path_buf(), 1).await;
    let dep_id = get_deployment_id(tmp.path().to_path_buf()).await;

    // Verify pre-fork consistency
    assert_registry_consistent(tmp.path());

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

    // Modify registry during fork (add a tag)
    let tmp_path = tmp.path().to_path_buf();
    let id = dep_id.clone();
    tokio::task::spawn_blocking(move || {
        treb().args(["tag", &id, "--add", "fork-test"]).current_dir(&tmp_path).assert().success();
    })
    .await
    .unwrap();

    // Consistency should hold even during fork
    assert_registry_consistent(tmp.path());

    // Fork exit → restores pre-fork state
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

    // Post-exit consistency: lookup.json must match restored deployments.json
    assert_registry_consistent(tmp.path());

    // Fork.json must have no active forks
    let fork_state = read_registry_file(tmp.path(), "fork.json");
    let forks = fork_state["forks"].as_object().expect("forks must be an object");
    assert!(forks.is_empty(), "no active forks after exit");

    // History must contain both enter and exit
    let history = fork_state["history"].as_array().expect("history must be array");
    assert!(
        history.iter().any(|h| h["action"].as_str() == Some("enter")),
        "history must contain 'enter' action"
    );
    assert!(
        history.iter().any(|h| h["action"].as_str() == Some("exit")),
        "history must contain 'exit' action"
    );

    // Snapshot directory must be cleaned up
    let snapshot_dir = tmp.path().join(".treb").join("priv/snapshots").join("anvil-31337");
    assert!(!snapshot_dir.exists(), "snapshot directory must be removed after exit");

    // Verify the fork-time tag is gone (state was restored)
    let show_json = run_json(tmp.path().to_path_buf(), vec!["show".into(), dep_id.clone()]).await;
    let tags = &show_json["deployment"]["tags"];
    assert!(
        tags.is_null() || tags.as_array().is_none_or(|a| a.is_empty()),
        "fork-test tag must be gone after fork exit, got: {tags}"
    );

    drop(anvil);
}
