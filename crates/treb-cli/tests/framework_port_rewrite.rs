//! Integration tests for `framework::workdir::port_rewrite_foundry_toml`.

mod framework;

use framework::workdir::{port_rewrite_foundry_toml, port_rewrite_foundry_toml_single};
use std::fs;

/// Sample foundry.toml with various localhost RPC patterns.
const SAMPLE_TOML: &str = r#"[profile.default]
src = "src"
out = "out"
libs = ["lib"]

[rpc_endpoints]
localhost = "http://localhost:8545"
local_ip = "http://127.0.0.1:8545"
testnet = "http://localhost:9545"
testnet_ip = "http://127.0.0.1:9545"
mainnet = "https://mainnet.infura.io/v3/KEY"
"#;

#[test]
fn single_rewrite_replaces_all_default_ports() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("foundry.toml"), SAMPLE_TOML).unwrap();

    port_rewrite_foundry_toml_single(tmp.path(), 12345).unwrap();

    let result = fs::read_to_string(tmp.path().join("foundry.toml")).unwrap();
    assert!(result.contains("localhost:12345"), "localhost:8545 not rewritten");
    assert!(result.contains("127.0.0.1:12345"), "127.0.0.1:8545 not rewritten");
    assert!(!result.contains(":8545"), "8545 still present");
    assert!(!result.contains(":9545"), "9545 still present");
    // External URLs are untouched.
    assert!(result.contains("mainnet.infura.io"));
}

#[test]
fn multiple_rewrites_in_single_call() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("foundry.toml"), SAMPLE_TOML).unwrap();

    // Rewrite 8545 → 11111 and 9545 → 22222.
    port_rewrite_foundry_toml(tmp.path(), &[(8545, 11111), (9545, 22222)]).unwrap();

    let result = fs::read_to_string(tmp.path().join("foundry.toml")).unwrap();
    assert!(result.contains("localhost:11111"));
    assert!(result.contains("127.0.0.1:11111"));
    assert!(result.contains("localhost:22222"));
    assert!(result.contains("127.0.0.1:22222"));
    assert!(!result.contains(":8545"));
    assert!(!result.contains(":9545"));
}

#[test]
fn error_when_foundry_toml_missing() {
    let tmp = tempfile::tempdir().unwrap();
    // No foundry.toml written.
    let err = port_rewrite_foundry_toml_single(tmp.path(), 9999);
    assert!(err.is_err(), "should error when foundry.toml is missing");
    assert_eq!(err.unwrap_err().kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn preserves_non_rpc_content() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("foundry.toml"), SAMPLE_TOML).unwrap();

    port_rewrite_foundry_toml_single(tmp.path(), 5555).unwrap();

    let result = fs::read_to_string(tmp.path().join("foundry.toml")).unwrap();
    assert!(result.contains("[profile.default]"));
    assert!(result.contains("src = \"src\""));
    assert!(result.contains("[rpc_endpoints]"));
}
