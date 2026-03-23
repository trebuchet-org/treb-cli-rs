#![allow(dead_code)]

//! Reusable helper for deploying governance infrastructure on Anvil.
//!
//! Runs `forge script DeployGovernance.s.sol --broadcast` against a running Anvil instance
//! and parses the broadcast artifacts to return deployed contract addresses.
//!
//! This is **blocking** — call via `tokio::task::spawn_blocking` from async test code.

use alloy_primitives::Address;
use std::{path::Path, process::Command, str::FromStr};

// ── Types ───────────────────────────────────────────────────────────────────

/// Addresses and configuration of a deployed governance stack on Anvil.
pub struct DeployedGovernor {
    pub governor_address: Address,
    pub timelock_address: Address,
    pub token_address: Address,
    pub timelock_delay: u64,
}

// ── Constants ───────────────────────────────────────────────────────────────

/// Anvil account #0 private key (well-known test key, pre-funded).
const DEPLOYER_PRIVATE_KEY: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

/// keccak256("PROPOSER_ROLE") — matches TrebTimelock.PROPOSER_ROLE().
const PROPOSER_ROLE: &str = "0xb09aa5aeb3702cfd50b6b62bc4532604938f21248a27a1d5ca736082b6819cc1";

// ── Public API ──────────────────────────────────────────────────────────────

/// Deploy governance infrastructure (token, timelock, governor) on Anvil.
///
/// Uses `forge script` to execute `script/DeployGovernance.s.sol` with the given
/// timelock delay. After deployment, grants PROPOSER_ROLE to the governor on the
/// timelock.
///
/// # Panics
///
/// Panics if `forge` is not installed, the script fails, or broadcast
/// artifacts cannot be parsed.
pub fn deploy_governor(project_dir: &Path, rpc_url: &str, timelock_delay: u64) -> DeployedGovernor {
    let output = Command::new("forge")
        .args([
            "script",
            "script/DeployGovernance.s.sol",
            "--broadcast",
            "--rpc-url",
            rpc_url,
            "--private-key",
            DEPLOYER_PRIVATE_KEY,
        ])
        .env("GOV_MIN_DELAY", timelock_delay.to_string())
        .current_dir(project_dir)
        .output()
        .expect("failed to execute forge — is it installed?");

    assert!(
        output.status.success(),
        "forge script DeployGovernance.s.sol failed.\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    parse_broadcast_artifacts(project_dir, rpc_url, timelock_delay)
}

// ── Verification ────────────────────────────────────────────────────────────

/// Verify a deployed governance stack via `cast call` eth_calls.
///
/// Checks that `getMinDelay()` returns the expected delay and that the governor
/// has `PROPOSER_ROLE` on the timelock. Panics on any mismatch.
pub fn verify_governor_via_eth_call(rpc_url: &str, gov: &DeployedGovernor) {
    // Verify getMinDelay() on timelock
    let delay_output = cast_call(rpc_url, &gov.timelock_address, "getMinDelay()(uint256)");
    let delay: u64 = delay_output
        .trim()
        .parse()
        .unwrap_or_else(|e| panic!("getMinDelay not a number: '{delay_output}': {e}"));
    assert_eq!(delay, gov.timelock_delay, "timelock delay mismatch");

    // Verify governor has PROPOSER_ROLE on timelock
    let has_role_sig =
        format!("hasRole(bytes32,address)(bool) {} {}", PROPOSER_ROLE, gov.governor_address);
    let has_role_output = cast_call(rpc_url, &gov.timelock_address, &has_role_sig);
    assert!(
        has_role_output.trim() == "true",
        "governor should have PROPOSER_ROLE on timelock, got: {has_role_output}"
    );
}

// ── Internal ────────────────────────────────────────────────────────────────

/// Parse `broadcast/DeployGovernance.s.sol/<chain_id>/run-latest.json` to extract
/// deployed contract addresses.
fn parse_broadcast_artifacts(
    project_dir: &Path,
    rpc_url: &str,
    timelock_delay: u64,
) -> DeployedGovernor {
    let chain_id = get_chain_id(rpc_url);
    let broadcast_path = project_dir
        .join("broadcast")
        .join("DeployGovernance.s.sol")
        .join(chain_id.to_string())
        .join("run-latest.json");

    let data = std::fs::read_to_string(&broadcast_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", broadcast_path.display()));
    let json: serde_json::Value =
        serde_json::from_str(&data).unwrap_or_else(|e| panic!("invalid broadcast JSON: {e}"));

    let transactions =
        json["transactions"].as_array().expect("broadcast JSON must have transactions array");

    let mut token = None;
    let mut timelock = None;
    let mut governor = None;

    for tx in transactions {
        let tx_type = tx["transactionType"].as_str().unwrap_or("");
        let name = tx["contractName"].as_str().unwrap_or("");

        if tx_type == "CREATE" {
            let addr = parse_address(tx["contractAddress"].as_str().unwrap());
            match name {
                "GovernanceToken" => token = Some(addr),
                "TrebTimelock" => timelock = Some(addr),
                "TrebGovernor" => governor = Some(addr),
                _ => {}
            }
        }
    }

    DeployedGovernor {
        governor_address: governor.expect("governor address not found in broadcast artifacts"),
        timelock_address: timelock.expect("timelock address not found in broadcast artifacts"),
        token_address: token.expect("token address not found in broadcast artifacts"),
        timelock_delay,
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
///
/// `sig` may contain whitespace-separated arguments after the function signature,
/// e.g. `"hasRole(bytes32,address)(bool) 0xabc... 0xdef..."`.
fn cast_call(rpc_url: &str, to: &Address, sig: &str) -> String {
    let to_str = format!("{to}");
    let mut cmd_args = vec!["call", &to_str];
    cmd_args.extend(sig.split_whitespace());
    cmd_args.extend(["--rpc-url", rpc_url]);

    let output = Command::new("cast").args(&cmd_args).output().expect("failed to execute cast");
    assert!(
        output.status.success(),
        "cast call {to} {sig} failed:\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}
