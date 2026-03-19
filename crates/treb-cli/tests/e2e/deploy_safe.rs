#![allow(dead_code)]

//! Reusable helper for deploying Safe infrastructure on Anvil.
//!
//! Runs `forge script DeploySafe.s.sol --broadcast` against a running Anvil instance
//! and parses the broadcast artifacts to return deployed contract addresses.
//!
//! This is **blocking** — call via `tokio::task::spawn_blocking` from async test code.

use alloy_primitives::Address;
use std::path::Path;
use std::process::Command;
use std::str::FromStr;

// ── Types ───────────────────────────────────────────────────────────────────

/// Addresses and configuration of a deployed Safe instance on Anvil.
pub struct DeployedSafe {
    pub proxy_address: Address,
    pub singleton_address: Address,
    pub factory_address: Address,
    pub multisend_address: Address,
    pub owners: Vec<Address>,
    pub threshold: u64,
}

// ── Constants ───────────────────────────────────────────────────────────────

/// Anvil account #0 private key (well-known test key, pre-funded).
const DEPLOYER_PRIVATE_KEY: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

// ── Public API ──────────────────────────────────────────────────────────────

/// Deploy Safe infrastructure (singleton, factory, multisend, proxy) on Anvil.
///
/// Uses `forge script` to execute `script/DeploySafe.s.sol` with the given
/// owners and threshold. The proxy is created via the factory's
/// `createProxyWithNonce` with salt nonce 0.
///
/// # Panics
///
/// Panics if `forge` is not installed, the script fails, or broadcast
/// artifacts cannot be parsed.
pub fn deploy_safe(
    project_dir: &Path,
    rpc_url: &str,
    owners: &[Address],
    threshold: u64,
) -> DeployedSafe {
    deploy_safe_with_salt(project_dir, rpc_url, owners, threshold, 0)
}

/// Like [`deploy_safe`] but with an explicit CREATE2 salt nonce.
///
/// Use different salt nonces when deploying multiple Safe proxies on the
/// same Anvil instance (singleton/factory/multisend are re-deployed each
/// time but the proxy address depends on the salt).
pub fn deploy_safe_with_salt(
    project_dir: &Path,
    rpc_url: &str,
    owners: &[Address],
    threshold: u64,
    salt_nonce: u64,
) -> DeployedSafe {
    let owners_csv = owners
        .iter()
        .map(|a| format!("{a}"))
        .collect::<Vec<_>>()
        .join(",");

    let output = Command::new("forge")
        .args([
            "script",
            "script/DeploySafe.s.sol",
            "--broadcast",
            "--rpc-url",
            rpc_url,
            "--private-key",
            DEPLOYER_PRIVATE_KEY,
        ])
        .env("SAFE_OWNERS", &owners_csv)
        .env("SAFE_THRESHOLD", threshold.to_string())
        .env("SAFE_SALT_NONCE", salt_nonce.to_string())
        .current_dir(project_dir)
        .output()
        .expect("failed to execute forge — is it installed?");

    assert!(
        output.status.success(),
        "forge script DeploySafe.s.sol failed.\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    parse_broadcast_artifacts(project_dir, rpc_url, owners, threshold)
}

// ── Verification ────────────────────────────────────────────────────────────

/// Verify a deployed Safe via `cast call` eth_calls.
///
/// Checks that `getOwners()`, `getThreshold()`, and `nonce()` return the
/// expected values. Panics on any mismatch.
pub fn verify_safe_via_eth_call(rpc_url: &str, safe: &DeployedSafe) {
    // Verify getOwners()
    let owners_output = cast_call(rpc_url, &safe.proxy_address, "getOwners()(address[])");
    for owner in &safe.owners {
        assert!(
            owners_output
                .to_lowercase()
                .contains(&format!("{owner}").to_lowercase()),
            "getOwners() should contain {owner}, got: {owners_output}"
        );
    }

    // Verify getThreshold()
    let threshold_output = cast_call(rpc_url, &safe.proxy_address, "getThreshold()(uint256)");
    let threshold: u64 = threshold_output
        .trim()
        .parse()
        .unwrap_or_else(|e| panic!("threshold not a number: '{threshold_output}': {e}"));
    assert_eq!(threshold, safe.threshold, "threshold mismatch");

    // Verify nonce()
    let nonce_output = cast_call(rpc_url, &safe.proxy_address, "nonce()(uint256)");
    let nonce: u64 = nonce_output
        .trim()
        .parse()
        .unwrap_or_else(|e| panic!("nonce not a number: '{nonce_output}': {e}"));
    assert_eq!(nonce, 0, "nonce should be 0 for a freshly deployed Safe");
}

// ── Internal ────────────────────────────────────────────────────────────────

/// Parse `broadcast/DeploySafe.s.sol/<chain_id>/run-latest.json` to extract
/// deployed contract addresses.
fn parse_broadcast_artifacts(
    project_dir: &Path,
    rpc_url: &str,
    owners: &[Address],
    threshold: u64,
) -> DeployedSafe {
    let chain_id = get_chain_id(rpc_url);
    let broadcast_path = project_dir
        .join("broadcast")
        .join("DeploySafe.s.sol")
        .join(chain_id.to_string())
        .join("run-latest.json");

    let data = std::fs::read_to_string(&broadcast_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", broadcast_path.display()));
    let json: serde_json::Value =
        serde_json::from_str(&data).unwrap_or_else(|e| panic!("invalid broadcast JSON: {e}"));

    let transactions = json["transactions"]
        .as_array()
        .expect("broadcast JSON must have transactions array");

    let mut singleton = None;
    let mut factory = None;
    let mut multisend = None;
    let mut proxy = None;

    for tx in transactions {
        let tx_type = tx["transactionType"].as_str().unwrap_or("");
        let name = tx["contractName"].as_str().unwrap_or("");

        if tx_type == "CREATE" {
            let addr = parse_address(tx["contractAddress"].as_str().unwrap());
            match name {
                "GnosisSafe" => singleton = Some(addr),
                "GnosisSafeProxyFactory" => factory = Some(addr),
                "MultiSend" => multisend = Some(addr),
                _ => {}
            }
        }

        // The factory's createProxyWithNonce deploys the proxy —
        // forge records it in additionalContracts.
        if let Some(additional) = tx["additionalContracts"].as_array() {
            for contract in additional {
                if let Some(addr_str) = contract["address"].as_str() {
                    proxy = Some(parse_address(addr_str));
                }
            }
        }
    }

    DeployedSafe {
        proxy_address: proxy.expect("proxy address not found in broadcast artifacts"),
        singleton_address: singleton.expect("singleton address not found in broadcast artifacts"),
        factory_address: factory.expect("factory address not found in broadcast artifacts"),
        multisend_address: multisend.expect("multisend address not found in broadcast artifacts"),
        owners: owners.to_vec(),
        threshold,
    }
}

fn parse_address(s: &str) -> Address {
    Address::from_str(s).unwrap_or_else(|e| panic!("invalid address '{s}': {e}"))
}

/// Query chain ID from a running node via `cast chain-id`.
fn get_chain_id(rpc_url: &str) -> u64 {
    let output = Command::new("cast")
        .args(["chain-id", "--rpc-url", rpc_url])
        .output()
        .expect("failed to execute cast");
    assert!(
        output.status.success(),
        "cast chain-id failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .trim()
        .parse()
        .expect("cast chain-id output must be a number")
}

/// Run `cast call` and return the decoded stdout.
fn cast_call(rpc_url: &str, to: &Address, sig: &str) -> String {
    let output = Command::new("cast")
        .args(["call", &format!("{to}"), sig, "--rpc-url", rpc_url])
        .output()
        .expect("failed to execute cast");
    assert!(
        output.status.success(),
        "cast call {to} {sig} failed:\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .trim()
        .to_string()
}
