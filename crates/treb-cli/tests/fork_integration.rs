//! Integration tests for the `treb fork` and `treb dev anvil` CLI workflows.
//!
//! Tests that do not require a live Anvil node use pre-populated `.treb/`
//! directories.  Tests that require an RPC endpoint (e.g. `fork enter`) spawn
//! a local Anvil instance in-process and pass its URL to the binary.

use assert_cmd::cargo::cargo_bin_cmd;
use chrono::Utc;
use predicates::prelude::*;
use std::{
    fs,
    path::{Path, PathBuf},
};
use tempfile::TempDir;
use treb_config::{LocalConfig, save_local_config};
use treb_core::types::fork::{ForkEntry, ForkHistoryEntry};
use treb_registry::{DEPLOYMENTS_FILE, FORK_STATE_FILE, ForkStateStore};

// ── helpers ───────────────────────────────────────────────────────────────────

fn treb() -> assert_cmd::Command {
    cargo_bin_cmd!("treb-cli")
}

/// Create a temporary project directory with an empty `.treb/` sub-directory.
fn make_project() -> (TempDir, PathBuf) {
    let root = TempDir::new().unwrap();
    let treb_dir = root.path().join(".treb");
    fs::create_dir_all(&treb_dir).unwrap();
    (root, treb_dir)
}

/// Build a minimal `ForkEntry` suitable for unit-testing fork state operations.
fn sample_entry(treb_dir: &Path, network: &str) -> ForkEntry {
    let snapshot_dir = treb_dir.join("snapshots").join(network);
    let now = Utc::now();
    ForkEntry {
        network: network.to_string(),
        instance_name: None,
        rpc_url: String::new(),
        port: 0,
        chain_id: 31337,
        fork_url: String::new(),
        fork_block_number: None,
        snapshot_dir: snapshot_dir.to_string_lossy().into_owned(),
        started_at: now,
        env_var_name: String::new(),
        original_rpc: String::new(),
        anvil_pid: 0,
        pid_file: String::new(),
        log_file: String::new(),
        entered_at: now,
        snapshots: vec![],
    }
}

/// Spawn Anvil for integration tests, skipping when process forking is blocked
/// by the current execution environment.
async fn spawn_anvil_or_skip() -> Option<treb_forge::AnvilInstance> {
    match treb_forge::anvil::AnvilConfig::new().port(0).spawn().await {
        Ok(anvil) => Some(anvil),
        Err(err) if err.to_string().contains("Operation not permitted") => None,
        Err(err) => panic!("failed to spawn Anvil: {err}"),
    }
}

// ── fork status with no forks ─────────────────────────────────────────────────

/// `treb fork status` should report "No active forks" when the fork-state file
/// is absent or empty.
#[test]
fn fork_status_with_no_forks() {
    let (root, _treb_dir) = make_project();

    treb()
        .args(["fork", "status"])
        .current_dir(root.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("No active forks"));
}

// ── fork status --json outputs valid JSON ─────────────────────────────────────

/// `treb fork status --json` should emit a valid JSON array even when there are
/// active fork entries (status field is port-reachability-dependent but the
/// schema should always be present).
#[test]
fn fork_status_json_outputs_valid_json() {
    let (root, treb_dir) = make_project();

    // Pre-populate fork state with one active entry.
    let mut entry = sample_entry(&treb_dir, "mainnet");
    entry.rpc_url = "http://127.0.0.1:9999".into();
    entry.port = 9999;
    entry.chain_id = 1;

    let mut store = ForkStateStore::new(&treb_dir);
    store.insert_active_fork(entry).unwrap();

    let output = treb()
        .args(["fork", "status", "--json"])
        .current_dir(root.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json_str = String::from_utf8(output).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&json_str).expect("fork status --json must emit valid JSON");

    let arr = parsed.as_array().expect("fork status --json must emit a JSON array");
    assert_eq!(arr.len(), 1, "one fork entry should be present");
    assert_eq!(arr[0]["network"], "mainnet");
    assert_eq!(arr[0]["port"], 9999);
    assert_eq!(arr[0]["chainId"], 1);
}

/// `treb fork status` should fall back to the stored upstream fork URL when no
/// local Anvil runtime exists yet, and should mark the configured network as
/// current.
#[test]
fn fork_status_uses_session_fork_url_and_marks_current_network() {
    let (root, treb_dir) = make_project();

    let mut entry = sample_entry(&treb_dir, "mainnet");
    entry.chain_id = 1;
    entry.fork_url = "https://eth.example.com".into();

    let mut store = ForkStateStore::new(&treb_dir);
    store.insert_active_fork(entry).unwrap();

    save_local_config(
        root.path(),
        &LocalConfig { namespace: "default".into(), network: "mainnet".into() },
    )
    .unwrap();

    treb()
        .args(["fork", "status"])
        .current_dir(root.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("  mainnet (current)"))
        .stdout(predicate::str::contains("Fork URL:     https://eth.example.com"));
}

// ── fork history with empty history ───────────────────────────────────────────

/// `treb fork history` should report "No fork history" when the history list
/// is empty (either because no forks have been entered or because the file does
/// not exist yet).
#[test]
fn fork_history_with_empty_history() {
    let (root, _treb_dir) = make_project();

    treb()
        .args(["fork", "history"])
        .current_dir(root.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("No fork history"));
}

// ── fork diff with no changes ─────────────────────────────────────────────────

/// `treb fork diff` should report "No changes detected" when the current
/// registry matches the snapshot exactly.
#[test]
fn fork_diff_with_no_changes() {
    let (root, treb_dir) = make_project();

    let network = "testnet";
    let snapshot_dir = treb_dir.join("snapshots").join(network);
    fs::create_dir_all(&snapshot_dir).unwrap();

    // Write identical deployments.json in both locations.
    let deployments_json = r#"{"Counter_1": {"address": "0xaaaa"}}"#;
    fs::write(treb_dir.join(DEPLOYMENTS_FILE), deployments_json).unwrap();
    fs::write(snapshot_dir.join(DEPLOYMENTS_FILE), deployments_json).unwrap();

    // Pre-populate fork state pointing to the snapshot dir.
    let now = Utc::now();
    let entry = ForkEntry {
        network: network.to_string(),
        instance_name: None,
        rpc_url: String::new(),
        port: 0,
        chain_id: 31337,
        fork_url: String::new(),
        fork_block_number: None,
        snapshot_dir: snapshot_dir.to_string_lossy().into_owned(),
        started_at: now,
        env_var_name: String::new(),
        original_rpc: String::new(),
        anvil_pid: 0,
        pid_file: String::new(),
        log_file: String::new(),
        entered_at: now,
        snapshots: vec![],
    };
    let mut store = ForkStateStore::new(&treb_dir);
    store.insert_active_fork(entry).unwrap();

    treb()
        .args(["fork", "diff", "--network", network])
        .current_dir(root.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("No changes detected"));
}

// ── fork exit restores registry ───────────────────────────────────────────────

/// `treb fork exit` should restore `deployments.json` from the snapshot and
/// remove the active fork entry from `fork.json`.
#[test]
fn fork_exit_restores_registry() {
    let (root, treb_dir) = make_project();

    let network = "testnet";
    let snapshot_dir = treb_dir.join("snapshots").join(network);
    fs::create_dir_all(&snapshot_dir).unwrap();

    // Write original deployments to both the registry and the snapshot.
    let original = r#"{"Counter_1": {"address": "0xaaaa"}}"#;
    fs::write(treb_dir.join(DEPLOYMENTS_FILE), original).unwrap();
    fs::write(snapshot_dir.join(DEPLOYMENTS_FILE), original).unwrap();

    // Pre-populate fork state.
    let now = Utc::now();
    let entry = ForkEntry {
        network: network.to_string(),
        instance_name: None,
        rpc_url: String::new(),
        port: 0,
        chain_id: 31337,
        fork_url: String::new(),
        fork_block_number: None,
        snapshot_dir: snapshot_dir.to_string_lossy().into_owned(),
        started_at: now,
        env_var_name: String::new(),
        original_rpc: String::new(),
        anvil_pid: 0,
        pid_file: String::new(),
        log_file: String::new(),
        entered_at: now,
        snapshots: vec![],
    };
    let mut store = ForkStateStore::new(&treb_dir);
    store.insert_active_fork(entry).unwrap();

    // Simulate a deployment during the fork session by modifying the registry.
    let modified = r#"{"Counter_1": {"address": "0xaaaa"}, "Token_2": {"address": "0xbbbb"}}"#;
    fs::write(treb_dir.join(DEPLOYMENTS_FILE), modified).unwrap();

    // Exit fork mode — should restore registry from snapshot.
    treb().args(["fork", "exit", "--network", network]).current_dir(root.path()).assert().success();

    // Registry should be back to the original state.
    let restored = fs::read_to_string(treb_dir.join(DEPLOYMENTS_FILE)).unwrap();
    assert_eq!(
        restored, original,
        "deployments.json should be restored from the snapshot after fork exit"
    );

    // Snapshot directory should have been removed.
    assert!(!snapshot_dir.exists(), "snapshot directory should be removed after fork exit");

    // Fork state should no longer contain the "testnet" entry.
    let mut store2 = ForkStateStore::new(&treb_dir);
    store2.load().unwrap();
    assert!(
        store2.get_active_fork(network).is_none(),
        "active fork entry should be removed after fork exit"
    );

    // History should contain an "exit" entry.
    assert!(
        store2.data().history.iter().any(|h| h.action == "exit" && h.network == network),
        "fork-state history should contain an 'exit' entry for '{network}'"
    );
}

// ── fork enter creates state and snapshot ─────────────────────────────────────

/// `treb fork enter` should create a fork.json entry and a snapshot
/// directory.  This test spawns a local Anvil node so that the chain-ID fetch
/// succeeds without requiring external network access.
#[tokio::test(flavor = "multi_thread")]
async fn fork_enter_creates_state_and_snapshot() {
    // Spawn a local Anvil so the binary can call eth_chainId.
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    let (root, treb_dir) = make_project();
    let network = "testnet";

    // Run `treb fork enter` — should snapshot registry and write fork state.
    tokio::task::spawn_blocking({
        let root_path = root.path().to_path_buf();
        let rpc_url = rpc_url.clone();
        let network = network.to_string();
        move || {
            treb()
                .args(["fork", "enter", "--network", &network, "--rpc-url", &rpc_url])
                .current_dir(&root_path)
                .assert()
                .success();
        }
    })
    .await
    .expect("fork enter task panicked");

    // fork.json should exist with an active entry for "testnet".
    assert!(
        treb_dir.join(FORK_STATE_FILE).exists(),
        "fork.json should be created by `treb fork enter`"
    );

    let mut store = ForkStateStore::new(&treb_dir);
    store.load().unwrap();
    let entry = store
        .get_active_fork(network)
        .expect("active fork entry should exist for 'testnet' after fork enter");

    assert_eq!(entry.network, network);
    assert_eq!(entry.fork_url, rpc_url, "fork_url should match the --rpc-url arg");
    assert!(entry.chain_id > 0, "chain_id should be populated");

    // Snapshot directory should have been created.
    let snapshot_dir = PathBuf::from(&entry.snapshot_dir);
    assert!(
        snapshot_dir.exists(),
        "snapshot directory should be created at {}",
        snapshot_dir.display()
    );

    // History should contain an "enter" entry.
    assert!(
        store.data().history.iter().any(|h| h.action == "enter" && h.network == network),
        "fork-state history should contain an 'enter' entry for '{network}'"
    );

    // Anvil kept alive until end of scope.
    drop(anvil);
}

// ── signal handling: SIGTERM shuts down anvil cleanly ─────────────────────────

/// On Unix, `treb dev anvil start` should respond to SIGTERM by shutting down
/// the Anvil instance and exiting cleanly.  The test also verifies that an
/// "anvil-stop" history entry is written to `fork.json` when `--network`
/// is provided.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn signal_handling_sigterm_shuts_down_anvil_cleanly() {
    use std::time::Duration;

    // Spawn an "upstream" Anvil to serve as the fork origin.  The subprocess
    // will fork from this URL so `run_anvil_start` has a valid `fork_url`.
    let Some(upstream) = spawn_anvil_or_skip().await else {
        return;
    };
    let fork_url = upstream.rpc_url().to_string();

    // Build a temporary project directory with a pre-populated fork entry.
    let (root, treb_dir) = make_project();
    let network = "testnet";

    {
        let snapshot_dir = treb_dir.join("snapshots").join(network);
        fs::create_dir_all(&snapshot_dir).unwrap();

        let now = Utc::now();
        let entry = ForkEntry {
            network: network.to_string(),
            instance_name: None,
            rpc_url: String::new(),
            port: 0,
            chain_id: 31337,
            fork_url: fork_url.clone(),
            fork_block_number: None,
            snapshot_dir: snapshot_dir.to_string_lossy().into_owned(),
            started_at: now,
            env_var_name: String::new(),
            original_rpc: String::new(),
            anvil_pid: 0,
            pid_file: String::new(),
            log_file: String::new(),
            entered_at: now,
            snapshots: vec![],
        };
        let mut store = ForkStateStore::new(&treb_dir);
        store.insert_active_fork(entry).unwrap();

        // Add an initial "enter" history entry so the history isn't empty.
        store
            .add_history(ForkHistoryEntry {
                action: "enter".into(),
                network: network.to_string(),
                timestamp: Utc::now(),
                details: None,
            })
            .unwrap();
    }

    // Use port 38247 — a high port that is very unlikely to be in use.
    let anvil_port: u16 = 38247;

    // Resolve the treb-cli binary path at runtime (set by Cargo during tests).
    #[allow(deprecated)]
    let treb_bin = assert_cmd::cargo::cargo_bin("treb-cli");

    // Spawn the binary as a background process.
    let mut child = std::process::Command::new(&treb_bin)
        .args(["dev", "anvil", "start", "--network", network, "--port", &anvil_port.to_string()])
        .current_dir(root.path())
        .spawn()
        .expect("failed to spawn `treb dev anvil start`");

    let pid = child.id();

    // Poll until the forked Anvil is accepting connections (up to 60 seconds
    // since forking from a live Anvil can take a moment).
    let addr = format!("127.0.0.1:{anvil_port}");
    let ready_deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let became_ready = loop {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if tokio::net::TcpStream::connect(&addr).await.is_ok() {
            break true;
        }
        if tokio::time::Instant::now() >= ready_deadline {
            break false;
        }
    };

    if !became_ready {
        // Kill the process to avoid leaving a zombie, then fail the test.
        let _ = child.kill();
        let _ = child.wait();
        panic!("treb dev anvil start did not become ready on port {anvil_port} within 60 seconds");
    }

    // Give the process a moment to finish updating fork state before signalling.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Send SIGTERM.  `kill <pid>` without a signal flag sends SIGTERM by default.
    std::process::Command::new("kill")
        .arg(pid.to_string())
        .status()
        .expect("failed to send SIGTERM");

    // Wait for the subprocess to exit (up to 10 seconds).
    let exit_status = tokio::task::spawn_blocking(move || {
        use std::time::Instant;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return Ok(status),
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err("process did not exit within 10 seconds after SIGTERM");
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => panic!("try_wait failed: {e}"),
            }
        }
    })
    .await
    .unwrap()
    .expect("subprocess did not exit in time after SIGTERM");

    // Process should have exited (signal termination is non-zero on Unix but
    // that is expected for SIGTERM; what matters is that it exited at all).
    let _ = exit_status; // status code is SIGTERM-terminated, not necessarily 0

    // Port should no longer be in use.
    let freed_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if tokio::net::TcpStream::connect(&addr).await.is_err() {
            break; // Port is free.
        }
        assert!(
            tokio::time::Instant::now() < freed_deadline,
            "port {anvil_port} still in use 5 seconds after SIGTERM"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Fork state should contain an "anvil-stop" history entry.
    let mut store = ForkStateStore::new(&treb_dir);
    store.load().expect("failed to reload fork state after SIGTERM");

    let has_stop_entry =
        store.data().history.iter().any(|h| h.action == "anvil-stop" && h.network == network);

    assert!(
        has_stop_entry,
        "fork-state history should contain an 'anvil-stop' entry for '{network}' after SIGTERM; \
         history = {:?}",
        store.data().history
    );

    drop(upstream);
}
