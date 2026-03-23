//! P6-US-003: Verify the Safe deployment helper deploys correctly on Anvil.
//!
//! Tests that `deploy_safe()` successfully deploys Safe(1/1) and Safe(2/3)
//! infrastructure and that the deployed contracts respond correctly to
//! `getOwners()`, `getThreshold()`, and `nonce()` eth_calls.

mod e2e;

use alloy_primitives::Address;
use e2e::{
    copy_dir_recursive,
    deploy_safe::{deploy_safe, verify_safe_via_eth_call},
    spawn_anvil_or_skip,
};
use std::{path::Path, str::FromStr};

/// Well-known Anvil test accounts.
const ACCOUNT_0: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const ACCOUNT_1: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
const ACCOUNT_2: &str = "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC";

fn fixture_project() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("project")
}

/// Deploy Safe(1/1) with Anvil account #0 as sole owner, verify via eth_call.
#[tokio::test(flavor = "multi_thread")]
async fn safe_1of1_deploy_and_verify() {
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    let tmp = tempfile::tempdir().unwrap();
    copy_dir_recursive(&fixture_project(), tmp.path());

    let owner = Address::from_str(ACCOUNT_0).unwrap();
    let project_dir = tmp.path().to_path_buf();
    let rpc = rpc_url.clone();

    let safe = tokio::task::spawn_blocking(move || deploy_safe(&project_dir, &rpc, &[owner], 1))
        .await
        .expect("deploy_safe should not panic");

    // Verify all addresses are non-zero.
    assert_ne!(safe.proxy_address, Address::ZERO, "proxy must be non-zero");
    assert_ne!(safe.singleton_address, Address::ZERO, "singleton must be non-zero");
    assert_ne!(safe.factory_address, Address::ZERO, "factory must be non-zero");
    assert_ne!(safe.multisend_address, Address::ZERO, "multisend must be non-zero");

    // Verify via eth_call that the deployed contracts work correctly.
    let rpc = rpc_url.clone();
    tokio::task::spawn_blocking(move || {
        verify_safe_via_eth_call(&rpc, &safe);
    })
    .await
    .unwrap();

    drop(anvil);
}

/// Deploy Safe(2/3) with accounts 0-2 as owners, verify via eth_call.
#[tokio::test(flavor = "multi_thread")]
async fn safe_2of3_deploy_and_verify() {
    let Some(anvil) = spawn_anvil_or_skip().await else {
        return;
    };
    let rpc_url = anvil.rpc_url().to_string();

    let tmp = tempfile::tempdir().unwrap();
    copy_dir_recursive(&fixture_project(), tmp.path());

    let owners = vec![
        Address::from_str(ACCOUNT_0).unwrap(),
        Address::from_str(ACCOUNT_1).unwrap(),
        Address::from_str(ACCOUNT_2).unwrap(),
    ];

    let project_dir = tmp.path().to_path_buf();
    let rpc = rpc_url.clone();
    let owners_clone = owners.clone();

    let safe =
        tokio::task::spawn_blocking(move || deploy_safe(&project_dir, &rpc, &owners_clone, 2))
            .await
            .expect("deploy_safe should not panic");

    // Verify addresses are non-zero and distinct.
    assert_ne!(safe.proxy_address, Address::ZERO);
    assert_ne!(safe.singleton_address, Address::ZERO);
    assert_ne!(safe.proxy_address, safe.singleton_address, "proxy and singleton must differ");

    // Verify ownership and threshold match the 2-of-3 configuration.
    assert_eq!(safe.owners.len(), 3);
    assert_eq!(safe.threshold, 2);

    // Verify via eth_call.
    let rpc = rpc_url.clone();
    tokio::task::spawn_blocking(move || {
        verify_safe_via_eth_call(&rpc, &safe);
    })
    .await
    .unwrap();

    drop(anvil);
}
