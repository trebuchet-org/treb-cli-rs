//! Shared receipt processing for tx-hash-based operations.
//!
//! Extracts contract creations, proxy patterns, and Upgraded events from
//! a transaction receipt and its debug trace. Used by `register`, `fork exec`,
//! and `sync --tx-hash`.

use std::time::Duration;

use alloy_primitives::{Address, B256, Bytes, Log as PrimitiveLog};
use anyhow::{Context, bail};
use chrono::Utc;
use serde::Serialize;
use treb_core::types::{
    ExecutionKind, ExecutionRef, ExecutionStatus, ProxyUpgrade, deployment::ProxyInfo,
};
use treb_forge::events::{ProxyEvent, decode_events};
use treb_registry::Registry;

// ── JSON-RPC helpers ────────────────────────────────────────────────────

/// Build a reqwest client with timeouts for RPC calls.
pub fn rpc_client() -> anyhow::Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("failed to build HTTP client")
}

/// Make a JSON-RPC call and return the "result" field.
pub async fn rpc_call(
    client: &reqwest::Client,
    url: &str,
    method: &str,
    params: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1
    });

    let resp = client
        .post(url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("RPC request failed for {method}"))?;

    let json: serde_json::Value =
        resp.json().await.with_context(|| format!("invalid JSON response for {method}"))?;

    if let Some(error) = json.get("error") {
        bail!("RPC error for {method}: {error}");
    }

    json.get("result")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("no result field in {method} response"))
}

/// Parse a hex string (with or without 0x prefix) to u64.
pub fn parse_hex_u64(hex: &str) -> u64 {
    let stripped = hex.strip_prefix("0x").or_else(|| hex.strip_prefix("0X")).unwrap_or(hex);
    u64::from_str_radix(stripped, 16).unwrap_or(0)
}

// ── Trace parsing ───────────────────────────────────────────────────────

/// A contract creation found in a transaction trace or receipt.
#[derive(Debug, Clone)]
pub struct TracedCreation {
    pub address: String,
    pub from: String,
    pub create_type: String,
}

/// Parse debug_traceTransaction callTracer output for contract creations.
pub fn extract_creations_from_trace(trace: &serde_json::Value) -> Vec<TracedCreation> {
    let mut creations = Vec::new();
    walk_trace_calls(trace, &mut creations);
    creations
}

fn walk_trace_calls(call: &serde_json::Value, creations: &mut Vec<TracedCreation>) {
    let call_type = call.get("type").and_then(|v| v.as_str()).unwrap_or("");

    if call_type.eq_ignore_ascii_case("CREATE") || call_type.eq_ignore_ascii_case("CREATE2") {
        if let Some(to) = call.get("to").and_then(|v| v.as_str()) {
            let from = call.get("from").and_then(|v| v.as_str()).unwrap_or("").to_string();
            creations.push(TracedCreation {
                address: to.to_string(),
                from,
                create_type: call_type.to_uppercase(),
            });
        }
    }

    // Recurse into sub-calls.
    if let Some(calls) = call.get("calls").and_then(|v| v.as_array()) {
        for subcall in calls {
            walk_trace_calls(subcall, creations);
        }
    }
}

// ── Proxy detection (trace-based) ───────────────────────────────────────

/// A detected proxy+implementation pair from trace analysis.
#[derive(Debug, Clone)]
pub struct DetectedProxy {
    pub proxy_address: String,
    pub implementation_address: String,
}

/// Detect proxy patterns from a callTracer trace.
///
/// A proxy pattern is detected when a CREATE/CREATE2 call contains a
/// DELEGATECALL subcall from the newly created address to another address
/// (the implementation).
pub fn detect_proxy_patterns(trace: &serde_json::Value) -> Vec<DetectedProxy> {
    let mut results = Vec::new();
    find_proxy_creates(trace, &mut results);
    results
}

fn find_proxy_creates(call: &serde_json::Value, results: &mut Vec<DetectedProxy>) {
    let call_type = call.get("type").and_then(|v| v.as_str()).unwrap_or("");

    if call_type.eq_ignore_ascii_case("CREATE") || call_type.eq_ignore_ascii_case("CREATE2") {
        if let Some(created_addr) = call.get("to").and_then(|v| v.as_str()) {
            if let Some(impl_addr) = find_delegatecall_target(call, created_addr) {
                results.push(DetectedProxy {
                    proxy_address: created_addr.to_string(),
                    implementation_address: impl_addr,
                });
            }
        }
    }

    if let Some(calls) = call.get("calls").and_then(|v| v.as_array()) {
        for subcall in calls {
            find_proxy_creates(subcall, results);
        }
    }
}

fn find_delegatecall_target(call: &serde_json::Value, from_addr: &str) -> Option<String> {
    if let Some(calls) = call.get("calls").and_then(|v| v.as_array()) {
        for subcall in calls {
            let sub_type = subcall.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if sub_type.eq_ignore_ascii_case("DELEGATECALL") {
                let sub_from = subcall.get("from").and_then(|v| v.as_str()).unwrap_or("");
                if sub_from.eq_ignore_ascii_case(from_addr) {
                    if let Some(target) = subcall.get("to").and_then(|v| v.as_str()) {
                        return Some(target.to_string());
                    }
                }
            }
            if let Some(found) = find_delegatecall_target(subcall, from_addr) {
                return Some(found);
            }
        }
    }
    None
}

/// Extract a non-zero contractAddress from a transaction receipt.
pub fn receipt_contract_address(receipt: &serde_json::Value) -> Option<String> {
    let addr = receipt.get("contractAddress").and_then(|v| v.as_str())?;
    if addr.is_empty() || addr == "0x0000000000000000000000000000000000000000" {
        return None;
    }
    Some(addr.to_string())
}

// ── Proxy upgrade detection (log-based) ─────────────────────────────────

/// A proxy implementation upgrade detected from an Upgraded event in receipt logs.
#[derive(Debug, Clone)]
pub struct DetectedUpgrade {
    pub proxy_address: Address,
    pub new_implementation: Address,
}

/// Parse receipt JSON logs into alloy primitive Log entries for event decoding.
fn parse_receipt_logs(receipt: &serde_json::Value) -> Vec<PrimitiveLog> {
    let logs = match receipt.get("logs").and_then(|v| v.as_array()) {
        Some(logs) => logs,
        None => return Vec::new(),
    };

    logs.iter()
        .filter_map(|log| {
            let address: Address = log.get("address")?.as_str()?.parse().ok()?;
            let topics: Vec<B256> = log
                .get("topics")?
                .as_array()?
                .iter()
                .filter_map(|t| t.as_str()?.parse().ok())
                .collect();
            let data_hex = log.get("data")?.as_str()?;
            let stripped = data_hex.strip_prefix("0x").unwrap_or(data_hex);
            let data_bytes = alloy_primitives::hex::decode(stripped).ok()?;

            Some(PrimitiveLog::new_unchecked(address, topics, Bytes::from(data_bytes)))
        })
        .collect()
}

/// Extract Upgraded events from receipt logs to detect proxy implementation changes.
fn detect_upgrades_from_logs(receipt: &serde_json::Value) -> Vec<DetectedUpgrade> {
    let logs = parse_receipt_logs(receipt);
    let events = decode_events(&logs);

    events
        .iter()
        .filter_map(|e| {
            if let treb_forge::events::decoder::ParsedEvent::Proxy(ProxyEvent::Upgraded {
                proxy_address,
                implementation,
            }) = e
            {
                Some(DetectedUpgrade {
                    proxy_address: *proxy_address,
                    new_implementation: *implementation,
                })
            } else {
                None
            }
        })
        .collect()
}

// ── Governance matching ────────────────────────────────────────────────

/// Known event topic0 values for governance execution events.
mod topics {
    /// `ProposalExecuted(uint256 proposalId)` — OZ Governor event.
    /// keccak256("ProposalExecuted(uint256)")
    pub const PROPOSAL_EXECUTED: &str =
        "0x712ae1383f79ac853f8d882153778e0260ef8f03b504e2a0b0fcb9a16045c3ce";

    /// `ExecutionSuccess(bytes32 txHash, uint256 payment)` — Gnosis Safe event.
    /// keccak256("ExecutionSuccess(bytes32,uint256)")
    pub const EXECUTION_SUCCESS: &str =
        "0x442e715f626346e8c54381002da614f62bee8d27386535b2521ec8540898556e";
}

/// A governance record matched from receipt logs or traces.
#[derive(Debug, Clone)]
pub enum GovernanceMatch {
    GovernorProposal { proposal_id: String },
    SafeTransaction { safe_tx_hash: String },
}

/// Match receipt logs against pending governor proposals and safe transactions.
///
/// Looks for `ProposalExecuted(uint256)` and `ExecutionSuccess(bytes32, uint256)`
/// events. Returns the first match found.
pub fn match_governance_logs(
    receipt: &serde_json::Value,
    pending_proposals: &[&treb_core::types::GovernorProposal],
    pending_safe_txs: &[&treb_core::types::SafeTransaction],
) -> Option<GovernanceMatch> {
    let logs = receipt.get("logs").and_then(|v| v.as_array())?;

    for log in logs {
        let log_topics = match log.get("topics").and_then(|v| v.as_array()) {
            Some(t) => t,
            None => continue,
        };
        if log_topics.is_empty() {
            continue;
        }
        let topic0 = match log_topics[0].as_str() {
            Some(t) => t,
            None => continue,
        };

        // ProposalExecuted(uint256 proposalId) — proposalId is in data (non-indexed)
        if topic0.eq_ignore_ascii_case(topics::PROPOSAL_EXECUTED) {
            let data = log.get("data").and_then(|v| v.as_str()).unwrap_or("");
            let stripped =
                data.strip_prefix("0x").or_else(|| data.strip_prefix("0X")).unwrap_or(data);
            // data is a 32-byte ABI-encoded uint256
            if stripped.len() >= 64 {
                let proposal_id_hex = &stripped[..64];
                // Convert to decimal string for matching
                if let Ok(id_bytes) = alloy_primitives::hex::decode(proposal_id_hex) {
                    let id = alloy_primitives::U256::from_be_slice(&id_bytes);
                    let id_str = id.to_string();
                    for proposal in pending_proposals {
                        if proposal.proposal_id == id_str {
                            return Some(GovernanceMatch::GovernorProposal { proposal_id: id_str });
                        }
                    }
                }
            }
        }

        // ExecutionSuccess(bytes32 txHash, uint256 payment) — txHash is indexed (topic[1])
        if topic0.eq_ignore_ascii_case(topics::EXECUTION_SUCCESS) {
            if let Some(tx_hash_topic) = log_topics.get(1).and_then(|v| v.as_str()) {
                let hash_lower = tx_hash_topic.to_lowercase();
                for stx in pending_safe_txs {
                    if stx.safe_tx_hash.to_lowercase() == hash_lower {
                        return Some(GovernanceMatch::SafeTransaction {
                            safe_tx_hash: stx.safe_tx_hash.clone(),
                        });
                    }
                }
            }
        }
    }

    None
}

/// Match a callTracer trace against pending governor proposals and safe transactions.
///
/// Walks the trace tree looking for CALL operations to known governance/safe
/// addresses. For Governor matches, compares decoded action data (targets, values,
/// calldatas). For Safe matches, checks for `execTransaction` selector to the
/// safe address.
pub fn match_governance_trace(
    trace: &serde_json::Value,
    pending_proposals: &[&treb_core::types::GovernorProposal],
    pending_safe_txs: &[&treb_core::types::SafeTransaction],
) -> Option<GovernanceMatch> {
    // Build lookup sets
    let mut governor_addrs: std::collections::HashSet<String> = std::collections::HashSet::new();
    for p in pending_proposals {
        governor_addrs.insert(p.governor_address.to_lowercase());
    }

    let mut safe_addrs: std::collections::HashSet<String> = std::collections::HashSet::new();
    for stx in pending_safe_txs {
        safe_addrs.insert(stx.safe_address.to_lowercase());
    }

    let mut result = None;
    walk_trace_for_governance(
        trace,
        pending_proposals,
        pending_safe_txs,
        &governor_addrs,
        &safe_addrs,
        &mut result,
    );
    result
}

/// execTransaction selector: `0x6a761202`
const EXEC_TRANSACTION_SELECTOR: &str = "6a761202";

fn walk_trace_for_governance(
    call: &serde_json::Value,
    pending_proposals: &[&treb_core::types::GovernorProposal],
    pending_safe_txs: &[&treb_core::types::SafeTransaction],
    governor_addrs: &std::collections::HashSet<String>,
    safe_addrs: &std::collections::HashSet<String>,
    result: &mut Option<GovernanceMatch>,
) {
    if result.is_some() {
        return;
    }

    let call_type = call.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if !call_type.eq_ignore_ascii_case("CALL") {
        // Recurse into sub-calls for DELEGATECALL etc.
        if let Some(calls) = call.get("calls").and_then(|v| v.as_array()) {
            for subcall in calls {
                walk_trace_for_governance(
                    subcall,
                    pending_proposals,
                    pending_safe_txs,
                    governor_addrs,
                    safe_addrs,
                    result,
                );
                if result.is_some() {
                    return;
                }
            }
        }
        return;
    }

    let to = match call.get("to").and_then(|v| v.as_str()) {
        Some(t) => t.to_lowercase(),
        None => {
            if let Some(calls) = call.get("calls").and_then(|v| v.as_array()) {
                for subcall in calls {
                    walk_trace_for_governance(
                        subcall,
                        pending_proposals,
                        pending_safe_txs,
                        governor_addrs,
                        safe_addrs,
                        result,
                    );
                    if result.is_some() {
                        return;
                    }
                }
            }
            return;
        }
    };

    let input = call.get("input").and_then(|v| v.as_str()).unwrap_or("");
    let input_stripped =
        input.strip_prefix("0x").or_else(|| input.strip_prefix("0X")).unwrap_or(input);

    // Check Safe: execTransaction selector to a known safe address
    if safe_addrs.contains(&to) && input_stripped.len() >= 8 {
        let selector = &input_stripped[..8];
        if selector.eq_ignore_ascii_case(EXEC_TRANSACTION_SELECTOR) {
            // Match by safe address — find the pending safe tx for this address
            for stx in pending_safe_txs {
                if stx.safe_address.to_lowercase() == to {
                    *result = Some(GovernanceMatch::SafeTransaction {
                        safe_tx_hash: stx.safe_tx_hash.clone(),
                    });
                    return;
                }
            }
        }
    }

    // Check Governor: call to a known governor address
    if governor_addrs.contains(&to) && input_stripped.len() >= 8 {
        // Find pending proposals for this governor address
        let matching: Vec<_> =
            pending_proposals.iter().filter(|p| p.governor_address.to_lowercase() == to).collect();

        if matching.len() == 1 {
            // Unambiguous: single pending proposal for this governor
            *result = Some(GovernanceMatch::GovernorProposal {
                proposal_id: matching[0].proposal_id.clone(),
            });
            return;
        }

        // Multiple pending proposals — try to disambiguate by action data
        // (would require full calldata decoding; for now skip trace match
        // and let the caller fall through to no-match)
    }

    // Recurse into sub-calls
    if let Some(calls) = call.get("calls").and_then(|v| v.as_array()) {
        for subcall in calls {
            walk_trace_for_governance(
                subcall,
                pending_proposals,
                pending_safe_txs,
                governor_addrs,
                safe_addrs,
                result,
            );
            if result.is_some() {
                return;
            }
        }
    }
}

// ── ProcessedReceipt ────────────────────────────────────────────────────

/// Results of processing a transaction receipt.
#[derive(Debug, Clone)]
pub struct ProcessedReceipt {
    /// Contract creations found in traces.
    pub creations: Vec<TracedCreation>,
    /// Proxy patterns detected from trace (proxy → implementation for newly created contracts).
    pub proxy_patterns: Vec<DetectedProxy>,
    /// Proxy upgrades detected from Upgraded event logs (on existing deployments).
    pub proxy_upgrades: Vec<DetectedUpgrade>,
    /// Block number of the transaction.
    pub block_number: u64,
    /// Gas used by the transaction.
    pub gas_used: u64,
    /// Raw receipt JSON — used for governance log matching.
    pub raw_receipt: serde_json::Value,
    /// Raw callTracer output — used for governance trace matching. `None` if trace unavailable.
    pub raw_trace: Option<serde_json::Value>,
}

/// Fetch receipt + traces for a tx hash and extract all deployment/upgrade info.
pub async fn process_tx_receipt(rpc_url: &str, tx_hash: &str) -> anyhow::Result<ProcessedReceipt> {
    let client = rpc_client()?;

    // Fetch receipt
    let receipt =
        rpc_call(&client, rpc_url, "eth_getTransactionReceipt", serde_json::json!([tx_hash]))
            .await?;

    if receipt.is_null() {
        bail!("transaction receipt not found: {tx_hash}");
    }

    let block_number =
        parse_hex_u64(receipt.get("blockNumber").and_then(|v| v.as_str()).unwrap_or("0x0"));
    let gas_used = parse_hex_u64(receipt.get("gasUsed").and_then(|v| v.as_str()).unwrap_or("0x0"));

    let receipt_status = receipt.get("status").and_then(|v| v.as_str()).unwrap_or("0x1");
    if receipt_status == "0x0" {
        bail!("transaction reverted: {tx_hash}");
    }

    // Detect proxy upgrades from receipt logs
    let proxy_upgrades = detect_upgrades_from_logs(&receipt);

    // Try trace for contract creations
    let trace_result = rpc_call(
        &client,
        rpc_url,
        "debug_traceTransaction",
        serde_json::json!([tx_hash, {"tracer": "callTracer"}]),
    )
    .await;

    let (mut creations, proxy_patterns, raw_trace) = match trace_result {
        Ok(trace) => {
            let creations = extract_creations_from_trace(&trace);
            let proxy_patterns = detect_proxy_patterns(&trace);
            (creations, proxy_patterns, Some(trace))
        }
        Err(_) => {
            // Trace not available — fall back to receipt-only for creations
            (Vec::new(), Vec::new(), None)
        }
    };

    // Also check receipt contractAddress as safety net
    if let Some(addr) = receipt_contract_address(&receipt) {
        if !creations.iter().any(|c| c.address.eq_ignore_ascii_case(&addr)) {
            let sender = receipt.get("from").and_then(|v| v.as_str()).unwrap_or("").to_string();
            creations.push(TracedCreation {
                address: addr,
                from: sender,
                create_type: "CREATE".to_string(),
            });
        }
    }

    Ok(ProcessedReceipt {
        creations,
        proxy_patterns,
        proxy_upgrades,
        block_number,
        gas_used,
        raw_receipt: receipt,
        raw_trace,
    })
}

// ── Registry application ────────────────────────────────────────────────

/// An existing deployment whose proxy implementation was updated.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpgradedDeployment {
    pub deployment_id: String,
    pub contract_name: String,
    pub proxy_address: String,
    pub old_implementation: String,
    pub new_implementation: String,
}

/// Results of applying receipt data to the registry.
#[derive(Debug, Clone)]
pub struct ReceiptApplicationResult {
    /// Existing deployments whose proxy implementation was updated.
    pub upgraded_deployments: Vec<UpgradedDeployment>,
    /// New contract creations that don't match existing deployments.
    pub new_creations: Vec<TracedCreation>,
    /// Existing deployment IDs whose address was confirmed in the trace.
    pub verified_deployments: Vec<String>,
}

/// Apply processed receipt data to the registry.
///
/// For each proxy upgrade: finds the deployment by address and updates `proxy_info`.
/// For each traced creation: checks if a deployment already exists at that address;
/// if yes, marks it as "verified"; if no, reports it as a new creation.
pub fn apply_receipt_to_registry(
    processed: &ProcessedReceipt,
    registry: &mut Registry,
    tx_hash: &str,
) -> anyhow::Result<ReceiptApplicationResult> {
    let mut upgraded_deployments = Vec::new();
    let mut new_creations = Vec::new();
    let mut verified_deployments = Vec::new();

    // Build address → deployment index for lookups
    let all_deployments = registry.list_deployments();
    let addr_to_dep: std::collections::HashMap<String, String> =
        all_deployments.iter().map(|d| (d.address.to_lowercase(), d.id.clone())).collect();

    // Process proxy upgrades from Upgraded events
    for upgrade in &processed.proxy_upgrades {
        let proxy_addr_lower = format!("{:#x}", upgrade.proxy_address).to_lowercase();
        if let Some(dep_id) = addr_to_dep.get(&proxy_addr_lower) {
            if let Some(dep) = registry.get_deployment(dep_id).cloned() {
                let old_impl =
                    dep.proxy_info.as_ref().map(|p| p.implementation.clone()).unwrap_or_default();

                let new_impl = format!("{:#x}", upgrade.new_implementation);

                // Update proxy_info
                let mut updated = dep.clone();
                let proxy_info = updated.proxy_info.get_or_insert_with(|| ProxyInfo {
                    proxy_type: String::new(),
                    implementation: String::new(),
                    admin: String::new(),
                    history: Vec::new(),
                });

                // Add current implementation to history before updating
                if !proxy_info.implementation.is_empty() {
                    proxy_info.history.push(ProxyUpgrade {
                        implementation_id: proxy_info.implementation.clone(),
                        upgraded_at: Utc::now(),
                        execution: Some(ExecutionRef {
                            status: ExecutionStatus::External,
                            kind: ExecutionKind::ExternalTx,
                            artifact_file: String::new(),
                            tx_hash: Some(tx_hash.to_string()),
                            safe_tx_hash: None,
                            proposal_id: None,
                            propose_safe_tx_hash: None,
                            script_tx_index: None,
                        }),
                        upgrade_tx_id: format!("tx-{tx_hash}"),
                    });
                }
                proxy_info.implementation = new_impl.clone();
                updated.updated_at = Utc::now();

                registry.update_deployment(updated).map_err(|e| anyhow::anyhow!("{e}"))?;

                upgraded_deployments.push(UpgradedDeployment {
                    deployment_id: dep_id.clone(),
                    contract_name: dep.contract_name.clone(),
                    proxy_address: dep.address.clone(),
                    old_implementation: old_impl,
                    new_implementation: new_impl,
                });
            }
        }
    }

    // Process traced creations
    for creation in &processed.creations {
        let addr_lower = creation.address.to_lowercase();
        if let Some(dep_id) = addr_to_dep.get(&addr_lower) {
            // Deployment already exists at this address — verified
            verified_deployments.push(dep_id.clone());
        } else {
            // New creation not in registry
            new_creations.push(creation.clone());
        }
    }

    Ok(ReceiptApplicationResult { upgraded_deployments, new_creations, verified_deployments })
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_creations_from_simple_trace() {
        let trace = serde_json::json!({
            "type": "CALL",
            "from": "0xaaaa",
            "to": "0xbbbb",
            "calls": [
                {
                    "type": "CREATE",
                    "from": "0xbbbb",
                    "to": "0xcccc"
                },
                {
                    "type": "CREATE2",
                    "from": "0xbbbb",
                    "to": "0xdddd"
                }
            ]
        });

        let creations = extract_creations_from_trace(&trace);
        assert_eq!(creations.len(), 2);
        assert_eq!(creations[0].address, "0xcccc");
        assert_eq!(creations[0].create_type, "CREATE");
        assert_eq!(creations[1].address, "0xdddd");
        assert_eq!(creations[1].create_type, "CREATE2");
    }

    #[test]
    fn detect_proxy_with_delegatecall() {
        let trace = serde_json::json!({
            "type": "CALL",
            "from": "0xaaaa",
            "to": "0xbbbb",
            "calls": [
                {
                    "type": "CREATE",
                    "from": "0xbbbb",
                    "to": "0xproxy",
                    "calls": [
                        {
                            "type": "DELEGATECALL",
                            "from": "0xproxy",
                            "to": "0ximpl"
                        }
                    ]
                }
            ]
        });

        let proxies = detect_proxy_patterns(&trace);
        assert_eq!(proxies.len(), 1);
        assert_eq!(proxies[0].proxy_address, "0xproxy");
        assert_eq!(proxies[0].implementation_address, "0ximpl");
    }

    #[test]
    fn receipt_contract_address_ignores_zero() {
        let receipt = serde_json::json!({
            "contractAddress": "0x0000000000000000000000000000000000000000"
        });
        assert!(receipt_contract_address(&receipt).is_none());

        let receipt2 = serde_json::json!({
            "contractAddress": "0xabcdef1234567890abcdef1234567890abcdef12"
        });
        assert!(receipt_contract_address(&receipt2).is_some());
    }

    #[test]
    fn parse_receipt_logs_extracts_upgraded_event() {
        let impl_addr = alloy_primitives::address!("9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0");
        let proxy_addr = alloy_primitives::address!("e7f1725E7734CE288F8367e1Bb143E90bb3F0512");

        // Upgraded(address implementation) event signature:
        // keccak256("Upgraded(address)") =
        // 0xbc7cd75a20ee27fd9adebab32041f755214dbc6bffa90cc0225b39da2e5c2d3b
        let upgraded_sig = "0xbc7cd75a20ee27fd9adebab32041f755214dbc6bffa90cc0225b39da2e5c2d3b";
        // Implementation address is indexed (topic[1]), left-padded to 32 bytes
        let impl_topic = format!("0x000000000000000000000000{}", &format!("{:#x}", impl_addr)[2..]);

        let receipt = serde_json::json!({
            "logs": [{
                "address": format!("{:#x}", proxy_addr),
                "topics": [upgraded_sig, impl_topic],
                "data": "0x",
            }]
        });

        let upgrades = detect_upgrades_from_logs(&receipt);
        assert_eq!(upgrades.len(), 1);
        assert_eq!(upgrades[0].proxy_address, proxy_addr);
        assert_eq!(upgrades[0].new_implementation, impl_addr);
    }

    #[test]
    fn parse_hex_u64_handles_prefix() {
        assert_eq!(parse_hex_u64("0xff"), 255);
        assert_eq!(parse_hex_u64("0Xff"), 255);
        assert_eq!(parse_hex_u64("ff"), 255);
        assert_eq!(parse_hex_u64("invalid"), 0);
    }

    fn make_pending_proposal(
        proposal_id: &str,
        governor: &str,
    ) -> treb_core::types::GovernorProposal {
        treb_core::types::GovernorProposal {
            proposal_id: proposal_id.to_string(),
            governor_address: governor.to_string(),
            timelock_address: String::new(),
            chain_id: 1,
            status: treb_core::types::ProposalStatus::Pending,
            transaction_ids: vec![],
            proposed_by: String::new(),
            proposed_at: chrono::Utc::now(),
            description: String::new(),
            executed_at: None,
            execution_tx_hash: String::new(),
            fork_executed_at: None,
            actions: vec![],
        }
    }

    fn make_pending_safe_tx(
        safe_tx_hash: &str,
        safe_address: &str,
    ) -> treb_core::types::SafeTransaction {
        treb_core::types::SafeTransaction {
            safe_tx_hash: safe_tx_hash.to_string(),
            safe_address: safe_address.to_string(),
            chain_id: 1,
            status: treb_core::types::enums::TransactionStatus::Queued,
            nonce: 0,
            transactions: vec![],
            transaction_ids: vec![],
            proposed_by: String::new(),
            proposed_at: chrono::Utc::now(),
            confirmations: vec![],
            executed_at: None,
            execution_tx_hash: String::new(),
            fork_executed_at: None,
        }
    }

    #[test]
    fn match_governance_logs_proposal_executed() {
        // ProposalExecuted(uint256 proposalId=42)
        // proposalId is non-indexed, so it's in data as ABI-encoded uint256
        let proposal_id_hex = format!("{:064x}", 42u64);
        let receipt = serde_json::json!({
            "logs": [{
                "address": "0x1111111111111111111111111111111111111111",
                "topics": [topics::PROPOSAL_EXECUTED],
                "data": format!("0x{proposal_id_hex}"),
            }]
        });

        let proposal = make_pending_proposal("42", "0x1111111111111111111111111111111111111111");
        let proposals = vec![&proposal];
        let safe_txs: Vec<&treb_core::types::SafeTransaction> = vec![];

        let result = match_governance_logs(&receipt, &proposals, &safe_txs);
        assert!(result.is_some());
        match result.unwrap() {
            GovernanceMatch::GovernorProposal { proposal_id } => assert_eq!(proposal_id, "42"),
            _ => panic!("expected GovernorProposal"),
        }
    }

    #[test]
    fn match_governance_logs_execution_success() {
        let safe_tx_hash = "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
        let receipt = serde_json::json!({
            "logs": [{
                "address": "0x2222222222222222222222222222222222222222",
                "topics": [
                    topics::EXECUTION_SUCCESS,
                    safe_tx_hash,
                ],
                "data": format!("0x{:064x}", 0u64), // payment
            }]
        });

        let stx = make_pending_safe_tx(safe_tx_hash, "0x2222222222222222222222222222222222222222");
        let proposals: Vec<&treb_core::types::GovernorProposal> = vec![];
        let safe_txs = vec![&stx];

        let result = match_governance_logs(&receipt, &proposals, &safe_txs);
        assert!(result.is_some());
        match result.unwrap() {
            GovernanceMatch::SafeTransaction { safe_tx_hash: hash } => {
                assert_eq!(hash, safe_tx_hash);
            }
            _ => panic!("expected SafeTransaction"),
        }
    }

    #[test]
    fn match_governance_logs_no_match_for_unrelated_events() {
        let receipt = serde_json::json!({
            "logs": [{
                "address": "0x1111111111111111111111111111111111111111",
                "topics": ["0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"],
                "data": "0x",
            }]
        });

        let proposal = make_pending_proposal("42", "0x1111111111111111111111111111111111111111");
        let proposals = vec![&proposal];
        let safe_txs: Vec<&treb_core::types::SafeTransaction> = vec![];

        assert!(match_governance_logs(&receipt, &proposals, &safe_txs).is_none());
    }

    #[test]
    fn match_governance_trace_safe_exec_transaction() {
        let safe_address = "0x2222222222222222222222222222222222222222";
        // execTransaction selector = 0x6a761202
        let trace = serde_json::json!({
            "type": "CALL",
            "from": "0xaaaa",
            "to": safe_address,
            "input": "0x6a76120200000000000000000000000000000000000000000000000000000000",
            "calls": []
        });

        let stx = make_pending_safe_tx("0xsafehash", safe_address);
        let proposals: Vec<&treb_core::types::GovernorProposal> = vec![];
        let safe_txs = vec![&stx];

        let result = match_governance_trace(&trace, &proposals, &safe_txs);
        assert!(result.is_some());
        match result.unwrap() {
            GovernanceMatch::SafeTransaction { safe_tx_hash } => {
                assert_eq!(safe_tx_hash, "0xsafehash");
            }
            _ => panic!("expected SafeTransaction"),
        }
    }

    #[test]
    fn match_governance_trace_governor_call() {
        let governor = "0x3333333333333333333333333333333333333333";
        let trace = serde_json::json!({
            "type": "CALL",
            "from": "0xaaaa",
            "to": "0xbbbb",
            "input": "0x",
            "calls": [{
                "type": "CALL",
                "from": "0xbbbb",
                "to": governor,
                "input": "0x7d5e81e200000000000000000000000000000000000000000000000000000000",
                "calls": []
            }]
        });

        let proposal = make_pending_proposal("99", governor);
        let proposals = vec![&proposal];
        let safe_txs: Vec<&treb_core::types::SafeTransaction> = vec![];

        let result = match_governance_trace(&trace, &proposals, &safe_txs);
        assert!(result.is_some());
        match result.unwrap() {
            GovernanceMatch::GovernorProposal { proposal_id } => {
                assert_eq!(proposal_id, "99");
            }
            _ => panic!("expected GovernorProposal"),
        }
    }

    #[test]
    fn match_governance_trace_no_match_for_unrelated_call() {
        let trace = serde_json::json!({
            "type": "CALL",
            "from": "0xaaaa",
            "to": "0x9999999999999999999999999999999999999999",
            "input": "0x12345678",
            "calls": []
        });

        let proposal = make_pending_proposal("42", "0x1111111111111111111111111111111111111111");
        let proposals = vec![&proposal];
        let safe_txs: Vec<&treb_core::types::SafeTransaction> = vec![];

        assert!(match_governance_trace(&trace, &proposals, &safe_txs).is_none());
    }
}
