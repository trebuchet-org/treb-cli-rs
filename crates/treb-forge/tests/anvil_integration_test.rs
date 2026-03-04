//! Integration tests for the AnvilInstance lifecycle and CreateX factory deployment.
//!
//! These tests exercise the public API of `treb_forge::anvil` and `treb_forge::createx`
//! end-to-end, verifying that:
//! - Anvil instances can be spawned and are immediately reachable.
//! - The CreateX factory can be deployed and verified at its canonical address.
//! - EVM snapshots taken after a CreateX deploy survive a revert cycle.
//! - Dropping an `AnvilInstance` frees the listening port.

use std::time::Duration;

use treb_forge::{
    anvil::AnvilConfig,
    createx::{deploy_createx, verify_createx},
};

// ── helpers ──────────────────────────────────────────────────────────────────

async fn test_instance() -> treb_forge::anvil::AnvilInstance {
    AnvilConfig::new().port(0).spawn().await.expect("failed to spawn Anvil")
}

async fn is_port_reachable(port: u16) -> bool {
    if port == 0 {
        return false;
    }
    let addr = format!("127.0.0.1:{port}");
    tokio::net::TcpStream::connect(&addr).await.is_ok()
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// Verify that `AnvilConfig::spawn()` returns a reachable RPC endpoint.
#[tokio::test]
async fn spawn_works() {
    let instance = test_instance().await;

    assert!(instance.port() > 0, "port should be assigned by OS");
    assert!(
        instance.rpc_url().starts_with("http://127.0.0.1:"),
        "rpc_url should be a loopback HTTP endpoint"
    );
    assert!(
        is_port_reachable(instance.port()).await,
        "Anvil port should be reachable immediately after spawn"
    );
}

/// Verify that `deploy_createx` succeeds on a fresh Anvil instance.
#[tokio::test]
async fn deploy_createx_succeeds() {
    let instance = test_instance().await;
    deploy_createx(&instance).await.expect("deploy_createx should succeed on a fresh Anvil");
}

/// Verify that `verify_createx` returns `true` after deploying the factory.
#[tokio::test]
async fn verify_createx_returns_true() {
    let instance = test_instance().await;
    deploy_createx(&instance).await.expect("deploy_createx");
    let present = verify_createx(&instance).await.expect("verify_createx");
    assert!(present, "CreateX should be present at its canonical address after deployment");
}

/// Verify that taking a snapshot after deploying CreateX and then reverting to
/// that snapshot still leaves CreateX deployed.
#[tokio::test]
async fn snapshot_revert_round_trip_preserves_createx() {
    let instance = test_instance().await;

    // Deploy CreateX and confirm it is present.
    deploy_createx(&instance).await.expect("deploy_createx");
    assert!(
        verify_createx(&instance).await.expect("verify before snapshot"),
        "CreateX should be present before taking snapshot"
    );

    // Take a snapshot with CreateX deployed.
    let snap_id = instance.snapshot().await.expect("evm_snapshot");

    // Revert to the snapshot.  Because the snapshot was taken *after* deploying
    // CreateX, CreateX should still be present after revert.
    let reverted = instance.revert(snap_id).await.expect("evm_revert");
    assert!(reverted, "evm_revert should return true");

    assert!(
        verify_createx(&instance).await.expect("verify after revert"),
        "CreateX should still be deployed after reverting to a post-deploy snapshot"
    );
}

/// Verify that dropping an `AnvilInstance` releases the listening port.
#[tokio::test]
async fn dropping_instance_frees_port() {
    let port = {
        let instance = test_instance().await;
        let p = instance.port();
        // Port is in use while the instance is alive.
        let addr = format!("127.0.0.1:{p}");
        tokio::net::TcpStream::connect(&addr)
            .await
            .expect("port should be reachable while AnvilInstance is alive");
        p
        // AnvilInstance is dropped here: abort handles cancel server tasks.
    };

    // Poll until the port is freed or we hit the 2-second deadline.
    let addr = format!("127.0.0.1:{port}");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if tokio::net::TcpStream::connect(&addr).await.is_err() {
            return; // Port freed — test passes.
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "port {port} still in use 2 seconds after dropping AnvilInstance"
        );
    }
}
