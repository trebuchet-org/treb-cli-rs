//! Focused compatibility coverage for renamed CLI commands and shorthand forms.

mod helpers;

use assert_cmd::cargo::cargo_bin_cmd;
use chrono::{TimeZone, Utc};
use predicates::prelude::*;
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::Output,
};
use treb_core::types::fork::{ForkEntry, ForkHistoryEntry, SnapshotEntry};
use treb_registry::{DEPLOYMENTS_FILE, ForkStateStore, TRANSACTIONS_FILE};

const ADDRESSBOOK_CHAIN_ID: &str = "1";
const ADDRESSBOOK_NAME: &str = "Alpha";
const ADDRESSBOOK_ADDRESS: &str = "0x1111111111111111111111111111111111111111";

fn treb() -> assert_cmd::Command {
    cargo_bin_cmd!("treb-cli")
}

const MINIMAL_FOUNDRY_TOML: &str = "[profile.default]\n";

fn setup_config_project() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("foundry.toml"), MINIMAL_FOUNDRY_TOML).unwrap();
    treb().arg("init").current_dir(tmp.path()).assert().success();
    tmp
}

fn setup_seeded_config_project() -> tempfile::TempDir {
    let tmp = setup_config_project();
    helpers::seed_registry(tmp.path());
    tmp
}

fn run_treb_in(dir: &Path, args: &[&str]) -> Output {
    treb().args(args).current_dir(dir).output().expect("command should run")
}

fn assert_matching_command_output<F>(
    description: &str,
    left_args: &[&str],
    right_args: &[&str],
    seed: F,
) where
    F: Fn(&Path),
{
    let left = setup_config_project();
    seed(left.path());
    let left_output = run_treb_in(left.path(), left_args);

    let right = setup_config_project();
    seed(right.path());
    let right_output = run_treb_in(right.path(), right_args);

    assert!(
        left_output.status.success(),
        "{description} left command should succeed for args {left_args:?}"
    );
    assert!(
        right_output.status.success(),
        "{description} right command should succeed for args {right_args:?}"
    );
    assert_eq!(
        left_output.stdout, right_output.stdout,
        "{description} stdout should match for {left_args:?} and {right_args:?}"
    );
    assert_eq!(
        left_output.stderr, right_output.stderr,
        "{description} stderr should match for {left_args:?} and {right_args:?}"
    );
}

fn sample_fork_entry(treb_dir: &Path, network: &str) -> ForkEntry {
    let timestamp = Utc.with_ymd_and_hms(2026, 1, 15, 10, 30, 0).unwrap();

    ForkEntry {
        network: network.to_string(),
        instance_name: None,
        rpc_url: String::new(),
        port: 0,
        chain_id: 1,
        fork_url: "https://eth.example.com".to_string(),
        fork_block_number: None,
        snapshot_dir: treb_dir.join("priv/snapshots/holistic").to_string_lossy().into_owned(),
        started_at: timestamp,
        env_var_name: format!("ETH_RPC_URL_{}", network.to_uppercase()),
        original_rpc: "https://eth.example.com".to_string(),
        anvil_pid: 0,
        pid_file: String::new(),
        log_file: String::new(),
        entered_at: timestamp,
        snapshots: vec![],
    }
}

fn seed_fork_exit(project_root: &Path) {
    let treb_dir = project_root.join(".treb");
    let entry = sample_fork_entry(&treb_dir, "mainnet");
    let snapshot_dir = Path::new(&entry.snapshot_dir);
    fs::create_dir_all(snapshot_dir).unwrap();
    fs::write(treb_dir.join(DEPLOYMENTS_FILE), r#"{"Counter_1":{"address":"0xaaa"}}"#).unwrap();
    fs::write(snapshot_dir.join(DEPLOYMENTS_FILE), r#"{"Counter_1":{"address":"0xaaa"}}"#).unwrap();
    fs::write(treb_dir.join(TRANSACTIONS_FILE), r#"{"tx_1":{"hash":"0x111"}}"#).unwrap();
    fs::write(snapshot_dir.join(TRANSACTIONS_FILE), r#"{"tx_1":{"hash":"0x111"}}"#).unwrap();

    let mut store = ForkStateStore::new(&treb_dir);
    store.enter_fork_mode(&entry.snapshot_dir).unwrap();
    store.insert_active_fork(entry).unwrap();
}

fn seed_fork_history(project_root: &Path) {
    let treb_dir = project_root.join(".treb");
    let mut store = ForkStateStore::new(&treb_dir);

    for entry in [
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
    ] {
        store.add_history(entry).unwrap();
    }
}

#[allow(dead_code)]
fn seed_fork_runtime_for_revert(project_root: &Path, rpc_url: &str, port: u16) {
    let treb_dir = project_root.join(".treb");
    let mut entry = sample_fork_entry(&treb_dir, "mainnet");
    let snapshot_dir = Path::new(&entry.snapshot_dir);
    entry.rpc_url = rpc_url.to_string();
    entry.port = port;
    entry.snapshots.push(SnapshotEntry {
        index: 0,
        snapshot_id: "0xold-snapshot".to_string(),
        command: "enter".to_string(),
        timestamp: Utc.with_ymd_and_hms(2026, 1, 15, 10, 31, 0).unwrap(),
    });

    fs::create_dir_all(snapshot_dir).unwrap();
    fs::write(treb_dir.join(DEPLOYMENTS_FILE), r#"{"Counter_1":{"address":"0xaaa"}}"#).unwrap();
    fs::write(snapshot_dir.join(DEPLOYMENTS_FILE), r#"{"Counter_1":{"address":"0xaaa"}}"#).unwrap();

    let mut store = ForkStateStore::new(&treb_dir);
    store.enter_fork_mode(&entry.snapshot_dir).unwrap();
    store.insert_active_fork(entry).unwrap();
}

#[allow(dead_code)]
fn seed_fork_runtime_for_restart(project_root: &Path, rpc_url: &str, port: u16) {
    let treb_dir = project_root.join(".treb");
    let mut entry = sample_fork_entry(&treb_dir, "mainnet");
    let snapshot_dir = Path::new(&entry.snapshot_dir);
    entry.rpc_url = rpc_url.to_string();
    entry.port = port;

    fs::create_dir_all(snapshot_dir).unwrap();
    fs::write(treb_dir.join(DEPLOYMENTS_FILE), r#"{"Counter_1":{"address":"0xaaa"}}"#).unwrap();
    fs::write(snapshot_dir.join(DEPLOYMENTS_FILE), r#"{"Counter_1":{"address":"0xaaa"}}"#).unwrap();

    let mut store = ForkStateStore::new(&treb_dir);
    store.enter_fork_mode(&entry.snapshot_dir).unwrap();
    store.insert_active_fork(entry).unwrap();
}

fn seed_addressbook_entry(project_root: &Path) {
    let output = treb()
        .args([
            "addressbook",
            "--network",
            ADDRESSBOOK_CHAIN_ID,
            "set",
            ADDRESSBOOK_NAME,
            ADDRESSBOOK_ADDRESS,
        ])
        .env("NO_COLOR", "1")
        .current_dir(project_root)
        .output()
        .expect("addressbook seed command should run");

    assert!(
        output.status.success(),
        "addressbook seed command should succeed: stdout={}, stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[test]
fn completion_bash_primary_and_compat_forms_succeed() {
    let primary =
        treb().args(["completion", "bash"]).output().expect("completion command should run");
    assert!(primary.status.success(), "treb completion bash should succeed");
    assert!(
        String::from_utf8_lossy(&primary.stdout).contains("treb"),
        "bash completions should mention treb"
    );

    let compat = treb()
        .args(["completions", "bash"])
        .output()
        .expect("legacy completions command should run");
    assert!(compat.status.success(), "treb completions bash should succeed");
    assert_eq!(
        primary.stdout, compat.stdout,
        "legacy completions alias should match completion output"
    );
}

#[test]
fn bare_config_matches_config_show() {
    let tmp = setup_config_project();

    let bare = treb().args(["config"]).current_dir(tmp.path()).output().expect("run treb config");
    let explicit = treb()
        .args(["config", "show"])
        .current_dir(tmp.path())
        .output()
        .expect("run treb config show");

    assert!(bare.status.success(), "treb config should succeed");
    assert!(explicit.status.success(), "treb config show should succeed");

    let bare_stdout = String::from_utf8_lossy(&bare.stdout);
    assert!(
        bare_stdout.contains("Namespace:"),
        "bare config output should include the config summary"
    );
    assert_eq!(bare.stdout, explicit.stdout);
    assert_eq!(bare.stderr, explicit.stderr);
}

#[test]
fn compatibility_suite_still_exposes_completion_output_shape() {
    treb().args(["completion", "bash"]).assert().success().stdout(predicate::str::contains("treb"));
}

#[test]
fn register_help_exposes_phase_10_flag_surface() {
    let output =
        treb().args(["registry", "add", "--help"]).output().expect("register help command should run");

    assert!(output.status.success(), "treb registry add --help should succeed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    for flag in ["--network", "--rpc-url", "--namespace", "--deployment-type", "--skip-verify"] {
        assert!(stdout.contains(flag), "register help should include {flag}");
    }
}

#[test]
fn sync_help_exposes_phase_10_flag_surface() {
    let output = treb().args(["registry", "sync", "--help"]).output().expect("sync help command should run");

    assert!(output.status.success(), "treb sync --help should succeed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    for flag in ["--network", "--clean", "--debug", "--json"] {
        assert!(stdout.contains(flag), "sync help should include {flag}");
    }
    assert!(
        stdout.contains("Remove invalid entries while syncing"),
        "sync help should include the Go-parity clean description"
    );
}

#[test]
fn list_short_flags_match_long_filter_output() {
    let tmp = setup_seeded_config_project();

    let long = treb()
        .args(["list", "--json", "--network", "1", "--namespace", "mainnet"])
        .current_dir(tmp.path())
        .output()
        .expect("run long-form list filters");
    let short = treb()
        .args(["list", "--json", "-n", "1", "-s", "mainnet"])
        .current_dir(tmp.path())
        .output()
        .expect("run short-form list filters");

    assert!(long.status.success(), "long-form list filters should succeed");
    assert!(short.status.success(), "short-form list filters should succeed");
    assert_eq!(long.stdout, short.stdout, "short-form list filters should match long-form stdout");
    assert_eq!(long.stderr, short.stderr, "short-form list filters should match long-form stderr");
}

/// Holistic fork exit: `treb fork exit` restores registry and reports success.
#[test]
fn fork_exit_holistic_succeeds() {
    let tmp = setup_config_project();
    seed_fork_exit(tmp.path());

    let output = run_treb_in(tmp.path(), &["fork", "exit"]);
    assert!(
        output.status.success(),
        "fork exit should succeed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Exited fork mode"),
        "exit output should confirm exit: {stdout}"
    );
}

/// Fork history: `--network` flag filters correctly.
#[test]
fn fork_history_network_filter() {
    let tmp = setup_config_project();
    seed_fork_history(tmp.path());

    let all = run_treb_in(tmp.path(), &["fork", "history", "--json"]);
    assert!(all.status.success());
    let all_json: serde_json::Value = serde_json::from_slice(&all.stdout).unwrap();
    assert_eq!(all_json["history"].as_array().unwrap().len(), 3);

    let filtered = run_treb_in(tmp.path(), &["fork", "history", "--network", "mainnet", "--json"]);
    assert!(filtered.status.success());
    let filtered_json: serde_json::Value = serde_json::from_slice(&filtered.stdout).unwrap();
    assert_eq!(filtered_json["history"].as_array().unwrap().len(), 2);
}

/// Fork status: `--json` output includes holistic `active` field.
#[test]
fn fork_status_json_includes_active_field() {
    let tmp = setup_config_project();
    seed_fork_exit(tmp.path()); // re-use seed since it enters fork mode

    let output = run_treb_in(tmp.path(), &["fork", "status", "--json"]);
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json["active"].as_bool().unwrap(), "status should show active=true");
    assert!(json["forks"].as_array().unwrap().len() >= 1);
}

#[test]
fn addressbook_vs_ab_list() {
    assert_matching_command_output(
        "addressbook vs ab list",
        &["addressbook", "--network", ADDRESSBOOK_CHAIN_ID, "list"],
        &["ab", "--network", ADDRESSBOOK_CHAIN_ID, "list"],
        seed_addressbook_entry,
    );
}

#[test]
fn addressbook_list_vs_ls() {
    assert_matching_command_output(
        "addressbook list vs ls",
        &["addressbook", "--network", ADDRESSBOOK_CHAIN_ID, "list"],
        &["addressbook", "--network", ADDRESSBOOK_CHAIN_ID, "ls"],
        seed_addressbook_entry,
    );
}

#[test]
fn addressbook_vs_ab_set() {
    assert_matching_command_output(
        "addressbook vs ab set",
        &[
            "addressbook",
            "--network",
            ADDRESSBOOK_CHAIN_ID,
            "set",
            ADDRESSBOOK_NAME,
            ADDRESSBOOK_ADDRESS,
        ],
        &["ab", "--network", ADDRESSBOOK_CHAIN_ID, "set", ADDRESSBOOK_NAME, ADDRESSBOOK_ADDRESS],
        |_| {},
    );
}

#[test]
fn addressbook_default_vs_list() {
    assert_matching_command_output(
        "addressbook default vs list",
        &["addressbook", "--network", ADDRESSBOOK_CHAIN_ID],
        &["addressbook", "--network", ADDRESSBOOK_CHAIN_ID, "list"],
        seed_addressbook_entry,
    );
}
