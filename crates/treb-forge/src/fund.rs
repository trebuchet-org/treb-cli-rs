//! Auto-fund sender addresses on an Anvil fork via `anvil_setBalance`.

use std::collections::{HashMap, HashSet};

use alloy_primitives::Address;

use crate::sender::ResolvedSender;

/// Fund all unique sender broadcast addresses on an Anvil fork.
///
/// Calls `anvil_setBalance` with `amount_eth` ETH for each unique
/// `broadcast_address()` in the resolved senders map.  For governor
/// senders with a timelock, this funds the timelock (the on-chain
/// executor that `vm.broadcast()` uses).
///
/// Returns `(role, address, success)` tuples — one per unique address.
/// Failures are best-effort and do not abort.
pub async fn fund_senders_on_fork(
    rpc_url: &str,
    resolved_senders: &HashMap<String, ResolvedSender>,
    amount_eth: u64,
) -> Vec<(String, Address, bool)> {
    let balance_wei_hex = format!("{:#x}", amount_eth as u128 * 1_000_000_000_000_000_000u128);
    let client = reqwest::Client::new();

    let mut seen = HashSet::new();
    let mut results = Vec::new();

    for (role, sender) in resolved_senders {
        let addr = sender.broadcast_address();
        if !seen.insert(addr) {
            continue;
        }

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "anvil_setBalance",
            "params": [format!("{:#x}", addr), &balance_wei_hex],
            "id": 1
        });

        let ok = client
            .post(rpc_url)
            .json(&body)
            .send()
            .await
            .is_ok();

        results.push((role.clone(), addr, ok));
    }

    results
}
