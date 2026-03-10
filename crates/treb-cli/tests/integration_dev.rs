//! Golden-file integration tests for `treb dev anvil` subcommands.
//!
//! Tests exercise status table output, JSON output, no-instances output,
//! and logs subcommand error paths.

mod framework;

use chrono::{TimeZone, Utc};
use std::fs;
use treb_core::types::fork::ForkEntry;
use treb_registry::ForkStateStore;

use framework::{
    context::TestContext,
    integration_test::{IntegrationTest, run_integration_test},
    normalizer::{PathNormalizer, UptimeNormalizer},
};

// ── Helpers ──────────────────────────────────────────────────────────────

/// Build a ForkEntry that looks like a tracked Anvil instance (non-zero port,
/// pid_file, log_file) with fixed values for golden file stability.
fn sample_anvil_entry(treb_dir: &std::path::Path) -> ForkEntry {
    let ts = Utc.with_ymd_and_hms(2026, 1, 15, 10, 30, 0).unwrap();
    let snapshot_dir = treb_dir.join("snapshots").join("mainnet");
    ForkEntry {
        network: "mainnet".to_string(),
        instance_name: None,
        rpc_url: "http://127.0.0.1:18545".to_string(),
        port: 18545,
        chain_id: 1,
        fork_url: "https://eth.example.com".to_string(),
        fork_block_number: None,
        snapshot_dir: snapshot_dir.to_string_lossy().into_owned(),
        started_at: ts,
        env_var_name: String::new(),
        original_rpc: String::new(),
        anvil_pid: 0,
        pid_file: treb_dir.join("anvil-mainnet.pid").to_string_lossy().into_owned(),
        log_file: treb_dir.join("anvil-mainnet.log").to_string_lossy().into_owned(),
        entered_at: ts,
        snapshots: vec![],
    }
}

fn sample_named_anvil_entry(
    treb_dir: &std::path::Path,
    network: &str,
    instance_name: &str,
    port: u16,
) -> ForkEntry {
    let ts = Utc.with_ymd_and_hms(2026, 1, 15, 10, 30, 0).unwrap();
    let snapshot_dir = treb_dir.join("snapshots").join(network);
    ForkEntry {
        network: network.to_string(),
        instance_name: Some(instance_name.to_string()),
        rpc_url: format!("http://127.0.0.1:{port}"),
        port,
        chain_id: 1,
        fork_url: format!("https://{network}.example.com"),
        fork_block_number: None,
        snapshot_dir: snapshot_dir.to_string_lossy().into_owned(),
        started_at: ts,
        env_var_name: String::new(),
        original_rpc: String::new(),
        anvil_pid: 0,
        pid_file: treb_dir
            .join(format!("anvil-{instance_name}.pid"))
            .to_string_lossy()
            .into_owned(),
        log_file: treb_dir
            .join(format!("anvil-{instance_name}.log"))
            .to_string_lossy()
            .into_owned(),
        entered_at: ts,
        snapshots: vec![],
    }
}

/// Pre-populate fork state with one tracked Anvil instance.
fn seed_anvil_status(project_root: &std::path::Path) {
    let treb_dir = project_root.join(".treb");
    let mut store = ForkStateStore::new(&treb_dir);
    store.insert_active_fork(sample_anvil_entry(&treb_dir)).unwrap();
}

/// Pre-populate fork state with one tracked Anvil instance and a sample log file.
fn seed_anvil_logs(project_root: &std::path::Path) {
    let treb_dir = project_root.join(".treb");
    seed_anvil_status(project_root);
    fs::write(treb_dir.join("anvil-mainnet.log"), "first log line\nsecond log line\n").unwrap();
}

fn seed_duplicate_named_anvil_status(project_root: &std::path::Path) {
    let treb_dir = project_root.join(".treb");
    let mut store = ForkStateStore::new(&treb_dir);
    store
        .insert_active_fork(sample_named_anvil_entry(&treb_dir, "mainnet", "alpha", 18545))
        .unwrap();
    store
        .insert_active_fork(sample_named_anvil_entry(&treb_dir, "sepolia", "alpha", 19545))
        .unwrap();
}

// ── dev anvil status: no instances ──────────────────────────────────────

/// `treb dev anvil status` with no tracked Anvil instances should print
/// "No active Anvil instances."
#[test]
fn dev_anvil_status_no_instances() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("dev_anvil_status_no_instances")
        .setup(&["init"])
        .test(&["dev", "anvil", "status"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── dev anvil status: with active instances ─────────────────────────────

/// `treb dev anvil status` with a tracked Anvil instance should display a
/// table with all columns including Uptime and colored Status.
#[test]
fn dev_anvil_status_with_instances() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("dev_anvil_status_with_instances")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_anvil_status(ctx.path()))
        .test(&["dev", "anvil", "status"])
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(UptimeNormalizer));

    run_integration_test(&test, &ctx);
}

#[test]
fn dev_anvil_status_duplicate_names_include_network() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("dev_anvil_status_duplicate_names_include_network")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_duplicate_named_anvil_status(ctx.path()))
        .test(&["dev", "anvil", "status", "--name", "alpha"])
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(UptimeNormalizer));

    run_integration_test(&test, &ctx);
}

// ── dev anvil status: JSON output ───────────────────────────────────────

/// `treb dev anvil status --json` should emit valid JSON with sorted keys,
/// including the `uptime` field.
#[test]
fn dev_anvil_status_json() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("dev_anvil_status_json")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_anvil_status(ctx.path()))
        .test(&["dev", "anvil", "status", "--json"])
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(UptimeNormalizer));

    run_integration_test(&test, &ctx);
}

// ── dev anvil logs ───────────────────────────────────────────────────────

/// `treb dev anvil logs` should print the same Go-format header as follow mode
/// and render the log-file line without extra indentation.
#[test]
fn dev_anvil_logs_header() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("dev_anvil_logs_header")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_anvil_logs(ctx.path()))
        .test(&["dev", "anvil", "logs", "--network", "mainnet"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

#[test]
fn dev_anvil_stop_duplicate_names_include_network() {
    let ctx = TestContext::new("minimal-project");

    let test = IntegrationTest::new("dev_anvil_stop_duplicate_names_include_network")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_duplicate_named_anvil_status(ctx.path()))
        .test(&["dev", "anvil", "stop", "--name", "alpha"]);

    run_integration_test(&test, &ctx);
}
