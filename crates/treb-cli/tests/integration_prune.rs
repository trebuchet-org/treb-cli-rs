//! Golden-file integration tests for `treb prune`.
//!
//! Tests exercise dry-run candidate display, destructive removal with backup,
//! chain ID filtering, --include-pending, JSON output, clean-registry, and
//! uninitialized project error paths.

mod framework;
mod helpers;

use std::collections::HashMap;

use chrono::Utc;
use treb_core::types::{
    ArtifactInfo, DeploymentMethod, DeploymentStrategy, DeploymentType, TransactionStatus,
    VerificationInfo, VerificationStatus,
};
use treb_registry::Registry;

use std::path::PathBuf;

use framework::{
    context::TestContext,
    golden::GoldenFile,
    integration_test::{IntegrationTest, run_integration_test},
    normalizer::{EpochNormalizer, Normalizer, NormalizerChain, PathNormalizer},
};

// ── Fixture builders ─────────────────────────────────────────────────────

fn make_deployment(id: &str, tx_id: &str, chain_id: u64) -> treb_core::types::Deployment {
    let ts = Utc::now();
    treb_core::types::Deployment {
        id: id.to_string(),
        namespace: "default".to_string(),
        chain_id,
        contract_name: "TestContract".to_string(),
        label: "v1".to_string(),
        address: format!("0x{:040x}", 1u64),
        deployment_type: DeploymentType::Singleton,
        transaction_id: tx_id.to_string(),
        deployment_strategy: DeploymentStrategy {
            method: DeploymentMethod::Create,
            salt: String::new(),
            init_code_hash: String::new(),
            factory: String::new(),
            constructor_args: String::new(),
            entropy: String::new(),
        },
        proxy_info: None,
        artifact: ArtifactInfo {
            path: "contracts/Test.sol".to_string(),
            compiler_version: "0.8.24".to_string(),
            bytecode_hash: "0xabc".to_string(),
            script_path: "script/Deploy.s.sol".to_string(),
            git_commit: "abc123".to_string(),
        },
        verification: VerificationInfo {
            status: VerificationStatus::Unverified,
            etherscan_url: String::new(),
            verified_at: None,
            reason: String::new(),
            verifiers: HashMap::new(),
        },
        tags: None,
        created_at: ts,
        updated_at: ts,
    }
}

fn make_transaction(
    id: &str,
    dep_ids: Vec<String>,
    chain_id: u64,
    status: TransactionStatus,
) -> treb_core::types::Transaction {
    let ts = Utc::now();
    treb_core::types::Transaction {
        id: id.to_string(),
        chain_id,
        hash: format!("0x{:064x}", 0u64),
        status,
        block_number: 1000,
        sender: "0x56fD3F2bEE130e9867942D0F463a16fBE49B8d81".to_string(),
        nonce: 0,
        deployments: dep_ids,
        operations: vec![],
        safe_context: None,
        environment: "testnet".to_string(),
        created_at: ts,
    }
}

/// Seed the registry with broken cross-references for prune testing.
///
/// Creates:
/// - dep-1 on chain 1 → references missing tx-missing-1 (BrokenTransactionRef)
/// - dep-2 on chain 42220 → references missing tx-missing-2 (BrokenTransactionRef)
/// - tx-orphan on chain 1 → references missing dep-gone (BrokenDeploymentRef)
/// - tx-pending on chain 1 → Queued status, no deps (OrphanedPendingEntry when --include-pending)
/// - dep-ok on chain 1 → references tx-ok (clean, should not be pruned)
/// - tx-ok on chain 1 → references dep-ok (clean, should not be pruned)
fn seed_prune_registry(project_root: &std::path::Path) {
    let mut registry = Registry::open(project_root).expect("registry should open");

    // Clean pair (should NOT be pruned)
    registry
        .insert_transaction(make_transaction(
            "tx-ok",
            vec!["dep-ok".to_string()],
            1,
            TransactionStatus::Executed,
        ))
        .unwrap();
    registry.insert_deployment(make_deployment("dep-ok", "tx-ok", 1)).unwrap();

    // Broken deployment refs (deployment → missing transaction)
    registry.insert_deployment(make_deployment("dep-1", "tx-missing-1", 1)).unwrap();
    registry.insert_deployment(make_deployment("dep-2", "tx-missing-2", 42220)).unwrap();

    // Broken transaction ref (transaction → missing deployment)
    registry
        .insert_transaction(make_transaction(
            "tx-orphan",
            vec!["dep-gone".to_string()],
            1,
            TransactionStatus::Executed,
        ))
        .unwrap();

    // Pending transaction (orphaned pending entry)
    registry
        .insert_transaction(make_transaction("tx-pending", vec![], 1, TransactionStatus::Queued))
        .unwrap();
}

// ── Tests ────────────────────────────────────────────────────────────────

/// Dry-run displays a table with ID, Kind, Reason, Chain ID columns and a
/// candidate count summary.
#[test]
fn prune_dry_run() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("prune_dry_run")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_prune_registry(ctx.path()))
        .test(&["prune", "--dry-run"])
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(EpochNormalizer));

    run_integration_test(&test, &ctx);
}

/// Destructive prune removes entries and prints removal summary with backup path.
#[test]
fn prune_destructive() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("prune_destructive")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_prune_registry(ctx.path()))
        .test(&["prune", "--yes"])
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(EpochNormalizer));

    run_integration_test(&test, &ctx);
}

/// Non-interactive destructive prune proceeds without requiring --yes.
#[test]
fn prune_non_interactive_without_yes() {
    let ctx = TestContext::new("minimal-project");

    ctx.run(["init"]).success();
    seed_prune_registry(ctx.path());

    let assertion = ctx.run_with_env(["prune"], [("TREB_NON_INTERACTIVE", "true")]);
    let stdout = String::from_utf8_lossy(&assertion.get_output().stdout).to_string();
    assertion.success();

    assert!(
        stdout.contains("Running in non-interactive mode. Proceeding with prune..."),
        "stdout should include the non-interactive proceed line: {stdout}"
    );
    assert!(
        stdout.contains("✅ Successfully pruned 3 items."),
        "stdout should report prune success: {stdout}"
    );

    let registry = Registry::open(ctx.path()).expect("registry should open after prune");
    assert!(registry.get_deployment("dep-1").is_none(), "dep-1 should be pruned");
    assert!(registry.get_deployment("dep-2").is_none(), "dep-2 should be pruned");
    assert!(registry.get_transaction("tx-orphan").is_none(), "tx-orphan should be pruned");
    assert!(registry.get_deployment("dep-ok").is_some(), "dep-ok should remain");
    assert!(registry.get_transaction("tx-ok").is_some(), "tx-ok should remain");
}

/// Clean registry outputs "Nothing to prune."
#[test]
fn prune_nothing() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("prune_nothing")
        .setup(&["init"])
        .test(&["prune", "--dry-run"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Chain ID filter restricts candidates to the specified chain only.
#[test]
fn prune_chain_filter() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("prune_chain_filter")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_prune_registry(ctx.path()))
        .test(&["prune", "--dry-run", "--network", "1"])
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(EpochNormalizer));

    run_integration_test(&test, &ctx);
}

/// --include-pending flags orphaned pending transactions.
#[test]
fn prune_include_pending() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("prune_include_pending")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_prune_registry(ctx.path()))
        .test(&["prune", "--dry-run", "--include-pending"])
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(EpochNormalizer));

    run_integration_test(&test, &ctx);
}

/// JSON dry-run output is valid JSON with id, kind, reason, chainId fields.
#[test]
fn prune_json_dry_run() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("prune_json_dry_run")
        .setup(&["init"])
        .post_setup_hook(|ctx| seed_prune_registry(ctx.path()))
        .test(&["prune", "--dry-run", "--json"])
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(EpochNormalizer));

    run_integration_test(&test, &ctx);
}

/// Uninitialized project (no foundry.toml) produces an error.
#[test]
fn prune_uninitialized() {
    let ctx = TestContext::new("compose-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("prune_uninitialized")
        .test(&["prune", "--dry-run"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── On-chain check tests ─────────────────────────────────────────────────

/// Returns the golden file directory for this crate's tests.
fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("golden")
}

/// Seed the registry with a clean deployment on chain 31337 whose address
/// has no contract deployed (will be flagged by --check-onchain).
fn seed_onchain_prune_registry(project_root: &std::path::Path, chain_id: u64) {
    let mut registry = Registry::open(project_root).expect("registry should open");

    // Clean cross-reference pair on the target chain, but the deployment
    // address (0x0000...1234) has no contract on Anvil.
    let mut dep = make_deployment("dep-nocode", "tx-nocode", chain_id);
    dep.address = "0x0000000000000000000000000000000000001234".to_string();
    registry
        .insert_transaction(make_transaction(
            "tx-nocode",
            vec!["dep-nocode".to_string()],
            chain_id,
            TransactionStatus::Executed,
        ))
        .unwrap();
    registry.insert_deployment(dep).unwrap();
}

/// Normalizer that replaces `http://127.0.0.1:<port>` with a placeholder URL.
struct RpcUrlNormalizer;

impl Normalizer for RpcUrlNormalizer {
    fn normalize(&self, input: &str) -> String {
        let re = regex::Regex::new(r"http://127\.0\.0\.1:\d+").unwrap();
        re.replace_all(input, "http://127.0.0.1:<PORT>").into_owned()
    }
}

/// --check-onchain --dry-run detects deployments with no on-chain bytecode
/// via eth_getCode against a live Anvil node.
#[tokio::test(flavor = "multi_thread")]
async fn prune_check_onchain_dry_run() {
    let ctx = match TestContext::new("minimal-project").with_anvil("local").await {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("Skipping prune_check_onchain_dry_run: Anvil not available ({e})");
            return;
        }
    };

    let anvil = ctx.anvil("local").unwrap();
    let rpc_url = anvil.rpc_url().to_string();
    let chain_id = anvil.chain_id();

    // Setup: init project
    ctx.run(["init"]).success();

    // Seed a deployment with no on-chain bytecode at its address
    seed_onchain_prune_registry(ctx.path(), chain_id);

    // Run prune with --check-onchain
    let chain_str = chain_id.to_string();
    let args =
        &["prune", "--check-onchain", "--rpc-url", &rpc_url, "--dry-run", "--network", &chain_str];
    let assertion = ctx.run(args);
    let stdout = String::from_utf8_lossy(&assertion.get_output().stdout).to_string();
    let stderr = String::from_utf8_lossy(&assertion.get_output().stderr).to_string();
    assertion.success();

    let mut output = format!("=== cmd 0: [{}] ===\n", args.join(" "));
    if !stdout.is_empty() {
        output.push_str(&stdout);
    }
    if !stderr.is_empty() {
        output.push_str(&stderr);
    }
    output.push('\n');

    // Build normalizer chain
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);
    let extra_normalizers: Vec<Box<dyn Normalizer>> =
        vec![Box::new(path_normalizer), Box::new(EpochNormalizer), Box::new(RpcUrlNormalizer)];
    let default_chain = NormalizerChain::default_chain();
    let normalize = |text: &str| -> String {
        let mut normalized = default_chain.normalize(text);
        for n in &extra_normalizers {
            normalized = n.normalize(&normalized);
        }
        normalized
    };

    let golden = GoldenFile::new(golden_dir());
    golden.compare_with_normalizer("prune_check_onchain_dry_run", "commands", &output, normalize);
}
