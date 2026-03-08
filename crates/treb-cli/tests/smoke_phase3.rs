//! Phase 3 smoke tests — end-to-end validation of pool, golden files,
//! and IntegrationTest framework.

mod framework;

use framework::{
    context::TestContext,
    golden::GoldenFile,
    integration_test::{IntegrationTest, run_integration_test},
    normalizer::{Normalizer, NormalizerChain, ShortHexNormalizer},
    pool::ContextPool,
};

use alloy_primitives::{U256, address};

use std::path::PathBuf;

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("golden")
}

// ---------------------------------------------------------------------------
// Smoke tests
// ---------------------------------------------------------------------------

/// Pool creates context, acquire/release cycle works, state is clean after release.
#[tokio::test(flavor = "multi_thread")]
async fn pool_acquire_release_clean_state() {
    let pool = match ContextPool::new(1, "minimal-project").await {
        Ok(pool) => pool,
        Err(err) if err.to_string().contains("Operation not permitted") => return,
        Err(err) => panic!("pool creation: {err}"),
    };

    let test_addr = address!("1234567890123456789012345678901234567890");

    // First acquire: modify chain state.
    {
        let guard = pool.acquire().await;
        let node = guard.anvil("local").expect("local node");
        node.instance()
            .api()
            .anvil_set_balance(test_addr, U256::from(999u64))
            .await
            .expect("set_balance");

        let balance = node.instance().api().balance(test_addr, None).await.expect("balance");
        assert_eq!(balance, U256::from(999u64));
        // guard dropped here — cleanup runs
    }

    // Second acquire: state should be clean after pool cleanup.
    {
        let guard = pool.acquire().await;
        let node = guard.anvil("local").expect("local node");

        let balance =
            node.instance().api().balance(test_addr, None).await.expect("balance after re-acquire");
        assert_eq!(balance, U256::ZERO, "balance should be zero after pool cleanup");
    }
}

/// treb version output matches golden file after normalization.
#[test]
fn golden_file_round_trip() {
    let ctx = TestContext::new("minimal-project");

    // Run `treb version` and capture output.
    let assertion = ctx.run(["version"]);
    let stdout = String::from_utf8_lossy(&assertion.get_output().stdout).to_string();
    assertion.success();

    // Apply default normalizer chain + short hex normalizer for commit hashes.
    let chain = NormalizerChain::default_chain();
    let normalized = chain.normalize(&stdout);
    let extra = ShortHexNormalizer;
    let normalized = extra.normalize(&normalized);

    // Compare against golden file.
    let golden = GoldenFile::new(golden_dir());
    golden.compare("golden_file_round_trip", "commands", &normalized);
}

/// IntegrationTest + run_integration_test() works end-to-end with skip_golden.
#[test]
fn integration_test_struct_smoke() {
    let ctx = TestContext::new("minimal-project");

    let test = IntegrationTest::new("smoke_version").test(&["version"]).skip_golden(true);

    run_integration_test(&test, &ctx);
}
