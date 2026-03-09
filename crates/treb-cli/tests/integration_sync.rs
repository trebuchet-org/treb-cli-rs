//! Golden-file integration tests for `treb sync`.
//!
//! Tests exercise empty-state paths: no safe transactions (plain/JSON),
//! network filter with no matches, uninitialized project, and no foundry project.

mod framework;
mod helpers;

use alloy_primitives::{Address, Bytes};
use framework::{
    context::TestContext,
    integration_test::{IntegrationTest, run_integration_test},
    normalizer::PathNormalizer,
};
use treb_forge::anvil::AnvilConfig;

// ── Tests ────────────────────────────────────────────────────────────────

/// Empty registry with no pending entries uses the Go-style summary output.
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

/// Empty registry with --network filter still uses the shared summary output.
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

const GOVERNOR_ADDRESS: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn governor_address() -> Address {
    GOVERNOR_ADDRESS.parse().unwrap()
}

fn governor_state_runtime(state: u8) -> Bytes {
    Bytes::from(vec![0x60, state, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3])
}

fn governor_revert_runtime() -> Bytes {
    Bytes::from(vec![0x60, 0x00, 0x60, 0x00, 0xfd])
}

async fn governor_sync_context(runtime_code: Bytes) -> Option<TestContext> {
    let ctx = match TestContext::new("minimal-project")
        .with_anvil_mapped("mainnet", AnvilConfig::new().chain_id(1).port(0), 8545)
        .await
    {
        Ok(ctx) => ctx,
        Err(err) if err.to_string().contains("Operation not permitted") => return None,
        Err(err) => panic!("failed to spawn anvil: {err}"),
    };

    ctx.run(["init"]).success();
    ctx.anvil("mainnet")
        .unwrap()
        .instance()
        .set_code(governor_address(), runtime_code)
        .await
        .expect("failed to set governor runtime code");
    seed_governor_proposal(&ctx);

    Some(ctx)
}

/// Helper: seed the registry with a governor proposal and linked transaction on chain 1.
fn seed_governor_proposal(ctx: &TestContext) {
    use chrono::{TimeZone, Utc};
    use treb_core::types::{
        GovernorProposal, Operation, ProposalStatus, Transaction, enums::TransactionStatus,
    };
    use treb_registry::Registry;

    let mut registry = Registry::open(ctx.path()).unwrap();
    let transaction = Transaction {
        id: "tx-0x0001".to_string(),
        chain_id: 1,
        hash: String::new(),
        status: TransactionStatus::Queued,
        block_number: 0,
        sender: GOVERNOR_ADDRESS.to_string(),
        nonce: 0,
        deployments: Vec::new(),
        operations: vec![Operation {
            operation_type: "CALL".to_string(),
            target: "0x1000000000000000000000000000000000000000".to_string(),
            method: "setValue".to_string(),
            result: std::collections::HashMap::new(),
        }],
        safe_context: None,
        environment: "default".to_string(),
        created_at: Utc.with_ymd_and_hms(2026, 3, 8, 9, 55, 0).unwrap(),
    };
    registry.insert_transaction(transaction).unwrap();

    let proposal = GovernorProposal {
        proposal_id: "12345678901234567890".to_string(),
        governor_address: GOVERNOR_ADDRESS.to_string(),
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
/// Verifies an on-chain state change is persisted and summarized.
#[tokio::test(flavor = "multi_thread")]
async fn sync_governor_human() {
    let Some(ctx) = governor_sync_context(governor_state_runtime(1)).await else {
        return;
    };
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("sync_governor_human")
        .test(&["sync"])
        .output_artifact(".treb/governor-txs.json")
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Sync with governor proposals but no matching RPC endpoint emits one warning section.
#[test]
fn sync_governor_missing_rpc_human() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    ctx.run(["init"]).success();
    seed_governor_proposal(&ctx);

    let test = IntegrationTest::new("sync_governor_missing_rpc_human")
        .test(&["sync"])
        .output_artifact(".treb/governor-txs.json")
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Sync with governor proposals in registry — JSON output.
///
/// Verifies executed-state propagation updates both proposal and linked transaction records.
#[tokio::test(flavor = "multi_thread")]
async fn sync_governor_json() {
    let Some(ctx) = governor_sync_context(governor_state_runtime(7)).await else {
        return;
    };
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("sync_governor_json")
        .test(&["sync", "--json"])
        .output_artifact(".treb/governor-txs.json")
        .output_artifact(".treb/transactions.json")
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Sync with --clean removes proposals whose governor call reverts.
#[tokio::test(flavor = "multi_thread")]
async fn sync_governor_clean() {
    let Some(ctx) = governor_sync_context(governor_revert_runtime()).await else {
        return;
    };
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("sync_governor_clean")
        .test(&["sync", "--clean"])
        .output_artifact(".treb/governor-txs.json")
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
