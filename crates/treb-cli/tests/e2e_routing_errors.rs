//! P8-US-004: Routing error path integration tests.
//!
//! Verifies user-facing error messages for invalid or missing sender
//! configurations:
//!
//! - Governor depth limit (5-level governor chain exceeds MAX_ROUTE_DEPTH)
//! - Safe threshold query failure (Safe address pointing to an EOA)
//! - Hardware wallet signer error (Ledger device not connected)

mod e2e;

use e2e::{copy_dir_recursive, spawn_anvil_or_skip, treb};
use std::path::Path;

fn fixture_project() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("project")
}

/// Write a treb.toml with a 5-level governor chain where each governor's
/// proposer is the next governor: gov1 → gov2 → gov3 → gov4 → gov5 → deployer.
/// Uses `timelock1_addr` as gov1's timelock (the script broadcasts from it).
fn write_depth_limit_treb_toml(project_dir: &Path, timelock1_addr: &str) {
    let toml = format!(
        r#"[accounts.deployer]
type = "private_key"
private_key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"

[accounts.gov1]
type = "governance"
address = "0x1111111111111111111111111111111111111111"
timelock = "{timelock1_addr}"
proposer = "gov2"

[accounts.gov2]
type = "governance"
address = "0x2222222222222222222222222222222222222222"
timelock = "0x3222222222222222222222222222222222222222"
proposer = "gov3"

[accounts.gov3]
type = "governance"
address = "0x3333333333333333333333333333333333333333"
timelock = "0x4333333333333333333333333333333333333333"
proposer = "gov4"

[accounts.gov4]
type = "governance"
address = "0x4444444444444444444444444444444444444444"
timelock = "0x5444444444444444444444444444444444444444"
proposer = "gov5"

[accounts.gov5]
type = "governance"
address = "0x5555555555555555555555555555555555555555"
timelock = "0x6555555555555555555555555555555555555555"
proposer = "deployer"

[namespace.default]
senders = {{ deployer = "deployer", gov1 = "gov1", gov2 = "gov2", gov3 = "gov3", gov4 = "gov4", gov5 = "gov5" }}
"#,
    );
    std::fs::write(project_dir.join("treb.toml"), toml).unwrap();
}

/// Write a treb.toml with a Safe sender whose `safe` field points to an EOA
/// (Anvil account #1) instead of a real Safe contract.
fn write_fake_safe_treb_toml(project_dir: &Path, eoa_address: &str) {
    let toml = format!(
        r#"[accounts.deployer]
type = "private_key"
private_key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"

[accounts.fake_safe]
type = "safe"
safe = "{eoa_address}"
signer = "deployer"

[namespace.default]
senders = {{ deployer = "deployer", safe_deployer = "fake_safe" }}
"#,
    );
    std::fs::write(project_dir.join("treb.toml"), toml).unwrap();
}

/// Write a treb.toml with a Ledger sender as the deployer.
fn write_ledger_treb_toml(project_dir: &Path) {
    let toml = r#"[accounts.ledger_deployer]
type = "ledger"

[namespace.default]
senders = { deployer = "ledger_deployer" }
"#;
    std::fs::write(project_dir.join("treb.toml"), toml).unwrap();
}

/// A 5-level governor chain in treb.toml triggers the routing depth limit
/// (MAX_ROUTE_DEPTH = 4) during transaction routing. The CLI should exit
/// non-zero with a clear error message about the depth limit.
#[tokio::test(flavor = "multi_thread")]
async fn governor_depth_limit_error() {
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    let tmp = tempfile::tempdir().unwrap();
    copy_dir_recursive(&fixture_project(), tmp.path());

    // Use a deterministic timelock address for gov1 (used by script's TIMELOCK_ADDRESS)
    let timelock1 = "0x2111111111111111111111111111111111111111";
    write_depth_limit_treb_toml(tmp.path(), timelock1);

    // treb init
    let tmp_path = tmp.path().to_path_buf();
    tokio::task::spawn_blocking(move || {
        treb().arg("init").current_dir(&tmp_path).assert().success();
    })
    .await
    .unwrap();

    // fork enter
    let tmp_path = tmp.path().to_path_buf();
    let rpc = rpc_url.clone();
    tokio::task::spawn_blocking(move || {
        treb()
            .args(["fork", "enter", "--network", "anvil-31337", "--rpc-url", &rpc])
            .current_dir(&tmp_path)
            .assert()
            .success();
    })
    .await
    .unwrap();

    // treb run should fail with depth limit error
    let timelock_env = format!("TIMELOCK_ADDRESS={timelock1}");
    let tmp_path = tmp.path().to_path_buf();
    let rpc = rpc_url.clone();
    tokio::task::spawn_blocking(move || {
        let output = treb()
            .args([
                "run",
                "script/DeployMixedWalletGovernor.s.sol",
                "--network",
                "anvil-31337",
                "--rpc-url",
                &rpc,
                "--broadcast",
                "--non-interactive",
                "--env",
                &timelock_env,
            ])
            .current_dir(&tmp_path)
            .output()
            .unwrap();

        assert!(
            !output.status.success(),
            "treb run should fail with depth limit error.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("routing queue depth exceeded (4)")
                || stderr.contains("check sender configuration for circular references"),
            "stderr should mention depth limit or circular references, got:\n{stderr}"
        );
    })
    .await
    .unwrap();

    drop(anvil);
}

/// Configuring a Safe sender whose `safe` field points to an EOA (not a real
/// Safe contract) causes a threshold query failure during routing on fork.
/// The CLI should exit non-zero with a threshold-related error.
#[tokio::test(flavor = "multi_thread")]
async fn threshold_query_failure_on_fork() {
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    let tmp = tempfile::tempdir().unwrap();
    copy_dir_recursive(&fixture_project(), tmp.path());

    // Use Anvil account #1 as the "Safe" address — it's an EOA, not a Safe contract
    let eoa_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    write_fake_safe_treb_toml(tmp.path(), eoa_address);

    // treb init
    let tmp_path = tmp.path().to_path_buf();
    tokio::task::spawn_blocking(move || {
        treb().arg("init").current_dir(&tmp_path).assert().success();
    })
    .await
    .unwrap();

    // fork enter
    let tmp_path = tmp.path().to_path_buf();
    let rpc = rpc_url.clone();
    tokio::task::spawn_blocking(move || {
        treb()
            .args(["fork", "enter", "--network", "anvil-31337", "--rpc-url", &rpc])
            .current_dir(&tmp_path)
            .assert()
            .success();
    })
    .await
    .unwrap();

    // treb run should fail with a threshold-related error
    let safe_env = format!("SAFE_ADDRESS={eoa_address}");
    let multisend_env = "MULTISEND_ADDRESS=0x0000000000000000000000000000000000000001";
    let tmp_path = tmp.path().to_path_buf();
    let rpc = rpc_url.clone();
    tokio::task::spawn_blocking(move || {
        let output = treb()
            .args([
                "run",
                "script/DeployMixedWalletSafe.s.sol",
                "--network",
                "anvil-31337",
                "--rpc-url",
                &rpc,
                "--broadcast",
                "--non-interactive",
                "--env",
                &safe_env,
                "--env",
                multisend_env,
            ])
            .current_dir(&tmp_path)
            .output()
            .unwrap();

        assert!(
            !output.status.success(),
            "treb run should fail with threshold query error.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );

        let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
        assert!(
            stderr.contains("threshold")
                || stderr.contains("eth_call")
                || stderr.contains("decode")
                || stderr.contains("has no code"),
            "stderr should contain a Safe query error, got:\n{stderr}"
        );
    })
    .await
    .unwrap();

    drop(anvil);
}

/// Configuring a Ledger hardware wallet sender when no device is connected
/// causes sender resolution to fail. The CLI should exit non-zero with an
/// error mentioning "Ledger".
#[tokio::test(flavor = "multi_thread")]
async fn hardware_wallet_signer_error() {
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    let tmp = tempfile::tempdir().unwrap();
    copy_dir_recursive(&fixture_project(), tmp.path());

    write_ledger_treb_toml(tmp.path());

    // treb init
    let tmp_path = tmp.path().to_path_buf();
    tokio::task::spawn_blocking(move || {
        treb().arg("init").current_dir(&tmp_path).assert().success();
    })
    .await
    .unwrap();

    // treb run should fail during sender resolution (no Ledger device)
    let tmp_path = tmp.path().to_path_buf();
    let rpc = rpc_url.clone();
    tokio::task::spawn_blocking(move || {
        let output = treb()
            .args([
                "run",
                "script/Deploy.s.sol",
                "--network",
                "anvil-31337",
                "--rpc-url",
                &rpc,
                "--broadcast",
                "--non-interactive",
            ])
            .current_dir(&tmp_path)
            .output()
            .unwrap();

        assert!(
            !output.status.success(),
            "treb run should fail with Ledger error.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("Ledger"),
            "stderr should mention 'Ledger', got:\n{stderr}"
        );
    })
    .await
    .unwrap();

    drop(anvil);
}
