//! Golden-file integration tests for `treb fork` subcommands.
//!
//! Tests exercise status, history, diff, enter, and exit subcommands including
//! table output, JSON output, filtering, error paths, and uninitialized states.

mod framework;

use chrono::{TimeZone, Utc};
use std::io::{Read, Write};
use treb_core::types::fork::{ForkEntry, ForkHistoryEntry};
use treb_registry::{DEPLOYMENTS_FILE, ForkStateStore, TRANSACTIONS_FILE};

use framework::{
    context::TestContext,
    integration_test::{IntegrationTest, run_integration_test},
    normalizer::{LabelNormalizer, PathNormalizer, UptimeNormalizer},
};

// ── Helpers ──────────────────────────────────────────────────────────────

/// Build a ForkEntry with fixed, deterministic values for golden file stability.
fn sample_fork_entry(treb_dir: &std::path::Path) -> ForkEntry {
    let ts = Utc.with_ymd_and_hms(2026, 1, 15, 10, 30, 0).unwrap();
    let snapshot_dir = treb_dir.join("snapshots").join("mainnet");
    ForkEntry {
        network: "mainnet".to_string(),
        instance_name: None,
        rpc_url: "http://localhost:18545".to_string(),
        port: 18545,
        chain_id: 1,
        fork_url: "https://eth.example.com".to_string(),
        fork_block_number: None,
        snapshot_dir: snapshot_dir.to_string_lossy().into_owned(),
        started_at: ts,
        env_var_name: "ETH_RPC_URL_MAINNET".to_string(),
        original_rpc: "https://eth.example.com".to_string(),
        anvil_pid: 0,
        pid_file: String::new(),
        log_file: String::new(),
        entered_at: ts,
        snapshots: vec![],
    }
}

/// Pre-populate fork state with one active entry for golden file tests.
fn seed_fork_status(project_root: &std::path::Path) {
    let treb_dir = project_root.join(".treb");
    let mut store = ForkStateStore::new(&treb_dir);
    store.insert_active_fork(sample_fork_entry(&treb_dir)).unwrap();
}

// ── fork status: no forks ────────────────────────────────────────────────

/// `treb fork status` with an empty fork state should print "No active forks."
#[test]
fn fork_status_no_forks() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_status_no_forks")
        .setup(&["init"])
        .test(&["fork", "status"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── fork status: with active fork ────────────────────────────────────────

/// `treb fork status` with an active fork should display a table with all
/// 7 columns (Network, RPC URL, Port, Chain ID, Fork Block, Started At, Status)
/// and status "stopped" since the port is not reachable.
#[test]
fn fork_status_with_active_fork() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_status_with_active_fork")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_fork_status(ctx.path()))
        .test(&["fork", "status"])
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(UptimeNormalizer));

    run_integration_test(&test, &ctx);
}

// ── fork status: JSON output ─────────────────────────────────────────────

/// `treb fork status --json` should emit valid JSON with camelCase field names.
#[test]
fn fork_status_json() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_status_json")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_fork_status(ctx.path()))
        .test(&["fork", "status", "--json"])
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(UptimeNormalizer));

    run_integration_test(&test, &ctx);
}

// ── fork status: not initialized ─────────────────────────────────────────

/// `treb fork status` on an uninitialized project (no .treb/) should error
/// and mention `treb init`.
#[test]
fn fork_status_not_initialized() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_status_not_initialized")
        .pre_setup_hook(|ctx| {
            std::fs::remove_dir_all(ctx.path().join(".treb")).unwrap();
        })
        .test(&["fork", "status"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── History helpers ─────────────────────────────────────────────────────

/// Pre-populate fork state with history entries for golden file tests.
///
/// Creates 3 entries in chronological order; the store prepends each, so the
/// final order in `history` is most-recent-first:
///   1. restart mainnet (with details)
///   2. enter  sepolia  (no details)
///   3. enter  mainnet  (no details)
fn seed_fork_history(project_root: &std::path::Path) {
    let treb_dir = project_root.join(".treb");
    let mut store = ForkStateStore::new(&treb_dir);

    // Oldest first — add_history prepends, so last add ends up at index 0.
    let entries = vec![
        ForkHistoryEntry {
            action: "enter".to_string(),
            network: "mainnet".to_string(),
            timestamp: Utc.with_ymd_and_hms(2026, 1, 10, 8, 0, 0).unwrap(),
            details: None,
        },
        ForkHistoryEntry {
            action: "enter".to_string(),
            network: "sepolia".to_string(),
            timestamp: Utc.with_ymd_and_hms(2026, 1, 12, 14, 0, 0).unwrap(),
            details: None,
        },
        ForkHistoryEntry {
            action: "restart".to_string(),
            network: "mainnet".to_string(),
            timestamp: Utc.with_ymd_and_hms(2026, 1, 15, 10, 30, 0).unwrap(),
            details: Some("Anvil reset; snapshot: 0x2".to_string()),
        },
    ];

    for entry in entries {
        store.add_history(entry).unwrap();
    }
}

// ── fork history: empty ─────────────────────────────────────────────────

/// `treb fork history` with no history entries should print "No fork history."
#[test]
fn fork_history_empty() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_history_empty")
        .setup(&["init"])
        .test(&["fork", "history"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── fork history: with entries ──────────────────────────────────────────

/// `treb fork history` with entries should display a table with 4 columns
/// (Timestamp, Action, Network, Details) in most-recent-first order.
/// Entries with details = None should show "-".
#[test]
fn fork_history_with_entries() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_history_with_entries")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_fork_history(ctx.path()))
        .test(&["fork", "history"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── fork history: network filter ────────────────────────────────────────

/// `treb fork history --network mainnet` should only display mainnet entries.
#[test]
fn fork_history_network_filter() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_history_network_filter")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_fork_history(ctx.path()))
        .test(&["fork", "history", "--network", "mainnet"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── fork history: JSON output ───────────────────────────────────────────

/// `treb fork history --json` should emit a valid JSON array with correct
/// camelCase field names. Entries with details = None should omit the field.
#[test]
fn fork_history_json() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_history_json")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_fork_history(ctx.path()))
        .test(&["fork", "history", "--json"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── fork history: not initialized ───────────────────────────────────────

/// `treb fork history` on an uninitialized project (no .treb/) should error
/// and mention `treb init`.
#[test]
fn fork_history_not_initialized() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_history_not_initialized")
        .pre_setup_hook(|ctx| {
            std::fs::remove_dir_all(ctx.path().join(".treb")).unwrap();
        })
        .test(&["fork", "history"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── Diff helpers ────────────────────────────────────────────────────────

/// Pre-populate fork state with an active fork and matching registry files
/// (no changes between current and snapshot).
fn seed_fork_diff_no_changes(project_root: &std::path::Path) {
    let treb_dir = project_root.join(".treb");
    let snapshot_dir = treb_dir.join("snapshots").join("mainnet");
    std::fs::create_dir_all(&snapshot_dir).unwrap();

    // Write identical registry files to both locations
    let deployments = r#"{"Counter_1": {"address": "0xaaa"}}"#;
    std::fs::write(treb_dir.join(DEPLOYMENTS_FILE), deployments).unwrap();
    std::fs::write(snapshot_dir.join(DEPLOYMENTS_FILE), deployments).unwrap();

    let transactions = r#"{"tx_1": {"hash": "0x111"}}"#;
    std::fs::write(treb_dir.join(TRANSACTIONS_FILE), transactions).unwrap();
    std::fs::write(snapshot_dir.join(TRANSACTIONS_FILE), transactions).unwrap();

    // Insert active fork entry
    let mut store = ForkStateStore::new(&treb_dir);
    store.insert_active_fork(sample_fork_entry(&treb_dir)).unwrap();
}

/// Pre-populate fork state with an active fork and differing registry files
/// (added, removed, and modified entries).
fn seed_fork_diff_with_changes(project_root: &std::path::Path) {
    let treb_dir = project_root.join(".treb");
    let snapshot_dir = treb_dir.join("snapshots").join("mainnet");
    std::fs::create_dir_all(&snapshot_dir).unwrap();

    // Snapshot: original state
    let snap_deployments =
        r#"{"Counter_1": {"address": "0xaaa"}, "Removed_2": {"address": "0xccc"}}"#;
    std::fs::write(snapshot_dir.join(DEPLOYMENTS_FILE), snap_deployments).unwrap();

    // Current: added Token_3, removed Removed_2
    let curr_deployments =
        r#"{"Counter_1": {"address": "0xaaa"}, "Token_3": {"address": "0xbbb"}}"#;
    std::fs::write(treb_dir.join(DEPLOYMENTS_FILE), curr_deployments).unwrap();

    // Transactions: no changes (both match)
    let transactions = r#"{"tx_1": {"hash": "0x111"}}"#;
    std::fs::write(treb_dir.join(TRANSACTIONS_FILE), transactions).unwrap();
    std::fs::write(snapshot_dir.join(TRANSACTIONS_FILE), transactions).unwrap();

    // Insert active fork entry
    let mut store = ForkStateStore::new(&treb_dir);
    store.insert_active_fork(sample_fork_entry(&treb_dir)).unwrap();
}

// ── fork diff: no changes ───────────────────────────────────────────────

/// `treb fork diff --network mainnet` with identical registry and snapshot
/// should print "No changes detected for network mainnet."
#[test]
fn fork_diff_no_changes() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_diff_no_changes")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_fork_diff_no_changes(ctx.path()))
        .test(&["fork", "diff", "--network", "mainnet"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── fork diff: with changes ─────────────────────────────────────────────

/// `treb fork diff --network mainnet` with added and removed entries should
/// display a table with Change, File, and Key columns.
#[test]
fn fork_diff_with_changes() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_diff_with_changes")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_fork_diff_with_changes(ctx.path()))
        .test(&["fork", "diff", "--network", "mainnet"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── fork diff: JSON output ──────────────────────────────────────────────

/// `treb fork diff --network mainnet --json` should emit valid JSON with
/// network, changes, and clean fields.
#[test]
fn fork_diff_json() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_diff_json")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_fork_diff_with_changes(ctx.path()))
        .test(&["fork", "diff", "--network", "mainnet", "--json"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── fork diff: not forked ───────────────────────────────────────────────

/// `treb fork diff --network mainnet` when no fork is active should error
/// with a message mentioning "not in fork mode".
#[test]
fn fork_diff_not_forked() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_diff_not_forked")
        .setup(&["init"])
        .test(&["fork", "diff", "--network", "mainnet"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── Enter/Exit helpers ─────────────────────────────────────────────────

/// Pre-populate fork state with an active mainnet fork and a snapshot
/// directory containing registry files so that `fork exit` can restore.
fn seed_fork_exit(project_root: &std::path::Path) {
    let treb_dir = project_root.join(".treb");
    let entry = sample_fork_entry(&treb_dir);
    let snapshot_dir = std::path::PathBuf::from(&entry.snapshot_dir);
    std::fs::create_dir_all(&snapshot_dir).unwrap();

    // Write registry files to both locations
    let deployments = r#"{"Counter_1": {"address": "0xaaa"}}"#;
    std::fs::write(treb_dir.join(DEPLOYMENTS_FILE), deployments).unwrap();
    std::fs::write(snapshot_dir.join(DEPLOYMENTS_FILE), deployments).unwrap();

    let transactions = r#"{"tx_1": {"hash": "0x111"}}"#;
    std::fs::write(treb_dir.join(TRANSACTIONS_FILE), transactions).unwrap();
    std::fs::write(snapshot_dir.join(TRANSACTIONS_FILE), transactions).unwrap();

    // Insert active fork entry
    let mut store = ForkStateStore::new(&treb_dir);
    store.insert_active_fork(entry).unwrap();
}

/// Configure `foundry.toml` with a one-shot local JSON-RPC endpoint that
/// responds to `eth_chainId` for deterministic `fork enter` golden testing.
/// Returns the dynamically-assigned port so callers can normalize output.
fn seed_fork_enter_success(project_root: &std::path::Path) -> std::io::Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();

    let foundry_toml = project_root.join("foundry.toml");
    let content = std::fs::read_to_string(&foundry_toml)?;
    let updated = content.replace(
        r#"localhost = "http://localhost:8545""#,
        &format!(r#"localhost = "http://127.0.0.1:{port}""#),
    );
    std::fs::write(foundry_toml, updated)?;

    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();

        // Read and ignore request payload.
        let mut buf = [0_u8; 2048];
        let _ = stream.read(&mut buf);

        // Reply with eth_chainId = 0x1.
        let body = r#"{"jsonrpc":"2.0","id":1,"result":"0x1"}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).unwrap();
        stream.flush().unwrap();
    });

    Ok(port)
}

fn read_http_request(stream: &mut std::net::TcpStream) -> std::io::Result<String> {
    stream.set_read_timeout(Some(std::time::Duration::from_millis(250)))?;

    let mut buf = Vec::new();
    let mut chunk = [0_u8; 2048];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);

                let Some(headers_end) = buf.windows(4).position(|window| window == b"\r\n\r\n")
                else {
                    continue;
                };
                let body_start = headers_end + 4;
                let headers = String::from_utf8_lossy(&buf[..headers_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        line.split_once(':').and_then(|(name, value)| {
                            if name.eq_ignore_ascii_case("content-length") {
                                value.trim().parse::<usize>().ok()
                            } else {
                                None
                            }
                        })
                    })
                    .unwrap_or(0);

                if buf.len() >= body_start + content_length {
                    break;
                }
            }
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                break;
            }
            Err(err) => return Err(err),
        }
    }

    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn spawn_json_rpc_server<F>(mut handler: F) -> std::io::Result<u16>
where
    F: FnMut(&serde_json::Value) -> serde_json::Value + Send + 'static,
{
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();

    std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let request = read_http_request(&mut stream).unwrap();
                    if request.is_empty() {
                        continue;
                    }

                    let body = request.split("\r\n\r\n").nth(1).unwrap_or("");
                    if body.is_empty() {
                        continue;
                    }

                    let json: serde_json::Value = serde_json::from_str(body).unwrap();
                    let response_body = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": json.get("id").cloned().unwrap_or_else(|| serde_json::json!(1)),
                        "result": handler(&json),
                    })
                    .to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        response_body.len(),
                        response_body
                    );
                    stream.write_all(response.as_bytes()).unwrap();
                    stream.flush().unwrap();
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(err) => panic!("json-rpc fixture accept failed: {err}"),
            }
        }
    });

    Ok(port)
}

fn seed_fork_revert_json_success(project_root: &std::path::Path, port: u16) -> std::io::Result<()> {
    let treb_dir = project_root.join(".treb");
    let mut entry = sample_fork_entry(&treb_dir);
    entry.rpc_url = format!("http://127.0.0.1:{port}");
    entry.port = port;
    entry.snapshots.push(treb_core::types::fork::SnapshotEntry {
        index: 0,
        snapshot_id: "0xrevert-old".to_string(),
        command: "enter".to_string(),
        timestamp: Utc.with_ymd_and_hms(2026, 1, 15, 10, 31, 0).unwrap(),
    });

    let snapshot_dir = std::path::PathBuf::from(&entry.snapshot_dir);
    std::fs::create_dir_all(&snapshot_dir)?;
    std::fs::write(treb_dir.join(DEPLOYMENTS_FILE), r#"{"Counter_1": {"address": "0xaaa"}}"#)?;
    std::fs::write(snapshot_dir.join(DEPLOYMENTS_FILE), r#"{"Counter_1": {"address": "0xaaa"}}"#)?;

    let mut store = ForkStateStore::new(&treb_dir);
    store.insert_active_fork(entry).unwrap();
    Ok(())
}

fn seed_fork_restart_json_success(
    project_root: &std::path::Path,
    port: u16,
) -> std::io::Result<()> {
    let treb_dir = project_root.join(".treb");
    let mut entry = sample_fork_entry(&treb_dir);
    entry.rpc_url = format!("http://127.0.0.1:{port}");
    entry.port = port;

    let snapshot_dir = std::path::PathBuf::from(&entry.snapshot_dir);
    std::fs::create_dir_all(&snapshot_dir)?;
    std::fs::write(treb_dir.join(DEPLOYMENTS_FILE), r#"{"Counter_1": {"address": "0xaaa"}}"#)?;
    std::fs::write(snapshot_dir.join(DEPLOYMENTS_FILE), r#"{"Counter_1": {"address": "0xaaa"}}"#)?;

    let mut store = ForkStateStore::new(&treb_dir);
    store.insert_active_fork(entry).unwrap();
    Ok(())
}

// ── fork enter: not initialized ─────────────────────────────────────────

/// `treb fork enter --network mainnet` on an uninitialized project (no .treb/)
/// should error and mention `treb init`.
#[test]
fn fork_enter_not_initialized() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_enter_not_initialized")
        .pre_setup_hook(|ctx| {
            std::fs::remove_dir_all(ctx.path().join(".treb")).unwrap();
        })
        .test(&["fork", "enter", "--network", "mainnet"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── fork enter: already forked ──────────────────────────────────────────

/// `treb fork enter --network mainnet` when mainnet is already forked should
/// error and suggest running `treb fork exit`.
#[test]
fn fork_enter_already_forked() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_enter_already_forked")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_fork_status(ctx.path()))
        .test(&["fork", "enter", "--network", "mainnet"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── fork enter: no RPC URL ──────────────────────────────────────────────

/// `treb fork enter --network mainnet` when mainnet has no RPC endpoint
/// configured in foundry.toml should error mentioning the missing network.
#[test]
fn fork_enter_no_rpc_url() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_enter_no_rpc_url")
        .setup(&["init"])
        .test(&["fork", "enter", "--network", "mainnet"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── fork enter: success ──────────────────────────────────────────────────

/// `treb fork enter --network localhost` with a deterministic local RPC
/// endpoint should succeed and print Go-matching indented field list.
#[test]
fn fork_enter_success() {
    let ctx = TestContext::new("minimal-project");
    let port = match seed_fork_enter_success(ctx.path()) {
        Ok(port) => port,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(err) => panic!("seed fork enter success: {err}"),
    };
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_enter_success")
        .setup(&["init"])
        .test(&["fork", "enter", "--network", "localhost"])
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(LabelNormalizer::new(format!("http://127.0.0.1:{port}"))));

    run_integration_test(&test, &ctx);
}

// ── fork exit: not forked ───────────────────────────────────────────────

/// `treb fork exit --network mainnet` when mainnet is not actively forked
/// should error with a message containing "not in fork mode".
#[test]
fn fork_exit_not_forked() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_exit_not_forked")
        .setup(&["init"])
        .test(&["fork", "exit", "--network", "mainnet"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── fork exit: success ──────────────────────────────────────────────────

/// `treb fork exit --network mainnet` with an active fork should succeed,
/// printing confirmation lines. A subsequent `fork status` should confirm
/// the fork state no longer contains mainnet.
#[test]
fn fork_exit_success() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_exit_success")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_fork_exit(ctx.path()))
        .test(&["fork", "exit", "--network", "mainnet"])
        .test(&["fork", "status"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── fork revert: not forked ─────────────────────────────────────────────

/// `treb fork revert --network mainnet` when mainnet is not forked should
/// error with a message mentioning "not in fork mode".
#[test]
fn fork_revert_not_forked() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_revert_not_forked")
        .setup(&["init"])
        .test(&["fork", "revert", "--network", "mainnet"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── fork revert: no active forks ────────────────────────────────────────

/// `treb fork revert --all --network mainnet` when no forks are active
/// should print "No active forks to revert." (not an error).
#[test]
fn fork_revert_no_active_forks() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_revert_no_active_forks")
        .setup(&["init"])
        .test(&["fork", "revert", "--network", "mainnet", "--all"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── fork revert: port unreachable ───────────────────────────────────────

/// `treb fork revert --network mainnet` when the fork's Anvil port (18545)
/// is not reachable should error mentioning the port and "not reachable".
#[test]
fn fork_revert_port_unreachable() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_revert_port_unreachable")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_fork_status(ctx.path()))
        .test(&["fork", "revert", "--network", "mainnet"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── fork restart: not forked ────────────────────────────────────────────

/// `treb fork restart --network mainnet` when mainnet is not forked should
/// error with a message mentioning "not in fork mode".
#[test]
fn fork_restart_not_forked() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_restart_not_forked")
        .setup(&["init"])
        .test(&["fork", "restart", "--network", "mainnet"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── fork restart: port unreachable ──────────────────────────────────────

/// `treb fork restart --network mainnet` when the fork's Anvil port (18545)
/// is not reachable should error mentioning the port and "not reachable".
#[test]
fn fork_restart_port_unreachable() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_restart_port_unreachable")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_fork_status(ctx.path()))
        .test(&["fork", "restart", "--network", "mainnet"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── fork enter: JSON output ─────────────────────────────────────────────

/// `treb fork enter --network localhost --json` should return a structured
/// JSON error because runtime-only fields are unavailable until anvil starts.
#[test]
fn fork_enter_json() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_enter_json")
        .setup(&["init"])
        .test(&["fork", "enter", "--network", "localhost", "--json"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── fork exit: JSON output ──────────────────────────────────────────────

/// `treb fork exit --network mainnet --json` should emit valid JSON with
/// camelCase field names (network, restoredEntries, cleanedUp).
#[test]
fn fork_exit_json() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_exit_json")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_fork_exit(ctx.path()))
        .test(&["fork", "exit", "--network", "mainnet", "--json"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── fork revert: JSON output ────────────────────────────────────────────

/// `treb fork revert --network mainnet --json` should emit valid JSON with
/// network, snapshotId, and newSnapshotId fields on the success path.
#[test]
fn fork_revert_json_success() {
    let port = match spawn_json_rpc_server(|request| match request["method"].as_str().unwrap() {
        "evm_revert" => serde_json::json!(true),
        "evm_snapshot" => serde_json::json!("0xrevert-new"),
        other => panic!("unexpected JSON-RPC method for revert fixture: {other}"),
    }) {
        Ok(port) => port,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(err) => panic!("seed fork revert success: {err}"),
    };
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_revert_json_success")
        .setup(&["init"])
        .post_setup_hook(move |ctx| seed_fork_revert_json_success(ctx.path(), port).unwrap())
        .test(&["fork", "revert", "--network", "mainnet", "--json"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// `treb fork revert --network mainnet --all --json` with no active forks
/// should output an empty JSON array.
#[test]
fn fork_revert_json_no_active() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_revert_json_no_active")
        .setup(&["init"])
        .test(&["fork", "revert", "--network", "mainnet", "--all", "--json"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── fork restart: JSON error (not forked) ───────────────────────────────

/// `treb fork restart --network mainnet --json` should emit valid JSON with
/// network, chainId, port, rpcUrl, and snapshotId fields on the success path.
#[test]
fn fork_restart_json_success() {
    let port = match spawn_json_rpc_server(|request| match request["method"].as_str().unwrap() {
        "anvil_reset" | "anvil_setCode" => serde_json::Value::Null,
        "evm_snapshot" => serde_json::json!("0xrestart-new"),
        other => panic!("unexpected JSON-RPC method for restart fixture: {other}"),
    }) {
        Ok(port) => port,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(err) => panic!("seed fork restart success: {err}"),
    };
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_restart_json_success")
        .setup(&["init"])
        .post_setup_hook(move |ctx| seed_fork_restart_json_success(ctx.path(), port).unwrap())
        .test(&["fork", "restart", "--network", "mainnet", "--json"])
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(LabelNormalizer::new(format!("http://127.0.0.1:{port}"))))
        .extra_normalizer(Box::new(LabelNormalizer::new(port.to_string())));

    run_integration_test(&test, &ctx);
}

/// `treb fork restart --network mainnet --json` when mainnet is not forked
/// should output a JSON error to stderr.
#[test]
fn fork_restart_json_not_forked() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_restart_json_not_forked")
        .setup(&["init"])
        .test(&["fork", "restart", "--network", "mainnet", "--json"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}
