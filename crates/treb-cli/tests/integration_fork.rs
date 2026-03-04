//! Golden-file integration tests for `treb fork status`.
//!
//! Tests exercise no-forks display, active-fork table output, JSON output,
//! and uninitialized project error paths.

mod framework;

use chrono::{TimeZone, Utc};
use treb_core::types::fork::ForkEntry;
use treb_registry::ForkStateStore;

use framework::context::TestContext;
use framework::integration_test::{run_integration_test, IntegrationTest};
use framework::normalizer::PathNormalizer;

// ── Helpers ──────────────────────────────────────────────────────────────

/// Build a ForkEntry with fixed, deterministic values for golden file stability.
fn sample_fork_entry(treb_dir: &std::path::Path) -> ForkEntry {
    let ts = Utc.with_ymd_and_hms(2026, 1, 15, 10, 30, 0).unwrap();
    let snapshot_dir = treb_dir.join("snapshots").join("mainnet");
    ForkEntry {
        network: "mainnet".to_string(),
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
        .extra_normalizer(Box::new(path_normalizer));

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
        .extra_normalizer(Box::new(path_normalizer));

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
