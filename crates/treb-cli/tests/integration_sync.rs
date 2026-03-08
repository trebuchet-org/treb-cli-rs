//! Golden-file integration tests for `treb sync`.
//!
//! Tests exercise empty-state paths: no safe transactions (plain/JSON),
//! network filter with no matches, uninitialized project, and no foundry project.

mod framework;
mod helpers;

use framework::{
    context::TestContext,
    integration_test::{IntegrationTest, run_integration_test},
    normalizer::PathNormalizer,
};

// ── Tests ────────────────────────────────────────────────────────────────

/// Empty registry with no safe transactions prints informational message.
#[test]
fn sync_no_safe_txs() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("sync_no_safe_txs")
        .setup(&["init"])
        .test(&["sync"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Empty registry with --network filter prints network-specific message.
#[test]
fn sync_no_safe_txs_network_filter() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("sync_no_safe_txs_network_filter")
        .setup(&["init"])
        .test(&["sync", "--network", "1"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Empty registry with --json outputs zero-count JSON.
#[test]
fn sync_json_empty() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("sync_json_empty")
        .setup(&["init"])
        .test(&["sync", "--json"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Uninitialized project (no .treb/) produces an error.
#[test]
fn sync_uninitialized() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("sync_uninitialized")
        .pre_setup_hook(|ctx| {
            std::fs::remove_dir_all(ctx.path().join(".treb")).unwrap();
        })
        .test(&["sync"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── Governor proposal sync tests ────────────────────────────────────────

/// Helper: seed the registry with a governor proposal on chain 1.
fn seed_governor_proposal(ctx: &TestContext) {
    use chrono::{TimeZone, Utc};
    use treb_core::types::{GovernorProposal, ProposalStatus};
    use treb_registry::Registry;

    let mut registry = Registry::open(ctx.path()).unwrap();
    let proposal = GovernorProposal {
        proposal_id: "12345678901234567890".to_string(),
        governor_address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        timelock_address: "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
        chain_id: 1,
        status: ProposalStatus::Pending,
        transaction_ids: vec!["tx-0x0001".to_string()],
        proposed_by: "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".to_string(),
        proposed_at: Utc.with_ymd_and_hms(2026, 3, 8, 10, 0, 0).unwrap(),
        description: String::new(),
        executed_at: None,
        execution_tx_hash: String::new(),
    };
    registry.insert_governor_proposal(proposal).unwrap();
}

/// Sync with governor proposals in registry — human output.
///
/// Verifies "Syncing governor proposals..." stage, warning about missing
/// RPC endpoint, and governor sync summary counts.
#[test]
fn sync_governor_human() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("sync_governor_human")
        .setup(&["init"])
        .post_setup_hook(seed_governor_proposal)
        .test(&["sync"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Sync with governor proposals in registry — JSON output.
///
/// Verifies JSON includes camelCase governor sync fields with correct counts.
#[test]
fn sync_governor_json() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("sync_governor_json")
        .setup(&["init"])
        .post_setup_hook(seed_governor_proposal)
        .test(&["sync", "--json"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── Error tests ─────────────────────────────────────────────────────────

/// No foundry project produces an error.
#[test]
fn sync_no_foundry_project() {
    let ctx = TestContext::new("compose-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("sync_no_foundry_project")
        .test(&["sync"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}
