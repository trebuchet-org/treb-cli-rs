//! P7-US-003: Verify the Governor deployment helper deploys correctly on Anvil.
//!
//! Tests that `deploy_governor()` successfully deploys GovernanceToken, TrebTimelock,
//! and TrebGovernor, and that the deployed contracts respond correctly to
//! `getMinDelay()` and `hasRole()` eth_calls.

mod e2e;

use alloy_primitives::Address;
use e2e::{
    copy_dir_recursive,
    deploy_governor::{deploy_governor, verify_governor_via_eth_call},
    spawn_anvil_or_skip,
};
use std::path::Path;

fn fixture_project() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("project")
}

/// Deploy governance stack with default delay (1s), verify via eth_call.
#[tokio::test(flavor = "multi_thread")]
async fn governor_deploy_and_verify() {
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    let tmp = tempfile::tempdir().unwrap();
    copy_dir_recursive(&fixture_project(), tmp.path());

    let project_dir = tmp.path().to_path_buf();
    let rpc = rpc_url.clone();

    let gov = tokio::task::spawn_blocking(move || deploy_governor(&project_dir, &rpc, 1))
        .await
        .expect("deploy_governor should not panic");

    // Verify all addresses are non-zero.
    assert_ne!(gov.governor_address, Address::ZERO, "governor must be non-zero");
    assert_ne!(gov.timelock_address, Address::ZERO, "timelock must be non-zero");
    assert_ne!(gov.token_address, Address::ZERO, "token must be non-zero");

    // All three addresses must be distinct.
    assert_ne!(gov.governor_address, gov.timelock_address, "governor and timelock must differ");
    assert_ne!(gov.governor_address, gov.token_address, "governor and token must differ");
    assert_ne!(gov.timelock_address, gov.token_address, "timelock and token must differ");

    // Verify via eth_call that the deployed contracts work correctly.
    let rpc = rpc_url.clone();
    tokio::task::spawn_blocking(move || {
        verify_governor_via_eth_call(&rpc, &gov);
    })
    .await
    .unwrap();

    drop(anvil);
}

/// Deploy governance stack with custom delay (42s), verify delay is correct.
#[tokio::test(flavor = "multi_thread")]
async fn governor_deploy_custom_delay() {
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    let tmp = tempfile::tempdir().unwrap();
    copy_dir_recursive(&fixture_project(), tmp.path());

    let project_dir = tmp.path().to_path_buf();
    let rpc = rpc_url.clone();

    let gov = tokio::task::spawn_blocking(move || deploy_governor(&project_dir, &rpc, 42))
        .await
        .expect("deploy_governor should not panic");

    assert_eq!(gov.timelock_delay, 42);

    // Verify via eth_call that the delay is 42.
    let rpc = rpc_url.clone();
    tokio::task::spawn_blocking(move || {
        verify_governor_via_eth_call(&rpc, &gov);
    })
    .await
    .unwrap();

    drop(anvil);
}
