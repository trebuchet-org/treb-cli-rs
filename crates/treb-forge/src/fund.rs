//! Auto-fund sender addresses on an Anvil fork via `anvil_setBalance`.

use std::collections::{HashMap, HashSet};

use alloy_primitives::{Address, U256};
use alloy_provider::Provider;

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
    let provider = match crate::provider::build_http_provider(rpc_url) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };

    let balance = U256::from(amount_eth) * U256::from(1_000_000_000_000_000_000u128);

    let mut seen = HashSet::new();
    let mut results = Vec::new();

    for (role, sender) in resolved_senders {
        let addr = sender.broadcast_address();
        if !seen.insert(addr) {
            continue;
        }

        let ok = provider
            .raw_request::<_, serde_json::Value>("anvil_setBalance".into(), (addr, balance))
            .await
            .is_ok();

        results.push((role.clone(), addr, ok));
    }

    results
}
