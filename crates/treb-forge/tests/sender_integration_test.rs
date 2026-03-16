//! Integration tests for the sender resolution pipeline.
//!
//! These tests exercise the full flow from `SenderConfig` → `resolve_sender` /
//! `resolve_all_senders` → `build_script_config_with_senders` → `ScriptArgs`,
//! validating that wallet keys and sender addresses are correctly wired.

use std::{collections::HashMap, path::PathBuf};

use alloy_primitives::{Address, address};
use treb_config::{ResolvedConfig, SenderConfig, SenderType};
use treb_core::error::TrebError;
use treb_forge::{ResolvedSender, build_script_config_with_senders, resolve_all_senders};

/// Anvil account 0 private key (well-known test key).
const ANVIL_KEY_0: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

/// Anvil account 0 address.
const ANVIL_ADDR_0: Address = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");

fn test_resolved_config(senders: HashMap<String, SenderConfig>) -> ResolvedConfig {
    ResolvedConfig {
        namespace: "test".to_string(),
        network: None,
        profile: "default".to_string(),
        senders,
        slow: false,
        fork_setup: None,
        config_source: "test".to_string(),
        project_root: PathBuf::from("/tmp"),
    }
}

fn pk_config(key: &str, address: Option<&str>) -> SenderConfig {
    SenderConfig {
        type_: Some(SenderType::PrivateKey),
        private_key: Some(key.to_string()),
        address: address.map(|a| a.to_string()),
        ..Default::default()
    }
}

fn safe_config(safe_addr: &str, signer_name: &str) -> SenderConfig {
    SenderConfig {
        type_: Some(SenderType::Safe),
        safe: Some(safe_addr.to_string()),
        signer: Some(signer_name.to_string()),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Integration test: PrivateKey sender → ScriptConfig → ScriptArgs
// ---------------------------------------------------------------------------

#[tokio::test]
async fn private_key_sender_end_to_end_pipeline() {
    // 1. Define sender config
    let senders = HashMap::from([("deployer".to_string(), pk_config(ANVIL_KEY_0, None))]);

    // 2. Resolve all senders
    let resolved_senders = resolve_all_senders(&senders).await.unwrap();

    // Verify resolved sender is a Wallet with the correct address
    let deployer = resolved_senders.get("deployer").unwrap();
    assert_eq!(deployer.sender_address(), ANVIL_ADDR_0);
    assert!(matches!(deployer, ResolvedSender::Wallet(_)), "deployer should be a Wallet variant");

    // 3. Build ScriptConfig with sender integration
    let resolved = test_resolved_config(senders);
    let config =
        build_script_config_with_senders(&resolved, "script/Deploy.s.sol", &resolved_senders)
            .unwrap();

    // 4. Convert to ScriptArgs and verify wiring
    let args = config.into_script_args().unwrap();

    // evm.sender should be set to the derived address
    assert_eq!(
        args.evm.sender,
        Some(ANVIL_ADDR_0),
        "evm.sender should match the private key's derived address"
    );

    // wallet opts should have the private key injected
    assert!(
        args.wallets.private_keys.contains(&ANVIL_KEY_0.to_string()),
        "wallet opts should contain the deployer's private key"
    );
}

// ---------------------------------------------------------------------------
// Integration test: multi-sender config (deployer=PrivateKey, admin=Safe)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn multi_sender_config_resolves_correctly() {
    let safe_addr = "0x0000000000000000000000000000000000000042";

    // deployer is a PrivateKey sender, admin is a Safe that references deployer as signer
    let senders = HashMap::from([
        ("deployer".to_string(), pk_config(ANVIL_KEY_0, None)),
        ("admin".to_string(), safe_config(safe_addr, "deployer")),
    ]);

    // Resolve all senders at once
    let resolved_senders = resolve_all_senders(&senders).await.unwrap();

    assert_eq!(resolved_senders.len(), 2, "should resolve both deployer and admin");

    // Verify deployer is a Wallet
    let deployer = resolved_senders.get("deployer").unwrap();
    assert!(matches!(deployer, ResolvedSender::Wallet(_)));
    assert_eq!(deployer.sender_address(), ANVIL_ADDR_0);

    // Verify admin is a Safe with the deployer as sub-signer
    let admin = resolved_senders.get("admin").unwrap();
    match admin {
        ResolvedSender::Safe { safe_address, signer } => {
            assert_eq!(
                *safe_address,
                safe_addr.parse::<Address>().unwrap(),
                "Safe address should match config"
            );
            // Sub-signer should be a Wallet with deployer's address
            assert!(
                matches!(signer.as_ref(), ResolvedSender::Wallet(_)),
                "Safe's signer should be a Wallet"
            );
            assert_eq!(
                signer.sender_address(),
                ANVIL_ADDR_0,
                "Safe's signer should be the deployer"
            );
        }
        other => panic!("expected Safe variant, got {other:?}"),
    }

    // Build ScriptConfig — deployer role drives evm.sender and wallet opts
    let resolved = test_resolved_config(senders);
    let config =
        build_script_config_with_senders(&resolved, "script/Deploy.s.sol", &resolved_senders)
            .unwrap();
    let args = config.into_script_args().unwrap();

    assert_eq!(args.evm.sender, Some(ANVIL_ADDR_0));
    assert!(args.wallets.private_keys.contains(&ANVIL_KEY_0.to_string()));
}

// ---------------------------------------------------------------------------
// Error test: invalid private key returns TrebError::Config with sender name
// ---------------------------------------------------------------------------

#[tokio::test]
async fn error_invalid_private_key_returns_config_error_with_sender_name() {
    let senders = HashMap::from([("deployer".to_string(), pk_config("not-a-valid-hex-key", None))]);

    let err = resolve_all_senders(&senders).await.unwrap_err();

    match err {
        TrebError::Config(msg) => {
            assert!(
                msg.contains("deployer"),
                "error should mention the sender name 'deployer': {msg}"
            );
            assert!(msg.contains("invalid private key"), "error should describe the issue: {msg}");
        }
        other => panic!("expected TrebError::Config, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Error test: address mismatch mentions both expected and derived addresses
// ---------------------------------------------------------------------------

#[tokio::test]
async fn error_address_mismatch_mentions_both_addresses() {
    let wrong_address = "0x0000000000000000000000000000000000000001";
    let senders =
        HashMap::from([("deployer".to_string(), pk_config(ANVIL_KEY_0, Some(wrong_address)))]);

    let err = resolve_all_senders(&senders).await.unwrap_err();

    match err {
        TrebError::Config(msg) => {
            assert!(
                msg.contains("address mismatch"),
                "error should mention 'address mismatch': {msg}"
            );
            assert!(
                msg.contains(wrong_address),
                "error should mention the expected (configured) address: {msg}"
            );
            // The derived address is ANVIL_ADDR_0
            assert!(
                msg.to_lowercase().contains(&ANVIL_ADDR_0.to_string().to_lowercase()),
                "error should mention the derived address: {msg}"
            );
        }
        other => panic!("expected TrebError::Config, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Error test: Safe referencing non-existent sender produces clear error
// ---------------------------------------------------------------------------

#[tokio::test]
async fn error_safe_referencing_nonexistent_sender() {
    let safe_addr = "0x0000000000000000000000000000000000000042";
    let senders =
        HashMap::from([("admin".to_string(), safe_config(safe_addr, "nonexistent-signer"))]);

    let err = resolve_all_senders(&senders).await.unwrap_err();

    match err {
        TrebError::Config(msg) => {
            assert!(
                msg.contains("nonexistent-signer"),
                "error should mention the missing sender name: {msg}"
            );
            assert!(msg.contains("not found"), "error should say 'not found': {msg}");
        }
        other => panic!("expected TrebError::Config, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Error test: circular reference (A→B→A) returns error instead of overflow
// ---------------------------------------------------------------------------

#[tokio::test]
async fn error_circular_reference_a_b_a_returns_error() {
    let senders = HashMap::from([
        ("a".to_string(), safe_config("0x0000000000000000000000000000000000000001", "b")),
        ("b".to_string(), safe_config("0x0000000000000000000000000000000000000002", "a")),
    ]);

    let err = resolve_all_senders(&senders).await.unwrap_err();

    match err {
        TrebError::Config(msg) => {
            assert!(msg.contains("circular"), "error should mention 'circular': {msg}");
        }
        other => panic!("expected TrebError::Config, got {other:?}"),
    }
}
