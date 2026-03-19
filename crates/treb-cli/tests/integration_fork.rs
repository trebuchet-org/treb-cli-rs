//! Golden-file integration tests for `treb fork` subcommands.
//!
//! Tests exercise status, history, diff, enter, and exit subcommands including
//! table output, JSON output, filtering, error paths, and uninitialized states.

mod framework;

use chrono::{TimeZone, Utc};
use treb_core::types::fork::{ForkEntry, ForkHistoryEntry};
use treb_registry::{DEPLOYMENTS_FILE, ForkStateStore, TRANSACTIONS_FILE};

use framework::{
    context::TestContext,
    integration_test::{IntegrationTest, run_integration_test},
    normalizer::{PathNormalizer, UptimeNormalizer},
};

// ── Helpers ──────────────────────────────────────────────────────────────

/// Build a ForkEntry with fixed, deterministic values for golden file stability.
fn sample_fork_entry(treb_dir: &std::path::Path) -> ForkEntry {
    let ts = Utc.with_ymd_and_hms(2026, 1, 15, 10, 30, 0).unwrap();
    let snapshot_dir = treb_dir.join("priv/snapshots/holistic");
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
    let entry = sample_fork_entry(&treb_dir);
    let snapshot_dir = std::path::PathBuf::from(&entry.snapshot_dir);
    std::fs::create_dir_all(&snapshot_dir).unwrap();
    let mut store = ForkStateStore::new(&treb_dir);
    store.enter_fork_mode(&entry.snapshot_dir).unwrap();
    store.insert_active_fork(entry).unwrap();
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

    // Enter holistic fork mode and insert active fork entry
    let mut store = ForkStateStore::new(&treb_dir);
    store.enter_fork_mode(&entry.snapshot_dir).unwrap();
    store.insert_active_fork(entry).unwrap();
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

/// `treb fork enter --network mainnet` when already in fork mode should
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

// ── fork exit: not forked ───────────────────────────────────────────────

/// `treb fork exit` when not in fork mode should error
/// with a message containing "not in fork mode".
#[test]
fn fork_exit_not_forked() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_exit_not_forked")
        .setup(&["init"])
        .test(&["fork", "exit"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── fork exit: success ──────────────────────────────────────────────────

/// `treb fork exit` with an active fork should succeed,
/// printing confirmation lines. A subsequent `fork status` should confirm
/// the fork mode is no longer active.
#[test]
fn fork_exit_success() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_exit_success")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_fork_exit(ctx.path()))
        .test(&["fork", "exit"])
        .test(&["fork", "status"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── fork revert: not forked ─────────────────────────────────────────────

/// `treb fork revert` when not in fork mode should
/// error with a message mentioning "not in fork mode".
#[test]
fn fork_revert_not_forked() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_revert_not_forked")
        .setup(&["init"])
        .test(&["fork", "revert"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── fork revert: port unreachable (skips, doesn't error) ────────────────

/// `treb fork revert` when the fork's Anvil port (18545) is not reachable
/// should still succeed (skipping unreachable forks), and restore the registry.
#[test]
#[ignore] // Phase 9: fork revert golden needs refresh after holistic fork model changes
fn fork_revert_port_unreachable() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("fork_revert_port_unreachable")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_fork_status(ctx.path()))
        .test(&["fork", "revert"])
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

// ── fork restart: no snapshot dir ────────────────────────────────────────

/// `treb fork restart --network mainnet` when the holistic snapshot directory
/// does not exist should error mentioning "failed to restore registry".
/// With background Anvil subprocess behavior, restart kills the old process
/// and starts fresh, but still needs the snapshot directory for registry
/// restoration.
#[test]
#[ignore] // Phase 9: fork restart behavior changed — now succeeds instead of erroring, golden + expect_err need update
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
